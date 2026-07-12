//
// Copyright 2016 Pixar
//
// Licensed under the terms set forth in the LICENSE.txt file available at
// https://openusd.org/license.
//
#ifndef {{NAME}}_TOKENS_H
#define {{NAME}}_TOKENS_H

/// \file {{Name}}/tokens.h

// XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
// 
// This is an automatically generated file (by usdGenSchema.py).
// Do not hand-edit!
// 
// XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX

#include "pxr/pxr.h"
#include "./api.h"
#include "pxr/base/tf/staticData.h"
#include "pxr/base/tf/token.h"
#include <vector>

PXR_NAMESPACE_OPEN_SCOPE


/// \class {{Name}}TokensType
///
/// \link {{Name}}Tokens \endlink provides static, efficient
/// \link TfToken TfTokens\endlink for use in all public USD API.
///
/// These tokens are auto-generated from the module's schema, representing
/// property names, for when you need to fetch an attribute or relationship
/// directly by name, e.g. UsdPrim::GetAttribute(), in the most efficient
/// manner, and allow the compiler to verify that you spelled the name
/// correctly.
///
/// {{Name}}Tokens also contains all of the \em allowedTokens values
/// declared for schema builtin attributes of 'token' scene description type.
/// Use {{Name}}Tokens like so:
///
/// \code
///     gprim.GetMyTokenValuedAttr().Set({{Name}}Tokens->{{identCamel}}Example);
/// \endcode
struct {{Name}}TokensType {
    {{NAME}}_API {{Name}}TokensType();
    /// \brief "{{ident}}:example"
    /// 
    /// {{Name}}ContractAPI
    const TfToken {{identCamel}}Example;
    /// \brief "{{Name}}ContractAPI"
    /// 
    /// Schema identifer and family for {{Name}}ContractAPI
    const TfToken {{Name}}ContractAPI;
    /// A vector of all of the tokens listed above.
    const std::vector<TfToken> allTokens;
};

/// \var {{Name}}Tokens
///
/// A global variable with static, efficient \link TfToken TfTokens\endlink
/// for use in all public USD API.  \sa {{Name}}TokensType
extern {{NAME}}_API TfStaticData<{{Name}}TokensType> {{Name}}Tokens;

PXR_NAMESPACE_CLOSE_SCOPE

#endif
