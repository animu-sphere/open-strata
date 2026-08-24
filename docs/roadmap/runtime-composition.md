---
title: v0.22.x runtime composition
status: active
owners:
  - openstrata-maintainers
created: 2026-08-24
updated: 2026-08-25
applies_to: v0.22.4-v0.22.9
---

# v0.22.x runtime composition

This is the execution plan for the proposed
[runtime-composition contract](../design/proposed/runtime-composition.md). It
contains only incomplete work. The v0.22.3 canonical runtime and artifact
foundation is recorded in its [release record](../releases/v0.22.3.md). The next
release is summarized in [current.md](current.md); later slices are ordered in
[backlog.md](backlog.md).

The series advances one contract at a time. DCC host adapters remain v0.23.0
work after this foundation has been dogfooded.

## v0.22.4 - runtime component model

**Objective:** several component artifacts can resolve into one runtime model.

- Define versioned component requirements/provisions for runtime, plugin,
  library, tool, renderer and data artifacts.
- Extend artifact manifests with dependency and environment-contribution
  metadata without forking existing plugin activation or Formation models.
- Resolve capabilities to providers deterministically, with explicit provider
  pins supported for release contracts.
- Diagnose missing providers, version/ABI/OpenUSD conflicts, singleton
  capability collisions, install-path collisions and incompatible environment
  contributions before materialization.
- Produce a canonical composition manifest and stable JSON inspection output.

### Exit criteria

A synthetic multi-artifact graph and the initial geospatial manifest resolve to
the same ordered model on Windows, Linux and macOS; conflicts fail before files
are written.

## v0.22.5 - locked composed runtime

**Objective:** a resolved runtime is reproducible and independently
transportable.

- Lock provider identity, version, digest, immutable source, dependency graph,
  target/variant and compatibility decisions.
- Derive runtime identity from canonical inputs and materialized inventory.
- Export the composed runtime as an OST artifact with provenance, attribution,
  SBOM and composition-level validation evidence.
- Reconstruct the same identity on a clean machine from the lock and immutable
  artifacts; caches remain optional.

## v0.22.6 - runtime SDK layout

**Objective:** applications consume a runtime through predictable native
conventions.

- Materialize the owner-recorded `bin`, `lib`, `include`, `share`, `plugins`,
  `python`, `node` and `metadata` roots.
- Generate deterministic activation/environment data using the existing
  Formation environment contract.
- Expose installed CMake config packages to a clean C++ consumer through normal
  `find_package(... CONFIG)` resolution.
- Validate loader, executable, plugin, resolver, schema and CMake-package
  reachability from an isolated prefix.

## v0.22.7 - geospatial runtime dogfood

**Objective:** build the first real composed runtime in parallel with
[`animu-sphere/usd-geospatial-runtime`](https://github.com/animu-sphere/usd-geospatial-runtime).

- Compose a digest-pinned canonical OpenUSD 26.08 cell, `usd-http-resolver`,
  `usd-pointcloud-plugins` and
  `usd-raster-plugins` from independently published artifacts.
- Keep the composition repository declarative: it owns capability selection,
  locks, fixtures and acceptance evidence, not a replacement build/solver.
- Exercise local file formats first, then prove HTTP/COPC Tier 2 through the
  packaged resolver and point-cloud artifacts.
- Publish the runtime artifact and reconstruct it on a clean consumer with the
  exact locked identity.

### Geospatial component readiness

The 2026-08-16
[pre-implementation report](https://github.com/animu-sphere/usd-pointcloud-plugins/blob/main/docs/reports/ost/04-2026-08-16-usd-http-resolver-preimplementation.md)
is historical and requested no OpenStrata fix. `usd-http-resolver` v0.4.0 now
ships the previously missing resolver/backend/test substrate. Remaining
acceptance is an immutable resolver artifact composed on a clean consumer with
COPC metadata/range reads, cache miss-to-hit reuse, validation-token invalidation
and a bytes-fetched baseline. `usd-raster-plugins` is pre-release; its component
artifact and selected GeoTIFF read/authoring contract must ship before the
geospatial runtime claims raster support.

## v0.22.8 - consumer packaging foundation

**Objective:** one canonical runtime can serve ecosystem-native entry points.

- Define Python wheel, npm/JavaScript/Wasm and native SDK packages as derived
  consumer distributions of pinned OST artifacts.
- Keep component/runtime identity, provenance and dependency truth in OST/OCI;
  ecosystem registries do not become competing canonical artifact stores.
- Prove one native SDK consumer and specify the binder/loader contract for
  Python and JavaScript without leaking OST internals into their public API.

## v0.22.9 - runtime UX and diagnostics

**Objective:** using a composed runtime is ordinary for humans, CI and agents.

- Stabilize the `runtime compose|explain|doctor|exec` (or final equivalent) CLI
  and JSON schemas.
- Extend diagnostics across artifact, dependency, plugin, resolver, loader,
  ABI, Python and device boundaries with stable error codes and remediation.
- Record component and composition validation separately and preserve explained
  host-capability SKIPs.
- Complete the end-to-end geospatial clean-consumer acceptance and decide
  whether the proposed design can be promoted to accepted.
