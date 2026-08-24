# Backlog

Only incomplete, ordered work lives here. The active milestone is in
[current.md](current.md); shipped work is recorded under
[releases/](../releases/) and in the
[delivery history](../reports/delivery-history.md).

Legend: ⬜ not started

## Milestone ladder (beyond next)

v0.22.3 is active. Detailed acceptance for the remaining v0.22.x runtime
composition slices is in [runtime-composition.md](runtime-composition.md).

- ⬜ **v0.22.4 - runtime component model.** Resolve the canonical OpenUSD base
  runtime plus plugin, library, tool, renderer and data artifacts through
  explicit dependency, capability/provider and environment metadata. Emit one
  canonical model and diagnose conflicts before materialization, reusing
  Formation resolution and activation.
- ⬜ **v0.22.5 - locked composed runtime.** Pin providers, component digests,
  immutable sources, dependency edges, target/variant and compatibility
  decisions. Derive runtime identity, export it with evidence and reconstruct
  the same identity on a clean machine.
- ⬜ **v0.22.6 - runtime SDK layout.** Materialize predictable `bin`, `lib`,
  `include`, `share`, `plugins`, `python`, `node` and `metadata` roots with
  owner-recorded activation. Prove normal CMake SDK consumption from an isolated
  prefix.
- ⬜ **v0.22.7 - geospatial runtime dogfood.** Compose a digest-pinned canonical
  OpenUSD 26.08 cell, HTTP resolver, point-cloud plugins and raster plugins from
  published artifacts in
  [`usd-geospatial-runtime`](https://github.com/animu-sphere/usd-geospatial-runtime).
  Keep that repository declarative and prove HTTP/COPC plus GeoTIFF through
  packaged components on a clean consumer.
- ⬜ **v0.22.8 - consumer packaging foundation.** Derive Python, npm/Wasm and
  native SDK entry points from canonical OST/OCI artifacts while preserving one
  runtime identity and provenance graph.
- ⬜ **v0.22.9 - runtime UX and diagnostics.** Stabilize runtime
  compose/explain/doctor/exec and machine-readable diagnostics, complete the
  geospatial clean-consumer pass and decide whether to accept the runtime
  composition design.
- ⬜ **v0.23.0 - DCC host adapters and matrix.** Run minimal headless
  load/open/validate probes with preserved output and explained SKIPs; generate
  Maya `.mod` and Houdini package JSON; publish matrix cells with pinned host
  records, artifact digests, tiers and execution evidence; and complete
  Linux/macOS discovery acceptance. OpenStrata does not install, license or
  mutate hosts, and adapters do not abstract DCC APIs. Direction:
  [dcc-hosts.md](../design/proposed/dcc-hosts.md).
- ⬜ **v1.0.0.** Cut after the produce → trust → trusted CI → Formation →
  DCC-host execution arc is supported, digest-addressed and dogfooded.

## Future phases

- ⬜ **OpenUSD template catalog maturity.** Automate clean-install consumer
  gates, prove compiled schemas on a second platform/OpenUSD line, harden the
  asset-resolver skeleton and add applied OpenExec/`ExecUsdSystem` evidence.
  Extend `ost plugin new`; add Hydra/tool templates only after independent
  evidence. Direction:
  [openusd-plugin-templates.md](../design/proposed/openusd-plugin-templates.md).
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
