// SPDX-License-Identifier: Apache-2.0
#pragma once

#include "pxr/pxr.h"
#include "pxr/usd/ar/packageResolver.h"

#include <memory>
#include <string>

PXR_NAMESPACE_OPEN_SCOPE

class ArAsset;

/// Minimal, read-only package resolver for `.{{extension}}` packages.
///
/// The starter maps `<pkg>.{{extension}}[<entry>]` onto a sidecar directory
/// `<pkg>.{{extension}}.contents/<entry>` so registration, extension dispatch,
/// entry lookup, and asset opening are verifiable end-to-end before any real
/// container format exists. Replace the sidecar mapping with the actual
/// container's entry table and bounded random access.
class {{Name}}PackageResolver final : public ArPackageResolver {
public:
    {{Name}}PackageResolver() = default;
    ~{{Name}}PackageResolver() override = default;

    std::string Resolve(
        const std::string& resolvedPackagePath,
        const std::string& packagedPath) override;

    std::shared_ptr<ArAsset> OpenAsset(
        const std::string& resolvedPackagePath,
        const std::string& resolvedPackagedPath) override;

    void BeginCacheScope(VtValue* cacheScopeData) override;
    void EndCacheScope(VtValue* cacheScopeData) override;
};

PXR_NAMESPACE_CLOSE_SCOPE
