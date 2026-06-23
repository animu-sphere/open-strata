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
- ✅ `--json` output and deterministic exit codes

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
  deterministic exit: 0 healthy / 1 issues)
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
- ⬜ Multi-plugin sessions (`ost plugin run/view --with <bundle>…`) and
  bundle-declared `requires.runtime_libs` (extra non-USD runtime lib dirs, e.g. a
  plugin's zlib) — replaces hand-rolled usdview launch batch files for the
  multi-plugin + 3rd-party-dep case
- ⬜ `ost plugin package | publish` and the runtime×plugin CI matrix
  (`artifact` source lands with Phase 6)

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

## Quality bar (applies to every phase)

- CLI errors must be actionable.
- All generated manifests must be deterministic.
- Runtime and extension identities always include version + target + digest.
- No hidden environment mutation outside `ost devshell` / `ost env`.
- Every published artifact includes provenance and validation result.
- Every published artifact carries complete third-party attribution (no missing
  upstream licenses/notices).
- OpenStrata must work without a preinstalled Python environment.
