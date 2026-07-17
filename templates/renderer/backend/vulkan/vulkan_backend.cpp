// SPDX-License-Identifier: Apache-2.0
#include <{{name}}/vulkan_backend.hpp>

#include <cstring>
#include <fstream>
#include <limits>
#include <optional>
#include <sstream>
#include <utility>
#include <vector>

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

#if defined({{NAME}}_HAS_VULKAN)
namespace {

constexpr std::uint32_t kWidth = 64;
constexpr std::uint32_t kHeight = 64;
constexpr VkFormat kColorFormat = VK_FORMAT_R8G8B8A8_UNORM;
constexpr VkFormat kDepthFormat = VK_FORMAT_D32_SFLOAT;

struct ValidationState {
  std::uint32_t message_count = 0;
  std::string first_message;
};

VKAPI_ATTR VkBool32 VKAPI_CALL ValidationCallback(
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

VkDebugUtilsMessengerCreateInfoEXT DebugMessengerCreateInfo(
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

GpuFrameEvidence Evidence(FrameStatus status, std::string detail) {
  GpuFrameEvidence evidence;
  evidence.status = status;
  evidence.detail = std::move(detail);
  return evidence;
}

bool VulkanOk(VkResult result, const char* operation, std::string& detail) {
  if (result == VK_SUCCESS) {
    return true;
  }
  std::ostringstream message;
  message << operation << " failed with VkResult " << result;
  detail = message.str();
  return false;
}

struct Context {
  VkInstance instance = VK_NULL_HANDLE;
  VkDebugUtilsMessengerEXT debug_messenger = VK_NULL_HANDLE;
  PFN_vkDestroyDebugUtilsMessengerEXT destroy_debug_messenger = nullptr;
  ValidationState validation;
  bool validation_available = false;
  std::string validation_detail;
  VkPhysicalDevice physical_device = VK_NULL_HANDLE;
  VkDevice device = VK_NULL_HANDLE;
  VkQueue queue = VK_NULL_HANDLE;
  VkCommandPool command_pool = VK_NULL_HANDLE;
  VkImage color_image = VK_NULL_HANDLE;
  VkDeviceMemory color_memory = VK_NULL_HANDLE;
  VkImageView color_view = VK_NULL_HANDLE;
  VkImage depth_image = VK_NULL_HANDLE;
  VkDeviceMemory depth_memory = VK_NULL_HANDLE;
  VkImageView depth_view = VK_NULL_HANDLE;
  VkBuffer color_readback = VK_NULL_HANDLE;
  VkDeviceMemory color_readback_memory = VK_NULL_HANDLE;
  bool color_readback_coherent = false;
  VkBuffer depth_readback = VK_NULL_HANDLE;
  VkDeviceMemory depth_readback_memory = VK_NULL_HANDLE;
  bool depth_readback_coherent = false;
  VkRenderPass render_pass = VK_NULL_HANDLE;
  VkFramebuffer framebuffer = VK_NULL_HANDLE;
  VkPipelineLayout pipeline_layout = VK_NULL_HANDLE;
  VkPipeline pipeline = VK_NULL_HANDLE;
  VkFence fence = VK_NULL_HANDLE;

  ~Context() {
    if (device != VK_NULL_HANDLE) {
      vkDeviceWaitIdle(device);
      vkDestroyFence(device, fence, nullptr);
      vkDestroyPipeline(device, pipeline, nullptr);
      vkDestroyPipelineLayout(device, pipeline_layout, nullptr);
      vkDestroyFramebuffer(device, framebuffer, nullptr);
      vkDestroyRenderPass(device, render_pass, nullptr);
      vkDestroyBuffer(device, depth_readback, nullptr);
      vkFreeMemory(device, depth_readback_memory, nullptr);
      vkDestroyBuffer(device, color_readback, nullptr);
      vkFreeMemory(device, color_readback_memory, nullptr);
      vkDestroyImageView(device, depth_view, nullptr);
      vkDestroyImage(device, depth_image, nullptr);
      vkFreeMemory(device, depth_memory, nullptr);
      vkDestroyImageView(device, color_view, nullptr);
      vkDestroyImage(device, color_image, nullptr);
      vkFreeMemory(device, color_memory, nullptr);
      vkDestroyCommandPool(device, command_pool, nullptr);
      vkDestroyDevice(device, nullptr);
    }
    if (instance != VK_NULL_HANDLE) {
      if (destroy_debug_messenger != nullptr &&
          debug_messenger != VK_NULL_HANDLE) {
        destroy_debug_messenger(instance, debug_messenger, nullptr);
      }
      vkDestroyInstance(instance, nullptr);
    }
  }
};

std::optional<std::uint32_t> FindGraphicsQueue(VkPhysicalDevice device) {
  std::uint32_t count = 0;
  vkGetPhysicalDeviceQueueFamilyProperties(device, &count, nullptr);
  std::vector<VkQueueFamilyProperties> properties(count);
  vkGetPhysicalDeviceQueueFamilyProperties(device, &count, properties.data());
  for (std::uint32_t index = 0; index < count; ++index) {
    if (properties[index].queueCount > 0 &&
        (properties[index].queueFlags & VK_QUEUE_GRAPHICS_BIT) != 0) {
      return index;
    }
  }
  return std::nullopt;
}

std::uint32_t FindMemoryType(VkPhysicalDevice device,
                             std::uint32_t allowed,
                             VkMemoryPropertyFlags required,
                             VkMemoryPropertyFlags preferred,
                             bool* coherent = nullptr) {
  VkPhysicalDeviceMemoryProperties properties{};
  vkGetPhysicalDeviceMemoryProperties(device, &properties);
  const auto find = [&](VkMemoryPropertyFlags wanted) {
    for (std::uint32_t index = 0; index < properties.memoryTypeCount; ++index) {
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
  if (coherent != nullptr && index != std::numeric_limits<std::uint32_t>::max()) {
    *coherent = (properties.memoryTypes[index].propertyFlags &
                 VK_MEMORY_PROPERTY_HOST_COHERENT_BIT) != 0;
  }
  return index;
}

bool CreateImage(Context& context,
                 VkFormat format,
                 VkImageUsageFlags usage,
                 VkImage& image,
                 VkDeviceMemory& memory,
                 std::string& detail) {
  VkImageCreateInfo create{VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO};
  create.imageType = VK_IMAGE_TYPE_2D;
  create.format = format;
  create.extent = {kWidth, kHeight, 1};
  create.mipLevels = 1;
  create.arrayLayers = 1;
  create.samples = VK_SAMPLE_COUNT_1_BIT;
  create.tiling = VK_IMAGE_TILING_OPTIMAL;
  create.usage = usage;
  create.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
  create.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
  if (!VulkanOk(vkCreateImage(context.device, &create, nullptr, &image),
                "vkCreateImage", detail)) {
    return false;
  }
  VkMemoryRequirements requirements{};
  vkGetImageMemoryRequirements(context.device, image, &requirements);
  const std::uint32_t memory_type =
      FindMemoryType(context.physical_device, requirements.memoryTypeBits,
                     VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT, 0);
  if (memory_type == std::numeric_limits<std::uint32_t>::max()) {
    detail = "no device-local image memory type is available";
    return false;
  }
  VkMemoryAllocateInfo allocate{VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO};
  allocate.allocationSize = requirements.size;
  allocate.memoryTypeIndex = memory_type;
  if (!VulkanOk(vkAllocateMemory(context.device, &allocate, nullptr, &memory),
                "vkAllocateMemory(image)", detail) ||
      !VulkanOk(vkBindImageMemory(context.device, image, memory, 0),
                "vkBindImageMemory", detail)) {
    return false;
  }
  return true;
}

bool CreateReadbackBuffer(Context& context,
                          VkDeviceSize size,
                          VkBuffer& buffer,
                          VkDeviceMemory& memory,
                          bool& coherent,
                          std::string& detail) {
  VkBufferCreateInfo create{VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO};
  create.size = size;
  create.usage = VK_BUFFER_USAGE_TRANSFER_DST_BIT;
  create.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
  if (!VulkanOk(vkCreateBuffer(context.device, &create, nullptr, &buffer),
                "vkCreateBuffer", detail)) {
    return false;
  }
  VkMemoryRequirements requirements{};
  vkGetBufferMemoryRequirements(context.device, buffer, &requirements);
  const std::uint32_t memory_type = FindMemoryType(
      context.physical_device, requirements.memoryTypeBits,
      VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT, VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
      &coherent);
  if (memory_type == std::numeric_limits<std::uint32_t>::max()) {
    detail = "no host-visible readback memory type is available";
    return false;
  }
  VkMemoryAllocateInfo allocate{VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO};
  allocate.allocationSize = requirements.size;
  allocate.memoryTypeIndex = memory_type;
  if (!VulkanOk(vkAllocateMemory(context.device, &allocate, nullptr, &memory),
                "vkAllocateMemory(readback)", detail) ||
      !VulkanOk(vkBindBufferMemory(context.device, buffer, memory, 0),
                "vkBindBufferMemory", detail)) {
    return false;
  }
  return true;
}

bool LoadSpirv(const std::string& path,
               std::vector<std::uint32_t>& words,
               std::string& detail) {
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

VkShaderModule CreateShader(VkDevice device,
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

bool InvalidateIfNeeded(VkDevice device,
                        VkDeviceMemory memory,
                        bool coherent,
                        std::string& detail) {
  if (coherent) {
    return true;
  }
  VkMappedMemoryRange range{VK_STRUCTURE_TYPE_MAPPED_MEMORY_RANGE};
  range.memory = memory;
  range.offset = 0;
  range.size = VK_WHOLE_SIZE;
  return VulkanOk(vkInvalidateMappedMemoryRanges(device, 1, &range),
                   "vkInvalidateMappedMemoryRanges", detail);
}

}  // namespace
#endif

GpuFrameEvidence RenderOffscreen(const DrawSummary& draw,
                                 const std::string& vertex_shader,
                                 const std::string& fragment_shader,
                                 std::uint32_t frame_count) {
#if !defined({{NAME}}_HAS_VULKAN)
  (void)draw;
  (void)vertex_shader;
  (void)fragment_shader;
  (void)frame_count;
  GpuFrameEvidence evidence;
  evidence.status = FrameStatus::Skip;
  evidence.detail = "Vulkan backend was not compiled for this configuration";
  evidence.validation_detail =
      "Vulkan validation capture is unavailable in the core-only configuration";
  return evidence;
#else
  if (draw.draw_count != 1 || draw.triangle_count != 1) {
    return Evidence(FrameStatus::Fail,
                    "bootstrap extraction did not produce one triangle draw");
  }
  if (frame_count == 0) {
    return Evidence(FrameStatus::Fail, "frame_count must be at least 1");
  }

  std::string detail;
  std::vector<std::uint32_t> vertex_words;
  std::vector<std::uint32_t> fragment_words;
  if (!LoadSpirv(vertex_shader, vertex_words, detail) ||
      !LoadSpirv(fragment_shader, fragment_words, detail)) {
    return Evidence(FrameStatus::Fail, detail);
  }

  Context context;
  std::uint32_t layer_count = 0;
  vkEnumerateInstanceLayerProperties(&layer_count, nullptr);
  std::vector<VkLayerProperties> layers(layer_count);
  vkEnumerateInstanceLayerProperties(&layer_count, layers.data());
  bool has_validation_layer = false;
  for (const VkLayerProperties& layer : layers) {
    if (std::strcmp(layer.layerName, "VK_LAYER_KHRONOS_validation") == 0) {
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
    if (std::strcmp(extension.extensionName, VK_EXT_DEBUG_UTILS_EXTENSION_NAME) == 0) {
      has_debug_utils = true;
      break;
    }
  }
  const bool enable_validation = has_validation_layer && has_debug_utils;
  const char* validation_layer = "VK_LAYER_KHRONOS_validation";
  const char* debug_extension = VK_EXT_DEBUG_UTILS_EXTENSION_NAME;
  VkDebugUtilsMessengerCreateInfoEXT debug_create =
      DebugMessengerCreateInfo(&context.validation);
  VkApplicationInfo application{VK_STRUCTURE_TYPE_APPLICATION_INFO};
  application.pApplicationName = "{{name}}-headless";
  application.applicationVersion = VK_MAKE_API_VERSION(0, 0, 1, 0);
  application.pEngineName = "{{name}}";
  application.engineVersion = VK_MAKE_API_VERSION(0, 0, 1, 0);
  application.apiVersion = VK_API_VERSION_1_3;
  VkInstanceCreateInfo instance_create{VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO};
  instance_create.pApplicationInfo = &application;
  if (enable_validation) {
    instance_create.enabledLayerCount = 1;
    instance_create.ppEnabledLayerNames = &validation_layer;
    instance_create.enabledExtensionCount = 1;
    instance_create.ppEnabledExtensionNames = &debug_extension;
    instance_create.pNext = &debug_create;
  }
  if (!VulkanOk(vkCreateInstance(&instance_create, nullptr, &context.instance),
                "vkCreateInstance", detail)) {
    return Evidence(FrameStatus::Skip, detail);
  }
  if (enable_validation) {
    const auto create_debug = reinterpret_cast<PFN_vkCreateDebugUtilsMessengerEXT>(
        vkGetInstanceProcAddr(context.instance, "vkCreateDebugUtilsMessengerEXT"));
    context.destroy_debug_messenger =
        reinterpret_cast<PFN_vkDestroyDebugUtilsMessengerEXT>(vkGetInstanceProcAddr(
            context.instance, "vkDestroyDebugUtilsMessengerEXT"));
    if (create_debug != nullptr && context.destroy_debug_messenger != nullptr &&
        create_debug(context.instance, &debug_create, nullptr,
                     &context.debug_messenger) == VK_SUCCESS) {
      context.validation_available = true;
    } else {
      context.validation_detail =
          "VK_EXT_debug_utils messenger could not be created";
    }
  } else if (!has_validation_layer) {
    context.validation_detail = "VK_LAYER_KHRONOS_validation is unavailable";
  } else {
    context.validation_detail = "VK_EXT_debug_utils is unavailable";
  }

  std::uint32_t physical_count = 0;
  if (!VulkanOk(vkEnumeratePhysicalDevices(context.instance, &physical_count, nullptr),
                "vkEnumeratePhysicalDevices", detail)) {
    return Evidence(FrameStatus::Fail, detail);
  }
  if (physical_count == 0) {
    return Evidence(FrameStatus::Skip, "no Vulkan physical device is available");
  }
  std::vector<VkPhysicalDevice> physical_devices(physical_count);
  vkEnumeratePhysicalDevices(context.instance, &physical_count, physical_devices.data());
  std::optional<std::uint32_t> queue_family;
  for (VkPhysicalDevice physical : physical_devices) {
    const auto candidate = FindGraphicsQueue(physical);
    if (candidate) {
      context.physical_device = physical;
      queue_family = candidate;
      break;
    }
  }
  if (!queue_family) {
    return Evidence(FrameStatus::Skip,
                    "no Vulkan physical device exposes a graphics queue");
  }

  // Slang lowers SV_VertexID to VertexIndex minus BaseVertex (D3D semantics),
  // so the generated SPIR-V declares the DrawParameters capability and the
  // device must enable the matching shaderDrawParameters feature.
  VkPhysicalDeviceVulkan11Features supported_vulkan11{
      VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES};
  VkPhysicalDeviceFeatures2 supported_features{
      VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2};
  supported_features.pNext = &supported_vulkan11;
  vkGetPhysicalDeviceFeatures2(context.physical_device, &supported_features);
  if (supported_vulkan11.shaderDrawParameters != VK_TRUE) {
    return Evidence(FrameStatus::Skip,
                    "the device does not support shaderDrawParameters, which "
                    "the Slang vertex-index lowering requires");
  }

  VkFormatProperties color_properties{};
  VkFormatProperties depth_properties{};
  vkGetPhysicalDeviceFormatProperties(context.physical_device, kColorFormat,
                                      &color_properties);
  vkGetPhysicalDeviceFormatProperties(context.physical_device, kDepthFormat,
                                      &depth_properties);
  const VkFormatFeatureFlags color_required =
      VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT | VK_FORMAT_FEATURE_TRANSFER_SRC_BIT;
  const VkFormatFeatureFlags depth_required =
      VK_FORMAT_FEATURE_DEPTH_STENCIL_ATTACHMENT_BIT |
      VK_FORMAT_FEATURE_TRANSFER_SRC_BIT;
  if ((color_properties.optimalTilingFeatures & color_required) != color_required ||
      (depth_properties.optimalTilingFeatures & depth_required) != depth_required) {
    return Evidence(FrameStatus::Skip,
                    "required RGBA8/depth32 attachment readback formats are unavailable");
  }

  const float priority = 1.0F;
  VkDeviceQueueCreateInfo queue_create{VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO};
  queue_create.queueFamilyIndex = *queue_family;
  queue_create.queueCount = 1;
  queue_create.pQueuePriorities = &priority;
  VkPhysicalDeviceVulkan11Features enabled_vulkan11{
      VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES};
  enabled_vulkan11.shaderDrawParameters = VK_TRUE;
  VkDeviceCreateInfo device_create{VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO};
  device_create.pNext = &enabled_vulkan11;
  device_create.queueCreateInfoCount = 1;
  device_create.pQueueCreateInfos = &queue_create;
  if (!VulkanOk(vkCreateDevice(context.physical_device, &device_create, nullptr,
                               &context.device),
                "vkCreateDevice", detail)) {
    return Evidence(FrameStatus::Fail, detail);
  }
  vkGetDeviceQueue(context.device, *queue_family, 0, &context.queue);

  VkCommandPoolCreateInfo pool_create{VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO};
  pool_create.queueFamilyIndex = *queue_family;
  if (!VulkanOk(vkCreateCommandPool(context.device, &pool_create, nullptr,
                                    &context.command_pool),
                "vkCreateCommandPool", detail)) {
    return Evidence(FrameStatus::Fail, detail);
  }

  if (!CreateImage(context, kColorFormat,
                   VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT |
                       VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
                   context.color_image, context.color_memory, detail) ||
      !CreateImage(context, kDepthFormat,
                   VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT |
                       VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
                   context.depth_image, context.depth_memory, detail)) {
    return Evidence(FrameStatus::Fail, detail);
  }

  VkImageViewCreateInfo view_create{VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO};
  view_create.viewType = VK_IMAGE_VIEW_TYPE_2D;
  view_create.subresourceRange.baseMipLevel = 0;
  view_create.subresourceRange.levelCount = 1;
  view_create.subresourceRange.baseArrayLayer = 0;
  view_create.subresourceRange.layerCount = 1;
  view_create.image = context.color_image;
  view_create.format = kColorFormat;
  view_create.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
  if (!VulkanOk(vkCreateImageView(context.device, &view_create, nullptr,
                                  &context.color_view),
                "vkCreateImageView(color)", detail)) {
    return Evidence(FrameStatus::Fail, detail);
  }
  view_create.image = context.depth_image;
  view_create.format = kDepthFormat;
  view_create.subresourceRange.aspectMask = VK_IMAGE_ASPECT_DEPTH_BIT;
  if (!VulkanOk(vkCreateImageView(context.device, &view_create, nullptr,
                                  &context.depth_view),
                "vkCreateImageView(depth)", detail)) {
    return Evidence(FrameStatus::Fail, detail);
  }

  const VkDeviceSize color_bytes = kWidth * kHeight * 4U;
  const VkDeviceSize depth_bytes = kWidth * kHeight * sizeof(float);
  if (!CreateReadbackBuffer(context, color_bytes, context.color_readback,
                            context.color_readback_memory,
                            context.color_readback_coherent, detail) ||
      !CreateReadbackBuffer(context, depth_bytes, context.depth_readback,
                            context.depth_readback_memory,
                            context.depth_readback_coherent, detail)) {
    return Evidence(FrameStatus::Fail, detail);
  }

  VkAttachmentDescription attachments[2]{};
  attachments[0].format = kColorFormat;
  attachments[0].samples = VK_SAMPLE_COUNT_1_BIT;
  attachments[0].loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR;
  attachments[0].storeOp = VK_ATTACHMENT_STORE_OP_STORE;
  attachments[0].stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
  attachments[0].stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
  attachments[0].initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
  attachments[0].finalLayout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
  attachments[1].format = kDepthFormat;
  attachments[1].samples = VK_SAMPLE_COUNT_1_BIT;
  attachments[1].loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR;
  attachments[1].storeOp = VK_ATTACHMENT_STORE_OP_STORE;
  attachments[1].stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
  attachments[1].stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
  attachments[1].initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
  attachments[1].finalLayout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
  VkAttachmentReference color_reference{0, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL};
  VkAttachmentReference depth_reference{1,
                                        VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL};
  VkSubpassDescription subpass{};
  subpass.pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
  subpass.colorAttachmentCount = 1;
  subpass.pColorAttachments = &color_reference;
  subpass.pDepthStencilAttachment = &depth_reference;
  VkSubpassDependency dependencies[2]{};
  dependencies[0].srcSubpass = VK_SUBPASS_EXTERNAL;
  dependencies[0].dstSubpass = 0;
  dependencies[0].srcStageMask = VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT;
  dependencies[0].dstStageMask = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT |
                                 VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT;
  dependencies[0].dstAccessMask = VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT |
                                  VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT;
  dependencies[1].srcSubpass = 0;
  dependencies[1].dstSubpass = VK_SUBPASS_EXTERNAL;
  dependencies[1].srcStageMask = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT |
                                 VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT;
  dependencies[1].srcAccessMask = VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT |
                                  VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT;
  dependencies[1].dstStageMask = VK_PIPELINE_STAGE_TRANSFER_BIT;
  dependencies[1].dstAccessMask = VK_ACCESS_TRANSFER_READ_BIT;
  VkRenderPassCreateInfo render_pass_create{VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO};
  render_pass_create.attachmentCount = 2;
  render_pass_create.pAttachments = attachments;
  render_pass_create.subpassCount = 1;
  render_pass_create.pSubpasses = &subpass;
  render_pass_create.dependencyCount = 2;
  render_pass_create.pDependencies = dependencies;
  if (!VulkanOk(vkCreateRenderPass(context.device, &render_pass_create, nullptr,
                                   &context.render_pass),
                "vkCreateRenderPass", detail)) {
    return Evidence(FrameStatus::Fail, detail);
  }

  const VkImageView framebuffer_attachments[] = {context.color_view,
                                                 context.depth_view};
  VkFramebufferCreateInfo framebuffer_create{VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO};
  framebuffer_create.renderPass = context.render_pass;
  framebuffer_create.attachmentCount = 2;
  framebuffer_create.pAttachments = framebuffer_attachments;
  framebuffer_create.width = kWidth;
  framebuffer_create.height = kHeight;
  framebuffer_create.layers = 1;
  if (!VulkanOk(vkCreateFramebuffer(context.device, &framebuffer_create, nullptr,
                                    &context.framebuffer),
                "vkCreateFramebuffer", detail)) {
    return Evidence(FrameStatus::Fail, detail);
  }

  VkShaderModule vertex_module = CreateShader(context.device, vertex_words, detail);
  VkShaderModule fragment_module = CreateShader(context.device, fragment_words, detail);
  if (vertex_module == VK_NULL_HANDLE || fragment_module == VK_NULL_HANDLE) {
    vkDestroyShaderModule(context.device, vertex_module, nullptr);
    vkDestroyShaderModule(context.device, fragment_module, nullptr);
    return Evidence(FrameStatus::Fail, detail);
  }
  VkPipelineShaderStageCreateInfo stages[2]{};
  stages[0].sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO;
  stages[0].stage = VK_SHADER_STAGE_VERTEX_BIT;
  stages[0].module = vertex_module;
  stages[0].pName = "main";
  stages[1].sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO;
  stages[1].stage = VK_SHADER_STAGE_FRAGMENT_BIT;
  stages[1].module = fragment_module;
  stages[1].pName = "main";
  VkPipelineVertexInputStateCreateInfo vertex_input{
      VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO};
  VkPipelineInputAssemblyStateCreateInfo input_assembly{
      VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO};
  input_assembly.topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
  VkViewport viewport{0.0F, 0.0F, static_cast<float>(kWidth),
                      static_cast<float>(kHeight), 0.0F, 1.0F};
  VkRect2D scissor{{0, 0}, {kWidth, kHeight}};
  VkPipelineViewportStateCreateInfo viewport_state{
      VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO};
  viewport_state.viewportCount = 1;
  viewport_state.pViewports = &viewport;
  viewport_state.scissorCount = 1;
  viewport_state.pScissors = &scissor;
  VkPipelineRasterizationStateCreateInfo raster{
      VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO};
  raster.polygonMode = VK_POLYGON_MODE_FILL;
  raster.cullMode = VK_CULL_MODE_NONE;
  raster.frontFace = VK_FRONT_FACE_COUNTER_CLOCKWISE;
  raster.lineWidth = 1.0F;
  VkPipelineMultisampleStateCreateInfo multisample{
      VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO};
  multisample.rasterizationSamples = VK_SAMPLE_COUNT_1_BIT;
  VkPipelineDepthStencilStateCreateInfo depth_state{
      VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO};
  depth_state.depthTestEnable = VK_TRUE;
  depth_state.depthWriteEnable = VK_TRUE;
  depth_state.depthCompareOp = VK_COMPARE_OP_LESS;
  VkPipelineColorBlendAttachmentState blend_attachment{};
  blend_attachment.colorWriteMask = VK_COLOR_COMPONENT_R_BIT |
                                    VK_COLOR_COMPONENT_G_BIT |
                                    VK_COLOR_COMPONENT_B_BIT |
                                    VK_COLOR_COMPONENT_A_BIT;
  VkPipelineColorBlendStateCreateInfo blend{
      VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO};
  blend.attachmentCount = 1;
  blend.pAttachments = &blend_attachment;
  VkPipelineLayoutCreateInfo layout_create{VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO};
  if (!VulkanOk(vkCreatePipelineLayout(context.device, &layout_create, nullptr,
                                       &context.pipeline_layout),
                "vkCreatePipelineLayout", detail)) {
    vkDestroyShaderModule(context.device, vertex_module, nullptr);
    vkDestroyShaderModule(context.device, fragment_module, nullptr);
    return Evidence(FrameStatus::Fail, detail);
  }
  VkGraphicsPipelineCreateInfo pipeline_create{
      VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO};
  pipeline_create.stageCount = 2;
  pipeline_create.pStages = stages;
  pipeline_create.pVertexInputState = &vertex_input;
  pipeline_create.pInputAssemblyState = &input_assembly;
  pipeline_create.pViewportState = &viewport_state;
  pipeline_create.pRasterizationState = &raster;
  pipeline_create.pMultisampleState = &multisample;
  pipeline_create.pDepthStencilState = &depth_state;
  pipeline_create.pColorBlendState = &blend;
  pipeline_create.layout = context.pipeline_layout;
  pipeline_create.renderPass = context.render_pass;
  pipeline_create.subpass = 0;
  const VkResult pipeline_result = vkCreateGraphicsPipelines(
      context.device, VK_NULL_HANDLE, 1, &pipeline_create, nullptr,
      &context.pipeline);
  vkDestroyShaderModule(context.device, vertex_module, nullptr);
  vkDestroyShaderModule(context.device, fragment_module, nullptr);
  if (!VulkanOk(pipeline_result, "vkCreateGraphicsPipelines", detail)) {
    return Evidence(FrameStatus::Fail, detail);
  }

  VkCommandBufferAllocateInfo command_allocate{
      VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO};
  command_allocate.commandPool = context.command_pool;
  command_allocate.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
  command_allocate.commandBufferCount = 1;
  VkCommandBuffer command = VK_NULL_HANDLE;
  if (!VulkanOk(vkAllocateCommandBuffers(context.device, &command_allocate, &command),
                "vkAllocateCommandBuffers", detail)) {
    return Evidence(FrameStatus::Fail, detail);
  }
  VkCommandBufferBeginInfo command_begin{VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};
  if (!VulkanOk(vkBeginCommandBuffer(command, &command_begin),
                "vkBeginCommandBuffer", detail)) {
    return Evidence(FrameStatus::Fail, detail);
  }
  VkClearValue clear[2]{};
  clear[0].color.float32[0] = 0.05F;
  clear[0].color.float32[1] = 0.10F;
  clear[0].color.float32[2] = 0.15F;
  clear[0].color.float32[3] = 1.0F;
  clear[1].depthStencil = {1.0F, 0};
  VkRenderPassBeginInfo render_begin{VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO};
  render_begin.renderPass = context.render_pass;
  render_begin.framebuffer = context.framebuffer;
  render_begin.renderArea.offset = {0, 0};
  render_begin.renderArea.extent = {kWidth, kHeight};
  render_begin.clearValueCount = 2;
  render_begin.pClearValues = clear;
  vkCmdBeginRenderPass(command, &render_begin, VK_SUBPASS_CONTENTS_INLINE);
  vkCmdBindPipeline(command, VK_PIPELINE_BIND_POINT_GRAPHICS, context.pipeline);
  vkCmdDraw(command, draw.triangle_count * 3U, 1, 0, 0);
  vkCmdEndRenderPass(command);

  VkBufferImageCopy color_copy{};
  color_copy.imageSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
  color_copy.imageSubresource.layerCount = 1;
  color_copy.imageExtent = {kWidth, kHeight, 1};
  vkCmdCopyImageToBuffer(command, context.color_image,
                         VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                         context.color_readback, 1, &color_copy);
  VkBufferImageCopy depth_copy{};
  depth_copy.imageSubresource.aspectMask = VK_IMAGE_ASPECT_DEPTH_BIT;
  depth_copy.imageSubresource.layerCount = 1;
  depth_copy.imageExtent = {kWidth, kHeight, 1};
  vkCmdCopyImageToBuffer(command, context.depth_image,
                         VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                         context.depth_readback, 1, &depth_copy);
  VkBufferMemoryBarrier host_barriers[2]{};
  for (VkBufferMemoryBarrier& barrier : host_barriers) {
    barrier.sType = VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER;
    barrier.srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;
    barrier.dstAccessMask = VK_ACCESS_HOST_READ_BIT;
    barrier.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    barrier.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    barrier.offset = 0;
    barrier.size = VK_WHOLE_SIZE;
  }
  host_barriers[0].buffer = context.color_readback;
  host_barriers[1].buffer = context.depth_readback;
  vkCmdPipelineBarrier(command, VK_PIPELINE_STAGE_TRANSFER_BIT,
                       VK_PIPELINE_STAGE_HOST_BIT, 0, 0, nullptr, 2,
                       host_barriers, 0, nullptr);
  if (!VulkanOk(vkEndCommandBuffer(command), "vkEndCommandBuffer", detail)) {
    return Evidence(FrameStatus::Fail, detail);
  }

  VkFenceCreateInfo fence_create{VK_STRUCTURE_TYPE_FENCE_CREATE_INFO};
  if (!VulkanOk(vkCreateFence(context.device, &fence_create, nullptr,
                              &context.fence),
                "vkCreateFence", detail)) {
    return Evidence(FrameStatus::Fail, detail);
  }
  VkSubmitInfo submit{VK_STRUCTURE_TYPE_SUBMIT_INFO};
  submit.commandBufferCount = 1;
  submit.pCommandBuffers = &command;
  std::uint64_t completion = 0;
  for (std::uint32_t frame = 0; frame < frame_count; ++frame) {
    if (frame > 0 &&
        !VulkanOk(vkResetFences(context.device, 1, &context.fence),
                  "vkResetFences", detail)) {
      return Evidence(FrameStatus::Fail, detail);
    }
    if (!VulkanOk(vkQueueSubmit(context.queue, 1, &submit, context.fence),
                  "vkQueueSubmit", detail) ||
        !VulkanOk(vkWaitForFences(context.device, 1, &context.fence, VK_TRUE,
                                  10'000'000'000ULL),
                  "vkWaitForFences", detail)) {
      return Evidence(FrameStatus::Fail, detail);
    }
    ++completion;
  }

  void* color_data = nullptr;
  void* depth_data = nullptr;
  if (!VulkanOk(vkMapMemory(context.device, context.color_readback_memory, 0,
                            color_bytes, 0, &color_data),
                "vkMapMemory(color)", detail) ||
      !VulkanOk(vkMapMemory(context.device, context.depth_readback_memory, 0,
                            depth_bytes, 0, &depth_data),
                "vkMapMemory(depth)", detail) ||
      !InvalidateIfNeeded(context.device, context.color_readback_memory,
                          context.color_readback_coherent, detail) ||
      !InvalidateIfNeeded(context.device, context.depth_readback_memory,
                          context.depth_readback_coherent, detail)) {
    if (color_data != nullptr) {
      vkUnmapMemory(context.device, context.color_readback_memory);
    }
    if (depth_data != nullptr) {
      vkUnmapMemory(context.device, context.depth_readback_memory);
    }
    return Evidence(FrameStatus::Fail, detail);
  }

  GpuFrameEvidence evidence = Evidence(FrameStatus::Pass, "");
  evidence.completion = completion;
  evidence.frames_rendered = frame_count;
  evidence.validation_available = context.validation_available;
  evidence.validation_message_count = context.validation.message_count;
  evidence.validation_detail = context.validation.first_message.empty()
                                   ? context.validation_detail
                                   : context.validation.first_message;
  evidence.color.width = kWidth;
  evidence.color.height = kHeight;
  evidence.color.row_pitch = kWidth * 4U;
  evidence.color.pixel_format = "rgba8-unorm";
  evidence.color.origin = "top-left";
  evidence.color.color_space = "linear";
  evidence.color.payload.resize(static_cast<std::size_t>(color_bytes));
  std::memcpy(evidence.color.payload.data(), color_data,
              evidence.color.payload.size());
  evidence.depth.width = kWidth;
  evidence.depth.height = kHeight;
  evidence.depth.row_pitch = kWidth * sizeof(float);
  evidence.depth.pixel_format = "d32-sfloat";
  evidence.depth.origin = "top-left";
  evidence.depth.payload.resize(kWidth * kHeight);
  std::memcpy(evidence.depth.payload.data(), depth_data,
              static_cast<std::size_t>(depth_bytes));
  vkUnmapMemory(context.device, context.color_readback_memory);
  vkUnmapMemory(context.device, context.depth_readback_memory);

  VkPhysicalDeviceProperties device_properties{};
  vkGetPhysicalDeviceProperties(context.physical_device, &device_properties);
  evidence.device_name = device_properties.deviceName;
  evidence.vendor_id = device_properties.vendorID;
  evidence.device_id = device_properties.deviceID;
  evidence.driver_version = std::to_string(device_properties.driverVersion);
  std::ostringstream api_version;
  api_version << VK_API_VERSION_MAJOR(device_properties.apiVersion) << '.'
              << VK_API_VERSION_MINOR(device_properties.apiVersion) << '.'
              << VK_API_VERSION_PATCH(device_properties.apiVersion);
  evidence.api_version = api_version.str();
  std::ostringstream success;
  success << "rendered " << frame_count << " deterministic frames on "
          << device_properties.deviceName;
  evidence.detail = success.str();
  return evidence;
#endif
}

}  // namespace {{Name}}
