//
// Copyright 2016 Pixar
//
// Licensed under the terms set forth in the LICENSE.txt file available at
// https://openusd.org/license.
//
#ifndef {{NAME}}_GENERATED_CONTRACTAPI_H
#define {{NAME}}_GENERATED_CONTRACTAPI_H

/// \file {{Name}}/contractAPI.h

#include "pxr/pxr.h"
#include "./api.h"
#include "pxr/usd/usd/apiSchemaBase.h"
#include "pxr/usd/usd/prim.h"
#include "pxr/usd/usd/stage.h"
#include "./tokens.h"

#include "pxr/base/vt/value.h"

#include "pxr/base/gf/vec3d.h"
#include "pxr/base/gf/vec3f.h"
#include "pxr/base/gf/matrix4d.h"

#include "pxr/base/tf/token.h"
#include "pxr/base/tf/type.h"

PXR_NAMESPACE_OPEN_SCOPE

class SdfAssetPath;

// -------------------------------------------------------------------------- //
// {{NAME}}CONTRACTAPI                                                       //
// -------------------------------------------------------------------------- //

/// \class {{Name}}ContractAPI
///
/// A compiled single-apply API schema. Replace the example property
/// with the authored-data contract for this bundle.
///
/// For any described attribute \em Fallback \em Value or \em Allowed \em Values below
/// that are text/tokens, the actual token is published and defined in \ref {{Name}}Tokens.
/// So to set an attribute to the value "rightHanded", use {{Name}}Tokens->rightHanded
/// as the value.
///
class {{Name}}ContractAPI : public UsdAPISchemaBase
{
public:
    /// Compile time constant representing what kind of schema this class is.
    ///
    /// \sa UsdSchemaKind
    static const UsdSchemaKind schemaKind = UsdSchemaKind::SingleApplyAPI;

    /// Construct a {{Name}}ContractAPI on UsdPrim \p prim .
    /// Equivalent to {{Name}}ContractAPI::Get(prim.GetStage(), prim.GetPath())
    /// for a \em valid \p prim, but will not immediately throw an error for
    /// an invalid \p prim
    explicit {{Name}}ContractAPI(const UsdPrim& prim=UsdPrim())
        : UsdAPISchemaBase(prim)
    {
    }

    /// Construct a {{Name}}ContractAPI on the prim held by \p schemaObj .
    /// Should be preferred over {{Name}}ContractAPI(schemaObj.GetPrim()),
    /// as it preserves SchemaBase state.
    explicit {{Name}}ContractAPI(const UsdSchemaBase& schemaObj)
        : UsdAPISchemaBase(schemaObj)
    {
    }

    /// Destructor.
    {{NAME}}_API
    virtual ~{{Name}}ContractAPI();

    /// Return a vector of names of all pre-declared attributes for this schema
    /// class and all its ancestor classes.  Does not include attributes that
    /// may be authored by custom/extended methods of the schemas involved.
    {{NAME}}_API
    static const TfTokenVector &
    GetSchemaAttributeNames(bool includeInherited=true);

    /// Return a {{Name}}ContractAPI holding the prim adhering to this
    /// schema at \p path on \p stage.  If no prim exists at \p path on
    /// \p stage, or if the prim at that path does not adhere to this schema,
    /// return an invalid schema object.  This is shorthand for the following:
    ///
    /// \code
    /// {{Name}}ContractAPI(stage->GetPrimAtPath(path));
    /// \endcode
    ///
    {{NAME}}_API
    static {{Name}}ContractAPI
    Get(const UsdStagePtr &stage, const SdfPath &path);


    /// Returns true if this <b>single-apply</b> API schema can be applied to 
    /// the given \p prim. If this schema can not be a applied to the prim, 
    /// this returns false and, if provided, populates \p whyNot with the 
    /// reason it can not be applied.
    /// 
    /// Note that if CanApply returns false, that does not necessarily imply
    /// that calling Apply will fail. Callers are expected to call CanApply
    /// before calling Apply if they want to ensure that it is valid to 
    /// apply a schema.
    /// 
    /// \sa UsdPrim::GetAppliedSchemas()
    /// \sa UsdPrim::HasAPI()
    /// \sa UsdPrim::CanApplyAPI()
    /// \sa UsdPrim::ApplyAPI()
    /// \sa UsdPrim::RemoveAPI()
    ///
    {{NAME}}_API
    static bool 
    CanApply(const UsdPrim &prim, std::string *whyNot=nullptr);

    /// Applies this <b>single-apply</b> API schema to the given \p prim.
    /// This information is stored by adding "{{Name}}ContractAPI" to the 
    /// token-valued, listOp metadata \em apiSchemas on the prim.
    /// 
    /// \return A valid {{Name}}ContractAPI object is returned upon success. 
    /// An invalid (or empty) {{Name}}ContractAPI object is returned upon 
    /// failure. See \ref UsdPrim::ApplyAPI() for conditions 
    /// resulting in failure. 
    /// 
    /// \sa UsdPrim::GetAppliedSchemas()
    /// \sa UsdPrim::HasAPI()
    /// \sa UsdPrim::CanApplyAPI()
    /// \sa UsdPrim::ApplyAPI()
    /// \sa UsdPrim::RemoveAPI()
    ///
    {{NAME}}_API
    static {{Name}}ContractAPI 
    Apply(const UsdPrim &prim);

protected:
    /// Returns the kind of schema this class belongs to.
    ///
    /// \sa UsdSchemaKind
    {{NAME}}_API
    UsdSchemaKind _GetSchemaKind() const override;

private:
    // needs to invoke _GetStaticTfType.
    friend class UsdSchemaRegistry;
    {{NAME}}_API
    static const TfType &_GetStaticTfType();

    static bool _IsTypedSchema();

    // override SchemaBase virtuals.
    {{NAME}}_API
    const TfType &_GetTfType() const override;

public:
    // --------------------------------------------------------------------- //
    // {{IDENT}}EXAMPLE 
    // --------------------------------------------------------------------- //
    /// Example attribute. Replace with the schema's real properties.
    ///
    /// | ||
    /// | -- | -- |
    /// | Declaration | `uniform token {{ident}}:example = ""` |
    /// | C++ Type | TfToken |
    /// | \ref Usd_Datatypes "Usd Type" | SdfValueTypeNames->Token |
    /// | \ref SdfVariability "Variability" | SdfVariabilityUniform |
    {{NAME}}_API
    UsdAttribute Get{{Ident}}ExampleAttr() const;

    /// See Get{{Ident}}ExampleAttr(), and also 
    /// \ref Usd_Create_Or_Get_Property for when to use Get vs Create.
    /// If specified, author \p defaultValue as the attribute's default,
    /// sparsely (when it makes sense to do so) if \p writeSparsely is \c true -
    /// the default for \p writeSparsely is \c false.
    {{NAME}}_API
    UsdAttribute Create{{Ident}}ExampleAttr(VtValue const &defaultValue = VtValue(), bool writeSparsely=false) const;

public:
    // ===================================================================== //
    // Feel free to add custom code below this line, it will be preserved by 
    // the code generator. 
    //
    // Just remember to: 
    //  - Close the class declaration with }; 
    //  - Close the namespace with PXR_NAMESPACE_CLOSE_SCOPE
    //  - Close the include guard with #endif
    // ===================================================================== //
    // --(BEGIN CUSTOM CODE)--
};

PXR_NAMESPACE_CLOSE_SCOPE

#endif
