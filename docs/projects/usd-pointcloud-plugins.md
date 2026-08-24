# USD Point Cloud Plugins

[`animu-sphere/usd-pointcloud-plugins`](https://github.com/animu-sphere/usd-pointcloud-plugins)
is an OpenStrata reference **OpenUSD plugin workspace** for point-cloud data.
It opens LAS, LAZ, COPC, and PLY sources through four `SdfFileFormat` bundles
backed by shared geospatial, point-stream, authoring, caching, and tiling code.

> This page summarizes what the project proves about OpenStrata and links to the
> project for everything else. The repository is authoritative for installation,
> architecture, format support, file-format arguments, release notes, and its
> own roadmap. See the
> [cross-repository link policy](README.md#cross-repository-link-policy).

## Overview

The workspace reads surveying, mapping, scanning, and general point-cloud data,
then authors `UsdGeomPoints`, metadata, OpenUSD 26.08 `usdLod` roots, and
payload-backed spatial tiles. LAS and LAZ cover conventional point records,
COPC adds local or resolver-backed hierarchy and range access, and PLY provides
ASCII and binary scalar-vertex streams with an explicit CRS contract.

The file-format adapters share format-independent point-cloud and authoring
libraries. A separate conversion tool handles long-running tiled generation;
rendering and LOD selection remain responsibilities of the consuming
application.

## Why it is an OpenStrata reference project

The project exercises a larger plugin-workspace shape than the other reference
repositories:

- four independently discoverable LAS, LAZ, COPC, and PLY file-format bundles;
- a shared native library stack for geospatial values, point streams, OpenUSD
  authoring, cache identity, and spatial tiling;
- a project-wide `cy2026` / `usd` contract against OpenUSD `>=26.08,<27.0`;
- digest-pinned OpenUSD 26.08 runtime artifacts with SBOM and provenance gates;
- generated pull-request and main CI lanes for Windows x86_64, macOS arm64, and
  Linux x86_64 — 24 cells after PLY joined the matrix;
- OpenStrata plugin discovery and smoke verification alongside the repository's
  own root CMake and CTest workflows; and
- strict file-format arguments that exposed a real limitation in the generic
  OpenStrata smoke-fixture contract.

## Workspace architecture

The authoritative dependency graph lives downstream. At a high level:

```text
.las  -> usdLas  -------\
.laz  -> usdLaz  --------+
.copc -> usdCopc ---------+-> shared point-cloud authoring/tiling -> OpenUSD stage
.ply  -> usdPly  --------/
                              |
                              +-> pointcloud-las / -laz / -copc / -ply bundles
                              +-> usd-pointcloud-convert
```

Readers stay independent of OpenUSD where practical. The adapters connect them
to shared USD authoring, while the tiling layer stays independent of any one
source format. OpenStrata adopts those existing CMake boundaries rather than
recasting each internal library as a separate package.

## OpenStrata integration

- **Project and bundle contracts** — `openstrata.toml` selects `cy2026` and the
  `usd` profile; each bundle declares its file-format capability, OpenUSD range,
  registration file, and available smoke fixtures.
- **Managed lifecycle** — the repository uses project-wide
  `ost configure|build|test` and bundle-scoped `ost plugin build|test|view`
  workflows against a managed runtime.
- **Generated CI** — `openstrata.ci.yaml` is the source of truth for the hosted
  matrix, including runner images, runtime and OCI digests, Python, host
  packages, verification depth, and evidence requirements.
- **Verification** — the generic pyramid checks bundle layout, library and
  file-format discovery, `usdcat`, and `Usd.Stage.Open()` as applicable to each
  cell.
- **Dual-mode builds** — shared libraries retain direct CMake/CTest coverage,
  while the OpenStrata lanes exercise the real OpenUSD plugin boundary.

## Workflows demonstrated

The downstream README and build guide are authoritative; representative current
commands are:

```sh
ost configure
ost build
ost test

ost plugin build plugins/pointcloud-ply
ost plugin test plugins/pointcloud-ply --up-to 4
ost plugin view plugins/pointcloud-las sample.las
```

The source CI contract is validated and rendered rather than duplicated in
hand-written workflow semantics:

```sh
ost ci validate --matrix openstrata.ci.yaml
ost ci generate github
```

## Dogfooding and roadmap intake

The first downstream
[`docs/reports/ost/`](https://github.com/animu-sphere/usd-pointcloud-plugins/tree/main/docs/reports/ost)
report records the PLY addition to the generated source-CI matrix. It found and
fixed two repository integration gaps: incomplete standalone CMake dependency
registration, and a direct `.ply` smoke fixture that could not provide the
format's required `epsg` argument. A small USDA reference carrying
`SDF_FORMAT_ARGS:epsg=4978` preserved the strict PLY contract and made all three
hosted PLY cells pass.

**Implemented for v0.22.0:** a smoke fixture may now declare both its path and
`file_format_arguments`. OST gives the normalized identifier to `usdcat`,
`Usd.Stage.Open()`, the smoke-to-roundtrip flatten fallback, and `usdview`, while
the legacy string form remains valid. The checked-in USDA wrapper remains a
portable option but is no longer required solely to supply `epsg`.

The report used the pinned `ost 0.21.0` available to the workspace while
preparing for v0.22.0. It intentionally does not claim validation against an
unreleased v0.22.0 binary.

The later
[HTTP resolver pre-implementation report](https://github.com/animu-sphere/usd-pointcloud-plugins/blob/main/docs/reports/ost/04-2026-08-16-usd-http-resolver-preimplementation.md)
exercised `ost 0.22.2` against the external `usd-http-resolver` skeleton. Runtime
resolution and its empty build passed; `ci validate` and `test` correctly
reported the missing matrix and tests. The report requests no OpenStrata change.
It is instead the readiness gate for the
[USD Geospatial Runtime](usd-geospatial-runtime.md): Tier 2 HTTP/COPC
interoperability waits for a resolver bundle, HTTP backend, stable identity
metadata and registered tests, then repeats the report with range-read, cache
reuse and invalidation evidence.

## Current boundaries

- The downstream capability matrix is the source of truth for supported LAS,
  LAZ, COPC, PLY, CRS, attribute, cache, tiling, and LOD behavior.
- The plugins import and author point-cloud data; they do not provide a renderer
  or claim that a particular Hydra implementation renders the result.
- PLY has no embedded CRS in the implemented contract and therefore requires an
  explicit `epsg` file-format argument when opened directly; a structured OST
  smoke fixture can now supply it.
- Broader real-world dataset measurement and future format expansion remain
  downstream roadmap work, not OpenStrata platform promises.

## Related documentation

- Repository:
  [`animu-sphere/usd-pointcloud-plugins`](https://github.com/animu-sphere/usd-pointcloud-plugins).
- PLY CI dogfooding report:
  [report 01](https://github.com/animu-sphere/usd-pointcloud-plugins/blob/main/docs/reports/ost/01-2026-08-11-v0.22.0-ply-fileformat-ci.md).
- HTTP resolver pre-implementation report:
  [report 04](https://github.com/animu-sphere/usd-pointcloud-plugins/blob/main/docs/reports/ost/04-2026-08-16-usd-http-resolver-preimplementation.md).
- Downstream capability matrix:
  [`docs/reference/CAPABILITY_MATRIX.md`](https://github.com/animu-sphere/usd-pointcloud-plugins/blob/main/docs/reference/CAPABILITY_MATRIX.md).
- OpenStrata plugin-workspace contract:
  [reference/plugin-workspace.md](../reference/plugin-workspace.md).
- Transferable procedure:
  [Adopt a plugin workspace](../guides/adopt-a-plugin-workspace.md).
- Reference Projects overview: [README.md](README.md).
