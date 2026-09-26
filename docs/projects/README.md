# Reference Projects

**Reference projects** are real downstream repositories that exercise, or are
being prepared to exercise, OpenStrata's contracts. OpenStrata is developed
against them rather than against isolated examples: active projects prove the
runtime, artifact, plugin, library, host, renderer, CI, and validation contracts
on substantially different, independently released project types. The status
of a new scaffold is stated on its own page.

They are **not** samples, demos, toy projects, or bundled examples. Each is its
own repository with its own architecture, release policy, and authoritative
documentation. These pages summarize *what each project proves about OpenStrata*
and link back to the project for everything else.

## Ecosystem map

```text
OpenStrata ecosystem
├── open-strata
│   └── runtime, build, test, package, artifact, CI, validation, and formation
│
├── usd-geospatial-runtime
│   └── reference composed-runtime acceptance repository (bootstrap)
│
├── usd-3dgs-plugins
│   └── reference Gaussian file-format workspace
│
├── usd-pointcloud-plugins
│   └── reference multi-format point-cloud workspace
│
├── usd-http-resolver
│   └── reference random-access HTTP resolver workspace
│
├── usd-raster-plugins
│   └── reference GeoTIFF/raster plugin workspace
│
├── usd-vector-plugins
│   └── reference GeoJSON/vector plugin workspace
│
├── usd-vrm-plugins
│   └── reference multi-bundle OpenUSD plugin workspace
│
├── usd-stage-runner
│   └── reference interactive Stage runtime and host workspace
│
├── hydra-merlin
│   └── reference renderer project
│
├── hydra-toon
│   └── avatar-renderer template bootstrap and validation
│
├── usd-physics-plugins
│   └── backend-neutral physics core and Jolt library workspace
│
├── usd-motion-plugins
│   └── shared motion values, sampling, recording and USD bridge
│
├── usd-mmd-plugins
│   └── PMX/VMD format workspace and MMD-specific motion semantics
│
└── motion-connectors
    └── planned live device and protocol connector workspace
```

`open-strata` owns the generic orchestration and compatibility model. Each
downstream repository retains ownership of its source architecture, native CMake
targets, domain behavior, feature support, release policy, and project-specific
documentation. OpenStrata **adopts** a project's architecture rather than forcing
every build unit into an artificial package abstraction.

## The projects

| Project | Category | What it proves | Main OpenStrata workflows |
| --- | --- | --- | --- |
| [OpenStrata](../concepts/overview.md) | Toolchain | Runtime, artifact, CI, validation, and Formation | `runtime`, `build`, `artifact`, `formation`, `ci` |
| [USD Geospatial Runtime](usd-geospatial-runtime.md) | Runtime composition | Locked multi-repository runtime, packaged geospatial probes, self-contained clean-consumer reconstruction | `runtime compose` / `validate` / `export` / `reconstruct` / `exec` |
| [USD 3DGS Plugins](usd-3dgs-plugins.md) | Plugin workspace | Fresh scaffold, bundle-to-library edge, Gaussian PLY import | `plugin build` / `test` / `run` / `view` / `package` |
| [USD Point Cloud Plugins](usd-pointcloud-plugins.md) | Plugin workspace | Four geospatial file formats, shared authoring/tiling stack, format arguments | `configure` / `build` / `test`, `plugin build` / `test` / `view` |
| [USD HTTP Resolver](usd-http-resolver.md) | Resolver workspace | Random-access HTTP transport, cache identity, resolver/file-format separation | `runtime pull`, `build`, `test` |
| [USD Raster Plugins](usd-raster-plugins.md) | Plugin workspace | Windowed GeoTIFF reads, explicit georeferencing, transport-neutral raster boundary | `configure`, `build`, `test` |
| [USD Vector Plugins](usd-vector-plugins.md) | Plugin workspace | GeoJSON import, deterministic vector authoring, mixed plugin/library product provenance | `configure` / `build` / `test`, `plugin build` / `test` / `run` / `package` |
| [USD VRM Plugins](usd-vrm-plugins.md) | Plugin workspace | Typed schemas, file formats, resolver, bundle graph | `plugin build` / `test` / `run` / `view` / `package` |
| [USD Stage Runner](usd-stage-runner.md) | Application workspace | Mixed library/schema/tool graph, named build intent, standalone and usdview host integration | `build` / `test`, `library build` / `test`, `plugin build` / `test` / `view` |
| [hdMerlin](hydra-merlin.md) | Renderer project | Managed renderer build, evidence, Hydra discovery | `build`, `validate`, `renderer view` |
| [hydra-toon](hydra-toon.md) | Renderer project | Renderer template bootstrap, OpenUSD 26.08, incremental evidence and viewport isolation | `build` / `test` / `validate`, `renderer viewport` |
| [USD Physics Plugins](usd-physics-plugins.md) | Library workspace | Backend-neutral physics, Jolt intent, installed consumers and artifact edges | `build` / `test`, `library build` / `test` / `verify-consumer` / `package` |
| [USD Motion Plugins](usd-motion-plugins.md) | Motion workspace | Shared motion libraries and tools, installed OpenUSD consumer, cross-repository library dependency need | `build` / `test`, `library build` / `test` / `verify-consumer` |
| [USD MMD Plugins](usd-mmd-plugins.md) | Plugin workspace | PMX/VMD format and motion boundaries, mixed library/tool/bundle graph | `build` / `test`, `plugin build` / `test` |
| [Motion Connectors](motion-connectors.md) | Connector workspace | Empty-workspace CI boundary today; future optional device/protocol members | `ci generate` after first member; connector lifecycle planned |

