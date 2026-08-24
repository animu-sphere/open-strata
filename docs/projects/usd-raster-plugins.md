# USD Raster Plugins

[`animu-sphere/usd-raster-plugins`](https://github.com/animu-sphere/usd-raster-plugins)
is an OpenStrata reference **OpenUSD raster plugin workspace**. It reads
geospatial raster data through a windowed, transport-independent interface,
starting with GeoTIFF, and targets existing OpenUSD schemas rather than defining
a custom raster schema.

> This page summarizes the OpenStrata integration. The repository owns format
> semantics, coordinate policy, capability status, releases and roadmap.

## Current status

The project is early and has no tagged release. Its repository/CI foundation,
OpenUSD-independent raster value model, GeoTIFF metadata reader and initial
uncompressed pixel-window reads are implemented and tested. Compression,
broader read planning and `UsdGeomMesh` authoring remain downstream work.

## Why it is an OpenStrata reference project

The repository adds a distinct geospatial plugin shape:

- core raster and GeoTIFF libraries that must build and test without OpenUSD;
- a metadata-first `SdfFileFormat` bundle for `.tif` / `.tiff`;
- explicit CRS, geotransform, pixel-anchor and NoData contracts;
- a project-wide `cy2026` / `usd` workspace and CI definition; and
- a transport-neutral `RandomAccessSource` / `ArAsset` boundary designed to
  compose with an active resolver at runtime.

OpenStrata adopts the project-owned library/plugin split. The raster repository
owns formats, sampling, tiling and USD authoring; `usd-http-resolver` owns HTTP,
range requests, caching and transport evidence.

## OpenStrata workflows

The current managed path is:

```sh
ost configure
ost build
ost test
```

The repository also retains an OpenUSD-free core CMake/CTest lane. That lane is
an architectural dependency check, not an alternative OpenStrata runtime.

## Runtime-composition intake

Before the first runnable geospatial runtime claim, the raster project must:

- tag and publish an immutable component artifact with provenance and
  attribution;
- complete the declared GeoTIFF read/authoring acceptance used by the runtime;
- prove local fixture opening through the packaged plugin; and
- prove remote window reads through the resolver without adding transport code
  or a direct resolver build dependency.

The composition validates reach and interoperability; the downstream capability
matrix remains authoritative for supported compression, metadata and authoring
behavior.

## Related documentation

- Repository capability matrix:
  [`docs/reference/CAPABILITY_MATRIX.md`](https://github.com/animu-sphere/usd-raster-plugins/blob/main/docs/reference/CAPABILITY_MATRIX.md).
- Implementation status:
  [`docs/roadmap/implementation-status.md`](https://github.com/animu-sphere/usd-raster-plugins/blob/main/docs/roadmap/implementation-status.md).
- Geospatial composition: [USD Geospatial Runtime](usd-geospatial-runtime.md).
- Transport provider: [USD HTTP Resolver](usd-http-resolver.md).
- Reference Projects overview: [README.md](README.md).
