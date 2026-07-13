// SPDX-License-Identifier: Apache-2.0
#pragma once

#include "pxr/pxr.h"

PXR_NAMESPACE_OPEN_SCOPE

class VdfContext;

namespace {{Name}}Computation {

/// Deterministic project-owned evaluation seam.
///
/// Keep stage mutation, graph construction, scheduling, caching, and device
/// execution outside this starter boundary. Add typed OpenExec inputs in the
/// registration and read only those declared inputs from `context` here.
double Evaluate(const VdfContext& context);

} // namespace {{Name}}Computation

PXR_NAMESPACE_CLOSE_SCOPE
