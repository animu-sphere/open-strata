// SPDX-License-Identifier: Apache-2.0
#pragma once

#include <pxr/pxr.h>

#include <pxr/imaging/hd/renderBuffer.h>
#include <pxr/imaging/hd/renderDelegate.h>

#include <cstddef>
#include <cstdint>
#include <memory>
#include <mutex>
#include <vector>

PXR_NAMESPACE_OPEN_SCOPE

// CPU-readable AOV storage is the portable Hydra presentation baseline. A
// generated renderer may replace the copy with explicit Hgi/Vulkan interop,
// but that is an optional project-owned capability rather than this seam.
class Hd{{Name}}RenderBuffer final : public HdRenderBuffer {
 public:
  explicit Hd{{Name}}RenderBuffer(const SdfPath& id);

  bool Allocate(const GfVec3i& dimensions, HdFormat format,
                bool multi_sampled) override;
  unsigned int GetWidth() const override;
  unsigned int GetHeight() const override;
  unsigned int GetDepth() const override;
  HdFormat GetFormat() const override;
  bool IsMultiSampled() const override;
  void* Map() override;
  void Unmap() override;
  bool IsMapped() const override;
  void Resolve() override;
  bool IsConverged() const override;

  bool WriteColor(const std::vector<std::uint8_t>& rgba8,
                  std::uint32_t source_width, std::uint32_t source_height);
  bool WriteDepth(const std::vector<float>& depth,
                  std::uint32_t source_width, std::uint32_t source_height);
  bool WriteIds(std::int32_t value);
  void SetConverged(bool converged);

 protected:
  void _Deallocate() override;

 private:
  mutable std::mutex mutex_;
  GfVec3i dimensions_{0};
  HdFormat format_{HdFormatInvalid};
  bool multi_sampled_{};
  bool converged_{};
  std::size_t map_count_{};
  std::vector<std::uint8_t> data_;
};

class Hd{{Name}}RenderDelegate final : public HdRenderDelegate {
 public:
  explicit Hd{{Name}}RenderDelegate(const HdRenderSettingsMap& settings = {});
  ~Hd{{Name}}RenderDelegate() override;

  const TfTokenVector& GetSupportedRprimTypes() const override;
  const TfTokenVector& GetSupportedSprimTypes() const override;
  const TfTokenVector& GetSupportedBprimTypes() const override;
  HdResourceRegistrySharedPtr GetResourceRegistry() const override;
  HdRenderPassSharedPtr CreateRenderPass(
      HdRenderIndex* index, const HdRprimCollection& collection) override;
  HdInstancer* CreateInstancer(HdSceneDelegate* delegate,
                               const SdfPath& id) override;
  void DestroyInstancer(HdInstancer* instancer) override;
  HdRprim* CreateRprim(const TfToken& type_id,
                       const SdfPath& rprim_id) override;
  void DestroyRprim(HdRprim* rprim) override;
  HdSprim* CreateSprim(const TfToken& type_id,
                       const SdfPath& sprim_id) override;
  HdSprim* CreateFallbackSprim(const TfToken& type_id) override;
  void DestroySprim(HdSprim* sprim) override;
  HdBprim* CreateBprim(const TfToken& type_id,
                       const SdfPath& bprim_id) override;
  HdBprim* CreateFallbackBprim(const TfToken& type_id) override;
  void DestroyBprim(HdBprim* bprim) override;
  void CommitResources(HdChangeTracker* tracker) override;
  HdAovDescriptor GetDefaultAovDescriptor(const TfToken& name) const override;

 private:
  class Impl;
  std::unique_ptr<Impl> impl_;
  HdResourceRegistrySharedPtr resources_;
};

PXR_NAMESPACE_CLOSE_SCOPE
