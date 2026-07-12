# Architecture

*Last verified against: v0.13.0 (workspace version 0.13.0).* This document
describes the system as it exists on the default branch; historical alternatives
belong in design notes, not here.

## Workspace layout

OpenStrata is a Rust workspace. The CLI is thin; domain logic lives in libraries so
it can be reused by future surfaces (CI helpers, a daemon, tests). The ten `ost-*`
crates and their boundaries are documented in [crates.md](crates.md).

```text
crates/           the ost-cli binary + nine ost-* domain libraries (see crates.md)
platforms/        built-in CY manifests, embedded into the binary
profiles/         capability bundles (core / dev / usd / lookdev)
extensions/       controlled extension manifests (openusd / materialx)
templates/        project + plugin scaffolds (usd-fileformat-cpp, usd-schema-codeless,
                  usd-schema-cpp, usd-asset-resolver-cpp, usd-package-resolver-cpp, …)
schemas/          JSON schemas for platform / project / lock / plugin-report documents
docs/             this documentation
```

## Domain model

| Concept | Meaning | Status |
| --- | --- | --- |
| **Platform** | A VFX Reference Platform calendar year as a machine-readable manifest (`cy2026`). | implemented |
| **Variant** | Concrete artifact identity: OS + arch + ABI + Python ABI, e.g. `linux-x86_64-glibc228-py313`. | implemented |
| **Project** | `openstrata.toml`: the runtime contract a project builds against (platform, profile, capabilities, extensions). | implemented |
| **Lock** | `strata.lock`: pinned runtime digest, variant, Python ABI, validation status. | implemented |
| **Profile** | A named bundle of capabilities (`core`, `usd`, `lookdev`, …). | implemented |
| **Capability** | A logical feature requested/provided (`usd-materialx`). | implemented |
| **Extension** | A controlled VFX-adjacent component (OpenUSD, MaterialX). | implemented |
| **Runtime** | Platform + variant + profile + resolved artifacts, with a digest and a backend source (`mock`/`local`/`build`). | implemented |
| **Plugin bundle** | A self-describing OpenUSD plugin (`openstrata.plugin.yaml` + sources + `plugInfo.json` + fixtures), verified by levels 0–5. | implemented |
| **Plugin workspace** | Deterministically discovered plugin bundles plus a read-only, version/contract-checked dependency graph; build ordering is not inferred yet. | implemented |
| **Artifact** | An immutable, digest-addressed bundle (`tar.zst` + manifest + validation report) in the local registry, transportable over OCI. | implemented |
| **Support matrix** | `openstrata.ci.yaml`: digest-pinned runtime×plugin support lines, runner profiles, and lanes, rendered to CI workflows. | implemented |
| **Session** | A mutable workspace over an immutable runtime. | planned |

## On-disk layout

Two roots matter:

```text
~/.ost/                      # user store (override with OST_HOME)
  config.toml
  platforms/ profiles/ extensions/   # user manifests, overlaid over built-ins
  runtimes/<id>/             # runtime.json (+ real artifacts for build/mock)
  artifacts/ cache/ sessions/ logs/  # cache/ holds e.g. usd-build/ trees

<project>/
  openstrata.toml            # authored project manifest
  .strata/                   # generated state (gitignored)
    targets/<target>/        # toolchain.cmake, env.json, target.lock.json, CMakePresets.json

<plugin-bundle>/             # an ost-plugin bundle (may live anywhere)
  openstrata.plugin.yaml     # bundle contract
  .strata/reports/<plugin>/<UTC>/   # report.json, summary.txt, environment.json
```

An adopted (`local`) runtime keeps only its `runtime.json` in the store; its real
artifacts stay at the external `--from-usd` prefix recorded in the manifest.

## Platform manifest resolution

Built-in CY manifests (`platforms/*.yaml`) are compiled into the binary so
`ost platform list` works on a fresh install with no network or store. User YAML in
`~/.ost/platforms/*.yaml` is layered on top and overrides built-ins by `id`. This is
the smallest expression of the "resolve from capability/manifest, layered" principle
and the seam where studio-specific platform definitions plug in.

## Output and CI

Every command renders either for a human terminal or as JSON (`--json`) and uses
deterministic exit codes, so the same commands drive both interactive use and CI
(§13.2 of the design). Error shapes are centralized in `ost-cli`'s output module.

## Toolchain pinning

The toolchain is pinned in `rust-toolchain.toml` (currently Rust 1.96) so local,
CI, and release builds all use the same compiler; `rust-version` in `Cargo.toml`
records the MSRV floor and is kept in sync. Top-level dependency versions are
pinned in `Cargo.toml` and transitive versions in `Cargo.lock` for reproducible
builds; bump them deliberately alongside a toolchain bump.
