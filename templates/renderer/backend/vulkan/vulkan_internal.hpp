// SPDX-License-Identifier: Apache-2.0
// Helpers shared by the offscreen and presentation translation units of the
// Vulkan capability pack. Private to backend/vulkan; adapters use the public
// headers only.
#pragma once

#include <cstdint>
#include <fstream>
#include <limits>
#include <sstream>
#include <string>
#include <string_view>
#include <vector>

#include <vulkan/vulkan.h>

namespace {{Name}}::vulkan_internal {

struct ValidationState {
  std::uint32_t message_count = 0;
  std::string first_message;
};

VKAPI_ATTR inline VkBool32 VKAPI_CALL ValidationCallback(
    VkDebugUtilsMessageSeverityFlagBitsEXT severity,
    VkDebugUtilsMessageTypeFlagsEXT message_type,
    const VkDebugUtilsMessengerCallbackDataEXT* callback_data,
    void* user_data) {
  const bool error =
      (severity & VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT) != 0;
  const bool renderer_warning =
      (severity & VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT) != 0 &&
      (message_type & (VK_DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT |
                       VK_DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE_BIT_EXT)) != 0;
  // General loader/environment warnings remain observable outside this report,
  // but do not become renderer validation failures.
  if (!error && !renderer_warning) {
    return VK_FALSE;
  }
  auto* state = static_cast<ValidationState*>(user_data);
  ++state->message_count;
  if (state->first_message.empty() && callback_data != nullptr &&
      callback_data->pMessage != nullptr) {
    state->first_message = callback_data->pMessage;
  }
  return VK_FALSE;
}

inline VkDebugUtilsMessengerCreateInfoEXT DebugMessengerCreateInfo(
    ValidationState* state) {
  VkDebugUtilsMessengerCreateInfoEXT create{
      VK_STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT};
  create.messageSeverity = VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT |
                           VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT;
  create.messageType = VK_DEBUG_UTILS_MESSAGE_TYPE_GENERAL_BIT_EXT |
                       VK_DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT |
                       VK_DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE_BIT_EXT;
  create.pfnUserCallback = ValidationCallback;
  create.pUserData = state;
  return create;
}

inline bool VulkanOk(VkResult result, const char* operation,
                     std::string& detail) {
  if (result == VK_SUCCESS) {
    return true;
  }
  std::ostringstream message;
  message << operation << " failed with VkResult " << result;
  detail = message.str();
  return false;
}

// The instance plus the validation capture attached to it. `validation`
// must outlive the instance; the messenger writes into it.
struct InstanceState {
  VkInstance instance = VK_NULL_HANDLE;
  VkDebugUtilsMessengerEXT debug_messenger = VK_NULL_HANDLE;
  PFN_vkDestroyDebugUtilsMessengerEXT destroy_debug_messenger = nullptr;
  bool validation_available = false;
  std::string validation_detail;
};

// Create the instance, opting in to the Khronos validation layer and a debug
// messenger whenever the loader offers both. Validation being unavailable is
// recorded, not an error: evidence stays explicit either way.
inline bool CreateInstanceWithValidation(
    const char* application_name,
    const std::vector<const char*>& required_extensions,
    ValidationState* validation, InstanceState& state, std::string& detail) {
  std::uint32_t layer_count = 0;
  vkEnumerateInstanceLayerProperties(&layer_count, nullptr);
  std::vector<VkLayerProperties> layers(layer_count);
  vkEnumerateInstanceLayerProperties(&layer_count, layers.data());
  bool has_validation_layer = false;
  for (const VkLayerProperties& layer : layers) {
    if (std::string_view(layer.layerName) == "VK_LAYER_KHRONOS_validation") {
      has_validation_layer = true;
      break;
    }
  }
  std::uint32_t extension_count = 0;
  vkEnumerateInstanceExtensionProperties(nullptr, &extension_count, nullptr);
  std::vector<VkExtensionProperties> extensions(extension_count);
  vkEnumerateInstanceExtensionProperties(nullptr, &extension_count,
                                         extensions.data());
  bool has_debug_utils = false;
  for (const VkExtensionProperties& extension : extensions) {
    if (std::string_view(extension.extensionName) ==
        VK_EXT_DEBUG_UTILS_EXTENSION_NAME) {
      has_debug_utils = true;
      break;
    }
  }
  const bool enable_validation = has_validation_layer && has_debug_utils;
  const char* validation_layer = "VK_LAYER_KHRONOS_validation";
  std::vector<const char*> enabled_extensions = required_extensions;
  VkDebugUtilsMessengerCreateInfoEXT debug_create =
      DebugMessengerCreateInfo(validation);
  VkApplicationInfo application{VK_STRUCTURE_TYPE_APPLICATION_INFO};
  application.pApplicationName = application_name;
  application.applicationVersion = VK_MAKE_API_VERSION(0, 0, 1, 0);
  application.pEngineName = "{{name}}";
  application.engineVersion = VK_MAKE_API_VERSION(0, 0, 1, 0);
  application.apiVersion = VK_API_VERSION_1_3;
  VkInstanceCreateInfo instance_create{VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO};
  instance_create.pApplicationInfo = &application;
  if (enable_validation) {
    instance_create.enabledLayerCount = 1;
    instance_create.ppEnabledLayerNames = &validation_layer;
    enabled_extensions.push_back(VK_EXT_DEBUG_UTILS_EXTENSION_NAME);
    instance_create.pNext = &debug_create;
  }
  instance_create.enabledExtensionCount =
      static_cast<std::uint32_t>(enabled_extensions.size());
  instance_create.ppEnabledExtensionNames = enabled_extensions.data();
  if (!VulkanOk(vkCreateInstance(&instance_create, nullptr, &state.instance),
                "vkCreateInstance", detail)) {
    return false;
  }
  if (enable_validation) {
    const auto create_debug =
        reinterpret_cast<PFN_vkCreateDebugUtilsMessengerEXT>(
            vkGetInstanceProcAddr(state.instance,
                                  "vkCreateDebugUtilsMessengerEXT"));
    state.destroy_debug_messenger =
        reinterpret_cast<PFN_vkDestroyDebugUtilsMessengerEXT>(
            vkGetInstanceProcAddr(state.instance,
                                  "vkDestroyDebugUtilsMessengerEXT"));
    if (create_debug != nullptr && state.destroy_debug_messenger != nullptr &&
        create_debug(state.instance, &debug_create, nullptr,
                     &state.debug_messenger) == VK_SUCCESS) {
      state.validation_available = true;
    } else {
      state.validation_detail =
          "VK_EXT_debug_utils messenger could not be created";
    }
  } else if (!has_validation_layer) {
    state.validation_detail = "VK_LAYER_KHRONOS_validation is unavailable";
  } else {
    state.validation_detail = "VK_EXT_debug_utils is unavailable";
  }
  return true;
}

inline void DestroyInstance(InstanceState& state) {
  if (state.instance != VK_NULL_HANDLE) {
    if (state.destroy_debug_messenger != nullptr &&
        state.debug_messenger != VK_NULL_HANDLE) {
      state.destroy_debug_messenger(state.instance, state.debug_messenger,
                                    nullptr);
    }
    vkDestroyInstance(state.instance, nullptr);
    state.instance = VK_NULL_HANDLE;
    state.debug_messenger = VK_NULL_HANDLE;
  }
}

// Slang lowers SV_VertexID to VertexIndex minus BaseVertex (D3D semantics),
// so SPIR-V built from the project shaders declares the DrawParameters
// capability and every device consuming them must enable the matching
// shaderDrawParameters feature.
inline bool SupportsShaderDrawParameters(VkPhysicalDevice device) {
  VkPhysicalDeviceVulkan11Features vulkan11{
      VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES};
  VkPhysicalDeviceFeatures2 features{
      VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2};
  features.pNext = &vulkan11;
  vkGetPhysicalDeviceFeatures2(device, &features);
  return vulkan11.shaderDrawParameters == VK_TRUE;
}

inline std::uint32_t FindMemoryType(VkPhysicalDevice device,
                                    std::uint32_t allowed,
                                    VkMemoryPropertyFlags required,
                                    VkMemoryPropertyFlags preferred,
                                    bool* coherent = nullptr) {
  VkPhysicalDeviceMemoryProperties properties{};
  vkGetPhysicalDeviceMemoryProperties(device, &properties);
  const auto find = [&](VkMemoryPropertyFlags wanted) {
    for (std::uint32_t index = 0; index < properties.memoryTypeCount;
         ++index) {
      if ((allowed & (1U << index)) != 0 &&
          (properties.memoryTypes[index].propertyFlags & wanted) == wanted) {
        return index;
      }
    }
    return std::numeric_limits<std::uint32_t>::max();
  };
  std::uint32_t index = find(required | preferred);
  if (index == std::numeric_limits<std::uint32_t>::max()) {
    index = find(required);
  }
  if (coherent != nullptr &&
      index != std::numeric_limits<std::uint32_t>::max()) {
    *coherent = (properties.memoryTypes[index].propertyFlags &
                 VK_MEMORY_PROPERTY_HOST_COHERENT_BIT) != 0;
  }
  return index;
}

inline bool LoadSpirv(const std::string& path,
                      std::vector<std::uint32_t>& words, std::string& detail) {
  std::ifstream input(path, std::ios::binary | std::ios::ate);
  if (!input) {
    detail = "cannot open SPIR-V shader: " + path;
    return false;
  }
  const std::streamsize size = input.tellg();
  if (size <= 0 || (size % 4) != 0) {
    detail = "SPIR-V shader has an invalid byte length: " + path;
    return false;
  }
  words.resize(static_cast<std::size_t>(size) / sizeof(std::uint32_t));
  input.seekg(0);
  if (!input.read(reinterpret_cast<char*>(words.data()), size)) {
    detail = "cannot read SPIR-V shader: " + path;
    return false;
  }
  return true;
}

inline VkShaderModule CreateShader(VkDevice device,
                                   const std::vector<std::uint32_t>& words,
                                   std::string& detail) {
  VkShaderModuleCreateInfo create{VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO};
  create.codeSize = words.size() * sizeof(std::uint32_t);
  create.pCode = words.data();
  VkShaderModule shader = VK_NULL_HANDLE;
  if (!VulkanOk(vkCreateShaderModule(device, &create, nullptr, &shader),
                "vkCreateShaderModule", detail)) {
    return VK_NULL_HANDLE;
  }
  return shader;
}

}  // namespace {{Name}}::vulkan_internal
