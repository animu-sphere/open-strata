# Backlog

Only incomplete, ordered work lives here. The active milestone is in
[current.md](current.md); shipped work is recorded under
[releases/](../releases/) and in the
[delivery history](../reports/delivery-history.md).

Legend: ⬜ not started

## Milestone ladder (beyond next)

v0.23.0 release CI, host adapters and matrix is active in
[current.md](current.md). Detailed acceptance is in
[runtime-composition.md](runtime-composition.md) and the
[downstream report intake](downstream-report-intake.md).

- ⬜ **v1.0.0.** Cut after the produce → trust → trusted CI → Formation →
  DCC-host execution arc is supported, digest-addressed and dogfooded.

## Future phases

These remain outside the v0.23.0 mainline. Device diagnostics may prepare later
GPU work, and `runtime exec` may prepare later sessions, but neither expands the
active milestone into those systems.

- ⬜ **OpenUSD template catalog maturity.** Automate clean-install consumer
  gates, prove compiled schemas on a second platform/OpenUSD line, harden the
  asset-resolver skeleton and add applied OpenExec/`ExecUsdSystem` evidence.
  Extend `ost plugin new`; add Hydra/tool templates only after independent
  evidence. Direction:
  [openusd-plugin-templates.md](../design/proposed/openusd-plugin-templates.md).
- ⬜ **Component package contracts and workspace architecture lint.** Prove each
  installed CMake package from only its declared closure; validate public target
  dependencies against package resolution; add standalone/aggregate package
  modes, explicit boundary policy, graph lint, and component-level CI evidence.
  This extends the existing workspace graph and aggregate membership contracts;
  it does not create another resolver. Direction:
  [component-package-contracts.md](../design/proposed/component-package-contracts.md);
  staged work: [component-package-contracts.md](component-package-contracts.md).
- ⬜ **Renderer skeleton promotion.** Complete the hosted OS/OpenUSD matrix and
  apply the contract to a second independent renderer. Instancing, materials,
  upload policy and zero-copy interop remain renderer-owned. Direction:
  [renderer-templates.md](../design/proposed/renderer-templates.md).
- ⬜ **Sessions / sandbox.** Add `ost session start|fork|diff|discard|promote`,
  workspace isolation and optional Linux namespace/overlayfs support on top of a
  resolved Formation.
- ⬜ **AI / GPU profiles.** Add GPU/driver diagnostics, CUDA/ROCm/MPS profiles
  and GPU-routed CI smoke tests.
- ⬜ **Kubernetes execution backend.** Add a pluggable local/Kubernetes
  execution interface, `ost submit|jobs`, safe digest-pinned `batch/v1 Job`
  export/submission/log/artifact flows and `ost doctor kubernetes`. Direction:
  [kubernetes.md](../design/proposed/kubernetes.md).

## Cross-cutting open items

- ⬜ **SEC-005 (P1) - release signing and verification.** Publish explicit
  signature/Sigstore material; verify it in `ost` and the installers; abort on
  mismatch. Canonical publication remains workflow-owned.
- ⬜ **Runtime/extension attribution.** Record upstream license metadata,
  collect `LICENSE`/`NOTICE`, expose runtime license inspection and refuse
  artifacts with incomplete third-party attribution.
- ⬜ **SEC-006 (P2) - runtime trust policy.** Record `local` / `verified` /
  `trusted` in manifests and locks, warn on world-writable roots and let build,
  test and release CI require a minimum trust level.
- ⬜ **Runtime distribution diagnostics.** Surface measured glibc floors before
  export, reject or warn on incompatible pulls without requiring an explicit
  target flag, and make `ost artifact push` the canonical OCI producer or match
  the equivalent OCI manifest byte-for-byte.

## Documentation & tooling

- ⬜ **Documentation website.** Render repository-owned Markdown as a searchable
  static site with pull-request previews and no manually duplicated generated
  reference content.