- **[USD Geospatial Runtime](usd-geospatial-runtime.md)** —
  [`animu-sphere/usd-geospatial-runtime`](https://github.com/animu-sphere/usd-geospatial-runtime):
  the first runtime-composition acceptance repository. It owns a declarative
  geospatial capability set, component pins, locks, fixtures and clean-consumer
  evidence while OpenStrata owns the generic resolution,
  materialization and verification machinery.
- **[USD 3DGS Plugins](usd-3dgs-plugins.md)** —
  [`animu-sphere/usd-3dgs-plugins`](https://github.com/animu-sphere/usd-3dgs-plugins):
  a read-only Gaussian PLY `SdfFileFormat` bundle backed by a
  format-independent ordinary CMake library. Read it for empty-repository
  scaffolding, bundle-to-library dependency composition, OpenUSD 26.05 Gaussian
  schema authoring, generated three-OS CI, package-origin validation, and
  reproducibility feedback.
- **[USD Point Cloud Plugins](usd-pointcloud-plugins.md)** —
  [`animu-sphere/usd-pointcloud-plugins`](https://github.com/animu-sphere/usd-pointcloud-plugins):
  four LAS, LAZ, COPC, and PLY file-format bundles backed by shared point-cloud
  libraries and authoring tools. Read it for a larger multi-format workspace,
  OpenUSD 26.08 generated CI, cross-platform verification, and a concrete case
  where a smoke fixture needs file-format arguments.
- **[USD HTTP Resolver](usd-http-resolver.md)** —
  [`animu-sphere/usd-http-resolver`](https://github.com/animu-sphere/usd-http-resolver):
  a released random-access HTTP/HTTPS `ArResolver` and transport substrate. Read
  it for library-first builds, resolver bundle integration, cache identity,
  validation tokens, measured range-transfer evidence, and the boundary between
  transport and file formats.
- **[USD Raster Plugins](usd-raster-plugins.md)** —
  [`animu-sphere/usd-raster-plugins`](https://github.com/animu-sphere/usd-raster-plugins):
  a GeoTIFF/raster workspace with released windowed reads, explicit
  georeferencing and an initial mesh authoring slice. Read it for the raster side of the
  resolver/file-format boundary and the staged path to packaged USD authoring.
- **[USD Vector Plugins](usd-vector-plugins.md)** —
  [`animu-sphere/usd-vector-plugins`](https://github.com/animu-sphere/usd-vector-plugins):
  a GeoJSON `SdfFileFormat` bundle backed by format-independent vector,
  GeoJSON-reader and OpenUSD-authoring libraries. Read it for deterministic
  GIS feature mapping, generated-template adoption, L0–L5 verification, and a
  release-provenance mismatch caught by downstream composition.
- **[USD VRM Plugins](usd-vrm-plugins.md)** —
  [`animu-sphere/usd-vrm-plugins`](https://github.com/animu-sphere/usd-vrm-plugins):
  a multi-bundle OpenUSD plugin workspace (schema bundle, `SdfFileFormat` plugin,
  `ArPackageResolver`, shared container library). Read it for workspace
  dependency composition, plugin lifecycle testing, packaging, clean-install
  validation, and generated CI matrices.
- **[USD Stage Runner](usd-stage-runner.md)** —
  [`animu-sphere/usd-stage-runner`](https://github.com/animu-sphere/usd-stage-runner):
  an experimental real-time Stage runtime composed from backend-neutral core
  libraries, SDL and Jolt adapters, a codeless schema plugin, a standalone tool,
  and a usdview host add-on. Read it for mixed-member release graphs, scoped
  library lifecycles, named build intents, and host-plugin composition feedback.
- **[hdMerlin](hydra-merlin.md)** —
  [`animu-sphere/hydra-merlin`](https://github.com/animu-sphere/hydra-merlin): a
  host-neutral Vulkan renderer with an optional Hydra 2 adapter. Read it for
  renderer projects that are *not* plugin workspaces — managed CMake execution,
  renderer evidence, capability-aware validation, runtime artifact adoption, and
  the managed `usdview` workflow.
- **[USD Motion Plugins](usd-motion-plugins.md)** —
  [`animu-sphere/usd-motion-plugins`](https://github.com/animu-sphere/usd-motion-plugins):
  the shared motion layer extracted from the VRM workspace. Read it for
  ordinary-library composition, installed OpenUSD consumers and the need for
  declared artifact-backed library dependencies between repositories.
- **[hydra-toon](hydra-toon.md)** —
  [`animu-sphere/hydra-toon`](https://github.com/animu-sphere/hydra-toon):
  a second renderer-template adopter with real Windows GPU, Hydra and viewport
  bootstrap evidence. Its first report drives the v0.23.7 renderer repairs;
  avatar-specific rendering remains downstream implementation work.
- **[USD Physics Plugins](usd-physics-plugins.md)** —
  [`animu-sphere/usd-physics-plugins`](https://github.com/animu-sphere/usd-physics-plugins):
  ordinary physics core and Jolt backend libraries, exercising installed
  package boundaries, named intents and published artifact consumption by
  Stage Runner. OpenUSD bridge and format policy remain separate boundaries.
- **[USD MMD Plugins](usd-mmd-plugins.md)** —
  [`animu-sphere/usd-mmd-plugins`](https://github.com/animu-sphere/usd-mmd-plugins):
  PMX stage import and VMD reading/binding through a mixed workspace. Read it
  for the boundary between format-owned semantics and shared motion values.
- **[Motion Connectors](motion-connectors.md)** —
  [`animu-sphere/motion-connectors`](https://github.com/animu-sphere/motion-connectors):
  a documented but currently empty build scaffold for device and protocol
  inputs. Read it for the planned connector boundary and the empty-workspace
  CI gap found during its adoption.

## Cross-project story

The strongest narrative is not that downstream projects independently use
`ost`. It is that **independently released OpenUSD components can be resolved,
validated, and composed into one reproducible execution environment**. The
plugin workspaces exercise format, schema and ordinary-library boundaries; the
HTTP resolver owns transport; the raster, vector and point-cloud projects keep
format behavior independent of transport; USD Motion Plugins owns reusable
motion values while USD VRM and USD MMD Plugins own avatar-format semantics;
Motion Connectors is the future live-input boundary; USD Stage Runner exercises
the application-host and interactive runtime boundary; USD Physics Plugins owns
backend-neutral simulation mechanics; hdMerlin and hydra-toon exercise the
renderer boundary; and USD Geospatial Runtime binds selected geospatial
artifacts into one locked, distributable runtime/SDK. A separate Formation case
opens VRM through the VRM bundles and renders it with hdMerlin; other stage
inspections do not imply renderer compatibility.

Per-command composition is the [Formation](../design/proposed/formations.md)
model: `ost formation resolve|inspect|lock|run` shipped in v0.19.0 and
`ost formation env|doctor` shipped in v0.20.0. Materializing a resolved graph as
an independently distributable runtime is the proposed
[runtime-composition](../design/proposed/runtime-composition.md) layer; it reuses
Formation rather than replacing it. Cross-repository Formation workflows are in
[combined-formations.md](combined-formations.md).

## Adopting OpenStrata for your own project

The transferable procedures behind these reference projects are written as
adoption guides that use the reference projects as worked examples without
becoming project-specific build guides:

- [Adopt a plugin workspace](../guides/adopt-a-plugin-workspace.md)
- [Adopt a renderer project](../guides/adopt-a-renderer-project.md)
- [Compose a formation](../guides/compose-a-formation.md) (v0.19.0–v0.20.0)

## Cross-repository link policy

OpenStrata and its reference projects link reciprocally without duplicating each
other's source of truth:

- **OpenStrata summarizes and links.** These pages describe which OpenStrata
  contract a project validates and point to the project's authoritative
  documentation for installation, source build, architecture internals, support
  matrices, troubleshooting, release notes, and roadmap. Large command
  references, support tables, and architecture documents are **not** copied here.
- **Downstream repositories link back.** Each reference project keeps a short
  *OpenStrata project* section linking to this repository, its OpenStrata
  reference-project page and the Formation documentation.
- **No duplicated source of truth.** Any snippet included on an OpenStrata page
  is minimal, labeled current or planned, and linked to the authoritative
  downstream guide. Downstream dogfooding reports are linked as evidence, not
  copied; the two v0.17.0 passes that drove the v0.18.0 plan are indexed in the
  [delivery reports](../reports/README.md), including the USD 3DGS bootstrap and
  package-provenance reports that informed the v0.19.0 release.

See the [documentation overview](../README.md) for how these pages relate to the
rest of the docs, and [concepts/overview.md](../concepts/overview.md) for what
OpenStrata is.
