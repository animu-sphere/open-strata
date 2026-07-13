// SPDX-License-Identifier: Apache-2.0
#include "adapter.hpp"

#include <pxr/pxr.h>

#include <pxr/base/tf/registryManager.h>
#include <pxr/imaging/hd/rendererCreateArgs.h>
#include <pxr/imaging/hd/rendererPlugin.h>
#include <pxr/imaging/hd/rendererPluginRegistry.h>

#include <string>

PXR_NAMESPACE_OPEN_SCOPE

class Hd{{Name}}RendererPlugin final : public HdRendererPlugin {
 public:
  bool IsSupported(const HdRendererCreateArgs& args,
                   std::string* reason_why_not) const override {
    if (!args.gpuEnabled) {
      if (reason_why_not != nullptr) {
        *reason_why_not = "{{Name}} requires a Vulkan-capable GPU";
      }
      return false;
    }
    return true;
  }

  HdRenderDelegate* CreateRenderDelegate() override {
    return new Hd{{Name}}RenderDelegate;
  }

  HdRenderDelegate* CreateRenderDelegate(
      const HdRenderSettingsMap& settings) override {
    return new Hd{{Name}}RenderDelegate(settings);
  }

  void DeleteRenderDelegate(HdRenderDelegate* delegate) override {
    delete delegate;
  }
};

TF_REGISTRY_FUNCTION(TfType) {
  HdRendererPluginRegistry::Define<Hd{{Name}}RendererPlugin>();
}

PXR_NAMESPACE_CLOSE_SCOPE
