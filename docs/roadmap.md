# Roadmap

Delivery is phased. Each phase is a usable increment, not a big-bang. Linux x86_64
is the first-class implementation target; other OS targets are modeled from the
start but may be unavailable initially.

Legend: ✅ done · 🚧 in progress · ⬜ not started

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
- ⬜ `ost doctor` structuring (§14.5): issues as
  `{id, severity, summary, next_action}`, runtime `kind`
  (mock/adopted/built/downloaded) + execution capabilities, and `warnings`
  (e.g. `MOCK_RUNTIME_ACTIVE`); absorbs the agent "status" need into `doctor`
  rather than a new command. Touches the runtime manifest schema
- ✅ `ost runtime validate` (schema, digest integrity, layout; records outcome
  in the manifest; deterministic exit)
- ✅ `ost runtime explain` (delivered in Phase 3)
- ✅ Project lockfile `strata.lock` via `ost lock [--check]` and refreshed by
  `ost configure`: pins runtime id/variant/digest, Python ABI + `uv.lock` hash,
  resolved extensions, and validation status; fully deterministic so `--check`
  gates CI
- ✅ Real runtime backends behind `pull` (Phase 4b): `local`/adopt and `build`
  (build_usd.py / CMake-direct) supersede the mock layout; fetched `artifact`
  source still ahead (Phase 6)
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
  files for the multi-plugin + 3rd-party-dep case. Dogfooding (usdVrm, reports
  #1/#2) surfaced these prerequisites and shapes:
  - **Every bundle path is absolutized at the `ost plugin` boundary** via
    `Bundle::load`, including every `--with <bundle>` arg. Its plugInfo root,
    `lib/`, `python/`, and any `requires.runtime_libs` dir are then composed as
    absolute session env entries, avoiding relative `CMAKE_TOOLCHAIN_FILE` and
    relative `PXR_PLUGINPATH_NAME` failures.
  - **`requires.runtime_libs` prepends to the session's dynamic-loader path**
    (`PATH` / `LD_LIBRARY_PATH` / `DYLD_LIBRARY_PATH`), absolutized and validated
    as bundle-relative. Empty/absent stays the common case: a plugin that
    statically links its 3rd-party deps (usdVrm vendors cgltf into one TU,
    exports no symbols) drags zero extra lib dirs — the opposite of a plugin
    shipping a sibling `zlib.dll`.
  - **`plugInfo.json` `LibraryPath` is generated/validated per target** (suffix +
    lib-dir), since multi-plugin × multi-OS sessions multiply the scaffold's
    cross-platform soft spot. Source bundles may carry templates, but built or
    packaged bundles must carry the concrete `plugInfo.json` for exactly one
    target (`.so` / `.dylib` / `.dll`) and one library layout. See the Phase-4
    fix backlog below.
- ⬜ `ost plugin package | publish` and the runtime×plugin CI matrix:
  package freezes the target-resolved `plugInfo.json`, resolved C++/Python ABI,
  runtime digest, and validation report into one binary bundle artifact
  (`artifact` source lands with Phase 6)

### Phase 4 — fix backlog (from usdVrm dogfooding, reports #1/#2)

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
  A real usdVrm-style bundle with
  `LibraryPath: "../../../lib/libUsdVrmFileFormat.dll"` is Windows-only even if
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
  `runtime.cxx_abi`. The binary package step will record the one resolved ABI
  for the artifact it publishes.
- ⬜ **P3 — repo-shape scaffold.** `ost init --bare` + `plugin new` leaves no
  top-level `CMakeLists.txt`, so the repo isn't `cmake -S .`-able by non-`ost`
  users. A project-with-bundles template could emit a dual-mode root
  `CMakeLists.txt` + `CMakePresets.json` that `add_subdirectory()`s each bundle
  and resolves USD via `find_package(pxr)`.

## Phase 5 — CI / Jenkins ⬜

- ⬜ CI-safe flags (`--ci`, `--no-interactive`, `--report junit|json`, `--jobs auto`)
- ⬜ Jenkinsfile template + matrix generation
- ⬜ `ost ci init | generate jenkins`

## Phase 6 — Artifact registry ⬜

- ⬜ Content-addressed artifact store (digest pinning)
- ⬜ `tar.zst` + manifest + checksums + validation report (MVP)
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

## Distribution — `ost` binary releases ⬜

The `ost` CLI is a single self-contained binary (no Python/USD dependency), so it
ships independently of the heavy runtime artifacts. Publish tagged builds to
GitHub Releases.

- ⬜ **Tag convention.** Releases are cut from an annotated tag `v<semver>` (e.g.
  `v0.1.0`) on `main`. The tag's version must match the workspace
  `Cargo.toml` `version`; a CI check fails the release on mismatch. Pre-releases
  use `-rc.N` / `-beta.N` suffixes and are marked "pre-release".
- ⬜ **Release workflow** (GitHub Actions, triggered on `v*` tags). A build
  matrix produces a binary per target, packaged with a checksum:
  - `linux-x86_64` (first-class), `macos-arm64`, `macos-x86_64`,
    `windows-x86_64` (modeled from the start; built on a best-effort basis).
  - Artifacts: `ost-<version>-<target>.tar.gz` (zip on Windows) + a combined
    `SHA256SUMS`, attached to the GitHub Release. Built on the pinned toolchain
    (see [architecture.md](architecture.md#toolchain-pinning)).
  - Release notes drawn from the changelog / merged PRs for the tag range.
- ⬜ **Install ergonomics.** A `cargo binstall` manifest and a `curl | sh`
  installer script that fetches the right asset for the host and verifies the
  checksum; document `cargo install --path crates/ost-cli` as the from-source
  fallback.
- ⬜ **Provenance (later).** Sign artifacts and attach SLSA/attestation metadata,
  reusing the digest/manifest discipline OpenStrata already applies to runtime
  and plugin artifacts.

This covers the **`ost` tool** itself; runtime/extension/plugin *content*
artifacts are distributed via the content-addressed store and the artifact
registry (Phase 6).

## Licensing & third-party attribution ⬜

OpenStrata must ship with a clear license of its own and **complete** attribution
for everything it bundles, links, or distributes. Today only the `Cargo.toml`
`license = "Apache-2.0"` field exists; the files and discipline below are still
to land.

- ✅ **OpenStrata's own license.** Top-level `LICENSE` (Apache-2.0, matching the
  manifests) and `NOTICE`; SPDX headers
  (`// SPDX-License-Identifier: Apache-2.0`) on all source files; `README` License
  section.
- ⬜ **Rust dependency attribution.** Generate and commit `THIRD_PARTY_NOTICES.md`
  for the crate tree with `cargo-about`, and gate licenses in CI with
  `cargo-deny` (allowlist of acceptable SPDX licenses; deny copyleft/unknown).
- ⬜ **Runtime/extension content attribution.** Anything OpenStrata builds or
  distributes (OpenUSD, MaterialX, TBB, OpenSubdiv, OpenEXR, OCIO, …, and their
  transitive deps) carries its upstream license. Each runtime/extension manifest
  records license metadata; built/adopted runtimes collect the upstream
  `LICENSE`/`NOTICE` files, and a runtime's licenses are inspectable
  (e.g. `ost runtime licenses <cy> --profile <p>`).
- ⬜ **Per-artifact notices + SBOM.** Every published artifact (runtime, plugin
  bundle, `ost` binary) includes its `LICENSE`/`NOTICE`/`THIRD_PARTY_NOTICES`
  and a generated SBOM (SPDX or CycloneDX); the package manifest/provenance lists
  component licenses by digest. **No artifact ships without complete third-party
  attribution** — this is a release gate.
- ⬜ **Plugin bundle license field.** `openstrata.plugin.yaml` gains a `license`
  (SPDX) and optional third-party notices, surfaced by `ost plugin inspect` and
  carried into `ost plugin package`.

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
- ⬜ **CI test gate.** Add a workflow running `cargo test` / `fmt --check` /
  `clippy` on PRs — the security regression tests above currently have no CI job
  to run them.

## Quality bar (applies to every phase)

- CLI errors must be actionable.
- All generated manifests must be deterministic.
- Runtime and extension identities always include version + target + digest.
- No hidden environment mutation outside `ost devshell` / `ost env`.
- Every published artifact includes provenance and validation result.
- Every published artifact carries complete third-party attribution (no missing
  upstream licenses/notices).
- OpenStrata must work without a preinstalled Python environment.
