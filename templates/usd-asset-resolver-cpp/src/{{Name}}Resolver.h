// SPDX-License-Identifier: Apache-2.0
#pragma once

#include "pxr/pxr.h"
#include "pxr/usd/ar/resolver.h"
#include "pxr/usd/ar/resolvedPath.h"

#include <memory>
#include <string>

PXR_NAMESPACE_OPEN_SCOPE

class ArAsset;
class ArWritableAsset;

/// Minimal, read-only URI resolver for the `{{scheme}}` scheme.
class {{Name}}Resolver final : public ArResolver {
public:
    {{Name}}Resolver() = default;
    ~{{Name}}Resolver() override = default;

protected:
    std::string _CreateIdentifier(
        const std::string& assetPath,
        const ArResolvedPath& anchorAssetPath) const override;
    std::string _CreateIdentifierForNewAsset(
        const std::string& assetPath,
        const ArResolvedPath& anchorAssetPath) const override;
    ArResolvedPath _Resolve(const std::string& assetPath) const override;
    ArResolvedPath _ResolveForNewAsset(const std::string& assetPath) const override;
    std::shared_ptr<ArAsset> _OpenAsset(const ArResolvedPath& resolvedPath) const override;
    std::shared_ptr<ArWritableAsset> _OpenAssetForWrite(
        const ArResolvedPath& resolvedPath,
        WriteMode writeMode) const override;
    bool _CanWriteAssetToPath(
        const ArResolvedPath& resolvedPath,
        std::string* whyNot) const override;
};

PXR_NAMESPACE_CLOSE_SCOPE
