# OpenStrata (`ost`)

> OpenStrata turns VFX compatibility into executable, validated, distributable runtime layers.

OpenStrata is a VFX Reference Platform aware **runtime / build / extension / validation**
platform for VFX and OpenUSD work. The CLI is `ost`. It treats each VFX Reference
Platform calendar year as a machine-readable *target* and turns it into certified,
reproducible runtime layers, extension artifacts, and builds.

See [`docs/`](docs/) — [overview](docs/overview.md), [architecture](docs/architecture.md),
[roadmap](docs/roadmap.md), and the full [design](docs/design.md).

## Status

Phases 0–3 are implemented (Linux x86_64 is the first-class target; other OS
targets are modeled and partially working — these examples were exercised on
Windows). Real artifact pulls, the USD plugin lifecycle, CI generation, sessions,
and GPU/AI are still ahead — see the [roadmap](docs/roadmap.md).

## Build

```bash
cargo build
cargo test
cargo clippy --workspace --all-targets
```

## Commands

```text
ost platform   list | show <cy> | diff <a> <b>     inspect VFX platform years
ost init                                            scaffold openstrata.toml + .strata/
ost runtime    pull | list | show | validate | explain   manage runtimes in the store
ost env <cy> --profile <p> [--shell bash|pwsh]      print the activating environment
ost devshell <cy> --profile <p>                     enter an interactive runtime shell
ost doctor [<cy> --profile <p>]                     host + tools + runtime diagnostics
ost configure [--target <cy>] [--profile <p>]       generate toolchain + CMake presets
ost build [--dry-run] [--jobs N] [--ninja <p>]      configure + cmake build (Ninja)
ost package                                         install + tar.zst artifact + manifest
ost validate                                        validate a built/packaged target
ost extension  list | why <id> | add <id>           inspect/request controlled extensions
ost lock [--check]                                  generate/verify strata.lock
ost uv <args...>                                    run uv pinned to the runtime Python
```

Every command accepts `--json` for machine-readable output and uses deterministic
exit codes for CI.

## Try it

```bash
ost="cargo run -q -p ost-cli --"

# Inspect a platform year and scaffold a project
$ost platform show cy2026
$ost init --name my-show --platform cy2026

# Pull a certified USD runtime (local/mock backend for now)
$ost runtime pull cy2026 --profile usd
$ost runtime explain cy2026 --profile usd      # capability -> extension graph

# Activate it
$ost env cy2026 --profile usd --shell bash     # eval "$(... )"
$ost devshell cy2026 --profile usd

# Build a project against the runtime, then package and validate
$ost configure                                  # writes .strata/targets/... + strata.lock
$ost build                                       # cmake --preset + cmake --build (Ninja)
$ost package                                     # dist/<name>/<ver>/<target>/*.tar.zst
$ost validate

# Diagnose, lock, and run uv pinned to the runtime Python
$ost doctor cy2026 --profile usd
$ost lock --check
$ost uv sync --locked
```

On Windows, `ost build` auto-loads the MSVC developer environment (`vcvars64.bat`);
pass `--no-vcvars` to opt out.

## Workspace layout

```text
crates/
  ost-cli/        the `ost` binary (argument parsing + rendering)
  ost-core/       shared primitives: catalog loader, paths, host, variant, digest, tools
  ost-platform/   platform manifest model, loader, diff
  ost-runtime/    runtime identity, profiles, env generation, runtime manifest + validation
  ost-build/      build target model, toolchain/preset generation, packaging, MSVC bootstrap
  ost-extension/  controlled extensions: model, loader, capability resolver
  ost-manifest/   project (openstrata.toml) + lock (strata.lock) models
platforms/        built-in VFX Reference Platform calendar-year manifests
profiles/         capability bundles (core / dev / usd / lookdev)
extensions/       controlled extension manifests (openusd / materialx)
schemas/          JSON schemas for platform / project / lock documents
```

## Store and overlays

State lives under `~/.ost` (override with `OST_HOME`): `runtimes/`, `extensions/`,
`artifacts/`, `cache/`, `sessions/`, `logs/`. User-provided manifests in
`~/.ost/{platforms,profiles,extensions}/*.yaml` are layered over the built-in
definitions and override them by id.

Tool overrides for non-PATH installs: `OST_NINJA` (ninja), `OST_UV` (uv).
