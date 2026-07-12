//
// Copyright 2016 Pixar
//
// Licensed under the terms set forth in the LICENSE.txt file available at
// https://openusd.org/license.
//
#include "./contractAPI.h"
#include "pxr/usd/usd/schemaRegistry.h"
#include "pxr/usd/usd/typed.h"

#include "pxr/usd/sdf/types.h"
#include "pxr/usd/sdf/assetPath.h"

PXR_NAMESPACE_OPEN_SCOPE

// Register the schema with the TfType system.
TF_REGISTRY_FUNCTION(TfType)
{
    TfType::Define<{{Name}}ContractAPI,
        TfType::Bases< UsdAPISchemaBase > >();
    
}

/* virtual */
{{Name}}ContractAPI::~{{Name}}ContractAPI()
{
}

/* static */
{{Name}}ContractAPI
{{Name}}ContractAPI::Get(const UsdStagePtr &stage, const SdfPath &path)
{
    if (!stage) {
        TF_CODING_ERROR("Invalid stage");
        return {{Name}}ContractAPI();
    }
    return {{Name}}ContractAPI(stage->GetPrimAtPath(path));
}


/* virtual */
UsdSchemaKind {{Name}}ContractAPI::_GetSchemaKind() const
{
    return {{Name}}ContractAPI::schemaKind;
}

/* static */
bool
{{Name}}ContractAPI::CanApply(
    const UsdPrim &prim, std::string *whyNot)
{
    return prim.CanApplyAPI<{{Name}}ContractAPI>(whyNot);
}

/* static */
{{Name}}ContractAPI
{{Name}}ContractAPI::Apply(const UsdPrim &prim)
{
    if (prim.ApplyAPI<{{Name}}ContractAPI>()) {
        return {{Name}}ContractAPI(prim);
    }
    return {{Name}}ContractAPI();
}

/* static */
const TfType &
{{Name}}ContractAPI::_GetStaticTfType()
{
    static TfType tfType = TfType::Find<{{Name}}ContractAPI>();
    return tfType;
}

/* static */
bool 
{{Name}}ContractAPI::_IsTypedSchema()
{
    static bool isTyped = _GetStaticTfType().IsA<UsdTyped>();
    return isTyped;
}

/* virtual */
const TfType &
{{Name}}ContractAPI::_GetTfType() const
{
    return _GetStaticTfType();
}

UsdAttribute
{{Name}}ContractAPI::Get{{Ident}}ExampleAttr() const
{
    return GetPrim().GetAttribute({{Name}}Tokens->{{ident}}Example);
}

UsdAttribute
{{Name}}ContractAPI::Create{{Ident}}ExampleAttr(VtValue const &defaultValue, bool writeSparsely) const
{
    return UsdSchemaBase::_CreateAttr({{Name}}Tokens->{{ident}}Example,
                       SdfValueTypeNames->Token,
                       /* custom = */ false,
                       SdfVariabilityUniform,
                       defaultValue,
                       writeSparsely);
}

namespace {
static inline TfTokenVector
_ConcatenateAttributeNames(const TfTokenVector& left,const TfTokenVector& right)
{
    TfTokenVector result;
    result.reserve(left.size() + right.size());
    result.insert(result.end(), left.begin(), left.end());
    result.insert(result.end(), right.begin(), right.end());
    return result;
}
}

/*static*/
const TfTokenVector&
{{Name}}ContractAPI::GetSchemaAttributeNames(bool includeInherited)
{
    static TfTokenVector localNames = {
        {{Name}}Tokens->{{ident}}Example,
    };
    static TfTokenVector allNames =
        _ConcatenateAttributeNames(
            UsdAPISchemaBase::GetSchemaAttributeNames(true),
            localNames);

    if (includeInherited)
        return allNames;
    else
        return localNames;
}

PXR_NAMESPACE_CLOSE_SCOPE

// ===================================================================== //
// Feel free to add custom code below this line. It will be preserved by
// the code generator.
//
// Just remember to wrap code in the appropriate delimiters:
// 'PXR_NAMESPACE_OPEN_SCOPE', 'PXR_NAMESPACE_CLOSE_SCOPE'.
// ===================================================================== //
// --(BEGIN CUSTOM CODE)--
