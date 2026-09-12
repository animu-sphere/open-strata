# Current

The next milestone. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## v0.22.10 - runtime correctness, UX and diagnostics

**Status:** in progress. **Depends on:** the
[v0.22.9 consumer-packaging foundation](../releases/v0.22.9.md).

Make composed runtimes ordinary to use and make every identity or execution
failure diagnosable without reconstructing hidden build state. This milestone
also closes the correctness work found by the 2026-09-12 audit of all 71
reference-repository OST reports.

### Workstreams

- **Ordinary runtime surface:** stabilize the final
  `runtime compose|explain|doctor|exec` surface and its JSON schemas, keeping
  provider, artifact and lock detail inspectable rather than mandatory.
- **Relocatable identity:** remove producer-absolute Python paths from exported
  OpenUSD CMake metadata and reject plugin/product packages whose selected or
  recorded runtime differs from `strata.lock`.
- **Lock and provenance truth:** compare normalized lock semantics, align
  documented exit behavior, and preserve the actual producer across mixed root
  and bundle-scoped build lifecycles.
- **Actionable diagnostics:** add bundle-free workspace graph validation,
  offline resolver dispatch probes, complete bounded probe output, declarative
  external CMake inputs, and first-stall process/log snapshots.
- **Consumer acceptance:** retain exact OST identity through external registry
  routing and complete the end-to-end geospatial clean-consumer pass.

The exact report sources, duplicates and already-closed items are recorded in
the [downstream report intake](downstream-report-intake.md).

### Acceptance

- One composed runtime can be explained, diagnosed and executed through a small
  stable CLI/JSON surface.
- Exported SDK consumers do not depend on producer-absolute Python paths.
- Packaging cannot silently publish against a runtime other than the project
  lock, and semantic lock equality is formatting-independent.
- Workspace and resolver failures preserve enough bounded evidence to identify
  the failing process, probe and dependency input.
- Registry-derived consumer packages resolve back to the exact canonical OST
  artifact identity.

### Exit criteria

The v0.22.10 items in the
[runtime-composition plan](runtime-composition.md) and
[downstream report intake](downstream-report-intake.md) are complete. CI/release
verbs, generated/hand-authored lane reconciliation, per-member test evidence and
the first-class host-add-on model remain v0.23.0 work.
