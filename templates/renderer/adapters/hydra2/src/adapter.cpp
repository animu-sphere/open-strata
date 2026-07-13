// SPDX-License-Identifier: Apache-2.0
#include "adapter.hpp"

#include <pxr/pxr.h>

#include <pxr/base/tf/diagnostic.h>
#include <pxr/imaging/hd/aov.h>
#include <pxr/imaging/hd/camera.h>
#include <pxr/imaging/hd/changeTracker.h>
#include <pxr/imaging/hd/instancer.h>
#include <pxr/imaging/hd/mesh.h>
#include <pxr/imaging/hd/renderIndex.h>
#include <pxr/imaging/hd/renderPass.h>
#include <pxr/imaging/hd/renderPassState.h>
#include <pxr/imaging/hd/resourceRegistry.h>
#include <pxr/imaging/hd/tokens.h>

#include <{{name}}/extraction.hpp>
#include <{{name}}/render_world.hpp>
#include <{{name}}/vulkan_backend.hpp>

#ifdef _WIN32
#include <Windows.h>
#else
#include <dlfcn.h>
#endif

#include <algorithm>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <limits>
#include <memory>
#include <mutex>
#include <stdexcept>
#include <string>
#include <unordered_map>
#include <utility>

PXR_NAMESPACE_OPEN_SCOPE

namespace {

std::filesystem::path PluginDirectory() {
#ifdef _WIN32
  static int module_anchor;
  HMODULE module{};
  const auto address = reinterpret_cast<LPCWSTR>(&module_anchor);
  if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS |
                              GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                          address, &module)) {
    throw std::runtime_error("could not locate the hd{{Name}} module");
  }
  std::wstring path(32768, L'\0');
  const DWORD length =
      GetModuleFileNameW(module, path.data(), static_cast<DWORD>(path.size()));
  if (length == 0 || length >= path.size()) {
    throw std::runtime_error("could not resolve the hd{{Name}} module path");
  }
  path.resize(length);
  return std::filesystem::path(path).parent_path();
#else
  static int module_anchor;
  Dl_info info{};
  if (dladdr(&module_anchor, &info) == 0 || info.dli_fname == nullptr) {
    throw std::runtime_error("could not locate the hd{{Name}} module");
  }
  return std::filesystem::path(info.dli_fname).parent_path();
#endif
}

void AppendHostEvidence(std::uint64_t frame_index,
                        const {{Name}}::GpuFrameEvidence& frame,
                        std::uint32_t width, std::uint32_t height,
                        std::size_t buffers_written,
                        std::uint64_t scene_revision) {
  const char* path = std::getenv("{{NAME}}_HYDRA_EVIDENCE");
  if (path == nullptr || *path == '\0') {
    return;
  }
  std::ofstream output(path, std::ios::binary | std::ios::app);
  if (!output) {
    TF_WARN("Could not append {{Name}} Hydra evidence to %s", path);
    return;
  }
  output << "frame=" << frame_index
         << " completion=" << frame.completion
         << " scene_revision=" << scene_revision
         << " width=" << width
         << " height=" << height
         << " buffers_written=" << buffers_written << '\n';
}

class AdapterState {
 public:
  void SyncMesh(const SdfPath& id, bool renderable) {
    std::scoped_lock lock(mutex_);
    meshes_[id.GetString()] = renderable;
    world_.MarkChanged();
    UpdateWorldLocked();
  }

  void RemoveMesh(const SdfPath& id) {
    std::scoped_lock lock(mutex_);
    meshes_.erase(id.GetString());
    UpdateWorldLocked();
  }

