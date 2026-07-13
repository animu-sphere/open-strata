// SPDX-License-Identifier: Apache-2.0
#include <{{name}}/extraction.hpp>

namespace {{Name}} {

DrawSummary ExtractDrawSummary(const FrameSnapshot& snapshot) {
  return DrawSummary{
      snapshot.revision,
      snapshot.triangle_count == 0 ? 0U : 1U,
      snapshot.triangle_count,
  };
}

}  // namespace {{Name}}
