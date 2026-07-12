//
// Copyright 2016 Pixar
//
// Licensed under the terms set forth in the LICENSE.txt file available at
// https://openusd.org/license.
//
#include "./tokens.h"

PXR_NAMESPACE_OPEN_SCOPE

{{Name}}TokensType::{{Name}}TokensType() :
    {{identCamel}}Example("{{ident}}:example", TfToken::Immortal),
    {{Name}}ContractAPI("{{Name}}ContractAPI", TfToken::Immortal),
    allTokens({
        {{identCamel}}Example,
        {{Name}}ContractAPI
    })
{
}

TfStaticData<{{Name}}TokensType> {{Name}}Tokens;

PXR_NAMESPACE_CLOSE_SCOPE