  void Render(const HdRenderPassAovBindingVector& bindings) {
    std::scoped_lock lock(mutex_);
    const {{Name}}::FrameSnapshot snapshot = world_.Commit();
    const {{Name}}::DrawSummary draw = {{Name}}::ExtractDrawSummary(snapshot);
    if (draw.triangle_count == 0) {
      for (const HdRenderPassAovBinding& binding : bindings) {
        if (auto* buffer =
                dynamic_cast<Hd{{Name}}RenderBuffer*>(binding.renderBuffer)) {
          buffer->SetConverged(false);
        }
      }
      return;
    }

    const std::filesystem::path shaders = PluginDirectory() / "shaders";
    const {{Name}}::GpuFrameEvidence frame = {{Name}}::RenderOffscreen(
        draw, (shaders / "triangle.vert.spv").string(),
        (shaders / "triangle.frag.spv").string(), 1);
    if (frame.status != {{Name}}::FrameStatus::Pass) {
      TF_RUNTIME_ERROR("{{Name}} Hydra frame failed: %s", frame.detail.c_str());
      return;
    }

    std::size_t buffers_written{};
    std::uint32_t width{};
    std::uint32_t height{};
    for (const HdRenderPassAovBinding& binding : bindings) {
      auto* buffer =
          dynamic_cast<Hd{{Name}}RenderBuffer*>(binding.renderBuffer);
      if (buffer == nullptr) {
        continue;
      }
      width = std::max(width, buffer->GetWidth());
      height = std::max(height, buffer->GetHeight());
      bool wrote = false;
      if (binding.aovName == HdAovTokens->color) {
        wrote = buffer->WriteColor(frame.color.payload, frame.color.width,
                                   frame.color.height);
      } else if (binding.aovName == HdAovTokens->depth) {
        wrote = buffer->WriteDepth(frame.depth.payload, frame.depth.width,
                                   frame.depth.height);
      } else if (binding.aovName == HdAovTokens->primId ||
                 binding.aovName == HdAovTokens->instanceId ||
                 binding.aovName == HdAovTokens->elementId) {
        wrote = buffer->WriteIds(-1);
      }
      buffer->SetConverged(wrote);
      if (wrote) {
        ++buffers_written;
      }
    }
    ++frame_index_;
    AppendHostEvidence(frame_index_, frame, width, height, buffers_written,
                       snapshot.revision);
  }

 private:
  void UpdateWorldLocked() {
    const bool any_renderable =
        std::any_of(meshes_.begin(), meshes_.end(),
                    [](const auto& entry) { return entry.second; });
    world_.SetTriangleCount(any_renderable ? 1U : 0U);
  }

  std::mutex mutex_;
  std::unordered_map<std::string, bool> meshes_;
  {{Name}}::RenderWorld world_;
  std::uint64_t frame_index_{};
};

class Hd{{Name}}Mesh final : public HdMesh {
 public:
  Hd{{Name}}Mesh(const SdfPath& id, std::shared_ptr<AdapterState> state)
      : HdMesh(id), state_(std::move(state)) {}

  ~Hd{{Name}}Mesh() override { state_->RemoveMesh(GetId()); }

  HdDirtyBits GetInitialDirtyBitsMask() const override {
    return HdChangeTracker::DirtyPoints | HdChangeTracker::DirtyTopology |
           HdChangeTracker::DirtyTransform | HdChangeTracker::DirtyVisibility |
           HdChangeTracker::DirtyRenderTag;
  }

  void Sync(HdSceneDelegate* delegate, HdRenderParam* render_param,
            HdDirtyBits* dirty_bits, const TfToken& repr_token) override {
    (void)render_param;
    (void)repr_token;
    const bool visible = delegate->GetVisible(GetId());
    const HdMeshTopology topology = GetMeshTopology(delegate);
    const bool has_face =
        std::any_of(topology.GetFaceVertexCounts().begin(),
                    topology.GetFaceVertexCounts().end(),
                    [](int count) { return count >= 3; });
    const bool has_points = !GetPoints(delegate).IsEmpty();
    state_->SyncMesh(GetId(), visible && has_face && has_points);
    *dirty_bits = HdChangeTracker::Clean;
  }

 protected:
  HdDirtyBits _PropagateDirtyBits(HdDirtyBits bits) const override {
    return bits;
  }

  void _InitRepr(const TfToken& repr_token, HdDirtyBits* dirty_bits) override {
    (void)repr_token;
    *dirty_bits |= GetInitialDirtyBitsMask();
  }

