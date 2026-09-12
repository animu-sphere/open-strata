---
title: v0.22.x runtime composition
status: active
owners:
  - openstrata-maintainers
created: 2026-08-24
updated: 2026-09-12
applies_to: v0.22.10
---

# v0.22.x runtime composition

This is the execution plan for the proposed
[runtime-composition contract](../design/proposed/runtime-composition.md). It
contains only incomplete work. The v0.22.3 canonical runtime and artifact
foundation and the v0.22.4 component model are recorded in their release records
([v0.22.3](../releases/v0.22.3.md), [v0.22.4](../releases/v0.22.4.md)). Locked
composition and the native SDK shipped together in [v0.22.7](../releases/v0.22.7.md).
The real geospatial dogfood shipped in [v0.22.8](../releases/v0.22.8.md), and the
consumer-packaging foundation shipped in [v0.22.9](../releases/v0.22.9.md). The
next release is summarized in [current.md](current.md); later slices are ordered
in [backlog.md](backlog.md).

The series advances one contract at a time. DCC host adapters remain v0.23.0
work after this foundation has been dogfooded.

The remaining order is intentional: make ordinary runtime UX and diagnostics
stable, then bind the same identity to expanded release CI and DCC hosts. New
low-level subsystems do not enter this v0.22.x line unless they close that
contract.

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
- Complete registry-facing acceptance for the derived native, Python and
  JavaScript packages shipped as a local clean-consumer foundation in v0.22.9.
- Close the runtime identity, lock, resolver and managed-build diagnostics in
  the [downstream report intake](downstream-report-intake.md).
