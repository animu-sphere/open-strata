# Command examples

A practical, copy-pasteable tour of the `ost` CLI. Every command also accepts
`--json` for machine-readable output and uses deterministic exit codes
(`0` success / non-zero failure), so the same invocations drive both interactive
use and CI.

In the examples `ost` is the built binary. To run from a checkout instead:

```bash
ost() { cargo run -q -p ost-cli -- "$@"; }
```

State lives under `~/.ost` (override with `OST_HOME`). See
[architecture.md](architecture.md#on-disk-layout) for the on-disk layout.

---

## platform — inspect VFX Reference Platform years

```bash
ost platform list                     # all known calendar years
ost platform show cy2026              # full component set for one year
ost platform diff cy2025 cy2026      # component differences between two years
ost platform show cy2026 --json      # machine-readable
```

## init — scaffold a project

```bash
ost init                                      # cpp-library template, dir name + latest platform
ost init --template usd-plugin                # scaffold a USD plugin project
ost init --name my-show --platform cy2026     # explicit name + platform
ost init --bare                               # manifest only — adopt an existing CMake project
ost init --force                              # overwrite an existing manifest / template files
```

Writes `openstrata.toml`, a `.strata/` state directory, and (unless `--bare`) a
minimal, buildable CMake project — so `ost build` works straight after
`ost runtime pull`.

## runtime — manage runtimes in the store

`runtime pull` materializes a runtime from a backend **source**. Pick the source
by flag; precedence is build > adopt > mock.

```bash
# mock — placeholder layout, no real OpenUSD (works offline, for the static checks)
ost runtime pull cy2026 --profile usd

# local / adopt — register an existing OpenUSD install in place (fastest real USD)
ost runtime pull cy2026 --profile usd --from-usd /opt/usd
OST_USD_ROOT=/opt/usd ost runtime pull cy2026 --profile usd

# build (build_usd.py) — build OpenUSD + deps from source into the store
ost runtime pull cy2026 --profile usd --build /src/OpenUSD --jobs 8
ost runtime pull cy2026 --profile usd --build /src/OpenUSD --build-arg --no-imaging

# build (CMake-direct) — build OpenUSD against pre-provided dependency prefixes
ost runtime pull cy2026 --profile usd \
  --build /src/OpenUSD --deps /opt/usd-deps \
  --build-arg -DPXR_BUILD_IMAGING=OFF
```

```bash
ost runtime list                          # what's in the store (+ SOURCE column)
ost runtime show cy2026 --profile usd      # manifest: source, prefix, deps, digest
ost runtime validate cy2026 --profile usd  # schema/digest/layout (+ usdcat/pxr if real)
ost runtime explain cy2026 --profile lookdev   # capability -> extension graph
ost runtime pull cy2026 --profile usd --force  # re-pull / rebuild
```

Source selection also reads env fallbacks: `OST_USD_ROOT` (adopt), `OST_USD_SRC`
(build source), `OST_USD_DEPS` (CMake deps, OS-path-separator list).

## env / devshell — activate a runtime

```bash
eval "$(ost env cy2026 --profile usd --shell bash)"   # apply to the current shell
ost env cy2026 --profile usd --shell pwsh             # PowerShell form
ost env cy2026 --profile usd --json                   # the resolved vars as data
ost devshell cy2026 --profile usd                     # spawn an interactive shell
```

No global mutation happens outside these two commands.

## doctor — diagnose host, tools, runtime

```bash
ost doctor                              # host descriptor + tool detection
ost doctor cy2026 --profile usd        # also diagnose a specific runtime
ost doctor cy2026 --profile usd --json
```

## configure / build / package / validate — the build lifecycle

Run inside a project; target/profile default to `openstrata.toml`.

```bash
ost configure                          # toolchain.cmake + CMakePresets + strata.lock
ost configure --compiler runtime       # use the runtime's bundled gcc/clang
ost configure --cc /usr/bin/clang --cxx /usr/bin/clang++   # explicit compiler
ost build                              # cmake --preset + cmake --build (Ninja)
ost build --check                      # preflight only, no side effects
ost build --dry-run                    # print the commands, runtime env + files only
ost build --jobs 8 --ninja /opt/ninja/ninja
ost build --no-vcvars                  # Windows: skip MSVC auto-bootstrap
ost build --progress plain             # CI: phase=… status=… lines instead of a TTY view
ost build --quiet                      # silence progress; output to .strata/targets/<id>/build.log
ost package                            # install + dist/<name>/<ver>/<target>/*.tar.zst
ost package --allow-empty              # permit a metadata-only artifact (empty install tree)
ost validate                           # configured / built / runtime / artifact checks
```

On Windows, `ost build` auto-loads the MSVC developer environment
(`vcvars64.bat`) unless `--no-vcvars` is given.

## extension — controlled components

```bash
ost extension list                     # the certified extension catalog
ost extension why materialx --profile lookdev   # trace why it's required (direct/transitive)
ost extension add materialx            # record it in openstrata.toml (idempotent)
```

## plugin — OpenUSD plugin bundles

The verification pyramid: L0–L1 are static (any backend); L2–L5 execute the
runtime's tools and need a **real** runtime (adopt or build a source first).

```bash
# scaffold a bundle (C++ SdfFileFormat + plugInfo.json + CMake + fixtures + manifest)
ost plugin new usd-fileformat toy --extension toy
ost plugin new usd-fileformat toy --extension toy --dir ./plugins/toy

ost plugin inspect toy                 # Level 0 structure (human + --json)
ost plugin build toy --target cy2026 --profile usd       # build the .so via ost-build
ost plugin build toy --dry-run         # show the cmake commands only

# static diagnostics (L0–L1) + session-env preview; writes .strata/reports/...
ost plugin doctor toy --target cy2026 --profile usd

# full pyramid L0..L5 against a real runtime; writes a report
ost plugin test toy --target cy2026 --profile usd
ost plugin test toy --up-to 3          # stop after usdcat read
ost plugin test toy --json

# launch any command inside the composed runtime session (real runtime)
ost plugin run toy --target cy2026 --profile usd -- usdcat tests/fixtures/basic.toy

# Level 6: open a fixture in usdview, or verify it launches (needs usdview + display)
ost plugin view      toy tests/fixtures/basic.toy --target cy2026 --profile usd
ost plugin test-view toy tests/fixtures/basic.toy --target cy2026 --profile usd
ost plugin test toy --up-to 6 --target cy2026 --profile usd   # full pyramid incl. L6
```

Reports land under `<bundle>/.strata/reports/<plugin>/<UTC>/`
(`report.json`, `summary.txt`, `environment.json`); see the
[plugin-report schema](../schemas/plugin-report.schema.json).

## lock — reproducibility

```bash
ost lock                               # write strata.lock for the resolved runtime
ost lock --check                       # verify it's up to date (exit 1 if not) — gate CI
```

## uv — Python pinned to the runtime

```bash
ost uv                                  # show how uv would be pinned (no run)
ost uv sync --locked                    # run uv with UV_PYTHON = the runtime interpreter
ost uv pip install ./my-tool
```

---

## End-to-end recipes

### Build & package a project against a runtime

```bash
ost init --name my-show --platform cy2026
ost runtime pull cy2026 --profile usd --from-usd /opt/usd   # a real USD
ost configure
ost build
ost package
ost validate
ost lock --check
```

### Verify an OpenUSD plugin

```bash
# 1. a real runtime (adopt an install, or build from source)
ost runtime pull cy2026 --profile usd --from-usd /opt/usd
ost runtime validate cy2026 --profile usd

# 2. scaffold, build, and run the full verification pyramid
ost plugin new usd-fileformat toy --extension toy
ost plugin build  toy --target cy2026 --profile usd
ost plugin test   toy --target cy2026 --profile usd      # L0..L5 + report
```

## Tool overrides

For tools not on `PATH`: `OST_NINJA` (ninja), `OST_UV` (uv). Runtime-source env
fallbacks: `OST_USD_ROOT`, `OST_USD_SRC`, `OST_USD_DEPS`. Store location:
`OST_HOME` (defaults to `~/.ost`).
