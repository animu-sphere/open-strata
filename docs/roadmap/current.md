# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## v0.22.4 - runtime component model

**Status:** 🚧 implementation landed; cross-repository acceptance pending ·
**Depends on:** the v0.22.3 canonical OpenUSD runtime matrix and
component-artifact contracts.

This is the first composition slice built on the canonical runtime foundation.
It resolves an OpenUSD base plus independently published plugin, library, tool,
renderer, and data artifacts into one deterministic component model before any
files are materialized.

### Acceptance

- Define versioned component requirements and provisions for runtime, plugin,
  library, tool, renderer, and data artifacts.
- Extend artifact manifests with dependency and environment-contribution
  metadata without forking the existing plugin activation or Formation models.
- Resolve capabilities to providers deterministically, including explicit
  provider pins for release contracts.
- Diagnose missing providers, version/ABI/OpenUSD conflicts, singleton
  capability collisions, install-path collisions, and incompatible environment
  contributions before materialization.
- Produce a canonical composition manifest and stable JSON inspection output.

### Exit criteria

A synthetic multi-artifact graph and the initial geospatial manifest resolve to
the same ordered model on Windows, Linux, and macOS; conflicts fail before files
are written. Full v0.22.4-v0.22.9 acceptance is in the
[runtime-composition plan](runtime-composition.md).

### Implemented

- `openstrata.component/v1alpha1` normalizes runtime, plugin, library, tool,
  renderer, and data requirements/provisions, compatibility, activation, and
  install ownership into artifact records.
- `ost runtime compose <manifest>` verifies digest-pinned candidates and emits
  a canonical `openstrata.runtime-composition-resolved/v1alpha1` JSON model.
- Provider pins and coded missing/version/ABI/OpenUSD/singleton/install/environment
  diagnostics run before archive extraction or materialization.
- Synthetic Windows, Linux, and macOS graphs preserve one topological component
  order; the checked-in geospatial fixture resolves OpenUSD, HTTP, COPC, and
  GeoTIFF providers in the same model.

Remaining release acceptance is the real published-artifact pass in
`usd-geospatial-runtime` plus the Linux/Windows v0.22.3 carry-over validation.

## Active carry-over

- **v0.22.3 post-release validation.** Run the host-specific 16-leaf OpenUSD
  builds, protected GHCR publication, clean digest pulls, and the USD VRM
  release-lane dogfood with the released binary. Corrections found by this pass
  belong to v0.22.4. The four macOS leaves are published; Linux and Windows
  remain.
- **SEC-002 — symlink escape inside a bundle.** Reject a real in-bundle symlink
  whose canonical target escapes the bundle root.
- **Packaging diagnostic.** Optionally warn when a same-basename PDB is older
  than its DLL; keep it non-fatal until PE/PDB identity can be compared.