 private:
  std::shared_ptr<AdapterState> state_;
};

class Hd{{Name}}Camera final : public HdCamera {
 public:
  explicit Hd{{Name}}Camera(const SdfPath& id) : HdCamera(id) {}
};

class Hd{{Name}}RenderPass final : public HdRenderPass {
 public:
  Hd{{Name}}RenderPass(HdRenderIndex* index,
                       const HdRprimCollection& collection,
                       std::shared_ptr<AdapterState> state)
      : HdRenderPass(index, collection), state_(std::move(state)) {}

 private:
  void _Execute(const HdRenderPassStateSharedPtr& render_pass_state,
                const TfTokenVector& render_tags) override {
    (void)render_tags;
    state_->Render(render_pass_state->GetAovBindings());
  }

  std::shared_ptr<AdapterState> state_;
};

}  // namespace

Hd{{Name}}RenderBuffer::Hd{{Name}}RenderBuffer(const SdfPath& id)
    : HdRenderBuffer(id) {}

bool Hd{{Name}}RenderBuffer::Allocate(const GfVec3i& dimensions,
                                      HdFormat format,
                                      bool multi_sampled) {
  std::scoped_lock lock(mutex_);
  if (map_count_ != 0 || dimensions[0] < 0 || dimensions[1] < 0 ||
      dimensions[2] < 0 || multi_sampled || format == HdFormatInvalid) {
    return false;
  }
  const std::size_t pixel_size = HdDataSizeOfFormat(format);
  const std::size_t width = static_cast<std::size_t>(dimensions[0]);
  const std::size_t height = static_cast<std::size_t>(dimensions[1]);
  const std::size_t depth = static_cast<std::size_t>(dimensions[2]);
  if (pixel_size == 0 ||
      (width != 0 && height > std::numeric_limits<std::size_t>::max() / width) ||
      (width * height != 0 &&
       depth > std::numeric_limits<std::size_t>::max() / (width * height)) ||
      (width * height * depth != 0 &&
       pixel_size > std::numeric_limits<std::size_t>::max() /
                        (width * height * depth))) {
    return false;
  }
  dimensions_ = dimensions;
  format_ = format;
  multi_sampled_ = multi_sampled;
  converged_ = false;
  data_.assign(width * height * depth * pixel_size, 0);
  return true;
}

unsigned int Hd{{Name}}RenderBuffer::GetWidth() const {
  std::scoped_lock lock(mutex_);
  return static_cast<unsigned int>(dimensions_[0]);
}

unsigned int Hd{{Name}}RenderBuffer::GetHeight() const {
  std::scoped_lock lock(mutex_);
  return static_cast<unsigned int>(dimensions_[1]);
}

unsigned int Hd{{Name}}RenderBuffer::GetDepth() const {
  std::scoped_lock lock(mutex_);
  return static_cast<unsigned int>(dimensions_[2]);
}

HdFormat Hd{{Name}}RenderBuffer::GetFormat() const {
  std::scoped_lock lock(mutex_);
  return format_;
}

bool Hd{{Name}}RenderBuffer::IsMultiSampled() const {
  std::scoped_lock lock(mutex_);
  return multi_sampled_;
}

void* Hd{{Name}}RenderBuffer::Map() {
  std::scoped_lock lock(mutex_);
  if (data_.empty()) {
    return nullptr;
  }
  ++map_count_;
  return data_.data();
}

void Hd{{Name}}RenderBuffer::Unmap() {
  std::scoped_lock lock(mutex_);
  if (map_count_ != 0) {
    --map_count_;
  }
}

bool Hd{{Name}}RenderBuffer::IsMapped() const {
  std::scoped_lock lock(mutex_);
  return map_count_ != 0;
}

void Hd{{Name}}RenderBuffer::Resolve() {}

bool Hd{{Name}}RenderBuffer::IsConverged() const {
  std::scoped_lock lock(mutex_);
  return converged_;
}

