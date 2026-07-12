// SPDX-License-Identifier: Apache-2.0
#include "{{Name}}PackageResolver.h"

#include "pxr/usd/ar/definePackageResolver.h"
#include "pxr/usd/ar/filesystemAsset.h"
#include "pxr/usd/ar/resolvedPath.h"

#include <filesystem>

PXR_NAMESPACE_OPEN_SCOPE

AR_DEFINE_PACKAGE_RESOLVER({{Name}}PackageResolver, ArPackageResolver);

namespace {

/// Placeholder container backing: entries of `foo.{{extension}}` live in the
/// sidecar directory `foo.{{extension}}.contents/`. A real implementation
/// replaces this with the container's entry table and bounded reads.
constexpr const char* ContentsSuffix = ".contents";

/// The entry-path guard: an entry path must stay inside the package, whatever
/// the backing container is. Reject empty, absolute, and rooted paths, and any
/// `..` segment that survives normalization, so a hostile reference string like
/// `pkg[../../secret]` cannot traverse out via the entry path. This guards the
/// path string only; with the sidecar backing it does not stop a symlink under
/// `.contents/` — a real container reads a self-contained byte range instead.
bool
IsSafeEntryPath(const std::filesystem::path& entry)
{
    if (entry.empty() || entry.is_absolute() || entry.has_root_name() ||
        entry.has_root_directory()) {
        return false;
    }
    for (const auto& part : entry) {
        if (part == "..") {
            return false;
        }
    }
    return true;
}

/// Where `packagedPath` lives on disk, or empty when the entry path is
/// unsafe. This is the entry-lookup seam a real container replaces.
std::filesystem::path
EntryLocation(const std::string& resolvedPackagePath, const std::string& packagedPath)
{
    const std::filesystem::path entry =
        std::filesystem::path(packagedPath).lexically_normal();
    if (!IsSafeEntryPath(entry)) {
        return {};
    }
    return std::filesystem::path(resolvedPackagePath + ContentsSuffix) / entry;
}

} // namespace

std::string
{{Name}}PackageResolver::Resolve(
    const std::string& resolvedPackagePath,
    const std::string& packagedPath)
{
    const std::filesystem::path location =
        EntryLocation(resolvedPackagePath, packagedPath);
    return !location.empty() && std::filesystem::is_regular_file(location)
        ? packagedPath
        : std::string();
}

std::shared_ptr<ArAsset>
{{Name}}PackageResolver::OpenAsset(
    const std::string& resolvedPackagePath,
    const std::string& resolvedPackagedPath)
{
    const std::filesystem::path location =
        EntryLocation(resolvedPackagePath, resolvedPackagedPath);
    if (location.empty()) {
        return nullptr;
    }
    return ArFilesystemAsset::Open(ArResolvedPath(location.generic_string()));
}

void
{{Name}}PackageResolver::BeginCacheScope(VtValue* cacheScopeData)
{
    (void)cacheScopeData;
}

void
{{Name}}PackageResolver::EndCacheScope(VtValue* cacheScopeData)
{
    (void)cacheScopeData;
}

PXR_NAMESPACE_CLOSE_SCOPE
