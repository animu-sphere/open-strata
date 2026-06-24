// SPDX-License-Identifier: Apache-2.0
//
// Minimal OpenUSD plugin translation unit. Plugin types are declared to USD via
// plugin/resources/plugInfo.json and registered through the plug/Tf machinery;
// add your schema, file format, or scene-index registrations below.

#include <pxr/pxr.h>
#include <pxr/base/tf/registryManager.h>

PXR_NAMESPACE_USING_DIRECTIVE

TF_REGISTRY_FUNCTION(TfType) {
    // Register your plugin's TfTypes here, e.g.:
    //   TfType::Define<{{Name}}, TfType::Bases<...>>();
}
