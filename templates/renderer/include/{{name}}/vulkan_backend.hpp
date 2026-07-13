// SPDX-License-Identifier: Apache-2.0
#pragma once

#include <string>

namespace {{Name}} {

struct BackendCapability {
  bool available = false;
  std::string detail;
};

// This is a capability probe, not a renderer implementation. Generated source
// never reports a GPU frame until the project implements and validates one.
[[nodiscard]] BackendCapability ProbeVulkanBackend();

}  // namespace {{Name}}
