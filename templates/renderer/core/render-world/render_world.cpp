// SPDX-License-Identifier: Apache-2.0
#include <{{name}}/render_world.hpp>

namespace {{Name}} {

void RenderWorld::SetTriangleCount(std::uint32_t triangle_count) {
  if (triangle_count_ != triangle_count) {
    triangle_count_ = triangle_count;
    dirty_ = true;
  }
}

void RenderWorld::MarkChanged() { dirty_ = true; }

void RenderWorld::SetBootstrapTriangle() {
  SetTriangleCount(1);
}

FrameSnapshot RenderWorld::Commit() {
  if (dirty_) {
    ++revision_;
    dirty_ = false;
  }
  return FrameSnapshot{revision_, triangle_count_};
}

}  // namespace {{Name}}
