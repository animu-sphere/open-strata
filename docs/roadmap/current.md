# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## v0.22.5 - locked composed runtime

**Status:** ⬜ not started · **Depends on:** the v0.22.4 runtime component model.

A resolved composition is only useful if it can be reproduced. This slice turns
the v0.22.4 resolved model into a lock plus an exportable, independently
transportable runtime artifact.

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
clean machine from the lock and immutable artifacts alone. Full v0.22.5-v0.22.9
acceptance is in the [runtime-composition plan](runtime-composition.md).

## Active carry-over

- **v0.22.4 cross-repository acceptance.** Run the real published-artifact
  composition pass in `usd-geospatial-runtime` against independently published
  artifacts rather than the checked-in fixture. Corrections belong to v0.22.5.
- **v0.22.3 post-release validation.** Run the host-specific canonical 16-leaf
  OpenUSD builds, protected GHCR publication, clean digest pulls, and the USD
  VRM release-lane dogfood with the released binary. The four macOS leaves are
  published; Linux and Windows remain.
- **SEC-002 — symlink escape inside a bundle.** Reject a real in-bundle symlink
  whose canonical target escapes the bundle root.
- **Packaging diagnostic.** Optionally warn when a same-basename PDB is older
  than its DLL; keep it non-fatal until PE/PDB identity can be compared.
