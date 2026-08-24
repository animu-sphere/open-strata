# USD HTTP Resolver

[`animu-sphere/usd-http-resolver`](https://github.com/animu-sphere/usd-http-resolver)
is an OpenStrata reference **OpenUSD asset-resolver workspace**. It provides
random-access `http://` and `https://` reads without embedding format knowledge
or requiring whole-asset downloads.

> This page summarizes the OpenStrata integration. The repository owns its
> architecture, capability matrix, performance baselines, releases and roadmap.

## Current status

The resolver has released v0.4.0. Its local and HTTP backends, boundary and
hostile-server suites, `ArResolver` bundle, aligned block cache, asset identity,
validation-token exposure and persistent cache tier are implemented. The
repository records measured request/byte baselines rather than treating remote
access as an unverified capability claim.

## Why it is an OpenStrata reference project

The repository exercises boundaries that ordinary file-format workspaces do not:

- a library-first graph whose read contract, backends and tests build without
  OpenUSD, with OpenUSD required only for the resolver bundle;
- one project-wide `cy2026` / `usd` build, test and CI contract;
- an `ArResolver` artifact that provides transport to independently released,
  random-access-compatible file-format plugins;
- cross-platform core CI plus sanitizer lanes; and
- stable cache identity, consistency and transfer evidence suitable for runtime
  composition and clean-consumer verification.

The design boundary is strict: the resolver owns byte ranges, caching,
validation and transport metrics, but never parses COPC, GeoTIFF or another
format. Consumers see `ArAsset`; they do not link this repository directly.

## OpenStrata workflows

The repository uses the root managed lifecycle against a certified runtime:

```sh
ost runtime pull cy2026 --profile usd
ost build
ost test
```

Its OpenUSD-independent core lane remains a downstream CMake/CTest contract.
OpenStrata adopts that split instead of forcing the internal libraries into
artificial plugin bundles.

## Runtime-composition intake

The 2026-08-16 pre-implementation report is historical: the missing resolver
bundle, HTTP backend, CI and registered tests it observed now exist. The
remaining OpenStrata acceptance is distribution and composition evidence:

- publish and pin the resolver as an immutable component artifact;
- reconstruct it with the canonical OpenUSD runtime on a clean consumer;
- compose it with point-cloud and raster plugins without source-tree coupling;
- prove COPC metadata/range reads, cold-to-warm cache reuse, validation-token
  invalidation and bytes-fetched evidence; and
- bind those results to resolver, plugin and runtime digests.

## Related documentation

- Repository capability matrix:
  [`docs/reference/CAPABILITY_MATRIX.md`](https://github.com/animu-sphere/usd-http-resolver/blob/main/docs/reference/CAPABILITY_MATRIX.md).
- Resolver design:
  [`docs/architecture/RESOLVER.md`](https://github.com/animu-sphere/usd-http-resolver/blob/main/docs/architecture/RESOLVER.md).
- Geospatial composition: [USD Geospatial Runtime](usd-geospatial-runtime.md).
- Point-cloud consumer: [USD Point Cloud Plugins](usd-pointcloud-plugins.md).
- Reference Projects overview: [README.md](README.md).
