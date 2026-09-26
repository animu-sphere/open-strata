// SPDX-License-Identifier: Apache-2.0
// Built against the selected runtime SDK, never against the plugin itself.
#include "pxr/base/plug/registry.h"
#include "pxr/base/tf/token.h"
#include "pxr/usd/usd/schemaRegistry.h"
#include "pxr/usdImaging/usdImaging/adapterRegistry.h"
#include <iostream>
#include <string>

PXR_NAMESPACE_USING_DIRECTIVE

int main(int argc, char** argv) {
    if (argc < 3) {
        std::cerr << "expected a plugInfo root and adapter keys\n";
        return 2;
    }
    if (!UsdImagingAdapterRegistry::AreExternalPluginsEnabled()) {
        std::cerr << "USDIMAGING_ENABLE_PLUGINS=0 disables external adapters\n";
        return 3;
    }
    PlugRegistry::GetInstance().RegisterPlugins(argv[1]);
    auto& registry = UsdImagingAdapterRegistry::GetInstance();
    auto& schemas = UsdSchemaRegistry::GetInstance();
    for (int i = 2; i < argc; ++i) {
        const std::string key(argv[i]);
        const bool api = key.rfind("usd-imaging:", 0) == 0;
        const bool prim = key.rfind("usd-imaging-prim:", 0) == 0;
        if (!api && !prim) {
            std::cerr << "invalid adapter key: " << key << '\n';
            return 4;
        }
        const TfToken name(key.substr(api ? 12 : 17));
        if (api && !schemas.FindAppliedAPIPrimDefinition(name)) {
            std::cerr << "adapted API schema is absent from the session: " << name
                      << "; declare its provider in requires.bundles\n";
            return 5;
        }
        const bool registered = api ? registry.HasAPISchemaAdapter(name) : registry.HasAdapter(name);
        if (!registered) {
            std::cerr << "adapter missing from UsdImagingAdapterRegistry: " << key << '\n';
            return 6;
        }
        const bool constructed = api ? bool(registry.ConstructAPISchemaAdapter(name))
                                     : bool(registry.ConstructAdapter(name));
        if (!constructed) {
            std::cerr << "adapter factory failed: " << key << '\n';
            return 7;
        }
        std::cout << "PASS " << key << '\n';
    }
    return 0;
}
