# Current

The next milestone. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## v0.22.9 - consumer packaging foundation

**Status:** in progress. **Depends on:** the
[v0.22.8 geospatial runtime dogfood](../releases/v0.22.8.md).

Derive ecosystem-native consumer entry points from canonical OST/OCI artifacts
without creating a second source of runtime identity or dependency truth.

The governing boundary is:

```text
source -> component artifacts -> Formation lock -> composed runtime artifact
       -> consumer package
```

The OST artifact digest, runtime identity, component graph, target, SBOM and
provenance remain canonical throughout that path. Registry package names and
versions are distribution/routing metadata only.

### Workstreams

- **Registry-neutral identity contract:** retain the exact composed-runtime
  artifact and runtime identities, target, component identities, SBOM,
  provenance, evidence and public entrypoints in the consumer manifest.
- **Native SDK reference path:** derive the manifest only from a verified,
  SDK-bearing composed runtime; require declared CMake entrypoints to exist; and
  re-verify the exact runtime at the clean/relocated consumer boundary.
- **Python and JavaScript/Wasm adapters:** keep their public import/export APIs
  above a package-private `verify -> extract -> activate` binder/loader
  protocol. Callers must not parse OST locks or activation metadata.

The manifest, native entrypoint check, consumer-to-runtime identity verification,
deterministic wheel/npm archive assembly, and generated package-private loaders
have landed on `main` after v0.22.8. The clean-consumer harness installs and
executes the wheel in a fresh store, installs the npm tarball, and executes its
loader when host policy permits Node.js child processes; strict SDK CI requires
all tools and does not accept that capability SKIP. Registry-facing acceptance
remains milestone work, so the unreleased implementation does not make v0.22.9
shipped.

### Acceptance

- Define Python wheel, npm/JavaScript/Wasm and native SDK packages as derived
  distributions of digest-pinned runtime artifacts.
- Preserve OST component/runtime identity, provenance and dependency truth when
  publishing through ecosystem registries.
- Prove one relocated, clean native SDK consumer and specify the binder/loader
  boundary for Python and JavaScript without exposing OST internals as public
  APIs.

### Exit criteria

One canonical runtime can serve the selected ecosystem entry points, and each
derived package resolves back to its exact OST artifact identity. Remaining
v0.22.9-v0.22.10 work is in the [runtime-composition plan](runtime-composition.md).
