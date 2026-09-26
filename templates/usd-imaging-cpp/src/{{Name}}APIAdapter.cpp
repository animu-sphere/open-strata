// SPDX-License-Identifier: Apache-2.0
#include "pxr/base/tf/registryManager.h"
#include "pxr/base/tf/type.h"
#include "pxr/usdImaging/usdImaging/apiSchemaAdapter.h"

PXR_NAMESPACE_OPEN_SCOPE

// Registration skeleton. Override GetImagingSubprimData and
// InvalidateImagingSubprim to expose the schema's data to scene-index hosts.
// Do not include hd/retainedDataSource.h: OpenUSD 26.08's specialization
// constructor is rejected by GCC 13 in C++20. Use custom data sources instead.
class {{Name}}APIAdapter : public UsdImagingAPISchemaAdapter {
public:
    ~{{Name}}APIAdapter() override = default;
};

TF_REGISTRY_FUNCTION(TfType) {
    TfType type = TfType::Define<{{Name}}APIAdapter,
        TfType::Bases<UsdImagingAPISchemaAdapter>>();
    type.SetFactory<UsdImagingAPISchemaAdapterFactory<{{Name}}APIAdapter>>();
}

PXR_NAMESPACE_CLOSE_SCOPE
