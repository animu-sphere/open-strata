# OpenStrata (`ost`)

> OpenStrata turns VFX compatibility into executable, validated, distributable runtime layers.

OpenStrata is a VFX Reference Platform aware **runtime / build / extension / validation**
platform for VFX and OpenUSD work. The CLI is `ost`. It treats each VFX Reference
Platform calendar year as a machine-readable *target* and turns it into certified,
reproducible runtime layers, extension artifacts, and builds.

See [`docs/design.md`](docs/design.md) for the full design and roadmap.

## Status

Early development — **Phase 0 (Foundation)** is implemented:

- Rust workspace + `ost` CLI skeleton
- Machine-readable VFX Reference Platform manifests (CY2025 / CY2026 / CY2027)
- `ost platform list | show | diff`
- `ost init` (project manifest + `.strata/` state)
- Project / lock / platform JSON schemas

Later phases (runtime + devshell, CMake builds, OpenUSD/MaterialX extensions, USD
plugin lifecycle, CI, artifacts, sessions, GPU/AI) are modeled but not yet built.

## Build

```bash
cargo build
cargo test
```

## Try it

```bash
# List known platform years
cargo run -p ost-cli -- platform list

# Show a calendar-year definition
cargo run -p ost-cli -- platform show cy2026

# Diff two years
cargo run -p ost-cli -- platform diff cy2025 cy2026

# Scaffold a project (writes openstrata.toml + .strata/)
cargo run -p ost-cli -- init --name my-show --platform cy2026

# Any command emits machine-readable JSON for CI with --json
cargo run -p ost-cli -- --json platform diff cy2025 cy2026
```

## Workspace layout

```text
crates/
  ost-cli/        the `ost` binary (argument parsing + rendering)
  ost-core/       shared primitives: paths, host, variant, errors
  ost-platform/   platform manifest model, loader, diff
  ost-manifest/   project (openstrata.toml) + lock (strata.lock) models
platforms/        built-in VFX Reference Platform calendar-year manifests
schemas/          JSON schemas for platform / project / lock documents
```

User-provided platform manifests in `~/.ost/platforms/*.yaml` are layered over the
built-in definitions and override them by id. Set `OST_HOME` to relocate the store.