bool Hd{{Name}}RenderBuffer::WriteColor(
    const std::vector<std::uint8_t>& rgba8, std::uint32_t source_width,
    std::uint32_t source_height) {
  std::scoped_lock lock(mutex_);
  const std::uint32_t width = static_cast<std::uint32_t>(dimensions_[0]);
  const std::uint32_t height = static_cast<std::uint32_t>(dimensions_[1]);
  if (map_count_ != 0 || format_ != HdFormatUNorm8Vec4 ||
      dimensions_[2] != 1 || source_width == 0 || source_height == 0 ||
      rgba8.size() != static_cast<std::size_t>(source_width) * source_height * 4U ||
      data_.size() != static_cast<std::size_t>(width) * height * 4U) {
    return false;
  }
  for (std::uint32_t y = 0; y < height; ++y) {
    const std::uint32_t source_y = y * source_height / height;
    for (std::uint32_t x = 0; x < width; ++x) {
      const std::uint32_t source_x = x * source_width / width;
      const std::size_t source =
          (static_cast<std::size_t>(source_y) * source_width + source_x) * 4U;
      const std::size_t target =
          (static_cast<std::size_t>(y) * width + x) * 4U;
      std::copy_n(rgba8.data() + source, 4, data_.data() + target);
    }
  }
  return true;
}

bool Hd{{Name}}RenderBuffer::WriteDepth(const std::vector<float>& depth,
                                        std::uint32_t source_width,
                                        std::uint32_t source_height) {
  std::scoped_lock lock(mutex_);
  const std::uint32_t width = static_cast<std::uint32_t>(dimensions_[0]);
  const std::uint32_t height = static_cast<std::uint32_t>(dimensions_[1]);
  if (map_count_ != 0 || format_ != HdFormatFloat32 || dimensions_[2] != 1 ||
      source_width == 0 || source_height == 0 ||
      depth.size() != static_cast<std::size_t>(source_width) * source_height ||
      data_.size() != static_cast<std::size_t>(width) * height * sizeof(float)) {
    return false;
  }
  for (std::uint32_t y = 0; y < height; ++y) {
    const std::uint32_t source_y = y * source_height / height;
    for (std::uint32_t x = 0; x < width; ++x) {
      const std::uint32_t source_x = x * source_width / width;
      const float value = depth[static_cast<std::size_t>(source_y) *
                                    source_width + source_x];
      const std::size_t target =
          (static_cast<std::size_t>(y) * width + x) * sizeof(float);
      std::memcpy(data_.data() + target, &value, sizeof(value));
    }
  }
  return true;
}

bool Hd{{Name}}RenderBuffer::WriteIds(std::int32_t value) {
  std::scoped_lock lock(mutex_);
  if (map_count_ != 0 || format_ != HdFormatInt32 || dimensions_[2] != 1 ||
      data_.size() % sizeof(value) != 0) {
    return false;
  }
  for (std::size_t offset = 0; offset < data_.size(); offset += sizeof(value)) {
    std::memcpy(data_.data() + offset, &value, sizeof(value));
  }
  return true;
}

void Hd{{Name}}RenderBuffer::SetConverged(bool converged) {
  std::scoped_lock lock(mutex_);
  converged_ = converged;
}

void Hd{{Name}}RenderBuffer::_Deallocate() {
  std::scoped_lock lock(mutex_);
  if (map_count_ != 0) {
    return;
  }
  dimensions_ = GfVec3i(0);
  format_ = HdFormatInvalid;
  multi_sampled_ = false;
  converged_ = false;
  data_.clear();
}

class Hd{{Name}}RenderDelegate::Impl {
 public:
  std::shared_ptr<AdapterState> state = std::make_shared<AdapterState>();
};

Hd{{Name}}RenderDelegate::Hd{{Name}}RenderDelegate(
    const HdRenderSettingsMap& settings)
    : HdRenderDelegate(settings),
      impl_(std::make_unique<Impl>()),
      resources_(std::make_shared<HdResourceRegistry>()) {}

Hd{{Name}}RenderDelegate::~Hd{{Name}}RenderDelegate() = default;

