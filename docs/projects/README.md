# Reference Projects

**Reference projects** are real downstream repositories that actively exercise
OpenStrata's contracts. OpenStrata is developed against them rather than against
isolated examples: they prove the runtime, artifact, plugin, renderer, CI, and
validation contracts work on substantially different, independently released
project types.

They are **not** samples, demos, toy projects, or bundled examples. Each is its
own repository with its own architecture, release policy, and authoritative
documentation. These pages summarize *what each project proves about OpenStrata*
and link back to the project for everything else.

## Ecosystem map

```text
OpenStrata ecosystem
├── open-strata
│   └── runtime, build, test, package, artifact, CI, validation, and (planned) formation
│
├── usd-3dgs-plugins
│   └── reference Gaussian file-format workspace
│
├── usd-vrm-plugins
│   └── reference multi-bundle OpenUSD plugin workspace
│
└── hydra-merlin
    └── reference renderer project
```

`open-strata` owns the generic orchestration and compatibility model. Each
downstream repository retains ownership of its source architecture, native CMake
targets, domain behavior, feature support, release policy, and project-specific
documentation. OpenStrata **adopts** a project's architecture rather than forcing
every build unit into an artificial package abstraction.

## The projects

| Project | Category | What it proves | Main OpenStrata workflows |
| --- | --- | --- | --- |
| [OpenStrata](../concepts/overview.md) | Toolchain | Runtime, artifact, CI, validation, and (planned) Formation | `runtime`, `build`, `artifact`, `ci` |
| [USD 3DGS Plugins](usd-3dgs-plugins.md) | Plugin workspace | Fresh scaffold, bundle-to-library edge, Gaussian PLY import | `plugin build` / `test` / `run` / `view` / `package` |
| [USD VRM Plugins](usd-vrm-plugins.md) | Plugin workspace | Typed schemas, file formats, resolver, bundle graph | `plugin build` / `test` / `run` / `view` / `package` |
| [hdMerlin](hydra-merlin.md) | Renderer project | Managed renderer build, evidence, Hydra discovery | `build`, `validate`, `renderer view` |

- **[USD 3DGS Plugins](usd-3dgs-plugins.md)** —
  [`animu-sphere/usd-3dgs-plugins`](https://github.com/animu-sphere/usd-3dgs-plugins):
  a read-only Gaussian PLY `SdfFileFormat` bundle backed by a
  format-independent ordinary CMake library. Read it for empty-repository
  scaffolding, bundle-to-library dependency composition, OpenUSD 26.05 Gaussian
  schema authoring, generated three-OS CI, package-origin validation, and
  reproducibility feedback.
- **[USD VRM Plugins](usd-vrm-plugins.md)** —
  [`animu-sphere/usd-vrm-plugins`](https://github.com/animu-sphere/usd-vrm-plugins):
  a multi-bundle OpenUSD plugin workspace (schema bundle, `SdfFileFormat` plugin,
  `ArPackageResolver`, shared container library). Read it for workspace
  dependency composition, plugin lifecycle testing, packaging, clean-install
  validation, and generated CI matrices.
- **[hdMerlin](hydra-merlin.md)** —
  [`animu-sphere/hydra-merlin`](https://github.com/animu-sphere/hydra-merlin): a
  host-neutral Vulkan renderer with an optional Hydra 2 adapter. Read it for
  renderer projects that are *not* plugin workspaces — managed CMake execution,
  renderer evidence, capability-aware validation, runtime artifact adoption, and
  the managed `usdview` workflow.

## Cross-project story

The strongest narrative is not that three downstream projects independently use
`ost`. It is that **independently released OpenUSD components can be resolved,
validated, and composed into one reproducible execution environment**. The two
plugin workspaces exercise different shapes — a single file-format bundle with
an ordinary library dependency and a multi-bundle avatar stack — while hdMerlin
exercises the renderer boundary. One concrete planned composition is a VRM file
opened through the VRM bundles and rendered by hdMerlin in a single Vulkan
viewport; Gaussian PLY stage inspection supplies another independent packaged
plugin dogfood without claiming renderer compatibility.

That composition is the planned [Formation](../design/proposed/formations.md)
model. `ost formation` is **planned for v0.19.0 and is not available today**; the
planned cross-repository workflows are documented — clearly labeled as planned —
in [combined-formations.md](combined-formations.md).

## Adopting OpenStrata for your own project

The transferable procedures behind these reference projects are written as
adoption guides that use the reference projects as worked examples without
becoming project-specific build guides:

- [Adopt a plugin workspace](../guides/adopt-a-plugin-workspace.md)
- [Adopt a renderer project](../guides/adopt-a-renderer-project.md)
- [Compose a formation](../guides/compose-a-formation.md) (v0.19.0-oriented)

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
  reference-project page, and — once v0.19.0 ships — the Formation documentation.
- **No duplicated source of truth.** Any snippet included on an OpenStrata page
  is minimal, labeled current or planned, and linked to the authoritative
  downstream guide. Downstream dogfooding reports are linked as evidence, not
  copied; the two v0.17.0 passes that drove the v0.18.0 plan are indexed in the
  [delivery reports](../reports/README.md), including the USD 3DGS bootstrap and
  package-provenance reports that extend the v0.19.0 reach plan.

See the [documentation overview](../README.md) for how these pages relate to the
rest of the docs, and [concepts/overview.md](../concepts/overview.md) for what
OpenStrata is.
