---
title: Runtime composition follow-through
status: active
owners:
  - openstrata-maintainers
created: 2026-08-24
updated: 2026-09-20
applies_to: post-v0.23.0
---

# Runtime composition follow-through

This plan contains only incomplete work on the proposed
[runtime-composition contract](../design/proposed/runtime-composition.md).
Canonical artifacts, locked composition, the native SDK, geospatial dogfood,
consumer packaging, release CI and initial host adapters are release facts
through [v0.23.0](../releases/v0.23.0.md). The remaining work is summarized in
[current.md](current.md); later slices are ordered in
[backlog.md](backlog.md).

## Post-v0.23.0 - release evidence and host integration

**Objective:** finish downstream and hosted evidence for the released paths,
then extend the same composed runtime identity across host adapters.

- Prove each packaged component's installed loadability and missing-file
  rejection across target operating systems and bundle build orders.
- Prove the cross-repository motion library migration and an independent
  installed consumer with the new digest-pinned library edge.
- Prove the pinned OpenUSD runtime's exported CMake package on a clean host;
  exercise generated optional OpenUSD flags on hosted macOS.
- Complete DCC add-on packaging and host-specific smoke suites with explained
  capability SKIPs and separate component, execution and plugin-load claims.
- Complete the host/OS/OpenUSD/Python matrix with pinned runtime and component
  artifact identities and bounded execution evidence.

The report sources and exact acceptance are retained in the
[downstream report intake](downstream-report-intake.md).
