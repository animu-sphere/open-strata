---
title: v0.22.x runtime composition
status: active
owners:
  - openstrata-maintainers
created: 2026-08-24
updated: 2026-08-27
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

## v0.22.9 - consumer packaging foundation

**Objective:** one canonical runtime can serve ecosystem-native entry points.

- Define Python wheel, npm/JavaScript/Wasm and native SDK packages as derived
  consumer distributions of pinned OST artifacts.
- Keep component/runtime identity, provenance and dependency truth in OST/OCI;
  ecosystem registries do not become competing canonical artifact stores.
- Prove one native SDK consumer and specify the binder/loader contract for
  Python and JavaScript without leaking OST internals into their public API.

## v0.22.10 - runtime UX and diagnostics

**Objective:** using a composed runtime is ordinary for humans, CI and agents.

- Stabilize the `runtime compose|explain|doctor|exec` (or final equivalent) CLI
  and JSON schemas.
- Extend diagnostics across artifact, dependency, plugin, resolver, loader,
  ABI, Python and device boundaries with stable error codes and remediation.
- Record component and composition validation separately and preserve explained
  host-capability SKIPs.
- Complete the end-to-end geospatial clean-consumer acceptance and decide
  whether the proposed design can be promoted to accepted.
