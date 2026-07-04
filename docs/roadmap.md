# Roadmap

Delivery is phased. Each phase is a usable increment, not a big-bang. Linux x86_64
is the first-class implementation target; other OS targets are modeled from the
start but may be unavailable initially.

Legend: ✅ done · 🚧 in progress · ⬜ not started

## Release milestones

Phases are the long-form structure; releases are the shipped increments cut from
them. Each release is a coherent slice, not a phase boundary.

- ✅ **v0.1.0** — first public release: foundation through OpenUSD/MaterialX
  profiles and the static plugin-verification framework (Phases 0–3, Phase 4a).
- ✅ **v0.2.0** — machine-readable `--json` output + error/exit-code contract,
  build-lifecycle hardening (per-target trees, runtime-env-consistent CMake,
  progress stream), and the security P0/P1 baseline (SEC-001…004).
- ✅ **v0.3.0** — Phase 4b plugin-harness dogfooding round: relative-path
  `plugin build|test`, MSVC bootstrap, loadable `plugInfo.json`, real USD-version
  detection, `plugin package` artifacts, the fmt/clippy/test CI gates, and the
  plugin bundle `license` field.
- ✅ **v0.4.0 — the schema plugin kind.** Where 0.3.0 made the *file-format*
  bundle path solid, 0.4.0 adds `usd-schema` as a first-class kind and closes the
  remaining Phase 4 scaffold/diagnostic gaps. Phase 6-independent. Scope:
  - **Schema bundles (A)** — the
    [Phase 4 — schema-bundle backlog](#phase-4--schema-bundle-backlog-from-downstream-plugin-dogfooding-reports-34)
    below. Done (✅): codeless `usd-schema` template + codeless-aware L0 doctor
    (e2e-hardened so it registers on a real runtime), the schema test contract
    (L2/L4, verified e2e on OpenUSD 26.08), co-hosting a schema in an existing
    bundle, per-variant `cxx_abi`, the `usdGenSchema` regenerate build step, and
    the `usdGenSchema` `Types` *merge* into a co-hosting bundle's existing
    `plugInfo.json` (all verified e2e on OpenUSD 26.08). **Deferred to a later
    release:** the compiled (non-codeless) schema variant — the codeless +
    co-hosting paths cover the data-contract use case; the typed-C++ importer API
    is a heavier, separable increment.
  - **Phase 4 close-out (B)** — P3 repo-shape scaffold and `ost doctor`
    structuring (§14.5), both tagged `(v0.4.0)` in-place below.
  - Out of scope (deferred): `plugin publish` + the runtime×plugin CI matrix
    (blocked on the Phase 6 artifact source) and runtime/extension content
    attribution (lands with the Phase 6 content store).
- ✅ **v0.5.0 — schema authoring hardening + workspace ergonomics.** A
  consolidation release after the schema-kind milestone: make the v0.4 codeless
  and co-hosted schema paths reliable across Windows/macOS, remove the remaining
  "works only if you know the trick" UX, and keep Phase 6-dependent publishing
  work out of this cut. Scope:
  - **Delivered:** UTF-8-forced schema generation, the
    `schema.library_prefix` double-prefix hint, per-target metadata adoption
    nudges, runtime-version drift reporting across `show`/`validate`/doctor JSON
    and human output, the discoverable `usd-plugin-workspace` template alias,
    `plugins/<name>/` workspace discovery, `ost plugin new` workspace guidance,
    macOS `runtime pull --build` notes for the known source-build edges, and the
    schema build-hook groundwork for a compiled co-located flow.
  - **Still out of scope:** `plugin publish`, the runtime×plugin CI matrix, and
    runtime/extension content attribution; those remain tied to the Phase 6
    artifact source/content store. A first-class compiled co-located schema UX
    (`ost plugin schema add` or a documented manifest-driven equivalent) also
    remains a v0.6.0 follow-up from the v0.5.0 dogfooding recheck.
- 🚧 **v0.6.0 — artifact registry + publishable plugin CI.** The first practical
  Phase 6 slice: make runtime/plugin/package artifacts addressable by digest,
  publish plugin package outputs into a local registry, and use those artifacts
  as the source of truth for a small runtime×plugin CI matrix. Scope:
  - **Artifact store MVP:** ✅ local content-addressed store and registry index
    (`ost-artifact`), `tar.zst` + manifest + checksums + validation report as
    the canonical bundle, digest-pinned `ost artifact import|export|list|show`,
    artifact integrity verification (`ost artifact verify`), and the
    `RuntimeSource::Artifact` path: `ost runtime export` packs a validated real
    runtime into the registry and `ost runtime pull --from-artifact <digest>`
    materializes it anywhere.
  - **Plugin publish MVP:** ✅ `ost plugin publish` consumes an existing
    `ost plugin package` output, refuses missing validation/provenance/license/
    notices with per-cause stable error codes, requires the frozen concrete
    target ABI (`package` already collapses `cxx_abi: inherit`), and publishes
    by digest rather than by mutable name.
  - **CI matrix MVP:** GitHub Actions first, Jenkins generator later. Matrix cells
    are explicit support lines (`runtime artifact digest × plugin artifact digest
    × target/profile`) rather than a naive Cartesian product. PR CI keeps cheap
    mock/static checks; scheduled or release CI runs real runtime/plugin L0..L6
    cells from the registry.
  - **Dogfooding #7 follow-ups:** make the compiled co-located schema path
    product-shaped (command or manifest UX, bundle-relative schema source paths
    such as `schema/schema.usda`, generated C++ linked into the existing plugin
    library, `Types` merge, `generatedSchema.usda` staging, export define);
    improve adopted-runtime drift repair UX (`--fix`, `repair`, or exact
    copy-paste re-adopt command); re-check macOS source-build ergonomics.
  - **Deferred:** OCI layout / ORAS transport, remote hosted registry, Kubernetes
    execution, full Jenkins command surface, and DCC host matrices.

## Phase 0 — Foundation ✅

Rust workspace + `ost` CLI skeleton, machine-readable platform manifests, project
and lock schemas.

- ✅ `ost-core` / `ost-platform` / `ost-manifest` / `ost-cli` crates
- ✅ Built-in CY2025 / CY2026 / CY2027 manifests (embedded + user overlay)
- ✅ `ost platform list | show | diff`
- ✅ `ost init` (writes `openstrata.toml` + `.strata/`)
- ✅ JSON schemas for platform / project / lock
- ✅ `--json` output and deterministic exit codes — a versioned
  `{ok, schema, data, warnings}` envelope with stable `error.code`/`category` and
  category-based exit codes ([json-schema.md](json-schema.md))

## Phase 1 — Runtime and devshell ✅

Resolve a runtime manifest, lay it out locally, generate environment, enter a shell.

- ✅ Runtime target model + resolver (`ost-runtime`)
- ✅ Profile model + loader (`core`/`dev`/`usd`/`lookdev`)
- ✅ Environment generation (`PATH`, `LD_LIBRARY_PATH`/`DYLD_*`/`PATH`, `PYTHONPATH`,
  `CMAKE_PREFIX_PATH`, `PXR_PLUGINPATH_NAME`)
- ✅ `ost env` and `ost devshell` (bash/pwsh)
- ✅ `ost runtime pull | list | show` against a local/mock backend
- ✅ Digest-bearing runtime manifest (`runtime.json`, deterministic digest)
- ✅ `ost doctor` (host descriptor, host tool detection, runtime report;
  deterministic exit: 0 healthy / precondition code (4) on issues)
- ✅ **(v0.4.0)** `ost doctor` structuring (§14.5): issues are now structured
  `{id, severity, summary, next_action}` (human + `--json`), the runtime report
  surfaces `kind` (mock/adopted/built/downloaded, derived from the manifest
  `source` — no schema change) plus its execution capability (real OpenUSD vs
  static-only), and an active mock runtime emits a `MOCK_RUNTIME_ACTIVE`
  *warning* that does not fail the run (only `error`-severity issues do). Absorbs
  the agent "status" need into `doctor` rather than a new command
- ✅ `ost runtime validate` (schema, digest integrity, layout; records outcome
  in the manifest; deterministic exit)
- ✅ `ost runtime explain` (delivered in Phase 3)
- ✅ Project lockfile `strata.lock` via `ost lock [--check]` and refreshed by
  `ost configure`: pins runtime id/variant/digest, Python ABI + `uv.lock` hash,
  resolved extensions, and validation status; fully deterministic so `--check`
  gates CI
- ✅ Real runtime backends behind `pull` (Phase 4b): `local`/adopt and `build`
  (build_usd.py / CMake-direct) supersede the mock layout; the fetched
  `artifact` source landed with the Phase 6 registry (v0.6.0:
  `runtime export` / `pull --from-artifact`)
- ✅ Richer runtime validation: `runtime validate` asserts `usdcat` + `pxr` on a
  real runtime; native library load + USD stage open are exercised by the plugin
  execution levels (L2–L4, Phase 4b)

## Phase 2 — CMake target build ✅

- ✅ Target model + id (`cy2026-linux-x86_64-py313-usd`) in `ost-build`
- ✅ `ost configure`: `toolchain.cmake`, `env.json`, `target.lock.json`,
  per-target `CMakePresets.json`, and a root `CMakePresets.json` that includes
  each target (verified with `cmake --list-presets`)
- ✅ `ost build`: regenerates the target then runs `cmake --preset` +
  `cmake --build`; locates ninja on PATH / `OST_NINJA` / `--ninja`; `--dry-run`
  and `--jobs`; propagates the build exit code (verified end-to-end: a real
  MSVC+Ninja build of a sample project produced and ran an executable)
- ✅ Windows MSVC-env auto-bootstrap inside `ost build`: locates `vcvars64.bat`
  (vswhere or known paths), captures the env delta, injects it into CMake/Ninja;
  `--no-vcvars` to opt out (verified: a plain shell with no developer prompt
  builds and runs an executable)
- ✅ `ost package`: `cmake --install` into a stage tree, pack to
  `dist/<name>/<version>/<target>/*.tar.zst` with per-file SHA-256, a
  content-addressed `manifest.json` (provenance + runtime digest + validation),
  and `SHA256SUMS` (verified: archive extracts and the binary runs)
- ✅ `ost validate`: checks configured / built / runtime-compatible (digest
  drift) / artifact-integrity (recomputed archive digest); skips the artifact
  check when not packaged; deterministic exit 0/1 (verified: tampering the
  archive fails the check)

## Phase 3 — OpenUSD / MaterialX profiles ✅

- ✅ OpenUSD extension family with feature sets (core/python/imaging/materialx/…)
  and MaterialX, in the new `ost-extension` crate (embedded + overlay loader)
- ✅ Capability resolver: capability → providing extension + feature, pulling in
  transitive extensions (usd-materialx → openusd[materialx] → materialx) and the
  packages each feature needs
- ✅ Compatible range vs certified build point (chosen per resolved feature set)
- ✅ `ost runtime explain` (capability → provider/extension graph, human/--json)
- ✅ `ost extension list | why | add`: list the catalog, trace why an extension
  is required by a profile (direct + transitive), and record it in
  `openstrata.toml` (idempotent, validated against the catalog)

## Phase 4 — OpenUSD plugin verification harness 🚧

Direction: [phase-4-plugin-harness.md](phase-4-plugin-harness.md). Split around
the one hard dependency — a real OpenUSD runtime (today's `runtime pull` is mock).

**4a — framework + static verification (mock backend, no real runtime): ✅**

- ✅ `ost-plugin` crate + Plugin Bundle contract (`openstrata.plugin.yaml`):
  manifest model, bundle loader, dependency-free version-range checks
- ✅ `ost plugin new` scaffold from the embedded `usd-fileformat-cpp` template
  (C++ `SdfFileFormat` + `plugInfo.json` + `CMakeLists` + fixtures + manifest)
- ✅ `ost plugin inspect` (Level 0 structure) and `ost plugin build` (generates a
  toolchain via `ost-build` and drives CMake; `--dry-run`)
- ✅ `ost plugin doctor`: Levels 0–1 (manifest, plugInfo, shared library,
  fixtures; OpenUSD range / ABI / required components) with stable diagnostic ids
  + session-env preview; Levels 2+ reported as `SKIP (needs real runtime)` —
  never a false PASS
- ✅ reports under `.strata/reports/<plugin>/<UTC>/` (`report.json` /
  `summary.txt` / `environment.json`) + published
  [plugin-report JSON schema](../schemas/plugin-report.schema.json);
  human + `--json`, deterministic exit codes

**4b — execution levels (gated on a real OpenUSD runtime backend): 🚧**

- ✅ pluggable runtime backend **sources** behind `pull`
  (`mock|local|build|artifact`), recorded in the manifest (`mock: bool` →
  `source`); `source`-aware validation and provenance everywhere
- ✅ **`local`/adopt source** — `ost runtime pull … --from-usd <path>` (or
  `OST_USD_ROOT`) adopts an existing OpenUSD install in place; `EnvSet` maps
  USD's own layout (`lib/python`, `plugin/usd`); `runtime validate` asserts
  `usdcat` + `pxr`; `plugin doctor` L1 surfaces the source (real but not
  reproducible/certified)
- ✅ `ost plugin run` session launcher (composes the runtime `EnvSet` + bundle
  roots, execs a command, propagates the exit code; no global mutation)
- ✅ Levels 2–5 executed against a real runtime via a `Probe` seam (unit-test
  injectable): L2 discovery (`Sdf.FileFormat.FindByExtension`), L3 `usdcat`
  read, L4 `Usd.Stage.Open`, L5 golden round-trip (`usdcat --flatten` vs
  `<fixture>.golden.usda`); `ost plugin test` orchestrates L0..L5 + report.
  `EnvSet::for_usd_install` probes `lib/python` vs `lib/site-packages`.
  Verified end-to-end against a real OpenUSD 25.05 build.
- ✅ `build` source — `ost runtime pull … --build <usd-src>` builds OpenUSD from
  source into the store (one-time; re-pull is a cache hit), bootstrapping the
  MSVC env on Windows like `ost build`. Two modes:
  - **build_usd.py** (default) — drives the source tree's
    `build_scripts/build_usd.py`, which fetches+builds dependencies itself.
  - **CMake-direct** (`--deps <prefix>…`) — builds OpenUSD directly with CMake
    against pre-provided dependency prefixes (`CMAKE_PREFIX_PATH`), faster and
    aligned with OpenStrata's resolver; sets up deps-as-extensions (Phase 6).

  `--jobs` and `--build-arg` (hyphen-allowed) tune either mode. Both verified by
  building a real OpenUSD 25.05 and running `ost plugin test` against it.
- ✅ Level 6 — `ost plugin view <bundle> <fixture>` opens a fixture in usdview
  inside the runtime session; `ost plugin test-view` (and `test --up-to 6`) runs
  the non-interactive `usdview --quitAfterStartup` launch probe (`usdview.launch`
  diagnostic), SKIPping cleanly when usdview or a display is unavailable.
  Verified against a real usdview-enabled OpenUSD 25.05 build.
- ✅ Multi-plugin sessions (`ost plugin doctor/run/test/view/test-view --with
  <bundle>…`) and bundle-declared `requires.runtime_libs` (extra non-USD runtime
  lib dirs, e.g. a plugin's zlib) — replaces hand-rolled usdview launch batch
  files for the multi-plugin + 3rd-party-dep case. Downstream plugin dogfooding
  (reports #1/#2) surfaced these prerequisites and shapes:
  - **Every bundle path is absolutized at the `ost plugin` boundary** via
    `Bundle::load`, including every `--with <bundle>` arg. Its plugInfo root,
    `lib/`, `python/`, and any `requires.runtime_libs` dir are then composed as
    absolute session env entries, avoiding relative `CMAKE_TOOLCHAIN_FILE` and
    relative `PXR_PLUGINPATH_NAME` failures.
  - **`requires.runtime_libs` prepends to the session's dynamic-loader path**
    (`PATH` / `LD_LIBRARY_PATH` / `DYLD_LIBRARY_PATH`), absolutized and validated
    as bundle-relative. Empty/absent stays the common case: a plugin that
    statically links its 3rd-party deps (e.g. vendoring a parser into one TU,
    exporting no symbols) drags zero extra lib dirs — the opposite of a plugin
    shipping a sibling `zlib.dll`.
  - **`plugInfo.json` `LibraryPath` is generated/validated per target** (suffix +
    lib-dir), since multi-plugin × multi-OS sessions multiply the scaffold's
    cross-platform soft spot. Source bundles may carry templates, but built or
    packaged bundles must carry the concrete `plugInfo.json` for exactly one
    target (`.so` / `.dylib` / `.dll`) and one library layout. See the Phase-4
    fix backlog below.
- ✅ `ost plugin package`: freezes the target-resolved `plugInfo.json`, resolved
  C++/Python ABI, runtime digest/source/validation, static validation report,
  and session environment into a target-specific binary bundle artifact
  (`tar.zst` + `manifest.json` + `SHA256SUMS` under
  `<bundle>/dist/plugins/<name>/<version>/<target>/`)
- ✅ **(v0.6.0)** `ost plugin publish` into the local artifact registry (Phase 6
  MVP; see Phase 6 for the gates). Still ⬜: the runtime×plugin CI matrix and the
  fetched `artifact` runtime source.

### Phase 4 — fix backlog (from downstream plugin dogfooding, reports #1/#2)

A freshly scaffolded `usd-fileformat` bundle did not survive `ost plugin
build`/`test` on Windows out of the box. Ranked, with the implicated code:

Policy from the new cross-platform review: a **source** plugin bundle is not a
universal binary bundle. Source may declare compatibility ranges and generation
templates; `ost plugin build/package` emits a **target-specific** binary bundle
whose `plugInfo.json`, ABI metadata, and provenance are resolved from the CMake
target + runtime variant. `doctor` should validate the resolved files for the
target being tested, not silently accept host-default metadata.

- ✅ **P1 — absolutize `<bundle>` once** in `Bundle::load`
  ([bundle.rs](../crates/ost-plugin/src/bundle.rs)): a single `canonicalize`
  removes *both* the relative-`CMAKE_TOOLCHAIN_FILE` build break and the
  relative-`PXR_PLUGINPATH_NAME` discovery break (single root cause), de-UNCing
  the `\\?\` prefix on Windows (CMake/USD mishandle it). Prerequisite for `--with`
  (above).
- ✅ **P1 — scaffold `plugInfo.json` can't load its own lib.** Was
  `LibraryPath: "lib{{Name}}FileFormat.so"` (wrong suffix off-Windows; beside
  `plugInfo.json` while the lib lands in `lib/`, and USD dlopens the absolutized
  path with no PATH fallback). Now a committed
  [`plugInfo.json.in`](../templates/usd-fileformat-cpp/plugin/resources/{{name}}/plugInfo.json.in)
  (`../../../lib/lib…@CMAKE_SHARED_LIBRARY_SUFFIX@`) that the CMake
  `configure_file` resolves per target; `ost plugin new` also writes a
  host-correct concrete `plugInfo.json` so `doctor`/`test` work before the first
  build (doctor L0 only checks existence + JSON parse, so no collision).
- ✅ **P1 — scaffold `CMakeLists.txt` stages to `${CMAKE_SOURCE_DIR}/lib`**
  ([templates/usd-fileformat-cpp/CMakeLists.txt](../templates/usd-fileformat-cpp/CMakeLists.txt)):
  now uses `CMAKE_CURRENT_SOURCE_DIR` (so an `add_subdirectory()` consumer stages
  the lib in the bundle, not the repo root) and guards `find_package(pxr)` with
  `if(NOT pxr_FOUND)` so a dual-mode project root can resolve it once.
- ✅ **P1 — `ost plugin build` doesn't bootstrap the MSVC env.** `run_step`
  ([commands/plugin.rs](../crates/ost-cli/src/commands/plugin.rs)) now loads the
  MSVC developer environment via `ost_build::msvc::bootstrap()` (Windows, `cl` not
  on PATH), as `ost build`/`runtime pull --build` already do.
- ✅ **P2 — default `CMAKE_BUILD_TYPE=Release` for plugin builds.** `ost plugin
  build`'s configure args now pass `-DCMAKE_BUILD_TYPE=Release`, so Ninja's
  single-config build no longer resolves USD's imported targets to Debug and
  links the missing `tbb12_debug.lib`. The runtimes OpenStrata ships/adopts are
  Release.
- ✅ **P2 — adopted-runtime version was the static placeholder.** `adopt_local`
  ([commands/runtime.rs](../crates/ost-cli/src/commands/runtime.rs)) now reads the
  real `PXR_MINOR/PATCH_VERSION` from the install's `include/pxr/pxr.h` and
  records it (e.g. `26.08`) instead of the catalog's `25.05.01`, so the Level-1
  version gate enforces the actual range. (Python-ABI detection — the `py313` id
  on a py310 install — is still a follow-up; the id parser would need the real
  interpreter version.)
- ✅ **P2 — `runtime show`/`validate` rejected the id `runtime list` prints.**
  Both now accept either `<platform> --profile <profile>` or the full
  `openstrata-cy2026-…-usd` id (the embedded platform/profile win); the variant
  slug is a fixed 3 tokens, so a hyphenated profile stays intact.
- ✅ **P1 — harden target-generated `plugInfo.json` beyond the scaffold.**
  A real downstream bundle with
  `LibraryPath: "../../../lib/lib<Name>FileFormat.dll"` is Windows-only even if
  README/CMake claim cross-platform support. Source commits `plugInfo.json.in`;
  CMake configures the concrete `plugInfo.json` with the target library prefix
  and `CMAKE_SHARED_LIBRARY_SUFFIX`; `ost plugin new` emits a host-concrete
  `plugInfo.json`; `doctor` now has `bundle.plug_info.library_path` to flag
  unresolved templates, non-`lib/` layout, mismatched built lib names, and suffix
  mismatches such as `.dll` on Linux/macOS or `.so` on Windows.
- ✅ **P1 — make source plugin C++ ABI metadata target-aware.**
  The scaffold no longer writes `runtime.cxx_abi: libstdcxx` into a source
  bundle. `ost plugin doctor` derives the runtime ABI from the resolved target
  (`linux → libstdcxx`, `macos → libcxx`, `windows-msvc143 → msvc143`) and still
  compares it when a hand-authored or future packaged manifest records a scalar
  `runtime.cxx_abi`. The binary package step records the one resolved ABI for
  the artifact it emits.
- ✅ **P3 (v0.4.0) — repo-shape scaffold.** `ost init --template plugin-workspace`
  emits a dual-mode root `CMakeLists.txt` + `CMakePresets.json`: it resolves USD
  once (`find_package(pxr)`) and **globs** every immediate subdirectory holding an
  `openstrata.plugin.yaml` + `CMakeLists.txt`, `add_subdirectory()`-ing each — so a
  repo of `ost plugin new` bundles is `cmake -S .`-able by non-`ost` users and new
  bundles are picked up with no edit. Each bundle's `if(NOT pxr_FOUND)` guard lets
  it build standalone (via `ost`) or under this root; the root `CMakePresets.json`
  is the user's own (untouched by `ost configure`, which uses
  `CMakeUserPresets.json`).

### Phase 4 — schema-bundle backlog (from downstream plugin dogfooding, reports #3/#4)

**Targeted for v0.4.0 (scope A).** A second dogfooding pass confirmed the #1/#2 fixes (relative-path
`plugin build|test`, MSVC bootstrap, `CMAKE_BUILD_TYPE=Release`,
`bundle.plug_info.library_path`, full-id `runtime show` all green) and took up the
typed-schema kind (`usd-schema`). `ost plugin new` advertises that kind but ships
no generator, and the harness models only file-format bundles. Ranked:

- ✅ **`usd-schema` (codeless) template + codeless-aware L0 doctor.** The embedded
  `usd-schema-codeless` template (starter `schema.usda` with one single-apply
  `*API` + the `customData` library block and `skipCodeGeneration`, a resource-only
  `plugInfo.json`, a `usdGenSchema` `CMakeLists.txt`, and an apply-the-API
  fixture); `ost plugin new usd-schema` scaffolds it instead of erroring. The
  manifest gained a `schema.codeless` flag (`is_codeless_schema()`), and
  `ost plugin doctor` L0 is now **codeless-aware** — it SKIPs
  `plugin.shared_library` and validates the `Types` block via a new
  `bundle.plug_info.schema_types` check instead of `bundle.plug_info.library_path`,
  so a valid resource-only schema no longer hard-fails. **E2e-hardened against a
  real OpenUSD 26.08:** the scaffold now commits *registerable* resources — a
  correct `Types` entry (`schemaIdentifier`/`schemaKind`/`bases`, no
  self-referential `alias`) plus a flattened `generatedSchema.usda` beside it — so
  a codeless schema registers in `Usd.SchemaRegistry` and applies out of the box
  with no build step.
- ✅ **Schema test contract (L2/L4), verified e2e.** `ost plugin test` runs
  schema-specific execution levels in place of the file-format discovery/read
  levels — **L2 `schema.registration`** (the `provides: usd-schema:<Type>` are
  known to `Usd.SchemaRegistry`) and **L4 `schema.apply_roundtrip`** (the smoke
  fixture applies an `*API` to a prim and its authored attributes survive a flatten
  round-trip), sharing the format-agnostic L5/L6. `ost plugin doctor`'s L2+ SKIP
  placeholders mirror these ids per kind, and the scaffold fixture authors a valid
  USD identifier namespace (`{{ident}}`, e.g. `vrm_schema:`). **Verified end-to-end
  against an adopted OpenUSD 26.08** (both levels PASS); also Probe-unit-tested.
- ✅ **Co-host a schema in an *existing* bundle (the consumable half).** USD lets
  one `plugInfo` provide both an `SdfFileFormat` and schema types; a bundle of any
  kind that declares `usd-schema:<Type>` in `provides` now runs the schema contract
  (L2/L4) *alongside* its primary-kind levels (gated on the explicit `provides`,
  not inferred `Info.Types`, so a file-format's own type is never mistaken for a
  schema). doctor's SKIP placeholders mirror it. Verified e2e: a `usd-fileformat`
  bundle co-hosting a codeless schema passes L2/L4. This is the co-location lean
  realized for the codeless case (commit the `Types` + `generatedSchema.usda` into
  the existing bundle — no second bundle, no `--with`).
- ✅ **`usdGenSchema` build step.** `ost plugin build` on a schema bundle runs the
  template's `usdGenSchema` `CMakeLists.txt` step to regenerate the codeless
  resources (`plugInfo.json` + `generatedSchema.usda`). The fix that made it work:
  the build now composes the runtime **session** env (not just the MSVC delta) so
  usdGenSchema can load `pxr` and resolve the base USD schemas
  (`@usd/schema.usda@`); and ost parses the regenerated `plugInfo.json` as
  JSON-with-comments (usdGenSchema writes a `#` banner). Note `usdGenSchema` itself
  must be present in the runtime and needs `jinja2`/`PyYAML` — OpenUSD skips
  installing it when those are absent at USD build time. **Verified end-to-end
  against an adopted OpenUSD 26.08**: build → regenerate → `ost plugin test`
  L0..L4 PASS.
- ✅ **(v0.5.0) Compiled, co-located schema flow — "add a schema to an existing plugin".**
  `ost plugin build` now recognizes a non-schema bundle that declares
  `usd-schema:<Type>` and ships `schema.usda`, runs `usdGenSchema` in the composed
  runtime/session environment, stages generated typed API sources into the same
  plugin library via a generated CMake fragment, drops Python-module helper files,
  defines the generated `*_EXPORTS` macro, merges the schema `Types` into the
  bundle's existing `plugInfo.json`, copies `generatedSchema.usda`, and also
  merges `Types` into matching `tests/cmake/**/plugInfo.json` registries when a
  bundle's CTest path carries its own plugin registry. If `usdGenSchema` emits no
  C++ files (for example a `skipCodeGeneration` codeless schema), the flow falls
  back to the resource-only merge path.
- ✅ **`usdGenSchema` `Types` merge into a co-hosting bundle.** `ost plugin build`
  on a co-hosting bundle (a non-schema kind shipping a `schema.usda` and declaring
  `usd-schema:<Type>`) runs usdGenSchema to a staging dir and **merges** the
  generated schema `Types` into the bundle's *existing* `plugInfo.json` —
  preserving the `SdfFileFormat` entry usdGenSchema would otherwise clobber — then
  copies `generatedSchema.usda` beside it. Backed by a pure, unit-tested
  `merge_schema_types`. **Verified e2e on OpenUSD 26.08:** the file-format type is
  kept alongside the merged schema, and `ost plugin test` passes L2/L4. A no-op
  (committed resources kept) when there is no `schema.usda` or no usdGenSchema.
- ✅ **Per-variant `cxx_abi` in the source manifest.** `runtime.cxx_abi` now
  accepts a scalar (`msvc143`), a per-OS map
  (`{ windows: msvc143, linux: libstdcxx, macos: libcxx }`), or the `inherit`
  sentinel (defer to the runtime), via a `CxxAbi` enum. The L1 `runtime.cxx_abi`
  check resolves the declared ABI against the target OS before comparing — PASS/FAIL
  on a match/mismatch, SKIP for `inherit` or a target the map doesn't list — so a
  cross-platform source bundle no longer needs hand-editing per target. `ost plugin
  package` freezes the one resolved ABI as a scalar into the artifact. The
  scaffold's file-format template documents the three forms. Unit-tested
  (parse + per-OS/inherit resolution + doctor PASS/FAIL/SKIP).

### Phase 4 — v0.5.0 stabilization backlog (reports #5/#6 + a macOS source-build pass)

A later dogfooding pass on **0.4.0** verified the shipped schema work end-to-end —
`ost plugin new usd-schema` scaffolds a real codeless bundle (asks #1/#3 met),
`ost init --template plugin-workspace` answers the "no root CMake" ask, and a
**macOS arm64 `ost runtime pull --build`** built OpenUSD 25.05.01 from source with
imaging/usdview, then `runtime validate` + `ost plugin test --up-to 6` + CTest all
passed (Phase 4b `build` source confirmed on a second platform). It also took the
Phase 4 schema lean further — building a *compiled, co-located* schema by hand —
and surfaced the v0.5.0 stabilization shape: close correctness/ergonomics gaps
first, keep the compiled schema flow as stretch unless it stays small.

- ✅ **Force UTF-8 for the schema-gen step (locale-encoding bug).** `usdGenSchema`
  writes generated files in the process locale encoding; on a Japanese-locale
  Windows host (cp932) a non-ASCII char (an em-dash) in a `doc=` string aborts with
  `'cp932' codec can't encode`, and the error points at the codec, not the offending
  doc string. The `ost`-owned schema step (the shipped build step and the compiled
  flow above) now sets `PYTHONUTF8=1` / `PYTHONIOENCODING=utf-8` in the composed
  schema build env; the codeless template's own CMake target does the same via
  `cmake -E env` and invokes `python usdGenSchema ...` so direct CMake builds
  are protected on Windows too. The starter `schema.usda` prose is ASCII, while
  edited UTF-8 doc text remains supported.
- ✅ **Schema name-composition guidance (the double-prefix footgun).**
  `usdGenSchema` prepends `libraryPrefix` to the class name for the C++/TfType, so a
  `libraryPrefix` equal to the plugin name plus a class already carrying that name
  doubles it (`Foo` + `FooBarAPI` → `FooFooBarAPI`), while the USD identifier/token
  stays the class name. The codeless scaffold now avoids this by keeping the
  source class unprefixed (`API`) while the generated/public schema type remains
  `<Name>API`; `ost plugin doctor` emits a non-failing `schema.library_prefix`
  hint if edited `schema.usda` reintroduces the repeated leading token shape.
- ✅ **Runtime OpenUSD version truth.** Still reported on 0.4.0: an adopted install
  that is actually 26.x can be recorded as the placeholder `25.05.01`, so the L1
  range check "passes" for the wrong reason. Landed: adopt-time
  `detect_openusd_version` reads `pxr.h`
  ([runtime.rs](../crates/ost-cli/src/commands/runtime.rs)); `ost plugin doctor`
  prefers the install's real `pxr.h` version for L1; `runtime show` flags a
  manifest/install drift in both human output and `--json`; and `runtime validate`
  fails stale manifests with an `openusd-version-drift` check. Repair stays
  explicit: re-pull with `--force --from-usd` so the manifest digest/provenance is
  refreshed deliberately.
- ✅ **`init --template` naming + discoverability.** `plugin-workspace` was hard to
  find: no `ost workspace` command reinforces the term; the `init --template`
  choices mix axes (`cpp-library` = language, `usd-plugin` = domain,
  `plugin-workspace` = repo shape); "plugin" is overloaded (an `init` template *and*
  the `ost plugin` subcommand that populates the repo); and `plugin-workspace` drops
  the `usd-` prefix the other USD templates carry. `usd-plugin-workspace` is now
  the canonical displayed name, `plugin-workspace` remains a compatibility alias,
  `init --help` surfaces the shape, and `ost plugin new` points multi-bundle users
  at `ost init --template usd-plugin-workspace`.
- ✅ **Workspace template: support a nested `plugins/<name>/` layout.** The
  `plugin-workspace` root auto-globs **root-level** bundle dirs; a repo that nests
  bundles under `plugins/` (the "one project → N bundles under `plugins/`"
  convention) isn't found. The workspace root now scans both immediate
  subdirectories and `plugins/*`, so `ost plugin new ... --dir plugins/<name>` is
  picked up by plain CMake without editing the root.
- ✅ **`--build` ergonomics surfaced by the macOS pass (overall a success).** Small
  `ost`-actionable follow-ups: (1) **Apple-Silicon codesign assumes full Xcode** —
  OpenUSD's `apple_utils.py` calls `xcodebuild -version`, which a Command-Line-Tools-
  only host lacks; the build needed a local patch to fall back to ad-hoc
  `codesign -s -`; `ost` now prints a macOS source-build note before `--build`.
  (2) **CMake 4 + bundled oneTBB** needs
  `-DCMAKE_POLICY_VERSION_MINIMUM=3.5`; README/examples/runtime notes document it
  as a known `--build-arg`. (3) **usdview needs Python UI packages** (`PySide6` /
  `PyOpenGL` / `Jinja2`) on `PATH`, and a direct `bin/usdview` without the composed
  env fails (no runtime `lib/python` on `PYTHONPATH`) — already solved by
  `ost plugin view`/`run` / `eval "$(ost env …)"`; the runtime build note now calls
  out the UI package prerequisite.
- ✅ **Doctor nudge: per-target metadata that 0.4.0 already supports but a bundle
  hasn't adopted.** The same pass found a hand-authored bundle still carrying a
  scalar `cxx_abi: msvc143` (fails on macOS `libcxx`) and a Windows `.dll`
  `LibraryPath` (macOS needs `.dylib`) — both already solvable in 0.4.0 (per-OS
  `cxx_abi` map; `plugInfo.json.in` per-target generation). A doctor hint when a
  scalar ABI or fixed-suffix `LibraryPath` mismatches the resolved target, pointing
  at the per-OS forms, now closes the adoption gap.

## Phase 5 — CI / Jenkins 🚧

- ⬜ CI-safe flags (`--ci`, `--no-interactive`, `--report junit|json`, `--jobs auto`)
- 🚧 Runtime×plugin CI matrix, backed by Phase 6 artifact digests:
  - ⬜ explicit support-cell manifest (`runtime digest`, plugin digest, target,
    profile, verification level, required host labels)
  - ⬜ GitHub Actions matrix for the first real-runtime support cells
  - ⬜ JUnit + JSON report upload from `ost plugin test`
  - ⬜ scheduled/release gate for L0..L6; PR gate keeps cheap mock/static jobs
- ⬜ Jenkinsfile template + matrix generation (after the GitHub Actions shape is
  proven)
- ⬜ `ost ci init | generate jenkins`

## Phase 6 — Artifact registry 🚧

- 🚧 **MVP boundary for v0.6.0:** local-first, digest-first artifact registry.
  The registry is a content source for runtimes/plugins/packages, not yet a
  remote service.
- ✅ Artifact identity model (`ost-artifact` crate): `{kind, name, version,
  target, profile, digest, created_unix, producer, source, validation, licenses,
  sbom}` as a fixed-field record with deterministic JSON and a stable schema
  version, always *derived* from a producer `manifest.json` (plugin-bundle,
  project package, or the future `openstrata.runtime` tag) — never hand-authored.
- ✅ Content-addressed artifact store (digest pinning) under `~/.ost/artifacts`:
  `objects/sha256/<hex>/` object dirs staged + renamed atomically, plus a small
  deterministic `index.json` (sorted by digest, rebuildable from the objects)
  before introducing SQLite.
- ✅ `tar.zst` + manifest + checksums as the canonical MVP payload: the store
  keeps the producer manifest byte-for-byte beside the archive and a regenerated
  `SHA256SUMS`; the plugin payload already carries its validation report inside
  the archive (`validation/report.json`).
- ✅ `ost artifact import|export|list|show|verify` for local registry operations
  and CI artifact handoff: import re-hashes the archive and refuses a
  digest/size mismatch (`ARTIFACT_DIGEST_MISMATCH`, exit 5); artifacts resolve
  by full digest or unique hex prefix; `verify` recomputes the archive digest
  *and* re-hashes every tar entry against the manifest `files[]`; `export`
  round-trips to a re-importable directory. Covered by unit + e2e tests
  ([artifact_registry.rs](../crates/ost-cli/tests/artifact_registry.rs)).
- ✅ `RuntimeSource::Artifact` fetch/use path for prebuilt runtimes.
  `ost runtime export` packs a pulled real runtime (effective prefix, minus the
  store's `runtime.json` — the runtime manifest travels in the producer
  manifest's `provenance.runtime_manifest`) and registers it as a `published`
  `openstrata.runtime` artifact, gated on a real source
  (`EXPORT_REAL_RUNTIME_REQUIRED`), no external `runtime_deps`
  (`EXPORT_DEPS_NOT_PORTABLE` — they would not travel), and passed validation
  (`EXPORT_VALIDATION_REQUIRED`). `ost runtime pull --from-artifact <digest>`
  re-verifies the archive bytes, refuses non-runtime kinds
  (`ARTIFACT_KIND_MISMATCH`) and target/profile mismatches
  (`ARTIFACT_RUNTIME_MISMATCH`), extracts into the store prefix, and restores
  the manifest with `source: artifact` + the registry digest
  (`artifact_digest`), surfaced by `runtime show`/`list` and `doctor`
  (kind `downloaded`). Covered by unit + e2e tests
  ([runtime_artifact.rs](../crates/ost-cli/tests/runtime_artifact.rs)),
  including a two-store export → handoff → fetch round-trip.
- ✅ `ost plugin publish`: consumes existing `ost plugin package` output (never
  re-packages) and registers it by digest as a `published` artifact. Entry is
  gated with per-cause stable codes CI can branch on:
  `PUBLISH_VALIDATION_REQUIRED` (validation must have passed),
  `PUBLISH_LICENSE_REQUIRED` (SPDX license), `PUBLISH_PROVENANCE_INCOMPLETE`
  (runtime id + digest), `PUBLISH_ABI_UNRESOLVED` (a concrete frozen `cxx_abi`,
  not `inherit`/per-OS), and `PUBLISH_NOTICES_MISSING` (declared notices must be
  in the archive). Prints the digest reference CI pins.
- ⬜ Runtime/extension content attribution and per-artifact SBOM generation:
  runtime manifests record upstream licenses/notices; published artifacts include
  complete notices and a generated SPDX or CycloneDX SBOM.
- ⬜ Trust policy hooks: distinguish `local`, `verified`, and `trusted`
  artifacts; allow release CI to require a minimum trust level.
- ⬜ OCI layout / registry / oras transport (later)

## Phase 7 — Sessions / sandbox ⬜

- ⬜ Session metadata; `ost session start | fork | diff | discard | promote`
- ⬜ Workspace isolation; optional Linux namespace / overlayfs

## Phase 8 — AI / GPU profiles ⬜

- ⬜ GPU host detection; driver requirement checks (`ost doctor gpu`)
- ⬜ AI runtime profiles (`ai-cuda124`, `ai-rocm`, `ai-mps`, hybrid `cy2026-lookdev-ai`)
- ⬜ Jenkins GPU routing labels; smoke tests

## Phase 9 — Kubernetes execution backend ⬜

Direction: [kubernetes.md](kubernetes.md). OpenStrata owns the runtime contract,
artifacts, and validation; Kubernetes is a pluggable **execution backend** that
runs those contracts on a cluster. `local` stays first-class; Kubernetes is
opt-in. Start narrow — generate → submit → monitor → collect a `batch/v1 Job` via
`kubectl` — not an Operator.

- ⬜ `ost-execution` crate: `ExecutionBackend` trait (`local` + `kubernetes`),
  domain `ResolvedTask` → `KubernetesJobRequest` → Job YAML separation
- ⬜ `ost submit build|validate|plugin-test|ai-validate|matrix --backend
  kubernetes` and `ost jobs list|show|logs|wait|artifacts|cancel`
  (logical `ostj_…` ids; `--output json` contract)
- ⬜ Phased: manifest export (`--dry-run --output yaml`) → kubectl submit/status/
  logs → artifact collection + provenance → matrix (`--max-parallel`,
  `--fail-fast`) → GPU tasks (with Phase 8) → Jenkins bridge (with Phase 5) →
  optional native `kube` client → CRD/Operator only if Jobs prove insufficient
- ⬜ Digest-pinned runtime/extension/source per Job (`latest` rejected);
  safe-by-default manifests; `ost doctor kubernetes`

## Phase 10 — DCC host support ⬜

Direction: [dcc-hosts.md](dcc-hosts.md). Runtime-native apps stay first-class;
existing DCCs (Maya/Houdini/Nuke) are supported as **third-party external hosts**
behind a host adapter boundary — discovered, fingerprinted, driven headlessly,
packaged for, and checked for cross-DCC USD compatibility. No DCC API
abstraction, install, license, or GUI-required path (§2.2).

- ⬜ `ost-host` crate: host record model, selectors, inventory, discovery
  providers (explicit/configured/known/env/PATH/registry/custom rules),
  `HostValidator` / `HostAdapter` traits; reuses the `--json` envelope + exit
  codes and the runtime `EnvSet`
- ⬜ Discovery + validation (candidate→validated→rejected, read-only/bounded/no
  GUI) and standard/deep fingerprints; Maya first, then Houdini + Nuke
- ⬜ `ost host discover|list|inspect|probe|run|test`; headless run with a composed
  env; host-standard packaging (Maya `.mod`, Houdini package JSON)
- ⬜ Matrix cells / support lines / tiers and cross-DCC USD compatibility edges
  (reusing the plugin-harness levels); `ost matrix …` / `ost compat …`
- ⬜ Fleet inventory export/import, `ost compat diff` / `ost reproduce`, optional
  Blender adapter

## Python / uv (§9)

- ✅ `ost uv <args>`: runs `uv` pinned to the project's runtime Python — applies
  the runtime environment and sets `UV_PYTHON` to the runtime interpreter, so uv
  never silently substitutes a different Python (§9.3, §20.3). No-args prints the
  pinning; `uv` is located via `OST_UV` or PATH. `uv.lock` is already hashed into
  `strata.lock`.
- ⬜ Diagnose/refuse app-local `uv` deps that shadow ABI-sensitive runtime
  packages (USD/Qt/OpenEXR bindings), recommending the matching extension.

## Distribution — `ost` binary releases 🚧

The `ost` CLI is a single self-contained binary (no Python/USD dependency), so it
ships independently of the heavy runtime artifacts. Publish tagged builds to
GitHub Releases. Implemented with **cargo-dist** (`dist-workspace.toml`,
`release.yml`); the generated workflow is hand-pinned to commit SHAs (SEC-004),
so a dist version bump needs it regenerated and re-pinned (`allow-dirty = ["ci"]`).

- ✅ **Tag convention.** Releases are cut from a tag `v<semver>` on `main`;
  cargo-dist parses the version from the tag and errors unless it matches the
  workspace `Cargo.toml` `version`. A `-rc.N` / prerelease suffix is marked
  "pre-release" automatically.
- ✅ **Release workflow** (GitHub Actions, triggered on `v*`/semver tags via
  cargo-dist). Builds a binary per target, each packaged with checksums:
  - `x86_64-unknown-linux-musl` (first-class, fully static for old-glibc
    portability), `aarch64-apple-darwin`, `x86_64-apple-darwin`,
    `x86_64-pc-windows-msvc`.
  - Per-archive `SHA256SUMS`, a `dist-manifest.json`, and `NOTICE` +
    `THIRD_PARTY_NOTICES.md` bundled into every archive; attached to the GitHub
    Release with generated notes. Built on the pinned toolchain.
- ✅ **Install ergonomics.** cargo-dist generates `shell` + `powershell`
  installers (fetch the right asset for the host, verify the checksum) hosted on
  the Release; `cargo binstall` works against the `dist-manifest.json`. Document
  `cargo install --path crates/ost-cli` as the from-source fallback.
- 🚧 **Provenance.** GitHub build provenance attestations (SLSA) are attached to
  release artifacts (`github-attestations = true`). Still ⬜: explicit
  signature/Sigstore key material and `ost`-side verification of it (tracks with
  Security baseline SEC-005).

This covers the **`ost` tool** itself; runtime/extension/plugin *content*
artifacts are distributed via the content-addressed store and the artifact
registry (Phase 6).

## Licensing & third-party attribution 🚧

OpenStrata must ship with a clear license of its own and **complete** attribution
for everything it bundles, links, or distributes. The project license, SPDX
headers, Rust-dependency attribution (CI-gated), and the plugin bundle license
field have landed; runtime/extension content attribution and per-artifact SBOM
remain (the latter with the Phase 6 content store).

- ✅ **OpenStrata's own license.** Top-level `LICENSE` (Apache-2.0, matching the
  manifests) and `NOTICE`; SPDX headers
  (`// SPDX-License-Identifier: Apache-2.0`) on all source files; `README` License
  section.
- ✅ **Rust dependency attribution.** `THIRD_PARTY_NOTICES.md` is generated for
  the crate tree with `cargo-about` (`about.toml`/`about.hbs`, host targets only)
  and committed; `licenses.yml` gates every PR with `cargo-deny` (SPDX allowlist
  in `deny.toml`, deny copyleft/unknown) and fails if `THIRD_PARTY_NOTICES.md` is
  stale (CRLF-normalized diff).
- ⬜ **Runtime/extension content attribution.** Anything OpenStrata builds or
  distributes (OpenUSD, MaterialX, TBB, OpenSubdiv, OpenEXR, OCIO, …, and their
  transitive deps) carries its upstream license. Each runtime/extension manifest
  records license metadata; built/adopted runtimes collect the upstream
  `LICENSE`/`NOTICE` files, and a runtime's licenses are inspectable
  (e.g. `ost runtime licenses <cy> --profile <p>`).
- 🚧 **Per-artifact notices + SBOM.** Notices: the `ost` binary archives bundle
  `LICENSE`/`NOTICE`/`THIRD_PARTY_NOTICES` (cargo-dist `include`), and plugin
  packages copy their `notices` files and record the bundle `license`. Still ⬜:
  a generated SBOM (SPDX or CycloneDX) per artifact and a package
  manifest/provenance that lists component licenses by digest (lands with the
  Phase 6 content store). **No artifact ships without complete third-party
  attribution** — this is a release gate.
- ✅ **Plugin bundle license field.** `openstrata.plugin.yaml` carries a `license`
  (SPDX) and optional bundle-relative `notices`. `ost plugin inspect` surfaces the
  license (human + `--json`/report.json), `ost plugin package` records it in the
  artifact `manifest.json` and copies the `notices` files into the package. The
  scaffold seeds `license: Apache-2.0`; `notices` paths are validated as
  bundle-relative (SEC-002).

## Security baseline 🚧

Shrinking the attack surface across build, runtime, plugins, CI, and the
distribution path before OpenStrata is used in production. IDs track the
security baseline document. P0 lands first; P1 next; P2 is continuous.

- ✅ **SEC-001 (P0) — package staging rejects unsafe files.** `ost package`
  classifies each entry by the entry itself (no symlink-following) and errors on
  a symlink, FIFO, socket, or device anywhere in the stage tree (including the
  root), so an artifact cannot absorb a link target's bytes or recurse outside
  the tree.
- ✅ **SEC-002 (P0) — plugin manifest paths stay in the bundle.** `Bundle::load`
  validates `usd.plug_info` and every fixture up front and rejects `..`,
  absolute, drive, and UNC paths (host-independent), so a malicious
  `openstrata.plugin.yaml` cannot steer reads outside the bundle.
- ✅ **SEC-003 (P1) — safe atomic writes.** `write_atomic` creates its temp file
  with `O_EXCL` and an unpredictable name, refuses to write over a symlinked
  destination, and fsyncs the parent directory (mode follows the umask, as a
  plain write would, since the current outputs are shared project config).
- ✅ **SEC-004 (P1) — CI supply-chain pinning.** Every third-party GitHub Action
  is pinned to a full commit SHA (with a `# vN` comment), and Dependabot manages
  SHA/dependency bumps as reviewable PRs. Release retains workflow-level
  `contents: read` with job-scoped grants and build provenance attestation.
- ⬜ **SEC-002 follow-up — symlink escape inside a bundle.** Reject a *real*
  symlink within a bundle that resolves outside the root at read time
  (canonicalize-and-contain), complementing the lexical manifest check.
- ⬜ **SEC-005 (P1) — installer & release-asset verification.** Publish per-release
  SHA-256 checksums, signature/Sigstore material, SBOM, and build provenance; the
  installer pins a version, verifies the checksum, and aborts on mismatch. Tracks
  with **Distribution → Install ergonomics / Provenance**.
- ⬜ **SEC-006 (P2) — runtime trust policy.** Introduce runtime trust levels
  (`local` / `verified` / `trusted`), record runtime source / version / platform
  / binary & plugin hashes / trust level in the manifest and lock, warn on
  world-writable runtime roots, and let `ost build` / `ost plugin test` require a
  minimum trust level (release/production CI refuses `local`).
- ✅ **CI test gate.** `.github/workflows/ci.yml` runs `fmt`
  (`cargo fmt --all -- --check`), `clippy`
  (`cargo clippy --workspace --all-targets --locked -- -D warnings`), and `test`
  (`cargo test --workspace --locked`) on every push to `main` and every PR, so the
  security regression tests above now run in CI. Linux-only / mock-runtime only
  (no real DCC, no OS matrix). Marked as required status checks (with `licenses`)
  on protected `main`. Actions are SHA-pinned (SEC-004).

## Quality bar (applies to every phase)

- CLI errors must be actionable.
- All generated manifests must be deterministic.
- Runtime and extension identities always include version + target + digest.
- No hidden environment mutation outside `ost devshell` / `ost env`.
- Every published artifact includes provenance and validation result.
- Every published artifact carries complete third-party attribution (no missing
  upstream licenses/notices).
- OpenStrata must work without a preinstalled Python environment.