const TfTokenVector& Hd{{Name}}RenderDelegate::GetSupportedRprimTypes() const {
  static const TfTokenVector types{HdPrimTypeTokens->mesh};
  return types;
}

const TfTokenVector& Hd{{Name}}RenderDelegate::GetSupportedSprimTypes() const {
  static const TfTokenVector types{HdPrimTypeTokens->camera};
  return types;
}

const TfTokenVector& Hd{{Name}}RenderDelegate::GetSupportedBprimTypes() const {
  static const TfTokenVector types{HdPrimTypeTokens->renderBuffer};
  return types;
}

HdResourceRegistrySharedPtr Hd{{Name}}RenderDelegate::GetResourceRegistry() const {
  return resources_;
}

HdRenderPassSharedPtr Hd{{Name}}RenderDelegate::CreateRenderPass(
    HdRenderIndex* index, const HdRprimCollection& collection) {
  return std::make_shared<Hd{{Name}}RenderPass>(index, collection,
                                                impl_->state);
}

HdInstancer* Hd{{Name}}RenderDelegate::CreateInstancer(
    HdSceneDelegate* delegate, const SdfPath& id) {
  (void)delegate;
  (void)id;
  return nullptr;
}

void Hd{{Name}}RenderDelegate::DestroyInstancer(HdInstancer* instancer) {
  delete instancer;
}

HdRprim* Hd{{Name}}RenderDelegate::CreateRprim(const TfToken& type_id,
                                                const SdfPath& rprim_id) {
  if (type_id == HdPrimTypeTokens->mesh) {
    return new Hd{{Name}}Mesh(rprim_id, impl_->state);
  }
  return nullptr;
}

void Hd{{Name}}RenderDelegate::DestroyRprim(HdRprim* rprim) { delete rprim; }

HdSprim* Hd{{Name}}RenderDelegate::CreateSprim(const TfToken& type_id,
                                                const SdfPath& sprim_id) {
  if (type_id == HdPrimTypeTokens->camera) {
    return new Hd{{Name}}Camera(sprim_id);
  }
  return nullptr;
}

HdSprim* Hd{{Name}}RenderDelegate::CreateFallbackSprim(
    const TfToken& type_id) {
  if (type_id == HdPrimTypeTokens->camera) {
    return new Hd{{Name}}Camera(SdfPath("/__{{name}}FallbackCamera"));
  }
  return nullptr;
}

void Hd{{Name}}RenderDelegate::DestroySprim(HdSprim* sprim) { delete sprim; }

HdBprim* Hd{{Name}}RenderDelegate::CreateBprim(const TfToken& type_id,
                                                const SdfPath& bprim_id) {
  if (type_id == HdPrimTypeTokens->renderBuffer) {
    return new Hd{{Name}}RenderBuffer(bprim_id);
  }
  return nullptr;
}

HdBprim* Hd{{Name}}RenderDelegate::CreateFallbackBprim(
    const TfToken& type_id) {
  if (type_id == HdPrimTypeTokens->renderBuffer) {
    return new Hd{{Name}}RenderBuffer(
        SdfPath("/__{{name}}FallbackRenderBuffer"));
  }
  return nullptr;
}

void Hd{{Name}}RenderDelegate::DestroyBprim(HdBprim* bprim) { delete bprim; }

void Hd{{Name}}RenderDelegate::CommitResources(HdChangeTracker* tracker) {
  (void)tracker;
}

HdAovDescriptor Hd{{Name}}RenderDelegate::GetDefaultAovDescriptor(
    const TfToken& name) const {
  if (name == HdAovTokens->color) {
    return {HdFormatUNorm8Vec4, false, VtValue(GfVec4f(0.0F))};
  }
  if (name == HdAovTokens->depth) {
    return {HdFormatFloat32, false, VtValue(1.0F)};
  }
  if (name == HdAovTokens->primId || name == HdAovTokens->instanceId ||
      name == HdAovTokens->elementId) {
    return {HdFormatInt32, false, VtValue(-1)};
  }
  return {};
}

PXR_NAMESPACE_CLOSE_SCOPE
