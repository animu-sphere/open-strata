---
title: Runtime composition follow-through
status: active
owners:
  - openstrata-maintainers
created: 2026-08-24
updated: 2026-09-13
applies_to: v0.23.0
---

# Runtime composition follow-through

This plan contains only incomplete work on the proposed
[runtime-composition contract](../design/proposed/runtime-composition.md).
Canonical artifacts, locked composition, the native SDK, geospatial dogfood,
consumer packaging, ordinary runtime UX and bounded correctness diagnostics are
release facts through [v0.22.10](../releases/v0.22.10.md). The next milestone is
summarized in [current.md](current.md); later slices are ordered in
[backlog.md](backlog.md).

## v0.23.0 - release CI and host integration

**Objective:** exercise the same composed runtime identity through complete
release lanes and truthful DCC-host entry points.

- Support runtime-free and per-cell intent-aware workspace CI.
- Extend generated verification through pyramid/package release claims and
  reconcile generated, externally managed and removed workflow lanes.
- Attribute test evidence to each discovered workspace member.
- Model `usdview` and DCC adapters as first-class components rather than
  codeless schema bundles or alternate dependency stores.
- Preserve distinct component, composition, execution, plugin-load, render and
  physical-device claims, including explained host-capability SKIPs.
- Complete the host/OS/OpenUSD/Python matrix with pinned runtime and plugin
  artifact identities and bounded execution evidence.

The report sources and exact acceptance are retained in the
[downstream report intake](downstream-report-intake.md).
