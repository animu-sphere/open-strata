// SPDX-License-Identifier: Apache-2.0
#include <pxr/pxr.h>

#include <pxr/base/plug/registry.h>
#include <pxr/base/tf/token.h>
#include <pxr/imaging/hd/pluginRenderDelegateUniqueHandle.h>
#include <pxr/imaging/hd/rendererPluginRegistry.h>

#include <iostream>
#include <string_view>

PXR_NAMESPACE_USING_DIRECTIVE

int main(int argc, char** argv) {
  if (argc != 3) {
    std::cerr << "usage: {{name}}-hydra2-probe "
                 "--discovery|--delegate <plugin-resource-directory>\n";
    return 2;
  }
  const std::string_view mode(argv[1]);
  if (mode != "--discovery" && mode != "--delegate") {
    std::cerr << "unknown probe mode: " << mode << '\n';
    return 2;
  }

  // RegisterPlugins returns only newly discovered plugins. An ambient
  // PXR_PLUGINPATH_NAME may have registered this module before main().
  (void)PlugRegistry::GetInstance().RegisterPlugins(argv[2]);
  const TfToken plugin_id("Hd{{Name}}RendererPlugin");
  if (!HdRendererPluginRegistry::GetInstance().IsRegisteredPlugin(plugin_id)) {
    std::cerr << plugin_id << " was not discovered\n";
    return 1;
  }
  if (mode == "--discovery") {
    std::cout << "renderer.plugin.discovery pass " << plugin_id << '\n';
    return 0;
  }

  auto delegate =
      HdRendererPluginRegistry::GetInstance().CreateRenderDelegate(plugin_id);
  if (!delegate) {
    std::cerr << plugin_id << " could not create a delegate\n";
    return 1;
  }
  std::cout << "renderer.delegate.creation pass " << plugin_id << '\n';
  return 0;
}
