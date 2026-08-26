# USD Geospatial Runtime

[`animu-sphere/usd-geospatial-runtime`](https://github.com/animu-sphere/usd-geospatial-runtime)
is the first reference **runtime-composition repository**. OpenStrata and this
repository are intended to evolve in parallel: OpenStrata supplies the generic
artifact, resolution, lock, materialization and validation primitives; the
runtime repository supplies a real geospatial capability set, pins, fixtures and
clean-consumer evidence.

> The repository is currently empty. This page records its intended OpenStrata
> role and bootstrap boundary, not an implemented runtime or published support
> claim. The repository becomes authoritative for its concrete components,
> versions, support matrix and release records once those files exist.

## Intended composition

The first target is a locked runtime assembled from independently released
artifacts:

```text
canonical OpenUSD 26.08 cell
  + usd-http-resolver
  + usd-pointcloud-plugins
  + usd-raster-plugins
  -> usd-geospatial-runtime
```

The composition is capability-oriented (`usd`, `usd.resolve.http`,
`usd.fileformat.copc`, `usd.fileformat.geotiff`) while the lock records the exact
provider, version and digest selected for each capability.

## What the repository owns

- the requested capability set and supported target matrix;
- provider policy and immutable component pins;
- the checked-in composition manifest and lock;
- representative local and HTTP geospatial fixtures with provenance/licensing;
- clean-consumer runtime and SDK acceptance; and
- append-only dogfooding and release evidence.

It does not own a parallel dependency solver, plugin activation model, artifact
format or monolithic source build. A project-specific script may orchestrate
acceptance commands, but it must not become the only definition of runtime
membership or environment layout.

## Parallel bootstrap

The repository can start before `ost runtime compose` is complete:

1. Declare the target capabilities, initial providers and non-goals.
2. Add acceptance fixtures and expected probes without claiming they pass.
3. Adopt the v0.22.3 component artifacts and explicit product/data membership.
4. Check in the composition manifest and lock as v0.22.4/v0.22.7 land.
5. Replace any temporary orchestration with the shipped OST lifecycle.
6. Publish clean-consumer evidence for the v0.22.8 dogfood.

This sequence lets both repositories expose missing primitives early without
turning temporary source-tree composition into a public contract.

## Current readiness gates

- **Runtime repository:** empty as of 2026-08-24; no implementation or release
  claim exists yet.
- **HTTP resolver:** v0.4.0 now includes the HTTP backend, `ArResolver` bundle,
  CI/test suites, block cache, stable identity/validation tokens and persistent
  cache. The 2026-08-16
  [pre-implementation report](https://github.com/animu-sphere/usd-pointcloud-plugins/blob/main/docs/reports/ost/04-2026-08-16-usd-http-resolver-preimplementation.md)
  is retained as historical evidence, not current readiness. Immutable component
  publication, digest pinning and clean-consumer COPC integration remain gates.
- **Point-cloud plugins:** local LAS/LAZ/COPC/PLY behavior remains owned and
  documented by `usd-pointcloud-plugins`; the composed runtime verifies reach
  and interoperability, not the formats' internal capability matrix.
- **Raster plugins:** the core model, GeoTIFF metadata and initial uncompressed
  window reads exist, but no release is tagged. Component publication and the
  raster read/authoring acceptance selected by the runtime remain gates.

## Acceptance evidence

The first release is accepted only when a clean machine reconstructs the locked
runtime from immutable artifacts and demonstrates:

- OpenUSD tool and Python reachability;
- plugin, resolver and schema discovery;
- local point-cloud and raster fixture opening;
- HTTP metadata/range access, cache reuse and invalidation through the packaged
  resolver and format artifacts;
- normal CMake consumption of at least one exported runtime library; and
- a composition report binding every result to component and runtime digests.

## Related documentation

- Proposed contract:
  [Runtime composition foundation](../design/proposed/runtime-composition.md).
- Delivery plan:
  [v0.22.x runtime composition](../roadmap/runtime-composition.md).
- Existing point-cloud reference workspace:
  [USD Point Cloud Plugins](usd-pointcloud-plugins.md).
- Transport provider: [USD HTTP Resolver](usd-http-resolver.md).
- Raster workspace: [USD Raster Plugins](usd-raster-plugins.md).
- Formation execution/composition model:
  [formations.md](../design/proposed/formations.md).
