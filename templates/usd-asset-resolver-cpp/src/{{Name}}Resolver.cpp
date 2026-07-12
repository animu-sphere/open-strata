// SPDX-License-Identifier: Apache-2.0
#include "{{Name}}Resolver.h"

#include "pxr/usd/ar/defineResolver.h"
#include "pxr/usd/ar/filesystemAsset.h"

#include <filesystem>

PXR_NAMESPACE_OPEN_SCOPE

AR_DEFINE_RESOLVER({{Name}}Resolver, ArResolver);

namespace {

constexpr const char* SchemePrefix = "{{scheme}}:";

std::string
StripScheme(const std::string& assetPath)
{
    if (assetPath.rfind(SchemePrefix, 0) != 0) {
        return {};
    }
    std::string path = assetPath.substr(std::char_traits<char>::length(SchemePrefix));
    if (path.rfind("//", 0) == 0) {
        path.erase(0, 2);
    }
    return path;
}

std::filesystem::path
NormalizeLocalPath(const std::string& assetPath, const ArResolvedPath& anchor)
{
    std::filesystem::path path(StripScheme(assetPath));
    if (path.empty()) {
        return {};
    }
    if (path.is_relative() && !anchor.empty()) {
        path = std::filesystem::path(anchor.GetPathString()).parent_path() / path;
    }
    return path.lexically_normal();
}

std::string
CreateIdentifier(const std::string& assetPath, const ArResolvedPath& anchor)
{
    const std::filesystem::path path = NormalizeLocalPath(assetPath, anchor);
    return path.empty() ? std::string() : std::string(SchemePrefix) + path.generic_string();
}

} // namespace

std::string
{{Name}}Resolver::_CreateIdentifier(
    const std::string& assetPath,
    const ArResolvedPath& anchorAssetPath) const
{
    return CreateIdentifier(assetPath, anchorAssetPath);
}

std::string
{{Name}}Resolver::_CreateIdentifierForNewAsset(
    const std::string& assetPath,
    const ArResolvedPath& anchorAssetPath) const
{
    return CreateIdentifier(assetPath, anchorAssetPath);
}

ArResolvedPath
{{Name}}Resolver::_Resolve(const std::string& assetPath) const
{
    std::filesystem::path path = NormalizeLocalPath(assetPath, ArResolvedPath());
    if (path.empty()) {
        return {};
    }
    if (path.is_relative()) {
        path = std::filesystem::absolute(path);
    }
    path = path.lexically_normal();
    return std::filesystem::is_regular_file(path)
        ? ArResolvedPath(path.generic_string())
        : ArResolvedPath();
}

ArResolvedPath
{{Name}}Resolver::_ResolveForNewAsset(const std::string& assetPath) const
{
    (void)assetPath;
    return {};
}

std::shared_ptr<ArAsset>
{{Name}}Resolver::_OpenAsset(const ArResolvedPath& resolvedPath) const
{
    return ArFilesystemAsset::Open(resolvedPath);
}

std::shared_ptr<ArWritableAsset>
{{Name}}Resolver::_OpenAssetForWrite(
    const ArResolvedPath& resolvedPath,
    WriteMode writeMode) const
{
    (void)resolvedPath;
    (void)writeMode;
    return {};
}

bool
{{Name}}Resolver::_CanWriteAssetToPath(
    const ArResolvedPath& resolvedPath,
    std::string* whyNot) const
{
    (void)resolvedPath;
    if (whyNot) {
        *whyNot = "{{Name}}Resolver is a read-only skeleton";
    }
    return false;
}

PXR_NAMESPACE_CLOSE_SCOPE
