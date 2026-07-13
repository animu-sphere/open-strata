// SPDX-License-Identifier: Apache-2.0
#pragma once

#include <cstdint>

namespace {{Name}} {

struct FrameSnapshot {
  std::uint64_t revision = 0;
  std::uint32_t triangle_count = 0;
};

class RenderWorld {
 public:
  void SetBootstrapTriangle();
  [[nodiscard]] FrameSnapshot Commit();

 private:
  std::uint64_t revision_ = 0;
  std::uint32_t triangle_count_ = 0;
  bool dirty_ = false;
};

}  // namespace {{Name}}
