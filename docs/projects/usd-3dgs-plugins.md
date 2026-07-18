# USD 3DGS Plugins

[`animu-sphere/usd-3dgs-plugins`](https://github.com/animu-sphere/usd-3dgs-plugins)
is an OpenStrata reference **OpenUSD plugin workspace** for 3D Gaussian
Splatting assets. Its initial vertical slice reads the common Graphdeco Gaussian
PLY dialect and authors OpenUSD 26.05's standard
`ParticleField3DGaussianSplat` schema.

> This page summarizes what the project proves about OpenStrata and links to the
> project for everything else. The repository is authoritative for installation,
> architecture, PLY mapping, supported configurations, release notes, and its
> own roadmap. See the
> [cross-repository link policy](README.md#cross-repository-link-policy).

## Overview

The repository contains one read-only `SdfFileFormat` bundle,
`gaussian-ply`, and one format- and USD-independent ordinary CMake library,
`gaussianCore`. The bundle detects and decodes ASCII or binary little-endian
Gaussian PLY, maps position, scale, orientation, opacity, and spherical
harmonics into a fully materialized USD layer, and leaves rendering to a
separately selected Hydra implementation.

This is deliberately a different workspace shape from
[USD VRM Plugins](usd-vrm-plugins.md): one bundle consumes one plain library,
rather than several bundles consuming one another. Both retain a plain-CMake
build alongside the OpenStrata lifecycle.

## Why it is an OpenStrata reference project

The project was bootstrapped from an empty repository with OpenStrata 0.18.0 and
exercises:

- `usd-plugin-workspace` and `usd-fileformat` scaffolding replaced by real C++
  implementation without restructuring the repository;
- an ordinary library dependency declared as `gaussian-ply -> gaussianCore`;
- dependency-ordered library installation before the plugin configure/build;
- real OpenUSD 26.05 plugin discovery, `usdcat`, and `Usd.Stage.Open()` checks;
- source-workspace verification through L5 and package-origin verification;
- bundle-relative notices and dependency-closure packaging;
- generated, digest-pinned Windows, macOS arm64, and Linux PR cells;
- dual-mode support through both `ost` and a plain root CMake build;
- clean-directory package consumption and package reproducibility dogfooding.

## Workspace architecture

The authoritative structure is maintained by the downstream repository. The
OpenStrata-relevant dependency is intentionally small:

```text
gaussian-ply (SdfFileFormat bundle)
    -> gaussianCore (ordinary CMake/OpenStrata library)
```

`gaussianCore` owns the format-independent Gaussian data model, validation,
scale/opacity/quaternion math, and spherical-harmonic layout utilities. The
bundle owns the tinyPLY adapter, dialect validation, semantic decoding, USD
authoring, `plugInfo.json`, and fixtures. OpenUSD types do not enter the public
`gaussianCore` API.

## OpenStrata integration

- **Scaffold adoption** — the checked-in project and bundle manifests retain
  scaffold provenance while the generated skeleton has become a real plugin.
- **Library composition** — the plugin manifest declares a versioned
  `gaussianCore` requirement; OpenStrata builds and installs the library into
  the workspace prefix before configuring the bundle.
- **Verification** — `ost plugin test --workspace --up-to 5` validates the
  graph, discovery, tool execution, stage opening, and golden output against a
  real runtime.
- **Packaging** — `ost plugin package` produces a self-describing bundle with
  the plugin binary, fixtures, notices, and library closure; an extracted root
  can be passed directly to `ost plugin run`.
- **Generated CI** — `openstrata.ci.yaml` pins one runtime artifact and OCI
  identity per hosted OS cell and generates the checked-in source workflow.

## Workflows demonstrated

These are current OpenStrata 0.18 commands; the repository's README and install
guide are authoritative:

```sh
ost plugin build plugins/gaussian-ply
ost plugin doctor plugins/gaussian-ply
ost plugin test plugins/gaussian-ply --up-to 5
ost plugin test --workspace --up-to 5
ost plugin package plugins/gaussian-ply
ost plugin test plugins/gaussian-ply --from-package --up-to 5
```

A compatible PLY can be opened in the composed plugin/runtime session. Stage
inspection does not imply that the active Hydra renderer draws Gaussian splats:

```sh
ost plugin view plugins/gaussian-ply scene.ply
ost plugin run plugins/gaussian-ply -- \
  usdcat --flatten --usdFormat usdc --out scene.usd scene.ply
```

## Dogfooding and roadmap intake

The downstream repository keeps an append-only
[`docs/reports/ost/`](https://github.com/animu-sphere/usd-3dgs-plugins/tree/main/docs/reports/ost)
series. The first two reports found four upstream product seams now tracked in
the [v0.19.0 reach plan](../roadmap/current.md):

- package-origin L5 receives its roundtrip fixture but not the adjacent golden;
- packaging is not bound to the output of the last managed plugin build, so a
  plain-CMake build can silently replace the staged binary;
- CI evidence-gap diagnostics do not print the exact safe repull command even
  when every immutable input is already known;
- a package run outside a project defaults to `core` instead of deriving or
  clearly reporting the profile required by the package capabilities.

Report #2 also proved that two clean Windows builds become digest-identical once
MSVC compile, archive, and link steps use `/Brepro`. That project-side fix leaves
an OpenStrata opportunity for an explicit across-build reproducibility check;
the existing package-twice gate only compares one build.

## Current limitations

- Package-origin L5 currently skips its golden comparison; source L5 passes.
- `ost plugin package` currently trusts the bundle's staged `lib/` contents
  without comparing them to the last managed build output.
- The importer is read-only and fully materialized; SPZ, glTF/GLB Gaussian
  extensions, SOG, writing, streaming, and rendering are downstream future work,
  not OpenStrata capabilities.
- Rendering requires an independently compatible Hydra renderer. OpenStrata
  does not claim hdMerlin or any other reference renderer currently implements
  the standard Gaussian schema.
- The authoritative platform and feature support matrices live downstream.

## Related documentation

- Repository:
  [`animu-sphere/usd-3dgs-plugins`](https://github.com/animu-sphere/usd-3dgs-plugins).
- Downstream OST reports:
  [`docs/reports/ost/`](https://github.com/animu-sphere/usd-3dgs-plugins/tree/main/docs/reports/ost).
- OpenStrata plugin-workspace contract:
  [reference/plugin-workspace.md](../reference/plugin-workspace.md).
- Transferable procedure:
  [Adopt a plugin workspace](../guides/adopt-a-plugin-workspace.md).
- Planned Formation examples:
  [combined-formations.md](combined-formations.md),
  [Formation design](../design/proposed/formations.md).
- Reference Projects overview: [README.md](README.md).
