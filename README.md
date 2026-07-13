# OpenStrata (`ost`)

[![ci](https://github.com/animu-sphere/open-strata/actions/workflows/ci.yml/badge.svg)](https://github.com/animu-sphere/open-strata/actions/workflows/ci.yml)
[![licenses](https://github.com/animu-sphere/open-strata/actions/workflows/licenses.yml/badge.svg)](https://github.com/animu-sphere/open-strata/actions/workflows/licenses.yml)
[![docs](https://github.com/animu-sphere/open-strata/actions/workflows/docs.yml/badge.svg)](https://github.com/animu-sphere/open-strata/actions/workflows/docs.yml)
[![release](https://github.com/animu-sphere/open-strata/actions/workflows/release.yml/badge.svg)](https://github.com/animu-sphere/open-strata/actions/workflows/release.yml)
[![latest release](https://img.shields.io/github/v/release/animu-sphere/open-strata?sort=semver)](https://github.com/animu-sphere/open-strata/releases/latest)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

> OpenStrata turns VFX compatibility into executable, validated, distributable runtime layers.

OpenStrata is a VFX Reference Platform aware **runtime / build / extension / validation**
platform for VFX and OpenUSD work. The CLI is `ost`. It treats each VFX Reference
Platform calendar year as a machine-readable *target* and turns it into certified,
reproducible runtime layers, extension artifacts, and builds.

See [`docs/`](docs/) — [overview](docs/concepts/overview.md), [architecture](docs/architecture/overview.md),
[examples](docs/guides/examples.md), [roadmap](docs/roadmap/README.md), and the full
[design](docs/design/spec.md).

## Status

Phases 0–3 are implemented, and Phase 4 (the OpenUSD plugin verification harness)
is largely in: a real OpenUSD runtime can be **adopted** (`--from-usd`) or
**built from source** (`--build`, via build_usd.py or CMake-direct), and the
plugin pyramid runs Levels 0–5 (`ost plugin new|inspect|build|doctor|run|test`).
`ost plugin build` also regenerates co-hosted `schema.usda` contracts and can
link generated typed schema APIs into an existing plugin library.
Tagged binary releases (`v*`) are live via cargo-dist. The digest-addressed
artifact registry, plugin publishing, artifact-backed runtime pulls (local and
read/write OCI transport), and GitHub support-matrix generation are in, along with
a portable CI contract (runner profiles, lanes, digest-pinned hosted source-CI).

The current release is **v0.17.0** — renderer build completion evidence,
recoverable managed CMake execution, one-command Hydra inspection through
`ost renderer view`, renderer adoption/evidence transport, and generated support
references for environment variables and platform support.
Per-release detail (objective, shipped capabilities, compatibility, known
limitations) lives in [docs/releases/](docs/releases/); active, incomplete work is
in the [roadmap](docs/roadmap/README.md).

Live downstream release-lane dogfood, sessions, GPU/AI, and broader DCC matrices
are still ahead.
Linux x86_64 is the first-class target; other OS targets are modeled and
partially working — these examples were exercised on Windows.

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
ost init [--template cpp-library|renderer|usd-plugin|usd-plugin-workspace|--bare]  scaffold a buildable project
ost runtime    pull | list | show | validate | repair | explain | export   manage runtimes in the store
ost env <cy> --profile <p> [--shell bash|pwsh]      print the activating environment
ost devshell <cy> --profile <p>                     enter an interactive runtime shell
ost doctor [<cy> --profile <p>]                     host + tools + runtime diagnostics
ost configure [--target <cy>] [--profile <p>]       generate toolchain + CMake presets
ost presets    install | diff | uninstall           wire per-target presets into CMakePresets.json
ost build [--check] [--dry-run] [--jobs N]          preflight, then cmake build (Ninja)
ost package                                         install + tar.zst artifact + manifest
ost validate                                        validate a built/packaged target
ost extension  list | why <id> | add <id>           inspect/request controlled extensions
ost plugin     new | inspect | build | doctor | run | test | view | test-view | schema add | package | publish   OpenUSD plugin bundles + workspace graph validation
ost renderer   view [scene]                          open a built Hydra renderer in the matching usdview session
ost artifact   import | list | show | verify | export | extract   local digest-addressed artifact registry
ost ci         init | validate | plan | generate github    runtime×plugin support matrix -> CI workflow
ost lock [--check]                                  generate/verify strata.lock
ost uv <args...>                                    run uv pinned to the runtime Python
```

Every command accepts `--json` for machine-readable output: a single, versioned
`{ok, schema, data, warnings}` envelope on stdout, with structured errors
(`error.code` / `error.category`) and category-based exit codes for CI. See
[docs/reference/json-output.md](docs/reference/json-output.md) for the contract and
[docs/guides/examples.md](docs/guides/examples.md) for a copy-pasteable tour of every command.

`ost runtime pull` materializes a runtime from a backend *source*: a placeholder
`mock` layout, an adopted existing install (`--from-usd`), a from-source
`build` (`--build`, via build_usd.py or CMake-direct with `--deps`), or a
prebuilt registry `artifact` (`--from-artifact <digest>`, produced by
`ost runtime export`).

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
$ost plugin test --workspace --up-to 1                # validate graph, then test every bundle
$ost plugin package toy --target cy2026 --profile usd
$ost plugin publish toy --target cy2026 --profile usd   # register by digest
$ost artifact list                                       # what the registry holds
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
  ost-artifact/   artifact registry: identity records, content-addressed store, verification
  ost-ci/         CI support matrix (openstrata.ci.yaml) and workflow generation
  ost-manifest/   project (openstrata.toml) + lock (strata.lock) models
platforms/        built-in VFX Reference Platform calendar-year manifests
profiles/         capability bundles (core / dev / usd / lookdev)
extensions/       controlled extension manifests (openusd / materialx)
templates/        project + plugin scaffolds (including codeless/compiled USD schemas)
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
[roadmap backlog](docs/roadmap/backlog.md#cross-cutting-open-items) (shipped
baseline detail: [delivery history](docs/reports/delivery-history.md#security-baseline)).

To report a vulnerability, see [SECURITY.md](SECURITY.md) (private disclosure).

## License

OpenStrata is licensed under the [Apache License, Version 2.0](LICENSE); see also
[NOTICE](NOTICE). Source files carry an SPDX header
(`SPDX-License-Identifier: Apache-2.0`).

Third-party components that OpenStrata bundles, links, or distributes (Rust
dependencies and runtime/extension content such as OpenUSD, MaterialX, and their
transitive dependencies) retain their own upstream licenses. Complete
per-artifact third-party attribution is tracked in the
[roadmap backlog](docs/roadmap/backlog.md#cross-cutting-open-items).
