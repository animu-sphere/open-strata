# OpenStrata (`ost`)

> OpenStrata turns VFX compatibility into executable, validated, distributable runtime layers.

OpenStrata is a VFX Reference Platform aware **runtime / build / extension / validation**
platform for VFX and OpenUSD work. The CLI is `ost`. It treats each VFX Reference
Platform calendar year as a machine-readable *target* and turns it into certified,
reproducible runtime layers, extension artifacts, and builds.

See [`docs/`](docs/) — [overview](docs/overview.md), [architecture](docs/architecture.md),
[examples](docs/examples.md), [roadmap](docs/roadmap.md), and the full
[design](docs/design.md).

## Status

Phases 0–3 are implemented, and Phase 4 (the OpenUSD plugin verification harness)
is largely in: a real OpenUSD runtime can be **adopted** (`--from-usd`) or
**built from source** (`--build`, via build_usd.py or CMake-direct), and the
plugin pyramid runs Levels 0–5 (`ost plugin new|inspect|build|doctor|run|test`).
`ost plugin build` also regenerates co-hosted `schema.usda` contracts and can
link generated typed schema APIs into an existing plugin library.
Tagged binary releases (`v*`) are live via cargo-dist; CI generation, sessions,
GPU/AI, and the fetched artifact registry are still ahead. Linux x86_64 is the
first-class target; other OS targets
are modeled and partially working — these examples were exercised on Windows. See
the [roadmap](docs/roadmap.md).

## Install

