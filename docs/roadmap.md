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
- ✅ **v0.6.0 — artifact registry + publishable plugin CI.** The first practical
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
  - **CI matrix MVP:** ✅ GitHub Actions first (Jenkins generator later). Matrix
    cells are explicit support lines (`runtime artifact digest × plugin artifact
    digest × target/profile`) in `openstrata.ci.yaml`, never a naive Cartesian
    product; `ost ci init | validate [--resolve] | generate github` scaffolds,
    gates, and renders them into a scheduled/dispatch workflow. PR CI keeps
    cheap mock/static checks; the generated matrix runs real runtime/plugin
    cells from the registry.
  - **Dogfooding #7 follow-ups:** ✅ the compiled co-located schema path is
    product-shaped — `ost plugin schema add` scaffolds a starter
    `schema/schema.usda` (compiled by default, `--codeless` opt-out) and wires
    the manifest (`provides: usd-schema:<Type>` + the new bundle-relative
    `schema.source`), feeding the existing build flow (usdGenSchema, generated
    C++ linked into the plugin library, `Types` merge, `generatedSchema.usda`
    staging, export define); ✅ adopted-runtime drift repair UX —
    `ost runtime repair` re-adopts a `local` runtime from its recorded USD root
    in one step, and every drift report (`show` human/JSON, `validate`) now
    prints the exact copy-paste fix per source.
  - **Deferred:** macOS source-build ergonomics re-check (needs a Mac), OCI
    layout / ORAS transport, remote hosted registry, Kubernetes execution, full
    Jenkins command surface, and DCC host matrices.
