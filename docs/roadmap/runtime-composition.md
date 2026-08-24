---
title: v0.22.x runtime composition
status: active
owners:
  - openstrata-maintainers
created: 2026-08-24
updated: 2026-08-24
applies_to: v0.22.3-v0.22.9
---

# v0.22.x runtime composition

This is the execution plan for the proposed
[runtime-composition contract](../design/proposed/runtime-composition.md). It
contains only incomplete work. The next release is summarized in
[current.md](current.md); later slices are ordered in [backlog.md](backlog.md).

The series advances one contract at a time. DCC host adapters remain v0.23.0
work after this foundation has been dogfooded.

## v0.22.3 - canonical OpenUSD runtimes and artifact contracts

**Objective:** publish the canonical OpenUSD CY2026 base-runtime matrix from one
normalized model, and make any additional build result needed by a composed
runtime a trustworthy OST artifact without changing its project ownership.

The normalized matrix, variant semantics, migration rules, producer model,
verification abstraction, OCI naming and trust decisions are owned by the
[canonical OpenUSD runtime proposal](../design/proposed/canonical-openusd-runtimes.md).
The v0.22.3 release owns the following implementation workstreams.

### P1 - normalized runtime model

- Keep `profile = usd` separate from the graphics variant. Add canonical
  `core`, `gl`, `vulkan` and `metal`; accept legacy `standard` as a warning-
  producing alias for `gl`; preserve schema-versioned reads without guessing
  missing modern facts.
- Extend the normalized compatibility cell and selector with the profile,
  variant, exact producer OpenUSD version, provider/toolchain/ABI facts,
  capabilities and the macOS SDK/deployment target.
- Centralize capability-to-build-option translation in a version-aware
  `OpenUsdBuildPlan`; callers and producer scripts must not independently
  assemble compatibility-critical `build_usd.py` or CMake flags. The plan makes
  the version-appropriate OpenUSD examples build mandatory for `gl`, `vulkan`
  and `metal`; `core` is exempt.
- Provide one canonical leaf-tag formatter
  (`<openusd-version>-<variant>-<os>-<arch>`) while keeping compatibility
  selector, immutable digest and human tag as separate contracts.

### P1 - producer and verification matrix

- Replace the specialized Vulkan publisher/validator shape with one data-driven
  OpenUSD runtime matrix producer and platform build adapters. Do not add one
  publisher per variant.
- Produce both OpenUSD 26.05 and 26.08 for Linux x86_64 and Windows x86_64 in
  `core`, `gl` and `vulkan`, and for macOS arm64 in `core`, `gl` and `metal`.
  macOS x86_64 is optional rather than a primary release gate. An imaging cell
  that skips the upstream examples build cannot enter the canonical release set;
  `core` is exempt.
- Treat macOS arm64 as a first-class producer: capture Apple Clang, Xcode/SDK,
  deployment target, Python and oneTBB identity; validate dylib relocation; and
  add HgiMetal capability evidence.
- Preserve independent compile, link, loader, physical-device and render states,
  but select `NoGraphicsVerifier`, `OpenGlVerifier`, `VulkanVerifier` or
  `MetalVerifier` behavior from the normalized variant. Linux GLX/EGL, Windows
  WGL and macOS framework policy stay platform-local.
- Generate build, validate, export, SBOM, provenance, protected OCI publish and
  clean pull-by-digest verification jobs from the same support declaration.
  Publish normalized leaf tags first; OCI multi-platform aliases follow only
  when deterministic index transport is proven.

### P1 - component artifact closure