`ost` is a single self-contained binary. Tagged releases (`v*`) publish
prebuilt binaries, checksums, and installers for Linux (static musl), macOS
(arm64 + x86_64), and Windows via [cargo-dist](https://opensource.axo.dev/cargo-dist/).
Each release also carries SLSA build-provenance attestations, verifiable with
`gh attestation verify <asset> --repo animu-sphere/open-strata`.

```bash
# Linux / macOS — fetch the installer for your host (from v0.1.0 onward)
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/animu-sphere/open-strata/releases/latest/download/ost-cli-installer.sh | sh

# Windows (PowerShell)
powershell -c "irm https://github.com/animu-sphere/open-strata/releases/latest/download/ost-cli-installer.ps1 | iex"

# With cargo-binstall (reads the release's dist manifest)
cargo binstall ost-cli
```

From source (no prebuilt needed):

```bash
cargo install --path crates/ost-cli   # installs the `ost` binary
```

## Build

```bash
cargo build
cargo test
cargo clippy --workspace --all-targets
```

For Windows or agent-driven local gates, prefer an isolated target directory so
stale file locks in `target/` do not block Cargo:

```powershell
$env:CARGO_TARGET_DIR = Join-Path $env:TEMP "ost-cargo-target-$PID"
$env:CARGO_INCREMENTAL = "0"
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Native build/package lifecycle tests run automatically on non-Windows hosts when
cmake, ninja, and a compiler are available. On Windows they are skipped by
default; opt in explicitly when you want that coverage:

```powershell
$env:OST_RUN_NATIVE_LIFECYCLE = "1"
cargo test -p ost-cli --test lifecycle --locked -- --nocapture
```

## Commands

```text
ost platform   list | show <cy> | diff <a> <b>     inspect VFX platform years
ost init [--template cpp-library|usd-plugin|usd-plugin-workspace|--bare]  scaffold a buildable project
ost runtime    pull | list | show | validate | explain   manage runtimes in the store
ost env <cy> --profile <p> [--shell bash|pwsh]      print the activating environment
ost devshell <cy> --profile <p>                     enter an interactive runtime shell
ost doctor [<cy> --profile <p>]                     host + tools + runtime diagnostics
ost configure [--target <cy>] [--profile <p>]       generate toolchain + CMake presets
ost presets    install | diff | uninstall           wire per-target presets into CMakePresets.json
ost build [--check] [--dry-run] [--jobs N]          preflight, then cmake build (Ninja)
ost package                                         install + tar.zst artifact + manifest
ost validate                                        validate a built/packaged target
ost extension  list | why <id> | add <id>           inspect/request controlled extensions
ost plugin     new | inspect | build | doctor | run | test | view | test-view   OpenUSD plugin bundles
ost lock [--check]                                  generate/verify strata.lock
ost uv <args...>                                    run uv pinned to the runtime Python
```

Every command accepts `--json` for machine-readable output: a single, versioned
`{ok, schema, data, warnings}` envelope on stdout, with structured errors
(`error.code` / `error.category`) and category-based exit codes for CI. See
[docs/json-schema.md](docs/json-schema.md) for the contract and
[docs/examples.md](docs/examples.md) for a copy-pasteable tour of every command.

`ost runtime pull` materializes a runtime from a backend *source*: a placeholder
`mock` layout, an adopted existing install (`--from-usd`), or a from-source
`build` (`--build`, via build_usd.py or CMake-direct with `--deps`).

## Try it

```bash
ost="cargo run -q -p ost-cli --"

# Inspect a platform year and scaffold a project
$ost platform show cy2026
$ost init --name my-show --platform cy2026

# Pull a USD runtime: mock (offline), or a real one via --from-usd / --build
$ost runtime pull cy2026 --profile usd                       # mock layout
$ost runtime pull cy2026 --profile usd --from-usd /opt/usd   # adopt a real install
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

# Scaffold and verify an OpenUSD plugin (L2+ need a real runtime)
$ost plugin new usd-fileformat toy --extension toy
$ost plugin build toy --target cy2026 --profile usd
$ost plugin test  toy --target cy2026 --profile usd   # L0..L5 + report
$ost plugin package toy --target cy2026 --profile usd
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
  ost-plugin/     OpenUSD plugin bundles: model, scaffold, verification levels, reports
  ost-manifest/   project (openstrata.toml) + lock (strata.lock) models
platforms/        built-in VFX Reference Platform calendar-year manifests
profiles/         capability bundles (core / dev / usd / lookdev)
extensions/       controlled extension manifests (openusd / materialx)
templates/        project scaffolds + plugin scaffolds (usd-fileformat-cpp, usd-schema-codeless)
schemas/          JSON schemas for platform / project / lock / plugin-report documents
```

## Store and overlays

State lives under `~/.ost` (override with `OST_HOME`): `runtimes/`, `extensions/`,
`artifacts/`, `cache/`, `sessions/`, `logs/`. User-provided manifests in
`~/.ost/{platforms,profiles,extensions}/*.yaml` are layered over the built-in
definitions and override them by id.

Tool overrides for non-PATH installs: `OST_NINJA` (ninja), `OST_UV` (uv). Runtime
source fallbacks for `ost runtime pull`: `OST_USD_ROOT` (adopt), `OST_USD_SRC`
(build), `OST_USD_DEPS` (CMake deps).

On macOS source builds, full Xcode may be needed for upstream codesign; with
CMake 4 and bundled dependencies, retry with
`--build-arg -DCMAKE_POLICY_VERSION_MINIMUM=3.5` if configure fails.

## Security

OpenStrata is being hardened across build, runtime, plugins, CI, and the
distribution path. Landed so far:

- **Packaging** rejects symlinks and special files (FIFO/socket/device) anywhere
  in the staging tree, so an artifact cannot absorb a link target's bytes or
  recurse outside the tree.
- **Plugin manifests** may only reference paths inside their bundle; `..`,
  absolute, drive, and UNC paths are refused when the bundle loads.
- **Atomic writes** create their temp file with `O_EXCL` under an unpredictable
  name and refuse to write through a symlinked destination.
- **CI** pins every third-party GitHub Action to a full commit SHA, and release
  artifacts carry SLSA build-provenance attestations.

Remaining work — installer/asset signature verification and a runtime trust
policy — is tracked in the
[roadmap](docs/roadmap.md#security-baseline).

## License

OpenStrata is licensed under the [Apache License, Version 2.0](LICENSE); see also
[NOTICE](NOTICE). Source files carry an SPDX header
(`SPDX-License-Identifier: Apache-2.0`).

Third-party components that OpenStrata bundles, links, or distributes (Rust
dependencies and runtime/extension content such as OpenUSD, MaterialX, and their
transitive dependencies) retain their own upstream licenses. Complete
per-artifact third-party attribution is tracked in the
[roadmap](docs/roadmap.md#licensing--third-party-attribution).
