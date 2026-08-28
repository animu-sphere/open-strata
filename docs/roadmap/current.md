# Current

The next milestone. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## v0.22.9 - consumer packaging foundation

**Status:** in progress. **Depends on:** the
[v0.22.8 geospatial runtime dogfood](../releases/v0.22.8.md).

Derive ecosystem-native consumer entry points from canonical OST/OCI artifacts
without creating a second source of runtime identity or dependency truth.

### Acceptance

- Define Python wheel, npm/JavaScript/Wasm and native SDK packages as derived
  distributions of digest-pinned runtime artifacts.
- Preserve OST component/runtime identity, provenance and dependency truth when
  publishing through ecosystem registries.
- Prove one native SDK consumer and specify the binder/loader boundary for Python
  and JavaScript without exposing OST internals as public APIs.

### Exit criteria

One canonical runtime can serve the selected ecosystem entry points, and each
derived package resolves back to its exact OST artifact identity. Remaining
v0.22.9-v0.22.10 work is in the [runtime-composition plan](runtime-composition.md).
