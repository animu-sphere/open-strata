// SPDX-License-Identifier: Apache-2.0
#pragma once

#include "pxr/pxr.h"
#include "pxr/base/tf/staticTokens.h"
#include "pxr/usd/sdf/fileFormat.h"

PXR_NAMESPACE_OPEN_SCOPE

// The tokens that identify this file format to USD's Sdf layer registry.
#define {{NAME}}_FILE_FORMAT_TOKENS \
    ((Id, "{{extension}}"))         \
    ((Version, "1.0"))              \
    ((Target, "usd"))              \
    ((Extension, "{{extension}}"))

TF_DECLARE_PUBLIC_TOKENS({{Name}}FileFormatTokens, {{NAME}}_FILE_FORMAT_TOKENS);

/// A minimal SdfFileFormat that reads `.{{extension}}` files and translates them
/// into USD. Replace the body of `Read` with your format's parser.
class {{Name}}FileFormat : public SdfFileFormat {
public:
    bool CanRead(const std::string& file) const override;
    bool Read(SdfLayer* layer, const std::string& resolvedPath, bool metadataOnly) const override;
    bool WriteToString(
        const SdfLayer& layer,
        std::string* str,
        const std::string& comment = std::string()) const override;

protected:
    SDF_FILE_FORMAT_FACTORY_ACCESS;

    {{Name}}FileFormat();
    ~{{Name}}FileFormat() override;
};

PXR_NAMESPACE_CLOSE_SCOPE
