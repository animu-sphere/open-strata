// SPDX-License-Identifier: Apache-2.0
// The window abstraction owned by the viewport adapter. GLFW is the native
// host boundary: its types stay inside window_glfw.cpp and never reach the
// core or backend targets. Input handling beyond close/resize/escape is an
// intentional project extension point.
#pragma once

#include <cstdint>
#include <memory>
#include <string_view>

#include <{{name}}/vulkan_present.hpp>

namespace {{Name}}::viewport {

enum class EventType {
  Close,
  Resize,
  KeyDown,
};

enum class Key {
  Unknown,
  Escape,
};

struct Event {
  EventType type = EventType::Close;
  Key key = Key::Unknown;
  std::uint32_t width = 0;
  std::uint32_t height = 0;
};

class Window {
 public:
  // Throws std::runtime_error when the windowing environment is unavailable
  // (no display, no Vulkan-capable GLFW); callers report that as a skip.
  static std::unique_ptr<Window> Create(std::string_view title,
                                        std::uint32_t width,
                                        std::uint32_t height, bool visible);
  virtual ~Window() = default;

  [[nodiscard]] virtual bool PollEvent(Event& event) = 0;
  virtual void WaitForEvent() = 0;
  virtual void SetTitle(std::string_view title) = 0;
  [[nodiscard]] virtual std::uint32_t width() const noexcept = 0;
  [[nodiscard]] virtual std::uint32_t height() const noexcept = 0;
};

// Bundles the GLFW-required instance extensions and the surface-creation
// callback for the backend-owned VkSurfaceKHR. The window must outlive every
// session created from the returned provider.
[[nodiscard]] PresentSurfaceProvider MakeSurfaceProvider(Window& window);

}  // namespace {{Name}}::viewport
