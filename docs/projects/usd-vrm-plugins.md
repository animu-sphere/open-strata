# USD VRM Plugins

[`animu-sphere/usd-vrm-plugins`](https://github.com/animu-sphere/usd-vrm-plugins)
is OpenStrata's reference **OpenUSD plugin workspace**: multiple independently
buildable and testable bundles developed together in one repository.

> This page summarizes what the project proves about OpenStrata and links to the
> project for everything else. The repository is authoritative for its
> installation, source build, architecture, feature/support matrix, and release
> notes. See the [cross-repository link policy](README.md#cross-repository-link-policy).

## Overview

The repository packages VRM (a glTF-based avatar format) support for OpenUSD as a
set of cooperating bundles plus a shared library. It is a **workspace**: the
bundles declare dependencies on one another and on an ordinary CMake library, and
OpenStrata validates and tests them in dependency order.

## Why it is an OpenStrata reference project

It is the concrete proof that OpenStrata supports a multi-bundle plugin workspace
end to end — not a toy example. It exercises:

- multi-bundle workspace discovery;
- declared bundle dependencies (`requires.bundles`) and dependency-order testing;
- ordinary CMake library dependencies alongside bundles;
- link-time versus runtime-only dependency separation;
- the full plugin `build → test → run → view → package` lifecycle;
- packaged clean-install validation (`--from-package`);
- generated multi-platform CI matrices;
- runtime artifact pinning by digest;
- dual-mode support — the same tree builds with `ost` **and** with plain CMake;
- downstream dogfooding reports that feed requirements back into OpenStrata.

## Workspace architecture

The workspace contains cooperating bundles and a shared library (see the
repository for the authoritative structure):

| Component | Kind | Role |
| --- | --- | --- |
| `vrmSchema` | USD schema bundle | typed VRM schemas |
| `usdVrmFileFormat` | `SdfFileFormat` bundle | read `.vrm` as a USD layer |
| `usdVrmPackageResolver` | `ArPackageResolver` bundle | resolve assets packaged inside a VRM |
| `vrmContainer` | ordinary CMake library | shared container code linked by the bundles |
| `execVrm` *(planned)* | runtime evaluation layer | future execution component |

OpenStrata **adopts** this architecture: it does not split these into an
artificial package abstraction, and it preserves the project's own CMake target
boundaries. The workspace graph OpenStrata reads is defined by the bundles'
declared dependencies; see
[reference/plugin-workspace.md](../reference/plugin-workspace.md) for the
contract.

## OpenStrata integration

- **Discovery & graph** — `ost plugin test --workspace` discovers every bundle
  and validates the dependency graph before testing.
- **Dependency kinds** — declared bundle dependencies and ordinary library
  dependencies are modeled distinctly, and link-time versus runtime-only edges
  are separated, so a consumer sees the real closure.
- **Packaging** — bundles package to immutable, digest-addressed artifacts;
  packaged output is re-validatable on a clean install with `--from-package`.
- **Runtime pinning** — CI cells pin a runtime artifact by digest, so the plugin
  is always tested against a known OpenUSD build.
- **Generated CI** — `ost ci generate github` renders the runtime × bundle
  support matrix into a workflow, rather than a hand-maintained matrix.

## Workflows demonstrated

These are **current** commands available today (v0.17 / v0.18). They are
illustrative; the repository's guide is authoritative.

Validate the graph, then test every bundle in dependency order:

```sh
ost plugin test --workspace
```

Build and test a single bundle:

```sh
ost plugin build plugins/usdVrmFileFormat
ost plugin test  plugins/usdVrmFileFormat
```

Run a tool inside the bundle's resolved runtime session:

```sh
ost plugin run plugins/usdVrmFileFormat \
    -- python plugins/usdVrmFileFormat/tools/inspect_vrm.py avatar.vrm
```

Open a fixture in `usdview` with the dependent bundles composed into the session:

```sh
ost plugin view plugins/usdVrmFileFormat avatar.vrm \
    --with plugins/vrmSchema \
    --with plugins/usdVrmPackageResolver
```

That last `--with` composition — assembling several bundles into one session by
hand — is exactly what the planned [Formation](../design/proposed/formations.md)
model makes declarative and reproducible. `ost formation` is **planned for
v0.19.0 and is not available today**; see
[combined-formations.md](combined-formations.md).

## Dogfooding and evidence

Adopting real OpenStrata releases in this repository surfaces defects that drive
OpenStrata's roadmap. The v0.17.0 pass recorded in the downstream report
`22-2026-07-17-v0.17.0-evidence-gate-v0.18.0-asks.md` found that `ost ci
generate` emitted an evidence gate no existing artifact could satisfy while
`ArtifactStore::import` silently dropped the evidence that would satisfy it — the
core of the [v0.18.0 evidence-integrity milestone](../roadmap/current.md).

These reports are linked as evidence, not copied. The downstream passes that
drove v0.18.0 are indexed in the [delivery reports](../reports/README.md).

## Current limitations

- The `execVrm` runtime evaluation layer is planned, not shipped.
- Cross-repository composition with a renderer (VRM rendered by hdMerlin) is a
  **planned** Formation workflow, not a current capability — see
  [combined-formations.md](combined-formations.md).
- Aggregate single-artifact workspace/product packaging is still being closed in
  OpenStrata (tracked in the [roadmap](../roadmap/current.md)); per-bundle
  packaging is available today.
- The authoritative, current feature and platform support matrix lives in the
  repository, not here.

## Related documentation

- Repository:
  [`animu-sphere/usd-vrm-plugins`](https://github.com/animu-sphere/usd-vrm-plugins)
  (authoritative for install, source build, architecture, support matrix, and
  release notes).
- OpenStrata plugin-workspace contract:
  [reference/plugin-workspace.md](../reference/plugin-workspace.md).
- Transferable procedure:
  [Adopt a plugin workspace](../guides/adopt-a-plugin-workspace.md).
- Planned cross-repository composition:
  [combined-formations.md](combined-formations.md),
  [Formation design](../design/proposed/formations.md).
- Reference Projects overview: [README.md](README.md).
