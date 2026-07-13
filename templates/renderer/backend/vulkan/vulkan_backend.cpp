// SPDX-License-Identifier: Apache-2.0
#include <{{name}}/vulkan_backend.hpp>

#include <sstream>

#if defined({{NAME}}_HAS_VULKAN)
#include <vulkan/vulkan.h>
#endif

namespace {{Name}} {

BackendCapability ProbeVulkanBackend() {
#if defined({{NAME}}_HAS_VULKAN)
  std::uint32_t version = VK_API_VERSION_1_0;
  const VkResult result = vkEnumerateInstanceVersion(&version);
  if (result != VK_SUCCESS) {
    return {false, "Vulkan loader version query failed"};
  }
  std::ostringstream detail;
  detail << "Vulkan loader API " << VK_API_VERSION_MAJOR(version) << '.'
         << VK_API_VERSION_MINOR(version) << '.' << VK_API_VERSION_PATCH(version);
  return {version >= VK_API_VERSION_1_3, detail.str()};
#else
  return {false, "Vulkan 1.3 SDK/loader was not available at configure time"};
#endif
}

}  // namespace {{Name}}
