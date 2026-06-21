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

## Phase 1 — Runtime and devshell 🚧

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
- ⬜ `ost runtime explain`
- ⬜ Real artifact backend behind `pull` (currently mock prefix layout)
- ⬜ Richer runtime validation (Python import, native library load, USD stage
  open) once the real backend lands

## Phase 2 — CMake target build 🚧

- ✅ Target model + id (`cy2026-linux-x86_64-py313-usd`) in `ost-build`
- ✅ `ost configure`: `toolchain.cmake`, `env.json`, `target.lock.json`,
  per-target `CMakePresets.json`, and a root `CMakePresets.json` that includes
  each target (verified with `cmake --list-presets`)
- ⬜ Ninja build wrapper: `ost build | validate | package`
- ⬜ `cmake --preset` configure + `cmake --build` invocation from `ost build`

## Phase 3 — OpenUSD / MaterialX profiles ⬜

- ⬜ OpenUSD extension family with feature sets (core/python/imaging/materialx/…)
- ⬜ Capability resolver; `usd` and `lookdev` profiles
- ⬜ Compatible range vs certified build point
- ⬜ `ost extension add | why`, `ost runtime explain`

## Phase 4 — USD file format plugin lifecycle ⬜

- ⬜ Plugin templates; `plugInfo.json` handling
- ⬜ `PXR_PLUGINPATH_NAME` generation; discovery + fixture validation
- ⬜ `ost plugin new | build | validate | package | publish`
- ⬜ `ost doctor usd` (discovered plugins, file formats, conflicts)

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

## Quality bar (applies to every phase)

- CLI errors must be actionable.
- All generated manifests must be deterministic.
- Runtime and extension identities always include version + target + digest.
- No hidden environment mutation outside `ost devshell` / `ost env`.
- Every published artifact includes provenance and validation result.
- OpenStrata must work without a preinstalled Python environment.
