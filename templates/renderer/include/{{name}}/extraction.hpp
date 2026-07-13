// SPDX-License-Identifier: Apache-2.0
#pragma once

#include <cstdint>

#include <{{name}}/render_world.hpp>

namespace {{Name}} {

struct DrawSummary {
  std::uint64_t source_revision = 0;
  std::uint32_t draw_count = 0;
  std::uint32_t triangle_count = 0;
};

[[nodiscard]] DrawSummary ExtractDrawSummary(const FrameSnapshot& snapshot);

}  // namespace {{Name}}
