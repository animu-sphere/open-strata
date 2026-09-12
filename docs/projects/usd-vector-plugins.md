# USD Vector Plugins

[`animu-sphere/usd-vector-plugins`](https://github.com/animu-sphere/usd-vector-plugins)
is an OpenStrata reference **OpenUSD plugin workspace** for GIS vector data.
Its first bundle opens GeoJSON as deterministic OpenUSD Points, BasisCurves,
Meshes and feature hierarchies through a transport-independent reader and a
separate authoring layer.

> This page summarizes what the project proves about OpenStrata and links to the
> project for format semantics, architecture, installation, capability status,
> releases and roadmap. See the
> [cross-repository link policy](README.md#cross-repository-link-policy).

## Current status

The project is on its v0.1.x stabilization line. The format- and
OpenUSD-independent vector model, buffered and cursor-based GeoJSON readers,
OpenUSD authoring library and `vector-geojson` FileFormat bundle are implemented.
The bundle has passed OpenStrata Levels 0–5 on the pinned Windows OpenUSD 26.08
runtime. Scalability work remains active; runtime-composition acceptance and a
second vector format remain later downstream milestones.

## Why it is an OpenStrata reference project

The repository exercises a focused plugin-plus-libraries product:

- three ordinary libraries with explicit dependency edges and installable
  CMake package metadata;
- one generated-template `usd-fileformat` bundle providing
  `usd-fileformat:geojson`;
- canonical `.geojson` and generic `.json` inputs, including roundtrip and
  negative fixtures;
- project-wide `cy2026` / `usd` runtime and workspace contracts;
- workspace graph, root build/CTest and bundle-scoped L0–L5 verification; and
- release-product provenance consumed and checked by an independent runtime
  composition repository.

The last point is important: a well-formed package can still be unsuitable for
composition if it was built against a different runtime than the project lock.
This repository supplied a concrete end-to-end case for that boundary.

## Workspace architecture

The authoritative graph lives downstream. Its OpenStrata-facing shape is:

```text
GeoJSON bytes
    -> usdGeoJson
        -> usdVectorCore
    -> usdVectorAuthoring
        -> usdVectorCore + OpenUSD
    -> vector-geojson FileFormat bundle
        -> deterministic OpenUSD Stage
```

`usdVectorCore` owns format- and USD-independent geometry and feature values;
`usdGeoJson` parses GeoJSON into that model; `usdVectorAuthoring` maps the model
to OpenUSD; and the bundle limits itself to registration, arguments, `ArAsset`
adaptation and composition of those libraries. Transport, credentials,
reprojection and rendering stay outside the plugin.

## OpenStrata integration

- **Workspace contract** — `openstrata.toml` selects `cy2026` / `usd`, declares
  `libs/*` and `plugins/*` as members, and installs acceptance probes as
  project-owned product data.
- **Bundle contract** — the plugin manifest declares OpenUSD
  `>=26.08,<27.0`, GeoJSON capability, registration metadata and positive,
  roundtrip and negative fixtures.
- **Dual lifecycle** — root `ost build|test` covers the workspace's own CMake
  graph, while `ost plugin build|test|run` exercises discovery and real file
  opening at the bundle boundary.
- **Packaging** — the workspace can publish the bundle and its library closure
  as a product for independent runtime composition.
- **Evidence** — downstream reports record both successful plugin verification
  and the runtime-identity failure found when the product was composed.

## Workflows demonstrated

Representative commands from the downstream dogfooding pass are:

```sh
ost plugin inspect plugins/vector-geojson
ost plugin build plugins/vector-geojson
ost plugin test plugins/vector-geojson --up-to 5
ost plugin test --workspace --graph-only
ost build
ost test
ost plugin run plugins/vector-geojson -- usdcat tests/fixtures/basic.geojson
```

The graph-only pass verified one bundle, three libraries and two library edges.
The L0–L5 pass covered registration, `usdcat`, Python Stage opening and flattened
golden output for both `.geojson` and GeoJSON-bearing `.json` inputs.

## Dogfooding and roadmap intake

The first report found repository-side manifest, CMake composition, OpenUSD
26.08 API and `WriteToString` integration gaps. After correction, the plugin
pyramid and all workspace CTests passed without an OpenStrata source change. It
also recorded a non-blocking provenance-attribution warning when a later root
build encountered unchanged outputs from an earlier bundle build.

The second report found a stronger release boundary. The v0.1.0 product had
been packaged against a hand-placed OpenUSD tree whose digest differed from the
runtime pinned by `strata.lock`. Packaging and validation passed, but
`usd-geospatial-runtime` correctly refused the product during composition. The
report asks OpenStrata to compare package runtime provenance with the project
lock, make `ost lock --check` compare normalized content rather than formatting,
and align its documented exit behavior. The vector repository records a local
release-gate workaround and plans a corrected release. These reusable asks are
scheduled in the
[downstream report intake](../roadmap/downstream-report-intake.md).

## Current boundaries

- GeoJSON is the implemented format; FlatGeobuf and indexed selective reads are
  not current capabilities.
- The project preserves source CRS metadata and uses an explicit local-origin
  policy, but does not perform implicit reprojection.
- Resolver implementations own byte transport, retries, authentication and
  caches; the vector reader owns feature semantics.
- Scalability baselines exist, but broader dataset and cross-platform evidence
  remain downstream work.
- The v0.1.0 provenance report must be considered before selecting that product
  as an immutable runtime-composition input.

## Related documentation

- Repository:
  [`animu-sphere/usd-vector-plugins`](https://github.com/animu-sphere/usd-vector-plugins).
- Downstream documentation:
  [`docs/README.md`](https://github.com/animu-sphere/usd-vector-plugins/blob/main/docs/README.md).
- GeoJSON bundle dogfooding:
  [report 01](https://github.com/animu-sphere/usd-vector-plugins/blob/main/docs/reports/ost/01-2026-09-02-v0.22.8-geojson-fileformat-dogfooding.md).
- Release-provenance dogfooding:
  [report 02](https://github.com/animu-sphere/usd-vector-plugins/blob/main/docs/reports/ost/02-2026-09-06-v0.22.8-release-provenance-dogfooding.md).
- Planned composition consumer:
  [USD Geospatial Runtime](usd-geospatial-runtime.md).
- Transport provider: [USD HTTP Resolver](usd-http-resolver.md).
- Transferable procedure:
  [Adopt a plugin workspace](../guides/adopt-a-plugin-workspace.md).
- Reference Projects overview: [README.md](README.md).