- Dogfood the implemented descriptor-scoped
  [`requires.libraries` lifecycle](../reference/plugin-workspace.md#source-workspace-composition)
  against the USD VRM non-leaf adapter using the release-lane OST pin, and
  retain build/test/package evidence that normal CMake discovery resolved the
  declared sibling closure.
- Add a data-only artifact/member contract, or an equivalent project-relative
  source-to-install mapping, so shared profiles and configuration can be staged
  once under `share/` and declared as a dependency. A tool-owned duplicate is
  not an acceptable substitute.
- Bind root `ost build` outputs to managed provenance consistently so packaging
  does not classify members from the same build as a mixture of managed and
  untracked outputs.

### P2/P3 hardening and migration

- Separate discovery from aggregate release membership. Let the project declare
  the exact members or explicit exclusions, print the resolved set, and support
  a pinned membership expectation for release lanes.
- Explain build-record migrations/mismatches caused by an OST upgrade; stage
  declared tool executables from the root build or report the actual missing
  step; name the affected member in every warning.
- When the running OST version differs from `bootstrap.ost.version` and a known
  discovery/packaging rule changes the result, report both versions and the
  resulting membership difference.
- Migrate reference projects to canonical runtime digests and generated support
  cells: USD plugin lanes choose `core`/`gl` as needed, hdMerlin chooses
  `vulkan`, and the macOS renderer lane proves `metal`.
- Retire ad-hoc aliases and superseded runtime packages only after the canonical
  cells are published and consumers are pinned. Retained legacy artifacts must
  be explicitly documented.

### Exit criteria

One normalized declaration produces the primary 26.05/26.08 Linux, Windows and
macOS arm64 matrix. Every published leaf passes build, runtime, backend-specific
graphics, artifact and clean digest-pull distribution gates, including the
mandatory OpenUSD examples build for imaging variants (`core` is exempt), with
a deterministic selector, SBOM and provenance. The USD VRM workspace can also
build and package a non-leaf adapter
library from its declared closure, ship shared motion profiles once under their
correct owner, and fail a release when aggregate membership differs from the
declared set. Every member carries coherent managed provenance and actionable
named diagnostics.

## v0.22.4 - runtime component model

**Objective:** several component artifacts can resolve into one runtime model.

- Define versioned component requirements/provisions for runtime, plugin,
  library, tool, renderer and data artifacts.
- Extend artifact manifests with dependency and environment-contribution
  metadata without forking existing plugin activation or Formation models.
- Resolve capabilities to providers deterministically, with explicit provider
  pins supported for release contracts.
- Diagnose missing providers, version/ABI/OpenUSD conflicts, singleton
  capability collisions, install-path collisions and incompatible environment
  contributions before materialization.
- Produce a canonical composition manifest and stable JSON inspection output.

### Exit criteria

A synthetic multi-artifact graph and the initial geospatial manifest resolve to
the same ordered model on Windows, Linux and macOS; conflicts fail before files
are written.

## v0.22.5 - locked composed runtime

**Objective:** a resolved runtime is reproducible and independently
transportable.

- Lock provider identity, version, digest, immutable source, dependency graph,
  target/variant and compatibility decisions.
- Derive runtime identity from canonical inputs and materialized inventory.
- Export the composed runtime as an OST artifact with provenance, attribution,
  SBOM and composition-level validation evidence.
- Reconstruct the same identity on a clean machine from the lock and immutable
  artifacts; caches remain optional.

## v0.22.6 - runtime SDK layout

**Objective:** applications consume a runtime through predictable native
conventions.

- Materialize the owner-recorded `bin`, `lib`, `include`, `share`, `plugins`,
  `python`, `node` and `metadata` roots.
- Generate deterministic activation/environment data using the existing
  Formation environment contract.
- Expose installed CMake config packages to a clean C++ consumer through normal
  `find_package(... CONFIG)` resolution.
- Validate loader, executable, plugin, resolver, schema and CMake-package
  reachability from an isolated prefix.

## v0.22.7 - geospatial runtime dogfood

**Objective:** build the first real composed runtime in parallel with
[`animu-sphere/usd-geospatial-runtime`](https://github.com/animu-sphere/usd-geospatial-runtime).

- Compose a digest-pinned canonical OpenUSD 26.08 cell, `usd-http-resolver`,
  `usd-pointcloud-plugins` and
  `usd-raster-plugins` from independently published artifacts.
- Keep the composition repository declarative: it owns capability selection,
  locks, fixtures and acceptance evidence, not a replacement build/solver.
- Exercise local file formats first, then prove HTTP/COPC Tier 2 through the
  packaged resolver and point-cloud artifacts.
- Publish the runtime artifact and reconstruct it on a clean consumer with the
  exact locked identity.

### Geospatial component readiness

The 2026-08-16
[pre-implementation report](https://github.com/animu-sphere/usd-pointcloud-plugins/blob/main/docs/reports/ost/04-2026-08-16-usd-http-resolver-preimplementation.md)
is historical and requested no OpenStrata fix. `usd-http-resolver` v0.4.0 now
ships the previously missing resolver/backend/test substrate. Remaining
acceptance is an immutable resolver artifact composed on a clean consumer with
COPC metadata/range reads, cache miss-to-hit reuse, validation-token invalidation
and a bytes-fetched baseline. `usd-raster-plugins` is pre-release; its component
artifact and selected GeoTIFF read/authoring contract must ship before the
geospatial runtime claims raster support.

## v0.22.8 - consumer packaging foundation

**Objective:** one canonical runtime can serve ecosystem-native entry points.

- Define Python wheel, npm/JavaScript/Wasm and native SDK packages as derived
  consumer distributions of pinned OST artifacts.
- Keep component/runtime identity, provenance and dependency truth in OST/OCI;
  ecosystem registries do not become competing canonical artifact stores.
- Prove one native SDK consumer and specify the binder/loader contract for
  Python and JavaScript without leaking OST internals into their public API.

## v0.22.9 - runtime UX and diagnostics

**Objective:** using a composed runtime is ordinary for humans, CI and agents.

- Stabilize the `runtime compose|explain|doctor|exec` (or final equivalent) CLI
  and JSON schemas.
- Extend diagnostics across artifact, dependency, plugin, resolver, loader,
  ABI, Python and device boundaries with stable error codes and remediation.
- Record component and composition validation separately and preserve explained
  host-capability SKIPs.
- Complete the end-to-end geospatial clean-consumer acceptance and decide
  whether the proposed design can be promoted to accepted.

## Dogfooding intake: USD VRM report 35

The 2026-08-24
[v0.22.2 release-artifact membership report](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/35-2026-08-24-v0.22.2-release-artifact-membership.md)
is the primary v0.22.3 intake. It confirms that recursive library/tool discovery
and `ost library` exist, then demonstrates the remaining artifact boundary:
non-leaf library dependencies are validated but not composed; aggregate
membership is inferred from discovery; shared data has no correct artifact
owner; and managed provenance/diagnostics vary across members. The P1/P2/P3
items above preserve all eight asks from the report rather than treating the
successful discovery count as closure.

The report also distinguishes workstation OST 0.22.2 from the repository's CI
pin at 0.21.0. Acceptance must exercise the version the release lane actually
runs, and a pin bump must not silently change product membership.

| Report ask | Priority | Roadmap owner |
| --- | --- | --- |
| 1. Compose `requires.libraries` in `ost library build` | P1 | v0.22.3 component artifact closure |
| 2. Add a data-only member or external source-to-install mapping | P1 | v0.22.3 component artifact closure; consumed by v0.22.4 components |
| 3. Bind root-build bundle/tool outputs to managed provenance | P1 | v0.22.3 component artifact closure |
| 4. Declare and pin aggregate-product membership | P2 | v0.22.3 component artifact closure |
| 5. Explain OST-upgrade build-record migration mismatches | P2 | v0.22.3 diagnostics |
| 6. Stage declared tool executables or report the actual prerequisite | P2 | v0.22.3 component artifact closure |
| 7. Name the affected member in packaging warnings | P3 | v0.22.3 diagnostics |
| 8. Explain result-changing workstation/CI OST-pin drift | P3 | v0.22.3 diagnostics; retained in v0.22.9 UX |
