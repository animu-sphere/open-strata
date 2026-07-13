// SPDX-License-Identifier: Apache-2.0
#include <{{name}}/extraction.hpp>
#include <{{name}}/render_world.hpp>
#include <{{name}}/vulkan_backend.hpp>

#include <algorithm>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>
#include <string_view>
#include <vector>

namespace {

struct Check {
  std::string id;
  std::string status;
  std::string detail;
};

std::string Escape(std::string_view value) {
  std::string escaped;
  for (const char character : value) {
    switch (character) {
      case '\\': escaped += "\\\\"; break;
      case '"': escaped += "\\\""; break;
      case '\n': escaped += "\\n"; break;
      case '\r': escaped += "\\r"; break;
      case '\t': escaped += "\\t"; break;
      default: escaped += character; break;
    }
  }
  return escaped;
}

bool WriteReport(const std::string& path,
                 const std::vector<Check>& checks,
                 const {{Name}}::GpuFrameEvidence& frame) {
  std::ofstream output(path, std::ios::binary | std::ios::trunc);
  if (!output) {
    return false;
  }
  output << "{\n"
         << "  \"schema\": \"openstrata.renderer-report/v1alpha1\",\n"
         << "  \"renderer\": {\"name\": \"{{name}}\"},\n";
  if (!frame.device_name.empty()) {
    output << "  \"device\": {\"backend\":\"vulkan\",\"name\":\""
           << Escape(frame.device_name) << "\",\"api_version\":\""
           << Escape(frame.api_version) << "\",\"driver_version\":\""
           << Escape(frame.driver_version) << "\",\"vendor_id\":"
           << frame.vendor_id << ",\"device_id\":" << frame.device_id << "},\n";
  }
  output << "  \"checks\": [\n";
  for (std::size_t index = 0; index < checks.size(); ++index) {
    const Check& check = checks[index];
    output << "    {\"id\":\"" << Escape(check.id) << "\",\"status\":\""
           << Escape(check.status) << "\"";
    if (!check.detail.empty()) {
      output << ",\"detail\":\"" << Escape(check.detail) << "\"";
    }
    output << '}' << (index + 1 == checks.size() ? "\n" : ",\n");
  }
  output << "  ]\n}\n";
  return output.good();
}

std::string Status({{Name}}::FrameStatus status) {
  switch (status) {
    case {{Name}}::FrameStatus::Pass: return "pass";
    case {{Name}}::FrameStatus::Fail: return "fail";
    case {{Name}}::FrameStatus::Skip: return "skip";
  }
  return "fail";
}

}  // namespace

