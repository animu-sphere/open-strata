// SPDX-License-Identifier: Apache-2.0
// Swapchain presentation for the bootstrap draw. The backend owns the
// VkSurfaceKHR and every swapchain object; the window layer only supplies the
// surface-creation callback and the platform instance extensions. Skeleton
// policy: one frame in flight, dynamic viewport/scissor, color-only pass.
#include <{{name}}/vulkan_present.hpp>

#include <algorithm>
#include <cstdint>
#include <limits>
#include <optional>
#include <string>
#include <type_traits>
#include <utility>
#include <vector>

#if defined({{NAME}}_HAS_VULKAN)
#include <vulkan/vulkan.h>

#include "vulkan_internal.hpp"
#endif

namespace {{Name}} {

#if !defined({{NAME}}_HAS_VULKAN)

std::unique_ptr<PresentSession> CreatePresentSession(
    const PresentSurfaceProvider& surface, const std::string& vertex_shader,
    const std::string& fragment_shader, bool vsync, PresentSetupStatus& status,
    std::string& error) {
  (void)surface;
  (void)vertex_shader;
  (void)fragment_shader;
  (void)vsync;
  status = PresentSetupStatus::Unavailable;
  error = "Vulkan backend was not compiled for this configuration";
  return nullptr;
}

#else

namespace {

using vulkan_internal::CreateInstanceWithValidation;
using vulkan_internal::CreateShader;
using vulkan_internal::DestroyInstance;
using vulkan_internal::InstanceState;
using vulkan_internal::LoadSpirv;
using vulkan_internal::SupportsShaderDrawParameters;
using vulkan_internal::ValidationState;
using vulkan_internal::VulkanOk;

constexpr std::uint64_t kFrameTimeoutNs = 10'000'000'000ULL;

template <typename Handle>
std::uintptr_t EncodeHandle(Handle handle) noexcept {
  if constexpr (std::is_pointer_v<Handle>) {
    return reinterpret_cast<std::uintptr_t>(handle);
  } else {
    return static_cast<std::uintptr_t>(handle);
  }
}

template <typename Handle>
Handle DecodeHandle(std::uintptr_t handle) noexcept {
  if constexpr (std::is_pointer_v<Handle>) {
    return reinterpret_cast<Handle>(handle);
  } else {
    return static_cast<Handle>(handle);
  }
}

bool HasDeviceExtension(VkPhysicalDevice device, const char* name) {
  std::uint32_t count = 0;
  vkEnumerateDeviceExtensionProperties(device, nullptr, &count, nullptr);
  std::vector<VkExtensionProperties> extensions(count);
  vkEnumerateDeviceExtensionProperties(device, nullptr, &count,
                                       extensions.data());
  for (const VkExtensionProperties& extension : extensions) {
    if (std::string_view(extension.extensionName) == name) {
      return true;
    }
  }
  return false;
}

std::optional<std::uint32_t> FindGraphicsPresentQueue(VkPhysicalDevice device,
                                                      VkSurfaceKHR surface) {
  std::uint32_t count = 0;
  vkGetPhysicalDeviceQueueFamilyProperties(device, &count, nullptr);
  std::vector<VkQueueFamilyProperties> properties(count);
  vkGetPhysicalDeviceQueueFamilyProperties(device, &count, properties.data());
  for (std::uint32_t index = 0; index < count; ++index) {
    if (properties[index].queueCount == 0 ||
        (properties[index].queueFlags & VK_QUEUE_GRAPHICS_BIT) == 0) {
      continue;
    }
    VkBool32 present_supported = VK_FALSE;
    if (vkGetPhysicalDeviceSurfaceSupportKHR(device, index, surface,
                                             &present_supported) ==
            VK_SUCCESS &&
        present_supported == VK_TRUE) {
      return index;
    }
  }
  return std::nullopt;
}

class VulkanPresentSession final : public PresentSession {
 public:
  ~VulkanPresentSession() override { Destroy(); }

  PresentSetupStatus Initialize(const PresentSurfaceProvider& provider,
                                const std::string& vertex_shader,
                                const std::string& fragment_shader, bool vsync,
                                std::string& error);

  [[nodiscard]] bool RenderFrame(const DrawSummary& draw, std::uint32_t width,
                                 std::uint32_t height, bool& presented,
                                 std::string& error) override;

  [[nodiscard]] const PresentStatistics& statistics() const override {
    return statistics_;
  }

