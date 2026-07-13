// SPDX-License-Identifier: Apache-2.0
#include "{{Name}}Computation.h"

#include "pxr/exec/vdf/context.h"

PXR_NAMESPACE_OPEN_SCOPE

double
{{Name}}Computation::Evaluate(const VdfContext& context)
{
    // The skeleton intentionally has no inputs. Add `.Inputs(...)` beside the
    // registration before reading them here. A constant result keeps the first
    // callback deterministic and makes the project-owned seam easy to test.
    (void)context;
    return 0.0;
}

PXR_NAMESPACE_CLOSE_SCOPE
