# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## v0.22.3 - canonical OpenUSD runtimes and artifact contracts

**Status:** 🚧 next milestone · **Depends on:** the v0.22.0-v0.22.2 OpenUSD
artifact, compatibility, provider, distribution and evidence contracts.

This starts the
[v0.22.x runtime-composition series](runtime-composition.md) with two coupled
contracts. First, OpenStrata establishes the
[canonical OpenUSD CY2026 runtime policy](../design/proposed/canonical-openusd-runtimes.md):
OpenUSD 26.05 and 26.08, `profile = usd`, normalized `core` / `gl` / `vulkan` /
`metal` variants, primary Linux x86_64, Windows x86_64 and macOS arm64 producers,
and one data-driven build/verification/publication model. Second, every needed
component becomes independently buildable, packageable, attributable and
explicit about its dependencies and release membership. The artifact work is
driven by the 2026-08-24 USD VRM
[release-artifact membership report](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/35-2026-08-24-v0.22.2-release-artifact-membership.md).

### Canonical runtime acceptance

- Keep `profile = usd` separate from the graphics variant; add canonical
  `core`, `gl`, `vulkan` and `metal`, with warning-producing legacy
  normalization (`standard -> gl`) and migration-safe artifact readers.
- Extend compatibility identity for the primary CY2026 matrix, including
  providers, ABI facts and the macOS SDK/deployment target; keep the
  deterministic selector, immutable digest and human OCI tag distinct.
- Replace variant-specific build/package logic with one version-aware
  `OpenUsdBuildPlan` and a data-driven producer matrix. Generalize the existing
  Vulkan publisher instead of cloning it for each backend. Every imaging cell
  must build the upstream OpenUSD examples; `core` is exempt.
- Make verification backend-aware while preserving independent compile, link,
  loader, physical-device and render evidence. Add a first-class macOS arm64
  producer and Metal verification without Linux/GLX assumptions.
- Publish and pull-by-digest the 26.05/26.08 primary leaf matrix with normalized
  manifests, SBOM and provenance through the protected release identity. OCI
  multi-platform aliases remain ordered after robust leaf transport.

### P1 acceptance

- Dogfood the implemented descriptor-scoped
  [`requires.libraries` lifecycle](../reference/plugin-workspace.md#source-workspace-composition)
  against the USD VRM non-leaf adapter and retain clean build/test/package
  evidence from the release-lane OST version.
- Shared profiles/configuration have a data-only artifact/member contract (or an
  equivalent project-relative install mapping) and ship once under the correct
  owner rather than being copied below one tool.
- Root `ost build` outputs and package members carry coherent managed provenance.

### P2/P3 hardening

- Workspace discovery and aggregate-product membership are separate decisions;
  the project declares or pins the exact release set and packaging prints it.
- OST-upgrade record migrations, missing tool staging and managed-build
  mismatches name their cause and affected member.
- A workstation/CI OST pin mismatch that changes discovery or membership is
  reported before it becomes a release-lane surprise.

### Exit criteria

The primary canonical runtime matrix is generated from one normalized support
declaration and each published leaf passes build, runtime, graphics, artifact and
clean digest-pull distribution gates. The USD VRM workspace also packages a
non-leaf adapter and its shared motion profiles from declared dependencies,
fails on an unexpected aggregate membership change, and emits member-specific
provenance and diagnostics. The full v0.22.3 workstreams and ordered
v0.22.4-v0.22.9 slices are in
[runtime-composition.md](runtime-composition.md). DCC host adapters and their
matrix remain deferred to v0.23.0 in the [backlog](backlog.md).

## Active carry-over

- **SEC-002 — symlink escape inside a bundle.** Reject a real in-bundle symlink
  whose canonical target escapes the bundle root.
- **Packaging diagnostic.** Optionally warn when a same-basename PDB is older
  than its DLL; keep it non-fatal until PE/PDB identity can be compared.