 private:
  bool RecreateSwapchain(std::uint32_t width, std::uint32_t height,
                         std::string& error);
  void DestroySwapchainObjects();
  void Destroy();

  InstanceState instance_;
  ValidationState validation_;
  VkSurfaceKHR surface_ = VK_NULL_HANDLE;
  VkPhysicalDevice physical_device_ = VK_NULL_HANDLE;
  std::uint32_t queue_family_ = 0;
  VkDevice device_ = VK_NULL_HANDLE;
  VkQueue queue_ = VK_NULL_HANDLE;
  VkCommandPool command_pool_ = VK_NULL_HANDLE;
  VkCommandBuffer command_ = VK_NULL_HANDLE;
  VkSurfaceFormatKHR surface_format_{};
  VkPresentModeKHR present_mode_ = VK_PRESENT_MODE_FIFO_KHR;
  VkRenderPass render_pass_ = VK_NULL_HANDLE;
  VkPipelineLayout pipeline_layout_ = VK_NULL_HANDLE;
  VkPipeline pipeline_ = VK_NULL_HANDLE;
  VkSemaphore image_available_ = VK_NULL_HANDLE;
  VkFence in_flight_ = VK_NULL_HANDLE;
  VkSwapchainKHR swapchain_ = VK_NULL_HANDLE;
  VkExtent2D extent_{};
  std::vector<VkImage> images_;
  std::vector<VkImageView> views_;
  std::vector<VkFramebuffer> framebuffers_;
  // Present-wait semaphores are per swapchain image: a single semaphore may
  // still be in use by an outstanding present when the next frame needs it.
  std::vector<VkSemaphore> render_finished_;
  bool swapchain_created_once_ = false;
  PresentStatistics statistics_;
};

PresentSetupStatus VulkanPresentSession::Initialize(
    const PresentSurfaceProvider& provider, const std::string& vertex_shader,
    const std::string& fragment_shader, bool vsync, std::string& error) {
  if (provider.create_surface == nullptr) {
    error = "the surface provider carries no create_surface callback";
    return PresentSetupStatus::Error;
  }
  std::vector<std::uint32_t> vertex_words;
  std::vector<std::uint32_t> fragment_words;
  if (!LoadSpirv(vertex_shader, vertex_words, error) ||
      !LoadSpirv(fragment_shader, fragment_words, error)) {
    return PresentSetupStatus::Error;
  }

  std::vector<const char*> instance_extensions;
  instance_extensions.reserve(provider.instance_extensions.size());
  for (const std::string& extension : provider.instance_extensions) {
    instance_extensions.push_back(extension.c_str());
  }
  if (!CreateInstanceWithValidation("{{name}}-viewport", instance_extensions,
                                    &validation_, instance_, error)) {
    return PresentSetupStatus::Unavailable;
  }

  std::uintptr_t encoded_surface = 0;
  const auto surface_result = static_cast<VkResult>(provider.create_surface(
      provider.user_data, EncodeHandle(instance_.instance), &encoded_surface));
  if (surface_result != VK_SUCCESS || encoded_surface == 0) {
    error = "the window layer could not create a presentation surface "
            "(VkResult " +
            std::to_string(static_cast<std::int32_t>(surface_result)) + ")";
    return PresentSetupStatus::Unavailable;
  }
  surface_ = DecodeHandle<VkSurfaceKHR>(encoded_surface);

  std::uint32_t physical_count = 0;
  if (!VulkanOk(vkEnumeratePhysicalDevices(instance_.instance, &physical_count,
                                           nullptr),
                "vkEnumeratePhysicalDevices", error)) {
    return PresentSetupStatus::Error;
  }
  if (physical_count == 0) {
    error = "no Vulkan physical device is available";
    return PresentSetupStatus::Unavailable;
  }
  std::vector<VkPhysicalDevice> physical_devices(physical_count);
  vkEnumeratePhysicalDevices(instance_.instance, &physical_count,
                             physical_devices.data());
  std::optional<std::uint32_t> queue_family;
  for (VkPhysicalDevice physical : physical_devices) {
    if (!HasDeviceExtension(physical, VK_KHR_SWAPCHAIN_EXTENSION_NAME) ||
        !SupportsShaderDrawParameters(physical)) {
      continue;
    }
    const auto candidate = FindGraphicsPresentQueue(physical, surface_);
    if (candidate) {
      physical_device_ = physical;
      queue_family = candidate;
      break;
    }
  }
  if (!queue_family) {
    error = "no Vulkan device offers a graphics+present queue, the swapchain "
            "extension, and shaderDrawParameters for this surface";
    return PresentSetupStatus::Unavailable;
  }
  queue_family_ = *queue_family;

  std::uint32_t format_count = 0;
  vkGetPhysicalDeviceSurfaceFormatsKHR(physical_device_, surface_,
                                       &format_count, nullptr);
  std::vector<VkSurfaceFormatKHR> formats(format_count);
  vkGetPhysicalDeviceSurfaceFormatsKHR(physical_device_, surface_,
                                       &format_count, formats.data());
  if (formats.empty()) {
    error = "the presentation surface reports no color formats";
    return PresentSetupStatus::Unavailable;
  }
  surface_format_ = formats.front();
  for (const VkSurfaceFormatKHR& format : formats) {
    if (format.format == VK_FORMAT_B8G8R8A8_UNORM ||
        format.format == VK_FORMAT_R8G8B8A8_UNORM) {
      surface_format_ = format;
      break;
    }
  }

  present_mode_ = VK_PRESENT_MODE_FIFO_KHR;
  if (!vsync) {
    std::uint32_t mode_count = 0;
    vkGetPhysicalDeviceSurfacePresentModesKHR(physical_device_, surface_,
                                              &mode_count, nullptr);
    std::vector<VkPresentModeKHR> modes(mode_count);
    vkGetPhysicalDeviceSurfacePresentModesKHR(physical_device_, surface_,
                                              &mode_count, modes.data());
    for (const VkPresentModeKHR preferred :
         {VK_PRESENT_MODE_IMMEDIATE_KHR, VK_PRESENT_MODE_MAILBOX_KHR}) {
      if (std::find(modes.begin(), modes.end(), preferred) != modes.end()) {
        present_mode_ = preferred;
        break;
      }
    }
  }

  const float priority = 1.0F;
  VkDeviceQueueCreateInfo queue_create{
      VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO};
  queue_create.queueFamilyIndex = queue_family_;
  queue_create.queueCount = 1;
  queue_create.pQueuePriorities = &priority;
  // The bootstrap shaders are Slang-built and declare DrawParameters; see
  // SupportsShaderDrawParameters.
  VkPhysicalDeviceVulkan11Features enabled_vulkan11{
      VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES};
  enabled_vulkan11.shaderDrawParameters = VK_TRUE;
  const char* swapchain_extension = VK_KHR_SWAPCHAIN_EXTENSION_NAME;
  VkDeviceCreateInfo device_create{VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO};
  device_create.pNext = &enabled_vulkan11;
  device_create.queueCreateInfoCount = 1;
  device_create.pQueueCreateInfos = &queue_create;
  device_create.enabledExtensionCount = 1;
  device_create.ppEnabledExtensionNames = &swapchain_extension;
  if (!VulkanOk(vkCreateDevice(physical_device_, &device_create, nullptr,
                               &device_),
                "vkCreateDevice", error)) {
    return PresentSetupStatus::Error;
  }
  vkGetDeviceQueue(device_, queue_family_, 0, &queue_);

  VkCommandPoolCreateInfo pool_create{
      VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO};
  pool_create.flags = VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT;
  pool_create.queueFamilyIndex = queue_family_;
  if (!VulkanOk(vkCreateCommandPool(device_, &pool_create, nullptr,
                                    &command_pool_),
                "vkCreateCommandPool", error)) {
    return PresentSetupStatus::Error;
  }
  VkCommandBufferAllocateInfo command_allocate{
      VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO};
  command_allocate.commandPool = command_pool_;
  command_allocate.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
  command_allocate.commandBufferCount = 1;
  if (!VulkanOk(vkAllocateCommandBuffers(device_, &command_allocate,
                                         &command_),
                "vkAllocateCommandBuffers", error)) {
    return PresentSetupStatus::Error;
  }

  VkAttachmentDescription attachment{};
  attachment.format = surface_format_.format;
  attachment.samples = VK_SAMPLE_COUNT_1_BIT;
  attachment.loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR;
  attachment.storeOp = VK_ATTACHMENT_STORE_OP_STORE;
  attachment.stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
  attachment.stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
  attachment.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
  attachment.finalLayout = VK_IMAGE_LAYOUT_PRESENT_SRC_KHR;
  VkAttachmentReference color_reference{
      0, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL};
  VkSubpassDescription subpass{};
  subpass.pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
  subpass.colorAttachmentCount = 1;
  subpass.pColorAttachments = &color_reference;
  VkSubpassDependency dependency{};
  dependency.srcSubpass = VK_SUBPASS_EXTERNAL;
  dependency.dstSubpass = 0;
  dependency.srcStageMask = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
  dependency.dstStageMask = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
  dependency.dstAccessMask = VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT;
  VkRenderPassCreateInfo render_pass_create{
      VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO};
  render_pass_create.attachmentCount = 1;
  render_pass_create.pAttachments = &attachment;
  render_pass_create.subpassCount = 1;
  render_pass_create.pSubpasses = &subpass;
  render_pass_create.dependencyCount = 1;
  render_pass_create.pDependencies = &dependency;
  if (!VulkanOk(vkCreateRenderPass(device_, &render_pass_create, nullptr,
                                   &render_pass_),
                "vkCreateRenderPass", error)) {
    return PresentSetupStatus::Error;
  }

  VkShaderModule vertex_module = CreateShader(device_, vertex_words, error);
  VkShaderModule fragment_module =
      CreateShader(device_, fragment_words, error);
  if (vertex_module == VK_NULL_HANDLE || fragment_module == VK_NULL_HANDLE) {
    vkDestroyShaderModule(device_, vertex_module, nullptr);
    vkDestroyShaderModule(device_, fragment_module, nullptr);
    return PresentSetupStatus::Error;
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
  VkPipelineViewportStateCreateInfo viewport_state{
      VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO};
  viewport_state.viewportCount = 1;
  viewport_state.scissorCount = 1;
  VkPipelineRasterizationStateCreateInfo raster{
      VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO};
  raster.polygonMode = VK_POLYGON_MODE_FILL;
  raster.cullMode = VK_CULL_MODE_NONE;
  raster.frontFace = VK_FRONT_FACE_COUNTER_CLOCKWISE;
  raster.lineWidth = 1.0F;
  VkPipelineMultisampleStateCreateInfo multisample{
      VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO};
  multisample.rasterizationSamples = VK_SAMPLE_COUNT_1_BIT;
  VkPipelineColorBlendAttachmentState blend_attachment{};
  blend_attachment.colorWriteMask =
      VK_COLOR_COMPONENT_R_BIT | VK_COLOR_COMPONENT_G_BIT |
      VK_COLOR_COMPONENT_B_BIT | VK_COLOR_COMPONENT_A_BIT;
  VkPipelineColorBlendStateCreateInfo blend{
      VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO};
  blend.attachmentCount = 1;
  blend.pAttachments = &blend_attachment;
  const VkDynamicState dynamic_states[] = {VK_DYNAMIC_STATE_VIEWPORT,
                                           VK_DYNAMIC_STATE_SCISSOR};
  VkPipelineDynamicStateCreateInfo dynamic{
      VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO};
  dynamic.dynamicStateCount = 2;
  dynamic.pDynamicStates = dynamic_states;
  VkPipelineLayoutCreateInfo layout_create{
      VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO};
  if (!VulkanOk(vkCreatePipelineLayout(device_, &layout_create, nullptr,
                                       &pipeline_layout_),
                "vkCreatePipelineLayout", error)) {
    vkDestroyShaderModule(device_, vertex_module, nullptr);
    vkDestroyShaderModule(device_, fragment_module, nullptr);
    return PresentSetupStatus::Error;
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
  pipeline_create.pColorBlendState = &blend;
  pipeline_create.pDynamicState = &dynamic;
  pipeline_create.layout = pipeline_layout_;
  pipeline_create.renderPass = render_pass_;
  pipeline_create.subpass = 0;
  const VkResult pipeline_result = vkCreateGraphicsPipelines(
      device_, VK_NULL_HANDLE, 1, &pipeline_create, nullptr, &pipeline_);
  vkDestroyShaderModule(device_, vertex_module, nullptr);
  vkDestroyShaderModule(device_, fragment_module, nullptr);
  if (!VulkanOk(pipeline_result, "vkCreateGraphicsPipelines", error)) {
    return PresentSetupStatus::Error;
  }

  VkSemaphoreCreateInfo semaphore_create{
      VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO};
  VkFenceCreateInfo fence_create{VK_STRUCTURE_TYPE_FENCE_CREATE_INFO};
  fence_create.flags = VK_FENCE_CREATE_SIGNALED_BIT;
  if (!VulkanOk(vkCreateSemaphore(device_, &semaphore_create, nullptr,
                                  &image_available_),
                "vkCreateSemaphore", error) ||
      !VulkanOk(vkCreateFence(device_, &fence_create, nullptr, &in_flight_),
                "vkCreateFence", error)) {
    return PresentSetupStatus::Error;
  }

  VkPhysicalDeviceProperties device_properties{};
  vkGetPhysicalDeviceProperties(physical_device_, &device_properties);
  statistics_.device_name = device_properties.deviceName;
  statistics_.validation_available = instance_.validation_available;
  statistics_.validation_detail = instance_.validation_detail;
  return PresentSetupStatus::Ready;
}

bool VulkanPresentSession::RecreateSwapchain(std::uint32_t width,
                                             std::uint32_t height,
                                             std::string& error) {
  vkDeviceWaitIdle(device_);
  DestroySwapchainObjects();

  VkSurfaceCapabilitiesKHR capabilities{};
  if (!VulkanOk(vkGetPhysicalDeviceSurfaceCapabilitiesKHR(
                    physical_device_, surface_, &capabilities),
                "vkGetPhysicalDeviceSurfaceCapabilitiesKHR", error)) {
    return false;
  }
  VkExtent2D extent = capabilities.currentExtent;
  if (extent.width == std::numeric_limits<std::uint32_t>::max()) {
    extent.width = std::clamp(width, capabilities.minImageExtent.width,
                              capabilities.maxImageExtent.width);
    extent.height = std::clamp(height, capabilities.minImageExtent.height,
                               capabilities.maxImageExtent.height);
  }
  if (extent.width == 0 || extent.height == 0) {
    // Minimized between the caller's size query and now; skip this frame.
    extent_ = {0, 0};
    return true;
  }

  std::uint32_t image_count = capabilities.minImageCount + 1;
  if (capabilities.maxImageCount > 0) {
    image_count = std::min(image_count, capabilities.maxImageCount);
  }
  VkCompositeAlphaFlagBitsKHR composite = VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR;
  if ((capabilities.supportedCompositeAlpha & composite) == 0) {
    for (const VkCompositeAlphaFlagBitsKHR candidate :
         {VK_COMPOSITE_ALPHA_PRE_MULTIPLIED_BIT_KHR,
          VK_COMPOSITE_ALPHA_POST_MULTIPLIED_BIT_KHR,
          VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR}) {
      if ((capabilities.supportedCompositeAlpha & candidate) != 0) {
        composite = candidate;
        break;
      }
    }
  }

  VkSwapchainCreateInfoKHR swapchain_create{
      VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR};
  swapchain_create.surface = surface_;
  swapchain_create.minImageCount = image_count;
  swapchain_create.imageFormat = surface_format_.format;
  swapchain_create.imageColorSpace = surface_format_.colorSpace;
  swapchain_create.imageExtent = extent;
  swapchain_create.imageArrayLayers = 1;
  swapchain_create.imageUsage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
  swapchain_create.imageSharingMode = VK_SHARING_MODE_EXCLUSIVE;
  swapchain_create.preTransform = capabilities.currentTransform;
  swapchain_create.compositeAlpha = composite;
  swapchain_create.presentMode = present_mode_;
  swapchain_create.clipped = VK_TRUE;
  if (!VulkanOk(vkCreateSwapchainKHR(device_, &swapchain_create, nullptr,
                                     &swapchain_),
                "vkCreateSwapchainKHR", error)) {
    return false;
  }

  std::uint32_t count = 0;
  vkGetSwapchainImagesKHR(device_, swapchain_, &count, nullptr);
  images_.resize(count);
  vkGetSwapchainImagesKHR(device_, swapchain_, &count, images_.data());
  views_.resize(count, VK_NULL_HANDLE);
  framebuffers_.resize(count, VK_NULL_HANDLE);
  render_finished_.resize(count, VK_NULL_HANDLE);
  for (std::uint32_t index = 0; index < count; ++index) {
    VkImageViewCreateInfo view_create{
        VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO};
    view_create.image = images_[index];
    view_create.viewType = VK_IMAGE_VIEW_TYPE_2D;
    view_create.format = surface_format_.format;
    view_create.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
    view_create.subresourceRange.levelCount = 1;
    view_create.subresourceRange.layerCount = 1;
    if (!VulkanOk(vkCreateImageView(device_, &view_create, nullptr,
                                    &views_[index]),
                  "vkCreateImageView(swapchain)", error)) {
      return false;
    }
    VkFramebufferCreateInfo framebuffer_create{
        VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO};
    framebuffer_create.renderPass = render_pass_;
    framebuffer_create.attachmentCount = 1;
    framebuffer_create.pAttachments = &views_[index];
    framebuffer_create.width = extent.width;
    framebuffer_create.height = extent.height;
    framebuffer_create.layers = 1;
    if (!VulkanOk(vkCreateFramebuffer(device_, &framebuffer_create, nullptr,
                                      &framebuffers_[index]),
                  "vkCreateFramebuffer(swapchain)", error)) {
      return false;
    }
    VkSemaphoreCreateInfo semaphore_create{
        VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO};
    if (!VulkanOk(vkCreateSemaphore(device_, &semaphore_create, nullptr,
                                    &render_finished_[index]),
                  "vkCreateSemaphore(present)", error)) {
      return false;
    }
  }

  extent_ = extent;
  if (swapchain_created_once_) {
    ++statistics_.swapchain_recreates;
  }
  swapchain_created_once_ = true;
  return true;
}

bool VulkanPresentSession::RenderFrame(const DrawSummary& draw,
                                       std::uint32_t width,
                                       std::uint32_t height, bool& presented,
                                       std::string& error) {
  presented = false;
  if (draw.draw_count != 1 || draw.triangle_count != 1) {
    error = "bootstrap extraction did not produce one triangle draw";
    return false;
  }
  if (width == 0 || height == 0) {
    return true;
  }
  if (swapchain_ == VK_NULL_HANDLE || width != extent_.width ||
      height != extent_.height) {
    if (!RecreateSwapchain(width, height, error)) {
      return false;
    }
    if (extent_.width == 0 || extent_.height == 0) {
      return true;
    }
  }

  if (!VulkanOk(vkWaitForFences(device_, 1, &in_flight_, VK_TRUE,
                                kFrameTimeoutNs),
                "vkWaitForFences", error)) {
    return false;
  }
  std::uint32_t image_index = 0;
  const VkResult acquire =
      vkAcquireNextImageKHR(device_, swapchain_, kFrameTimeoutNs,
                            image_available_, VK_NULL_HANDLE, &image_index);
  if (acquire == VK_ERROR_OUT_OF_DATE_KHR) {
    return RecreateSwapchain(width, height, error);
  }
  if (acquire != VK_SUCCESS && acquire != VK_SUBOPTIMAL_KHR) {
    return VulkanOk(acquire, "vkAcquireNextImageKHR", error);
  }
  if (!VulkanOk(vkResetFences(device_, 1, &in_flight_), "vkResetFences",
                error)) {
    return false;
  }

  if (!VulkanOk(vkResetCommandBuffer(command_, 0), "vkResetCommandBuffer",
                error)) {
    return false;
  }
  VkCommandBufferBeginInfo begin{VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};
  if (!VulkanOk(vkBeginCommandBuffer(command_, &begin), "vkBeginCommandBuffer",
                error)) {
    return false;
  }
  VkClearValue clear{};
  clear.color.float32[0] = 0.05F;
  clear.color.float32[1] = 0.10F;
  clear.color.float32[2] = 0.15F;
  clear.color.float32[3] = 1.0F;
  VkRenderPassBeginInfo render_begin{
      VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO};
  render_begin.renderPass = render_pass_;
  render_begin.framebuffer = framebuffers_[image_index];
  render_begin.renderArea.extent = extent_;
  render_begin.clearValueCount = 1;
  render_begin.pClearValues = &clear;
  vkCmdBeginRenderPass(command_, &render_begin, VK_SUBPASS_CONTENTS_INLINE);
  vkCmdBindPipeline(command_, VK_PIPELINE_BIND_POINT_GRAPHICS, pipeline_);
  const VkViewport viewport{0.0F,
                            0.0F,
                            static_cast<float>(extent_.width),
                            static_cast<float>(extent_.height),
                            0.0F,
                            1.0F};
  const VkRect2D scissor{{0, 0}, extent_};
  vkCmdSetViewport(command_, 0, 1, &viewport);
  vkCmdSetScissor(command_, 0, 1, &scissor);
  vkCmdDraw(command_, draw.triangle_count * 3U, 1, 0, 0);
  vkCmdEndRenderPass(command_);
  if (!VulkanOk(vkEndCommandBuffer(command_), "vkEndCommandBuffer", error)) {
    return false;
  }

  const VkPipelineStageFlags wait_stage =
      VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
  VkSubmitInfo submit{VK_STRUCTURE_TYPE_SUBMIT_INFO};
  submit.waitSemaphoreCount = 1;
  submit.pWaitSemaphores = &image_available_;
  submit.pWaitDstStageMask = &wait_stage;
  submit.commandBufferCount = 1;
  submit.pCommandBuffers = &command_;
  submit.signalSemaphoreCount = 1;
  submit.pSignalSemaphores = &render_finished_[image_index];
  if (!VulkanOk(vkQueueSubmit(queue_, 1, &submit, in_flight_),
                "vkQueueSubmit", error)) {
    return false;
  }

  VkPresentInfoKHR present{VK_STRUCTURE_TYPE_PRESENT_INFO_KHR};
  present.waitSemaphoreCount = 1;
  present.pWaitSemaphores = &render_finished_[image_index];
  present.swapchainCount = 1;
  present.pSwapchains = &swapchain_;
  present.pImageIndices = &image_index;
  const VkResult present_result = vkQueuePresentKHR(queue_, &present);
  if (present_result == VK_ERROR_OUT_OF_DATE_KHR ||
      present_result == VK_SUBOPTIMAL_KHR) {
    if (present_result == VK_SUBOPTIMAL_KHR) {
      ++statistics_.frames_presented;
      presented = true;
    }
    if (!RecreateSwapchain(width, height, error)) {
      return false;
    }
  } else if (!VulkanOk(present_result, "vkQueuePresentKHR", error)) {
    return false;
  } else {
    ++statistics_.frames_presented;
    presented = true;
  }
  statistics_.validation_message_count = validation_.message_count;
  if (!validation_.first_message.empty()) {
    statistics_.validation_detail = validation_.first_message;
  }
  return true;
}

void VulkanPresentSession::DestroySwapchainObjects() {
  for (VkSemaphore semaphore : render_finished_) {
    vkDestroySemaphore(device_, semaphore, nullptr);
  }
  render_finished_.clear();
  for (VkFramebuffer framebuffer : framebuffers_) {
    vkDestroyFramebuffer(device_, framebuffer, nullptr);
  }
  framebuffers_.clear();
  for (VkImageView view : views_) {
    vkDestroyImageView(device_, view, nullptr);
  }
  views_.clear();
  images_.clear();
  if (swapchain_ != VK_NULL_HANDLE) {
    vkDestroySwapchainKHR(device_, swapchain_, nullptr);
    swapchain_ = VK_NULL_HANDLE;
  }
  extent_ = {0, 0};
}

void VulkanPresentSession::Destroy() {
  if (device_ != VK_NULL_HANDLE) {
    vkDeviceWaitIdle(device_);
    DestroySwapchainObjects();
    vkDestroyFence(device_, in_flight_, nullptr);
    vkDestroySemaphore(device_, image_available_, nullptr);
    vkDestroyPipeline(device_, pipeline_, nullptr);
    vkDestroyPipelineLayout(device_, pipeline_layout_, nullptr);
    vkDestroyRenderPass(device_, render_pass_, nullptr);
    vkDestroyCommandPool(device_, command_pool_, nullptr);
    vkDestroyDevice(device_, nullptr);
    device_ = VK_NULL_HANDLE;
  }
  if (surface_ != VK_NULL_HANDLE && instance_.instance != VK_NULL_HANDLE) {
    vkDestroySurfaceKHR(instance_.instance, surface_, nullptr);
    surface_ = VK_NULL_HANDLE;
  }
  DestroyInstance(instance_);
}

}  // namespace

std::unique_ptr<PresentSession> CreatePresentSession(
    const PresentSurfaceProvider& surface, const std::string& vertex_shader,
    const std::string& fragment_shader, bool vsync, PresentSetupStatus& status,
    std::string& error) {
  auto session = std::make_unique<VulkanPresentSession>();
  status = session->Initialize(surface, vertex_shader, fragment_shader, vsync,
                               error);
  if (status != PresentSetupStatus::Ready) {
    return nullptr;
  }
  return session;
}

#endif

}  // namespace {{Name}}
