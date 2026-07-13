// SPDX-License-Identifier: Apache-2.0
#include "{{Name}}Computation.h"

#include "pxr/base/tf/staticTokens.h"
#include "pxr/exec/exec/registerSchema.h"

PXR_NAMESPACE_USING_DIRECTIVE

TF_DEFINE_PRIVATE_TOKENS(
    _tokens,
    (compute{{Name}})
);

// OpenExec discovers this registration through the matching Info.Exec.Schemas
// metadata in plugInfo.json. Keep one macro invocation per schema type and add
// all computations for that schema inside the same registration body.
EXEC_REGISTER_COMPUTATIONS_FOR_SCHEMA({{SchemaType}})
{
    self.PrimComputation(_tokens->compute{{Name}})
        .Callback<double>(&{{Name}}Computation::Evaluate);
}
