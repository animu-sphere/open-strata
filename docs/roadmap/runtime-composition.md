---
title: v0.22.x runtime composition
status: active
owners:
  - openstrata-maintainers
created: 2026-08-24
updated: 2026-08-29
applies_to: v0.22.9-v0.22.10
---

# v0.22.x runtime composition

This is the execution plan for the proposed
[runtime-composition contract](../design/proposed/runtime-composition.md). It
contains only incomplete work. The v0.22.3 canonical runtime and artifact
foundation and the v0.22.4 component model are recorded in their release records
([v0.22.3](../releases/v0.22.3.md), [v0.22.4](../releases/v0.22.4.md)). Locked
composition and the native SDK shipped together in [v0.22.7](../releases/v0.22.7.md).
The real geospatial dogfood shipped in [v0.22.8](../releases/v0.22.8.md). The next
release is summarized in [current.md](current.md); later slices are ordered in
[backlog.md](backlog.md).

The series advances one contract at a time. DCC host adapters remain v0.23.0
work after this foundation has been dogfooded.

The order is intentional: distribute the canonical runtime first, make its
ordinary runtime UX and diagnostics stable second, and only then bind it to DCC
hosts. New low-level subsystems do not enter this v0.22.x line unless they close
one of those contracts.

## v0.22.9 - consumer packaging foundation

**Objective:** one canonical runtime can serve ecosystem-native entry points.

- Define Python wheel, npm/JavaScript/Wasm and native SDK packages as derived
  consumer distributions of pinned OST artifacts.
- Keep component/runtime identity, provenance and dependency truth in OST/OCI;
  ecosystem registries do not become competing canonical artifact stores.
- Treat ecosystem package names and versions as routing metadata. A consumer
  manifest must retain the exact artifact digest, runtime and component
  identities, target, SBOM, provenance, evidence and public entrypoints.
- Use the native SDK as the reference implementation: derive from a verified
  composed runtime, require real installed CMake config entrypoints, relocate to
  a clean consumer and verify that runtime identity does not change.
- Specify Python and JavaScript/Wasm public APIs above a package-private
  `verify -> extract -> activate` binder/loader contract. Do not expose OST
  locks, component graphs or activation metadata as their public API.

The registry-neutral manifest, native entrypoint validation, exact
consumer/runtime identity check, deterministic wheel/npm assembly and generated
private loaders are present on `main` after v0.22.8. Clean-consumer wheel
execution and npm install/loader probes now run in isolated stores, retaining an
explained Node child-process capability SKIP outside strict SDK CI.
Registry-facing evidence remains open until the milestone is released.

## v0.22.10 - runtime UX and diagnostics

**Objective:** using a composed runtime is ordinary for humans, CI and agents.

- Stabilize the `runtime compose|explain|doctor|exec` (or final equivalent) CLI
  and JSON schemas.
- Keep ordinary workflows on this small task-oriented surface; platform,
  profile, provider, artifact and lock detail remains available through
  `explain` and structured output rather than becoming mandatory user input.
- Extend diagnostics across artifact, dependency, plugin, resolver, loader,
  ABI, Python, device, DCC-prerequisite and host-capability boundaries with
  stable categories, error codes and remediation.
- Record component verification, composition verification, runtime execution,
  plugin load, render and physical-device validation as distinct claims.
  Preserve explained host-capability SKIPs instead of converting them to PASS
  or failure.
- Complete the end-to-end geospatial clean-consumer acceptance and decide
  whether the proposed design can be promoted to accepted.