- ✅ **v0.7.0 — CI contract v2 + 0.6.0 CI/lock/package fixes.** Released from
  dogfooding report #8 (2026-07-04) and the CI build-matrix policy notes
  (2026-07-05): first make the 0.6.0 CI surface trustworthy, then promote
  `openstrata.ci.yaml` from an artifact-seeded support matrix into a portable
  CI contract that GitHub Actions merely renders. Details in the
  [Phase 5 — v0.7.0 backlog](#phase-5--v070-backlog-from-dogfooding-report-8--the-ci-build-matrix-policy-notes).
  Delivered:
  - **Correctness first (report #8):** valid + deterministic
    `ost ci generate github` output, `strata.lock` extension versions that
    match `runtime show` (the lockfile must be safe as a CI gate), idempotent
    `ost plugin package` reruns, and placeholder digests that cannot be
    mistaken for a usable matrix.
  - **CI contract v2 (policy notes):** named runner profiles (`github-hosted`
    image vs `self-hosted` labels — repo workflows stop hard-coding raw
    `runs-on`) and lanes (`pull_request` / `main` / `scheduled` /
    `workflow_dispatch`); a generated **source-CI** PR workflow that
    builds+tests a plugin on a GitHub-hosted runner from a digest-pinned
    runtime SDK artifact, keeping the 0.6.0 artifact-seeded workflow as the
    scheduled **support** lane; hosted-runner billing acknowledgement +
    `ost ci plan`; fork-PR safety; CI evidence (profile/lane/digests/outcome)
    in reports.
  - **Workspace + docs:** workspace-level plugin test orchestration
    (`ost plugin test --workspace`) and a documented co-located schema
    migration path for existing (non-scaffold) bundles.
  - **Acceptance shape (from the policy notes):** generated source-CI jobs are
    modeled to build plugin PRs on GitHub-hosted runners from digest-pinned
    runtime artifacts once `ost` is available on the runner; cells reference
    runner profiles, never raw labels; fork PRs cannot publish or reach
    privileged self-hosted runners; a scheduled self-hosted cell revalidates a
    pinned runtime/plugin pair; generated workflows are valid, deterministic
    YAML.
  - **Deferred:** Jenkins renderer, remote/OCI registry transport, macOS
    ergonomics re-check, source-CI bootstrapping beyond the existing
    `ost --version` assertion, dispatch inputs, privileged-runner trust policy,
    and DCC host matrices.
- ✅ **v0.8.0 — packaging reruns survive transient locked stage trees.** A
  narrow follow-up from dogfooding report #9: `ost plugin package` and
  `ost package` now reset staging directories with bounded retry, clear
  read-only entries during cleanup, and fall back to a fresh sibling stage when
  scanners or indexers still hold old files open. The fallback is visible as a
  structured `STAGE_FALLBACK` warning, so CI can keep moving without hiding the
  host condition.
- 🚧 **v0.9.0 — remote artifact transport + hosted source-CI closure + macOS
  plugin-build robustness.** Planned from dogfooding report #10 (2026-07-05),
  the remote-artifact-transport plan
  ([remote-artifact-transport.md](remote-artifact-transport.md)), and the
  macOS dogfooding report (2026-07-05, `ost 0.8.0` on macOS arm64). Report
  #10 produced real digests for both lanes on 0.8.0, but the rendered PR lane
  still cannot run on a GitHub-hosted runner: nothing installs `ost` there,
  and nothing can seed the ~1.93 GiB runtime SDK artifact into the runner's
  local-only registry. v0.9.0 closes that bootstrap gap end to end — the
  plan's P0 slice. The macOS report showed `ost plugin build` reproducibly
  failing on macOS for the co-hosted schema workspace; v0.9.0 carries those
  fixes as its P1 host-robustness slice. Scope (details in the
  [Phase 6 — v0.9.0 backlog](#phase-6--v090-backlog-from-dogfooding-report-10--the-remote-artifact-transport-plan)
  and the
  [Phase 4 — v0.9.0 macOS backlog](#phase-4--v090-macos-backlog-from-the-macos-dogfooding-report-2026-07-05)):
  - ✅ **Transport abstraction + read-only OCI pull (plan Phase 1):** an
    `ArtifactTransport` contract with the existing filesystem store as one
    adapter and a read-only OCI backend (GHCR-class registries) as the second;
    `ost artifact resolve | pull` with digest-pin enforcement, the full
    verification chain before an atomic local import, JSON pull evidence, and
    stable `ARTIFACT_*` error codes. Landed (#86).
  - ✅ **CI contract + generated hosted bootstrap (plan Phase 2):**
    `openstrata.ci.yaml` support lines gain a `runtime_remote` reference
    (OCI uri + expected digest) beside the artifact digest; `ost ci generate
    github` renders a pinned, checksum-verified `ost` install step and the
    digest-pinned `ost artifact pull`, with `actions/cache` keyed by digest as
    an optional optimization (cache is speed, never correctness). The public
    E2E fixture repository (`snkmcb/_ost_runner_test`) proves fork-PR / push /
    cache-miss runs green end to end — see the Phase 6 P0 item below (#89/#90).
  - ✅ **Runtime export ergonomics (report #10):** a slim/SDK-profile export
    (`include/lib/bin/plugin`-only) cuts the 14.4 GB adopted-tree payload, and
    `export` now packs with multithreaded zstd by default (`--jobs`), takes a
    `--level` knob, and prints throttled progress — replacing the previously
    ~52-minute silent `ost runtime export`.
  - ✅ **macOS plugin-build robustness (macOS report):** the co-hosted schema
    build now resolves Python from the runtime interpreter (never a bare
    `python` on PATH — a precondition error names what was searched);
    scopes the schema-regeneration env to the runtime's plugin registry so
    `usdGenSchema` no longer discovers (and tries to dlopen) the bundle's
    not-yet-built plugin library; fails early with the doctor fix when a
    committed `plugInfo.json` carries another platform's `LibraryPath` suffix;
    and attributes every build failure to a phase (`schema-generate` /
    `configure` / `compile-link` / `schema-merge` / `plugin-discovery`) in
    both human and `--json` output.
  - **Deferred (v0.10.0+):** OCI push + protected publish policy + OIDC
    (plan Phase 3), and trust levels / provenance / SBOM / allowlist
    hardening (plan Phase 4, tracks SEC-006 and the Phase 6 trust-policy
    hooks).

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
- ✅ **(v0.6.0) First-class co-located schema UX: `ost plugin schema add`.**
  One command turns an existing non-schema bundle into a schema co-host: it
  scaffolds a starter schema source (default `schema/schema.usda`; compiled by
  default, `--codeless` for resource-only; `--class` picks the source class,
  composed as `<PascalBundleName><Class>` to stay clear of the
  `schema.library_prefix` double-prefix footgun) and wires the manifest
  *textually* — `provides: usd-schema:<Type>` plus the new bundle-relative
  `schema.source` field (validated in-bundle, SEC-002) — preserving the user's
  comments and re-parsing before writing back. The build flow and the
  `schema.library_prefix` doctor hint honor `schema.source`; a
  declared-but-missing source is a configuration error rather than a silent
  no-op.
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
  refreshed deliberately. **(v0.6.0)** one-step repair landed: `ost runtime
  repair` re-adopts a `local` runtime from its recorded USD root (re-reads
  `pxr.h`, re-probes the layout, resets validation to pending), and every drift
  report prints the exact per-source fix command — `repair` for adopted
  runtimes, the pinned `--from-artifact <digest>` re-pull for artifact
  runtimes, a `--build` re-pull for built ones.
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

### Phase 4 — v0.9.0 macOS backlog (from the macOS dogfooding report, 2026-07-05)

**Released in v0.9.0.** The macOS dogfooding report (2026-07-05, `ost 0.8.0`
on macOS arm64, `plugins/usdVrm`, cy2026/usd) found `ost plugin build`
reproducibly failing on a co-hosted schema workspace, with three stacked
blockers — none of them C++ compilation itself: compile/link completes once
the early failures are forced past. The reliability gap is in
toolchain/loader assumptions during schema regeneration and plugin
discovery. Ranked:

- ✅ **P1 — resolve Python from the runtime, never a bare `python` on PATH.**
  The co-hosted schema regeneration step used to die with
  `error[IO_ERROR]: i/o error at run python: No such file or directory` on
  any host without a bare `python` executable (macOS ships `python3` only).
  `ost_build::resolve_run_python` now resolves the interpreter argv from the
  runtime — its bundled `bin/python3` first, then a version-matched host
  `python{ver}` / Windows `py -<ver>` / tool-cache interpreter, then
  `python3`, and only last a bare `python` — probed for runnability
  (`--version`), and `prepare_cohosted_schema`
  ([plugin.rs](../crates/ost-cli/src/commands/plugin.rs)) runs `usdGenSchema`
  through it. When nothing runs, a `PRECONDITION_FAILED` error names every
  candidate searched and the fix (unit-tested ordering; no more `IO_ERROR`).
- ✅ **P1 — schema regeneration must not require a pre-existing plugin
  binary.** `usdGenSchema` previously ran with the bundle's own
  `plugin/resources/…/plugInfo.json` discoverable, so USD tried to load the
  plugin library the build had not produced yet (or an old one with the
  wrong platform suffix) and failed. The schema-generation env is now scoped
  to the **runtime session alone** (`compose_build_env(&msvc_env, &r.env)`)
  — the bundle's `PXR_PLUGINPATH_NAME`/lib entries are left out, so
  `usdGenSchema` resolves the base USD schemas through the runtime registry
  but never discovers the bundle's own not-yet-built plugin. The report's
  temporary `plugInfo.json` move is no longer needed.
- ✅ **P1 — platform-aware `LibraryPath` in the co-hosted build flow.** A
  committed `plugInfo.json` carrying another platform's library suffix
  (`.dll` on a macOS host) used to fail plugin load mid-build. The
  regeneration phase no longer consumes that value (per the isolation item
  above), and where the file *is* consumed `plugin build` now runs
  `verify_target_library_suffix` after the build: a `plugInfo.json.in` source
  bundle has already had the per-target suffix configured, and a committed
  concrete path with the wrong suffix fails early as a `PRECONDITION_FAILED`
  carrying the doctor hint's exact fix — never USD's opaque loader error.
- ✅ **P2 — phase-attributed build diagnostics.** Every `ost plugin build`
  subprocess step now carries a phase — `schema-generate`, `configure`,
  `compile-link`, `schema-merge`, `plugin-discovery` — threaded onto the
  failure via a new `phase` slot on `Error::Coded` (surfaced as `error[CODE]
  (phase: …)` in human output and an `error.phase` field in the `--json`
  envelope). Verified e2e: a failing configure/compile-link build reports its
  phase in both modes; a wrong-suffix `plugInfo.json` reports
  `plugin-discovery`.

## Phase 5 — CI / Jenkins 🚧

- ⬜ CI-safe flags (`--ci`, `--no-interactive`, `--report junit|json`, `--jobs auto`)
- 🚧 Runtime×plugin CI matrix, backed by Phase 6 artifact digests:
  - ✅ **(v0.6.0)** explicit support-cell manifest (`openstrata.ci.yaml`, new
    `ost-ci` crate): each cell pins `runtime_artifact` × `plugin_artifact` by
    **full** registry digest (prefixes rejected — a prefix can silently start
    matching a different artifact) plus platform/profile, verification level
    (`up_to`), and host os/labels. `ost ci init` scaffolds it, `ost ci
    validate` checks structure, `--resolve` additionally requires every pinned
    digest to exist in the local registry.
  - ✅ **(v0.6.0)** GitHub Actions generation: `ost ci generate github` renders
    the matrix into a scheduled/dispatch workflow (`--stdout`/`--out`/
    `--force`), one job per cell via explicit `matrix.include` (never a
    Cartesian product, `fail-fast: false`), SHA-pinned actions (SEC-004). Each
    job re-verifies both artifacts, materializes the runtime
    (`pull --from-artifact`), extracts the plugin (`artifact extract`), runs
    `ost plugin test --up-to <level>`, and uploads the report. Runners need
    `ost` on PATH and the pinned artifacts in their `OST_HOME` registry
    (self-hosted labels are the expected case). e2e:
    [ci_matrix.rs](../crates/ost-cli/tests/ci_matrix.rs).
  - ⬜ JUnit + JSON report upload from `ost plugin test` (the generated
    workflow uploads the existing report dir; a JUnit format is still ahead)
  - ✅ scheduled/release gate for L0..L6 (the generated workflow is
    schedule + dispatch only); PR gate keeps cheap mock/static jobs
- ⬜ Jenkinsfile template + matrix generation (after the GitHub Actions shape is
  proven) — `ost ci generate jenkins` on the same `openstrata.ci.yaml` model

### Phase 5 — v0.7.0 backlog (from dogfooding report #8 + the CI build-matrix policy notes)

**Released in v0.7.0.** A downstream `usd-plugin-workspace` pass on 0.6.0
(report #8, 2026-07-04) exercised `ost ci init|validate|generate`, `ost lock
--check`, and `ost plugin package`, and a follow-up policy read (2026-07-05,
after a self-hosted-labeled PR workflow queued forever on a repo with no
registered runner) settled the CI model: `openstrata.ci.yaml` + named runner
profiles + digest-pinned artifacts is the portable contract; GitHub Actions is
its first renderer, not the source of CI semantics. Ranked:

- ✅ **P0 — `ost ci generate github` emits invalid YAML.** The workflow
  template joined the rendered `matrix.include` block with a `\` string-literal
  continuation ([github.rs](../crates/ost-ci/src/github.rs)); Rust's
  continuation also strips the next line's leading whitespace, so `steps:`
  landed at column 0 instead of under `jobs.cell`. Fixed with an
  `\x20`-protected indent; the unit test and the e2e now assert *placement*
  (`jobs.cell.steps` non-empty, no stray top-level key) — a column-0 `steps:`
  still parses as YAML, so a parse-only assertion misses the regression.
- ✅ **P0 — `strata.lock` extension versions don't match `runtime show`.**
  `build_lock` ([lock.rs](../crates/ost-cli/src/commands/lock.rs)) resolved
  extensions from the static catalog (`ost_extension::resolve`), not from the
  pulled runtime's manifest — an adopted OpenUSD 26.08 runtime locked as the
  catalog's certified `25.05.01`, and `ost lock --check` still reported
  `up_to_date: true` because `--check` re-derived from the same source. The
  lock now pins the pulled runtime manifest's extension records (the same
  source of truth `runtime show` reports); catalog resolution remains only as
  the pre-pull fallback. A lifecycle e2e reproduces the drift and asserts
  `--check` fails until a re-lock records the real version.
- ✅ **P1 — `ost plugin package` reruns are not idempotent.** A second package
  on Windows failed with access-denied (os error 5) at the reused
  `.strata/targets/<id>/package-stage`
  ([plugin.rs](../crates/ost-cli/src/commands/plugin.rs)): staging copies with
  `fs::copy`, which preserves the source's read-only attribute, and Windows
  refuses to delete read-only files. The stage reset now clears the attribute
  recursively and retries once (Windows-only; other platforms delete by
  parent-dir permission), unit-tested with a read-only staged file.
  *Incomplete — report #9 hit the same error on a stage with no read-only
  entries at all; the real second cause was transient scanner-held file
  locks. Superseded by the v0.8.0 staging-fallback fix below.*
- ✅ **P1 — placeholder digests pass validation too quietly.** `ost ci init`
  writes all-zero example digests and `ost ci validate` (without `--resolve`)
  accepted them silently. `validate` now warns per hit (human `WARNING:` lines
  + structured `CI_PLACEHOLDER_DIGEST` warnings in the `--json` envelope's
  `warnings` array — its first real use), and `ci generate github` refuses a
  placeholder matrix with the stable code `CI_PLACEHOLDER_DIGESTS` (exit 5)
  unless `--allow-placeholders` is passed.
- ✅ **P1 — runner profiles + lanes in `openstrata.ci.yaml`.** Cells reference
  named `runners:` profiles — `kind: github-hosted` (fixed image, e.g.
  `windows-2022`, optional `billing.acknowledgement`) or `kind: self-hosted`
  (labels + capability tags) — instead of raw host labels (`host:` stays as
  the legacy fallback; declaring both is a structural error), and declare a
  `lane` (`pull_request` / `main` / `scheduled` / `workflow_dispatch`, default
  `scheduled`) plus a `publish` policy (default `never`; `pull_request` +
  `publish` is rejected outright). The GitHub renderer maps profiles to
  `runs-on` (`image` → the image, `labels` → the list); support cells stay
  explicit support claims, never an inferred Cartesian product. Still ⬜:
  dispatch-input restrictions are moot for now — the generated workflows
  accept no `workflow_dispatch` inputs at all.
- ✅ **P1 — source-CI lane: GitHub-hosted SDK build jobs.**
  `ost ci generate github` renders `pull_request`/`main` cells into a
  second workflow (`ost-source-ci.yml`): checkout (SHA-pinned) →
  `ost ci validate` → `ost artifact verify` + `ost runtime pull
  --from-artifact <digest>` → `ost plugin build <bundle>` → `ost plugin test
  --up-to <level>` → `ost plugin package` (never publish, `contents: read`
  token, no secrets) → upload reports; per-cell `bundle:` selects the bundle
  in a workspace repo. The 0.6.0 artifact-seeded workflow remains the
  scheduled **support** lane. The two gaps report #10 confirmed ("Check ost
  is available" fails on a hosted runner) closed in v0.9.0: hosted cells now
  get a pinned, checksum-verified `ost` bootstrap step and a digest-pinned
  `ost artifact pull` from the cell's `runtime_remote` reference (plus an
  optional digest-keyed registry cache) — see the
  [Phase 6 — v0.9.0 backlog](#phase-6--v090-backlog-from-dogfooding-report-10--the-remote-artifact-transport-plan).
- 🚧 **P2 — hosted-runner cost visibility + fork-PR safety.**
  ✅ `ost ci validate` warns (`CI_HOSTED_BILLING_UNACKNOWLEDGED`) when a
  referenced `github-hosted` profile lacks `billing.acknowledgement:
  required`, and fails (exit 5) when a publish-capable cell sits on such a
  profile; generated hosted jobs print a `::notice` billing annotation before
  work starts; generated PR workflows cannot publish (structural gate +
  no publish step, no secrets, read-only token). Still ⬜: dispatch
  approved-choice inputs (none generated yet) and trust levels for privileged
  self-hosted runners (tracks SEC-006 / Phase 6 trust policy).
- ✅ **P2 — `ost ci plan --json`.** Preflight execution facts without money
  estimates: cells per lane, the workflows `generate` would write, hosted job
  count, metered vs operator-managed runner classes, the hosted profiles still
  missing billing acknowledgement (`requires_billing_acknowledgement`), and
  the publish-capable job count. Facts only — never a currency estimate.
- ✅ **P2 — CI evidence in reports.** Generated workflows export a job-level
  `OST_CI_*` contract (cell, lane, runner profile, `join()`-resolved
  `runs-on`, pinned runtime/plugin digests) from the include entry, and every
  report written inside the job — `report.json` and the `--json` envelope, via
  `ost_plugin::ci_evidence_from_env` — records it as an additive `ci` block,
  so a support claim is reconstructible from its report. Absent outside CI
  (no `OST_CI_CELL`), so local reports are unchanged; the published
  [plugin-report schema](../schemas/plugin-report.schema.json) documents the
  block. Target/profile, verification level, and validation outcome were
  already in the report body; package provenance stays in the package
  `manifest.json`.
- ✅ **P2 — workspace-level plugin testing.** `ost plugin test --workspace`
  discovers the workspace's plugin bundles (immediate subdirectories +
  `plugins/*`, matching the v0.5.0 CMake discovery), runs the verification
  pyramid on each against one resolved runtime session, prints per-bundle
  reports plus an aggregate summary (`--json`: one envelope with every
  bundle's report + `report_dir`), and fails if any bundle fails. `--with`
  bundles compose into every session; a bundle path together with
  `--workspace` is a usage error.
- ✅ **P2 — document the co-located schema migration path for existing
  bundles.** [co-located-schema-migration.md](co-located-schema-migration.md):
  when to co-host vs split a schema bundle, the `ost plugin schema add` fast
  path and the hand-wiring equivalent (`schema.source` + `provides:
  usd-schema:<Type>`), what the next build automates (usdGenSchema in the
  session env, the `OPENSTRATA_SCHEMA_SOURCES_FILE` hook, the `Types` merge
  that preserves the `SdfFileFormat` entry, `generatedSchema.usda` staging),
  the committed-vs-build-tree decision, the `library_prefix` footgun, L2/L4
  verification, and the per-target ABI/`LibraryPath` notes.

### Phase 5 — v0.8.0 backlog (from dogfooding report #9, the v0.7.0 CI policy decision)

Released in v0.8.0. Report #9 (2026-07-05) adopted `openstrata.ci.yaml` as the
downstream repo's CI policy surface and verified the v0.7.0 CI/lock fixes; what
it carried back is the one v0.7.0 fix that didn't hold plus consumer-side
blockers (real artifact digests, a golden L5 fixture) that are theirs, not ours.
Ranked:

- ✅ **P1 — `ost plugin package` rerun still hits access-denied (os error 5).**
  The v0.7.0 read-only fix addressed the wrong (or only half the) cause: the
  failing host's `package-stage` had *no* read-only entries — the reset dies
  when a scanner (Defender, indexer) still holds the previous run's fresh
  files open without `FILE_SHARE_DELETE`, an inherently transient lock the
  old clear-attribute-and-retry-once path never waited out. Staging now goes
  through `ost_core::fs::prepare_staging_dir`
  ([fs.rs](../crates/ost-core/src/fs.rs)): bounded remove retries (~0.4s,
  clearing read-only between attempts), then **fall back to a fresh sibling
  stage** (`package-stage-<16 hex>`) instead of failing — the rerun always
  proceeds; the stuck tree is swept best-effort by every later run once the
  handles close. A fallback surfaces as a `STAGE_FALLBACK` warning (the
  `--json` envelope's `warnings` array / a stderr `warning:` line). Applied to
  both `ost plugin package` and `ost package` (which still had the naked
  `remove_dir_all`); unit-tested on Windows with a genuinely locked file
  (opened without `FILE_SHARE_DELETE`) plus sweep/reset/readonly cases.

## Phase 6 — Artifact registry 🚧

- ✅ **MVP boundary for v0.6.0:** local-first, digest-first artifact registry.
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
- ✅ `ost artifact import|export|list|show|verify|extract` for local registry
  operations and CI artifact handoff: import re-hashes the archive and refuses a
  digest/size mismatch (`ARTIFACT_DIGEST_MISMATCH`, exit 5); artifacts resolve
  by full digest or unique hex prefix; `verify` recomputes the archive digest
  *and* re-hashes every tar entry against the manifest `files[]`; `export`
  round-trips to a re-importable directory; `extract` unpacks an artifact's
  archive after re-verifying its digest (the runtime fetch and the CI matrix's
  plugin-under-test step share it). Covered by unit + e2e tests
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
  artifacts; allow release CI to require a minimum trust level. Direction now
  settled in [remote-artifact-transport.md](remote-artifact-transport.md)
  (integrity vs trust split, initial `local`/`verified`/`trusted` levels);
  implementation lands with the plan's Phase 4 (post-v0.9.0).
- 🚧 OCI layout / registry / oras transport — **targeted for v0.9.0** (read-only
  pull first). Direction:
  [remote-artifact-transport.md](remote-artifact-transport.md); ranked backlog
  below. The read-only pull slice (transport contract + OCI backend +
  `ost artifact resolve|pull`) has landed; push and the publish policy stay
  deferred to v0.10.0+.

### Phase 6 — v0.9.0 backlog (from dogfooding report #10 + the remote-artifact-transport plan)

**Targeted for v0.9.0.** Report #10 (2026-07-05) ran the v0.7.0 decision's
next steps to completion on 0.8.0 — real runtime + plugin digests, a
placeholder-free `openstrata.ci.yaml`, both workflows rendered, L5 golden
gate green — and isolated the one remaining blocker: the generated
`ost-source-ci.yml` fails at "Check ost is available" on any GitHub-hosted
runner, because `ost` install and runtime-artifact transport are both left to
the operator. The remote-artifact-transport plan
([remote-artifact-transport.md](remote-artifact-transport.md)) is the design
contract; this backlog is its P0 slice plus the report's export-ergonomics
asks. Ranked:

- ✅ **P0 — `ArtifactTransport` contract + read-only OCI pull (plan Phase 1).**
  A `resolve / pull` transport trait (`push` declared, refused until the
  publish phase) in front of the registry: the existing filesystem flow is
  one adapter (`file://<dist-dir>`, behavior unchanged — same chain, same
  evidence), and a read-only OCI backend (GHCR-class, ORAS artifact model,
  bearer token exchange, manual cross-host redirects that never replay
  `Authorization`) is the second. `ost artifact resolve <ref>` (tag →
  immutable digest) and `ost artifact pull oci://…@sha256:<digest>` landed —
  pull downloads, runs the full verification chain (OCI blob digests →
  archive digest → manifest schema → pre-extraction safety → per-file
  digests → kind / target / pinned-digest match → trust policy), then
  imports atomically into the local registry; transport success alone is
  never success, and a failed step never leaves a usable artifact. `--json`
  evidence records the remote locator, resolved OCI digest, registry
  identity, auth mode, per-step verification status, and local import path.
  Stable error codes (`ARTIFACT_REFERENCE_MUTABLE`,
  `ARTIFACT_OCI_DIGEST_MISMATCH`, `ARTIFACT_ARCHIVE_DIGEST_MISMATCH`,
  `ARTIFACT_ARCHIVE_UNSAFE`, `ARTIFACT_TRANSPORT_FAILED`, …) so CI can
  branch on cause. Integration-tested against a mock OCI registry
  ([transport_pull.rs](../crates/ost-artifact/tests/transport_pull.rs)):
  corrupt archive, manifest substitution, wrong platform / kind, unsafe
  archive entries, and mutable-only refs all fail.
- ✅ **P0 — digest-pin policy.** Tags are convenience, digests are the
  contract: `ost artifact pull` refuses mutable-only refs
  (`ARTIFACT_REFERENCE_MUTABLE`) and every digest-verification failure is an
  error, never a warning (landed with the transport); `openstrata.ci.yaml`
  `runtime_remote` references must themselves be digest-pinned and matching
  their `expected_oci_digest` (landed with the CI contract below).
- ✅ **P0 — CI contract: remote runtime reference per support line.**
  `openstrata.ci.yaml` runtime pins gain a `runtime_remote` block (`uri:
  oci://…@sha256:<digest>` + `expected_oci_digest`) beside the existing
  OpenStrata `runtime_artifact` digest, and a matrix-level `bootstrap.ost`
  pin (`version`, release `repository`, optional per-triple exact-byte
  `sha256`). Source cells (`pull_request`/`main`) resolving to GitHub-hosted
  runners require both; self-hosted lanes may keep air-gapped local import,
  and CI evidence records the runtime's source either way
  (`.ost-ci/runtime-source.json`). `ost ci validate` enforces the policy,
  `ost ci plan` reports the bootstrap pin, remote-pulling cells, and
  air-gapped source cells.
- ✅ **P0 — generated hosted bootstrap (plan Phase 2).** `ost ci generate
  github` renders, for hosted cells: a bootstrap step that installs the
  version-pinned `ost` release asset with checksum verification (the
  release's published `.sha256` plus the matrix's exact-byte pin when
  declared; bootstrap failure is its own step, never conflated with an
  artifact failure; the observed version is asserted against the pin and
  saved into `.ost-ci/bootstrap.json` / `ost-version.json`); an optional
  `actions/cache` restore of the registry keyed by `{ost-version, os, arch,
  support-line, runtime-digest}` (never branch names or run ids, disableable
  via the `OST_CI_DISABLE_CACHE` repository variable, a poisoned hosted
  cache is wiped and re-pulled); then a digest-pinned `ost artifact pull
  --expect-artifact --require-kind runtime` with `--json` evidence teed to
  `.ost-ci/runtime-pull.json`, falling back cleanly on a cache hit; then the
  existing build → test → package → report-upload chain, with `.ost-ci/`
  evidence uploaded beside the reports. Replaces the "assert `ost
  --version`" placeholder; the generated bootstrap was executed end to end
  against the real v0.8.0 release assets (download → checksum → extract →
  PATH → version assert) as part of verification.
- ✅ **P0 — public E2E fixture repository.**
  [`snkmcb/_ost_runner_test`](https://github.com/snkmcb/_ost_runner_test): a
  tiny `usd-fileformat` plugin (`plugins/toy`) built from source on
  GitHub-hosted `windows-2022`, with the runtime SDK pulled from a public
  GHCR reference
  (`oci://ghcr.io/snkmcb/openstrata-cy2026-windows-x86_64-py313-usd@sha256:39a588fde380…`,
  archive digest `sha256:7b410d92…`) and `ost` bootstrapped from a pinned
  release. **PR source CI, push (main) source CI, and an explicit
  cache-disabled run (`OST_CI_DISABLE_CACHE=true`) all green**, full pyramid
  L0–L5 passing on the runner; workflow verified read-only (no `secrets.`,
  no publish command, no self-hosted labels, `permissions: contents: read`).
  Standing this up end to end surfaced — and drove fixes for — six ways an
  **adopted USD build-tree runtime is not relocatable** to a clean host
  (landed v0.9.0, all with a stale-only guard so a developer's in-place tree
  is never mutated):
  1. `pxrConfig.cmake` bakes the export machine's Python behind
     `if(NOT DEFINED)` guards → `ost` resolves a matching host interpreter
     and pins `Python3_*` in the generated toolchain (required version read
     from pxrConfig, not `runtime.json` — the runtime was labeled `py313`
     but its USD linked Python 3.10).
  2. `pxrTargets.cmake` bakes the Python include into imported targets'
     `INTERFACE_INCLUDE_DIRECTORIES` → relocated to the host include.
  3. `pxrConfig.cmake` bakes the build-tree's own absolute prefix into the
     external-dependency imported targets (TBB/MaterialX) → relocated to the
     runtime's on-host store prefix (old prefix discovered from the baked
     files).
  4. adopted build trees don't bundle `pythonXY.dll`, so `usdcat`/`usdview`
     couldn't start → a matching host interpreter's dir is put on the
     session loader path.
  5. Windows Python 3.8+ doesn't search PATH for an extension's dependent
     DLLs, so `import pxr` failed on `_tf` → `os.add_dll_directory` preamble
     over the session PATH in the Python level scripts.
  6. `usdcat --flatten` stamps the absolute root-layer path into the golden's
     `doc`, so a committed golden never matched off its origin host → the L5
     comparison normalizes that line. The fixture is now a product-level
     contract in continuous CI for renderer/transport changes.
- ✅ **P1 — slim/SDK-profile `runtime export`.** `ost runtime export --slim`
  ships only the SDK layout — `include`, `lib`, `bin`, `plugin`, `cmake`,
  `libraries` (MaterialX standard defs), plus the top-level CMake package
  config and attribution files — dropping the source/`build` tree and sample
  `resources/` a runtime adopted from a full USD build carries. The predicate
  (`ost_build::is_sdk_path`) is pure and unit-tested; the excluded top-level
  entries are reported in the human and `--json` output, and the producer
  manifest records `layout_profile: sdk|full` so a fetch can tell a slim
  artifact from a full one (distinct digests of the same runtime). Measured on
  report #10's runtime: **1.93 GiB → 27 MiB archive (~73×), 18,029 → 3,818
  files, ~30 s vs ~52 min**, and the slim artifact — materialized into a clean
  `OST_HOME` — builds and runs the toy fixture's full L0–L5 pyramid green
  (12 pass / 0 fail / 3 skip). This is the clean-install answer to the adopted
  build-tree relocatability issues above (the `build/` tree that carried the
  stale absolute paths is simply gone).
- ✅ **P1 — `runtime export` performance + progress.** `export` now packs with
  multithreaded zstd by default (`--jobs`, defaulting to the host's available
  parallelism; `--jobs 1` forces the single-threaded encoder) and takes a
  `--level` knob (1–22, default 19). Throttled in-place progress prints to
  stderr (`N/M files, <bytes> in <secs>s`) so a long pack shows liveness
  instead of looking hung, and the finished archive is stream-hashed rather
  than read whole into memory. Small artifacts (`ost package`/`ost plugin`)
  keep the byte-stable single-threaded default via `pack_dir`.
- ✅ **P2 — L5 golden skip-message clarity (Phase 4 harness UX).** The L5
  `golden.roundtrip` SKIP now names the concrete expected file
  (`<fixture-filename>.golden.usda`, fixture extension retained — e.g.
  `basic.toy.golden.usda`), states it must be the *flattened* stage, and
  carries the generation recipe as a suggested action
  (`ost plugin run <bundle> -- usdcat --flatten <fixture> --out <golden>`),
  rendered under the diagnostic in human and `--json` output
  ([levels.rs](../crates/ost-plugin/src/levels.rs) `level5_golden`).
- **Deferred to v0.10.0+:** `ost artifact push` + plugin publish over OCI +
  protected publish policy + OIDC federation (plan Phase 3); trust levels in
  manifest/CI contract, publisher identity/provenance, SBOM attach, trusted
  runtime allowlist (plan Phase 4 — tracks SEC-006 and the trust-policy hooks
  above); registry mirroring / air-gapped sync / multi-registry failover.

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
