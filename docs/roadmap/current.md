# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## v0.22.7 - locked composed runtime

**Status:** implementation and Windows local gates complete; release CI and
publication pending. **Depends on:** the v0.22.4 runtime component model.

A resolved composition is only useful if it can be reproduced. This slice turns
the v0.22.4 resolved model into a lock plus an exportable, independently
transportable runtime artifact.

The working implementation provides `compose --lock/--locked/--output`,
`reconstruct`, and `export/validate --composition`. See the
[draft release record](../releases/v0.22.7.md) and
[consumer guide](../guides/compose-a-runtime.md). Unix-specific symlink/mode tests
are included but still need a Linux Rust test runner; the available WSL has no
Rust installation. The current published version remains v0.22.6.

### Acceptance

- Lock provider identity, version, digest, immutable source, dependency graph,
  target/variant, and compatibility decisions.
- Derive runtime identity from canonical inputs and materialized inventory.
- Export the composed runtime as an OST artifact with provenance, attribution,
  SBOM, and composition-level validation evidence.
- Reconstruct the same identity on a clean machine from the lock and immutable
  artifacts; caches remain optional.

### Exit criteria

A composed runtime locked on one machine reconstructs to the same identity on a
clean machine from the lock and immutable artifacts alone. Full v0.22.7-v0.22.11
acceptance is in the [runtime-composition plan](runtime-composition.md).

## Active carry-over

- **v0.22.6 remaining host-probe evidence.** The maintainer confirmed successful
  USD VRM real-artifact consumption with v0.22.6 on 2026-08-26. That consumer
  blocker is closed; it does not independently establish Qt/device/render
  observations on every hosted Windows/macOS cell. Keep those observations
  separate from artifact consumption. Historical investigation:
  [render-prerequisite report](../reports/2026-08-26-usd-vrm-ci-render-prerequisites.md).
- **v0.22.4 cross-repository acceptance.** Run the real published-artifact
  composition pass in `usd-geospatial-runtime` against independently published
  artifacts rather than the checked-in fixture. Corrections belong to v0.22.7.
- **v0.22.3 post-release validation.** Run the host-specific canonical 16-leaf
  OpenUSD builds, protected GHCR publication, clean digest pulls, and the USD
  VRM release-lane dogfood with the released binary. The four macOS leaves are
  published; Linux and Windows remain.
- **CI cells cannot state the OpenUSD variant the pull can require.**
  `ost artifact pull` accepts `--require-openusd <platform>/<os>/<arch>/<variant>`
  and `--require-openusd-version`, but `openstrata.ci.yaml`'s cell schema has no
  field for either, so a contract pins bytes without saying what kind of runtime
  the project requires. A wrong re-pin is then caught at CMake configure on every
  runner rather than by the matrix validator (USD VRM report 36 §5.3, P3).
- **`[[workspace.install_data]]` takes whole directories.** The CMake rule it
  parallels can filter (`PATTERN "*.yaml"`); the mapping cannot, so a source
  directory ships every file it holds. An include filter would close the gap
  (USD VRM report 36 §4, P3).
- **SEC-002 — symlink escape inside a bundle.** Reject a real in-bundle symlink
  whose canonical target escapes the bundle root.
- **Packaging diagnostic.** Optionally warn when a same-basename PDB is older
  than its DLL; keep it non-fatal until PE/PDB identity can be compared.
