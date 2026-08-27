# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## v0.22.9 - consumer packaging foundation

**Status:** planned. **Depends on:** the
[v0.22.8 geospatial runtime dogfood](../releases/v0.22.8.md).

Derive ecosystem-native consumer entry points from canonical OST/OCI artifacts
without creating a second source of runtime identity or dependency truth.

### Acceptance

- Define Python wheel, npm/JavaScript/Wasm and native SDK packages as derived
  distributions of digest-pinned runtime artifacts.
- Preserve OST component/runtime identity, provenance and dependency truth when
  publishing through ecosystem registries.
- Prove one native SDK consumer and specify the binder/loader boundary for Python
  and JavaScript without exposing OST internals as public APIs.

### Exit criteria

One canonical runtime can serve the selected ecosystem entry points, and each
derived package resolves back to its exact OST artifact identity. Remaining
v0.22.9-v0.22.10 work is in the [runtime-composition plan](runtime-composition.md).

## Active carry-over

- **v0.22.6 remaining host-probe evidence.** The maintainer confirmed successful
  USD VRM real-artifact consumption with v0.22.6 on 2026-08-26. That consumer
  blocker is closed; it does not independently establish Qt/device/render
  observations on every hosted Windows/macOS cell. Keep those observations
  separate from artifact consumption. Historical investigation:
  [render-prerequisite report](../reports/2026-08-26-usd-vrm-ci-render-prerequisites.md).
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