int main(int argc, char** argv) {
  std::string report_path = "renderer-report.json";
  bool install_tree = false;
  for (int index = 1; index < argc; ++index) {
    const std::string_view argument(argv[index]);
    if (argument == "--report" && index + 1 < argc) {
      report_path = argv[++index];
    } else if (argument == "--install-tree") {
      install_tree = true;
    } else {
      std::cerr << "usage: {{name}}-headless [--report <path>] [--install-tree]\n";
      return 2;
    }
  }

  {{Name}}::RenderWorld world;
  world.SetBootstrapTriangle();
  const {{Name}}::FrameSnapshot first = world.Commit();
  const {{Name}}::FrameSnapshot unchanged = world.Commit();
  const {{Name}}::DrawSummary draw = {{Name}}::ExtractDrawSummary(first);
  const bool core_ok = first.revision == 1 && unchanged.revision == first.revision &&
                       draw.draw_count == 1 && draw.triangle_count == 1;

  const {{Name}}::BackendCapability capability = {{Name}}::ProbeVulkanBackend();
  const std::filesystem::path shader_directory =
      std::filesystem::absolute(argv[0]).parent_path() / "shaders";
  const {{Name}}::GpuFrameEvidence frame = {{Name}}::RenderOffscreen(
      draw, (shader_directory / "triangle.vert.spv").string(),
      (shader_directory / "triangle.frag.spv").string(), 1000);

  bool color_ok = false;
  bool depth_ok = false;
  bool persistence_ok = false;
  if (frame.status == {{Name}}::FrameStatus::Pass) {
    const std::size_t center =
        (frame.color.height / 2U) * frame.color.row_pitch +
        (frame.color.width / 2U) * 4U;
    color_ok = frame.color.width == 64 && frame.color.height == 64 &&
               frame.color.row_pitch == 64U * 4U &&
               frame.color.pixel_format == "rgba8-unorm" &&
               frame.color.origin == "top-left" &&
               frame.color.color_space == "linear" &&
               frame.color.payload.size() == 64U * 64U * 4U &&
               center + 3U < frame.color.payload.size() &&
               frame.color.payload[center] > 150U &&
               frame.color.payload[center + 1U] < 100U &&
               frame.color.payload[center + 2U] < 80U &&
               frame.color.payload[center + 3U] > 240U;

    const std::size_t depth_center =
        (frame.depth.height / 2U) * frame.depth.width + frame.depth.width / 2U;
    depth_ok = frame.depth.width == 64 && frame.depth.height == 64 &&
               frame.depth.row_pitch == 64U * sizeof(float) &&
               frame.depth.pixel_format == "d32-sfloat" &&
               frame.depth.origin == "top-left" &&
               frame.depth.payload.size() == 64U * 64U &&
               depth_center < frame.depth.payload.size() &&
               frame.depth.payload[depth_center] > 0.0F &&
               frame.depth.payload[depth_center] < 0.9F &&
               frame.depth.payload.front() > 0.99F;
    persistence_ok = frame.frames_rendered == 1000 && frame.completion == 1000;
  }

  std::vector<Check> checks;
  checks.push_back({"renderer.core.boundary", core_ok ? "pass" : "fail",
                    core_ok ? "" : "commit/extraction contract mismatch"});
  checks.push_back({"renderer.backend.capability",
                    capability.available ? "pass" : "skip", capability.detail});
  checks.push_back({"renderer.gpu.frame", Status(frame.status), frame.detail});
  if (frame.validation_available) {
    checks.push_back({"renderer.validation.messages",
                      frame.validation_message_count == 0 ? "pass" : "fail",
                      frame.validation_message_count == 0
                          ? ""
                          : frame.validation_detail});
  } else {
    checks.push_back({"renderer.validation.messages", "skip",
                      frame.validation_detail.empty()
                          ? "Vulkan validation capture was unavailable"
                          : frame.validation_detail});
  }
  if (frame.status == {{Name}}::FrameStatus::Pass) {
    checks.push_back({"renderer.render_product.color", color_ok ? "pass" : "fail",
                      color_ok ? "" : "RGBA8 metadata or center pixel mismatch"});
    checks.push_back({"renderer.render_product.depth", depth_ok ? "pass" : "fail",
                      depth_ok ? "" : "depth metadata or numeric payload mismatch"});
    checks.push_back({"renderer.frame.persistence",
                      persistence_ok ? "pass" : "fail",
                      persistence_ok ? "" : "1,000-frame completion count mismatch"});
  } else {
    const std::string dependent = "renderer.gpu.frame did not pass: " + frame.detail;
    checks.push_back({"renderer.render_product.color", "skip", dependent});
    checks.push_back({"renderer.render_product.depth", "skip", dependent});
    checks.push_back({"renderer.frame.persistence", "skip", dependent});
  }
  checks.push_back({"renderer.install_tree", install_tree ? "pass" : "skip",
                    install_tree ? "" : "run the renderer install-tree CTest"});

  if (!WriteReport(report_path, checks, frame)) {
    std::cerr << "cannot write renderer report: " << report_path << '\n';
    return 1;
  }
  const bool failed = std::any_of(checks.begin(), checks.end(),
                                  [](const Check& check) {
                                    return check.status == "fail";
                                  });
  return failed ? 1 : 0;
}
