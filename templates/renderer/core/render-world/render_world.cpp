// SPDX-License-Identifier: Apache-2.0
#include <{{name}}/render_world.hpp>

namespace {{Name}} {

void RenderWorld::SetBootstrapTriangle() {
  if (triangle_count_ != 1) {
    triangle_count_ = 1;
    dirty_ = true;
  }
}

FrameSnapshot RenderWorld::Commit() {
  if (dirty_) {
    ++revision_;
    dirty_ = false;
  }
  return FrameSnapshot{revision_, triangle_count_};
}

}  // namespace {{Name}}
