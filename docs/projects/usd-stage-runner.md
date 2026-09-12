# USD Stage Runner

[`animu-sphere/usd-stage-runner`](https://github.com/animu-sphere/usd-stage-runner)
is an OpenStrata reference **application workspace** for turning an OpenUSD
Stage into a transient interactive runtime world. It combines backend-neutral
runtime libraries, optional SDL and Jolt adapters, a codeless schema plugin, a
standalone executable and a usdview host add-on.

> This page summarizes what the project proves about OpenStrata and links to the
> repository for runtime behavior, authored schema contracts, architecture,
> build prerequisites and roadmap. See the
> [cross-repository link policy](README.md#cross-repository-link-policy).

## Current status

The project is experimental and has no tagged release. Fixed-step execution,
play-session lifecycle, input, physics, character control, camera following and
collision avoidance are implemented as vertical slices. The standalone
`stage_runner` host and the usdview adapter share the same Stage session and
write transient simulation state into a discardable anonymous layer rather
than changing persistent authored layers. Behavior, OpenExec and vehicle work
remain future slices.

## Why it is an OpenStrata reference project

The repository exercises a heterogeneous release workspace rather than a
file-format-only plugin tree:

- six reusable library members covering runtime, input, physics, character,
  camera and Stage integration;
- SDL and Jolt adapter libraries whose external SDKs are optional at configure
  time;
- a codeless OpenUSD schema bundle with package-origin verification;
- a standalone executable declared as a tool member;
- explicit workspace and release membership across all ten OpenStrata members;
- a named build intent that stages a native/Python usdview add-on into a bundle;
  and
- both project-wide and descriptor-scoped build/test lifecycles.

This makes it a useful consumer for the boundaries between libraries, plugins,
tools and host-specific deployment, including shapes OpenStrata does not yet
model directly.

## Workspace architecture

The detailed runtime graph is owned downstream. At a high level:

```text
inputCore -----------\
runtimeCore ----------+-> stageRuntime -> stage_runner
physicsCore ----------+       |          + SDL input
characterCore --------+       |          + Jolt physics
cameraCore -----------/       |
                              +-> usdviewStageRunner

runnerSchema -> OpenUSD schema registration for physics, character and camera
```

Core libraries keep OpenUSD, SDL, Jolt and host UI types out of their public
contracts. `stageRuntime` is the OpenUSD-facing boundary; the standalone and
usdview adapters drive the same session lifecycle. OpenStrata adopts these
project-owned CMake boundaries and does not recast every subsystem as a plugin.

## OpenStrata integration

- **Project contract** — `openstrata.toml` selects `cy2026` / `usd`, names every
  library, backend, schema and tool member explicitly, and declares all ten as
  release members.
- **Managed and scoped builds** — root `ost build|test` covers the complete
  CMake tree; `ost library build|test` independently exercises reusable members.
- **Schema lifecycle** — `ost plugin build|test` verifies the codeless
  `runnerSchema` bundle, registration and apply/flatten round-tripping.
- **Tool delivery** — `stage_runner` is a first-class executable member rather
  than an output accidentally reached through the plugin dependency graph.
- **Named intent** — `ost build --intent plugin-view` supplies a portable bundle
  path and stages the usdview Python/native payload with managed provenance.
- **Dual-mode builds** — plain CMake and OpenStrata remain supported; optional
  SDK adapters can report unavailable capability without blocking core tests.

## Workflows demonstrated

The repository documents these representative paths:

```sh
ost runtime pull cy2026 --profile usd
ost build
ost test

ost library build libs/runtimeCore
ost library test libs/runtimeCore

ost build --intent plugin-view
ost plugin build plugins/runnerSchema
ost plugin test plugins/runnerSchema --from-package --up-to 5
ost plugin view plugins/runnerSchema tests/fixtures/minimal.usda
```

The standalone smoke path runs from an activated development shell with a
deterministic frame bound. Interactive input and real physics additionally
require discoverable SDL and Jolt packages.

## Dogfooding and roadmap intake

The downstream report series records three OpenStrata integration boundaries:

1. Local Windows builds could finish their visible work but remain alive after
   Ninja output stopped, while equivalent Windows CI paths passed.
2. A pulled Windows runtime exported producer-specific Python paths and needed
   dependency-hint/imported-target repair before a direct consumer configured.
3. `ost plugin view --with` could not truthfully represent the usdview Python
   `PluginContainer` as one of the existing OpenUSD bundle kinds.

For the third case, the repository stages the host add-on into
`runnerSchema`'s conventional `python/` root through the `plugin-view` intent.
The root test suite, schema packaging and package-origin Level 0–5 verification
passed with that workaround. A full Level 6 interactive launch remained a SKIP
because the selected runtime did not contain usdview; the report asks for a
first-class host-add-on component shape and capability-aware view selection.
That work, the shared runtime-Python relocation issue and the first-stall
snapshot are scheduled in the
[downstream report intake](../roadmap/downstream-report-intake.md).

## Current boundaries

- The `usd` profile supplies OpenUSD but not SDL or Jolt; interactive input and
  Jolt-backed simulation require those packages separately.
- A full usdview launch requires a runtime with usdview, Qt and a display. The
  recorded package test proves registration/import and session behavior, not a
  successful interactive Level 6 launch.
- Current authored physics supports box colliders and constrained transform
  shapes; exact behavior and restrictions are maintained downstream.
- OpenExec, behavior and vehicle targets are not implemented.
- The usdview add-on is currently delivered inside the schema bundle because
  OpenStrata has no independent host-add-on member kind.

## Related documentation

- Repository:
  [`animu-sphere/usd-stage-runner`](https://github.com/animu-sphere/usd-stage-runner).
- Downstream documentation:
  [`docs/README.md`](https://github.com/animu-sphere/usd-stage-runner/blob/main/docs/README.md).
- Architecture overview:
  [`docs/architecture/overview.md`](https://github.com/animu-sphere/usd-stage-runner/blob/main/docs/architecture/overview.md).
- OpenStrata report series:
  [`docs/reports/ost/`](https://github.com/animu-sphere/usd-stage-runner/tree/main/docs/reports/ost).
- usdview host-add-on report:
  [report 03](https://github.com/animu-sphere/usd-stage-runner/blob/main/docs/reports/ost/03-2026-09-02-v0.22.8-usdview-host-plugin-composition.md).
- OpenStrata plugin-workspace contract:
  [reference/plugin-workspace.md](../reference/plugin-workspace.md).
- Reference Projects overview: [README.md](README.md).
