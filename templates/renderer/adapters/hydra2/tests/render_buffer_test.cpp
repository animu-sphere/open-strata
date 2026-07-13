// SPDX-License-Identifier: Apache-2.0
#include "adapter.hpp"

#include <pxr/pxr.h>

#include <cstdint>
#include <iostream>
#include <vector>

PXR_NAMESPACE_USING_DIRECTIVE

namespace {

bool Check(bool condition, const char* message) {
  if (!condition) {
    std::cerr << message << '\n';
  }
  return condition;
}

}  // namespace

int main() {
  Hd{{Name}}RenderBuffer color(SdfPath("/color"));
  if (!Check(color.Allocate(GfVec3i(4, 4, 1), HdFormatUNorm8Vec4, false),
             "color allocation failed")) {
    return 1;
  }
  const std::vector<std::uint8_t> source{
      255, 0, 0, 255, 0, 255, 0, 255,
      0, 0, 255, 255, 255, 255, 255, 255};
  if (!Check(color.WriteColor(source, 2, 2), "color write failed")) {
    return 1;
  }
  color.SetConverged(true);
  const auto* pixels = static_cast<const std::uint8_t*>(color.Map());
  if (!Check(pixels != nullptr && pixels[0] == 255 && pixels[3] == 255,
             "scaled color payload is incorrect") ||
      !Check(color.IsMapped(), "color map state is incorrect") ||
      !Check(color.IsConverged(), "color did not converge")) {
    return 1;
  }
  color.Unmap();

  Hd{{Name}}RenderBuffer depth(SdfPath("/depth"));
  if (!Check(depth.Allocate(GfVec3i(2, 2, 1), HdFormatFloat32, false),
             "depth allocation failed") ||
      !Check(depth.WriteDepth({0.25F}, 1, 1), "depth write failed")) {
    return 1;
  }
  const auto* depths = static_cast<const float*>(depth.Map());
  if (!Check(depths != nullptr && depths[0] == 0.25F &&
                 depths[3] == 0.25F,
             "scaled depth payload is incorrect")) {
    return 1;
  }
  depth.Unmap();

  Hd{{Name}}RenderBuffer ids(SdfPath("/primId"));
  if (!Check(ids.Allocate(GfVec3i(2, 1, 1), HdFormatInt32, false),
             "id allocation failed") ||
      !Check(ids.WriteIds(-1), "id write failed")) {
    return 1;
  }
  const auto* id_values = static_cast<const std::int32_t*>(ids.Map());
  if (!Check(id_values != nullptr && id_values[0] == -1 && id_values[1] == -1,
             "id payload is incorrect")) {
    return 1;
  }
  ids.Unmap();
  return 0;
}
