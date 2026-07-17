// SPDX-License-Identifier: Apache-2.0
// Standalone viewport host for the project bootstrap draw. `ost renderer
// viewport` builds and launches this executable; it also runs headless-style
// as a GPU smoke test (`--hidden --frames N`). Exit codes: 0 success, 1
// failure, 77 skip (the environment cannot present).
#include "window.hpp"

#include <{{name}}/extraction.hpp>
#include <{{name}}/render_world.hpp>
#include <{{name}}/vulkan_present.hpp>

#include <chrono>
#include <cstdint>
#include <filesystem>
#include <iostream>
#include <sstream>
#include <stdexcept>
#include <string>
#include <string_view>

namespace {

constexpr int kExitSkip = 77;

using Clock = std::chrono::steady_clock;

struct Arguments {
  std::uint32_t width = 1280;
  std::uint32_t height = 720;
  std::uint64_t frame_limit = 0;
  bool visible = true;
  bool vsync = true;
};

std::uint64_t ReadUnsigned(std::string_view value, std::string_view name) {
  // std::stoull would accept and wrap a leading minus sign.
  if (value.empty() || value.front() < '0' || value.front() > '9') {
    throw std::invalid_argument(std::string(name) +
                                " must be a non-negative integer");
  }
  std::size_t consumed = 0;
  std::uint64_t result = 0;
  try {
    result = std::stoull(std::string(value), &consumed);
  } catch (const std::exception&) {
    throw std::invalid_argument(std::string(name) +
                                " must be a non-negative integer");
  }
  if (consumed != value.size()) {
    throw std::invalid_argument(std::string(name) +
                                " must be a non-negative integer");
  }
  return result;
}

Arguments ParseArguments(int argc, char** argv) {
  Arguments result;
  for (int index = 1; index < argc; ++index) {
    const std::string_view option(argv[index]);
    const auto next = [&]() -> std::string_view {
      if (++index >= argc) {
        throw std::invalid_argument(std::string(option) + " requires a value");
      }
      return argv[index];
    };
    if (option == "--width") {
      result.width = static_cast<std::uint32_t>(ReadUnsigned(next(), option));
    } else if (option == "--height") {
      result.height = static_cast<std::uint32_t>(ReadUnsigned(next(), option));
    } else if (option == "--frames") {
      result.frame_limit = ReadUnsigned(next(), option);
    } else if (option == "--hidden") {
      result.visible = false;
    } else if (option == "--vsync") {
      const auto value = next();
      if (value != "on" && value != "off") {
        throw std::invalid_argument("--vsync must be on or off");
      }
      result.vsync = value == "on";
    } else if (option == "--help") {
      std::cout << "Usage: {{name}}-viewport [options]\n"
                   "  --width N --height N     window size (default 1280x720)\n"
                   "  --frames N               exit after N presented frames\n"
                   "  --vsync on|off           FIFO or immediate present\n"
                   "  --hidden                 do not show the window\n"
                   "Esc or closing the window exits.\n";
      std::exit(0);
    } else {
      throw std::invalid_argument("unknown option: " + std::string(option));
    }
  }
  if (result.width == 0 || result.height == 0) {
    throw std::invalid_argument("viewport extent must be non-zero");
  }
  return result;
}

std::string WindowTitle(std::string_view device, std::uint32_t width,
                        std::uint32_t height, std::uint64_t frames) {
  std::ostringstream title;
  title << "{{name}}-viewport | " << device << " | " << width << 'x' << height
        << " | " << frames << " frames";
  return title.str();
}

}  // namespace

int main(int argc, char** argv) {
  try {
    const auto arguments = ParseArguments(argc, argv);

    {{Name}}::RenderWorld world;
    world.SetBootstrapTriangle();
    const {{Name}}::FrameSnapshot snapshot = world.Commit();
    const {{Name}}::DrawSummary draw = {{Name}}::ExtractDrawSummary(snapshot);

    std::unique_ptr<{{Name}}::viewport::Window> window;
    {{Name}}::PresentSurfaceProvider provider;
    try {
      window = {{Name}}::viewport::Window::Create(
          "{{name}}-viewport", arguments.width, arguments.height,
          arguments.visible);
      provider = {{Name}}::viewport::MakeSurfaceProvider(*window);
    } catch (const std::exception& environment) {
      std::cerr << "{{name}}-viewport: skip: " << environment.what() << '\n';
      return kExitSkip;
    }

    const auto shader_directory =
        std::filesystem::absolute(argv[0]).parent_path() / "shaders";
    {{Name}}::PresentSetupStatus status = {{Name}}::PresentSetupStatus::Error;
    std::string error;
    auto session = {{Name}}::CreatePresentSession(
        provider, (shader_directory / "triangle.vert.spv").string(),
        (shader_directory / "triangle.frag.spv").string(), arguments.vsync,
        status, error);
    if (session == nullptr) {
      if (status == {{Name}}::PresentSetupStatus::Unavailable) {
        std::cerr << "{{name}}-viewport: skip: " << error << '\n';
        return kExitSkip;
      }
      std::cerr << "{{name}}-viewport: " << error << '\n';
      return 1;
    }
    std::cout << "Presenting on: " << session->statistics().device_name
              << '\n';

    bool running = true;
    auto title_update = Clock::now();
    while (running && (arguments.frame_limit == 0 ||
                       session->statistics().frames_presented <
                           arguments.frame_limit)) {
      {{Name}}::viewport::Event event;
      while (window->PollEvent(event)) {
        switch (event.type) {
          case {{Name}}::viewport::EventType::Close:
            running = false;
            break;
          case {{Name}}::viewport::EventType::KeyDown:
            if (event.key == {{Name}}::viewport::Key::Escape) {
              running = false;
            }
            break;
          case {{Name}}::viewport::EventType::Resize:
            break;
        }
      }
      if (!running) {
        break;
      }
      const std::uint32_t width = window->width();
      const std::uint32_t height = window->height();
      if (width == 0 || height == 0) {
        window->WaitForEvent();
        continue;
      }
      bool presented = false;
      if (!session->RenderFrame(draw, width, height, presented, error)) {
        std::cerr << "{{name}}-viewport: " << error << '\n';
        return 1;
      }
      const auto now = Clock::now();
      if (now - title_update >= std::chrono::milliseconds(250)) {
        window->SetTitle(WindowTitle(session->statistics().device_name, width,
                                     height,
                                     session->statistics().frames_presented));
        title_update = now;
      }
    }

    const {{Name}}::PresentStatistics& statistics = session->statistics();
    if (statistics.validation_message_count != 0) {
      std::cerr << "{{name}}-viewport: Vulkan validation reported "
                << statistics.validation_message_count
                << " message(s): " << statistics.validation_detail << '\n';
      return 1;
    }
    if (arguments.frame_limit != 0 &&
        statistics.frames_presented < arguments.frame_limit) {
      std::cerr << "{{name}}-viewport: presented "
                << statistics.frames_presented << " of "
                << arguments.frame_limit << " requested frames\n";
      return 1;
    }
    std::cout << "Presented " << statistics.frames_presented << " frames on "
              << statistics.device_name
              << " (swapchain recreates: " << statistics.swapchain_recreates
              << ")\n";
    return 0;
  } catch (const std::exception& fatal) {
    std::cerr << "{{name}}-viewport: " << fatal.what() << '\n';
    return 1;
  }
}
