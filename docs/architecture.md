# Architecture

*Last verified against: v0.12.0 (workspace version 0.12.0).* This document
describes the system as it exists on the default branch; historical alternatives
belong in design notes, not here.

## Workspace layout

OpenStrata is a Rust workspace. The CLI is thin; domain logic lives in libraries so
it can be reused by future surfaces (CI helpers, a daemon, tests).

```text
crates/
  ost-cli/        the `ost` binary: argument parsing + human/JSON rendering
  ost-core/       shared primitives: catalog loader, paths, host, variant, digest, tools, errors
  ost-platform/   VFX Reference Platform CY model, loader, diff
  ost-runtime/    runtime identity, profiles, env generation, runtime manifest + validation
  ost-build/      build target model, toolchain/preset generation, packaging, MSVC bootstrap
  ost-extension/  controlled extensions: model, loader, capability resolver
  ost-plugin/     OpenUSD plugin bundles: model, scaffold, verification levels, reports
  ost-artifact/   artifact registry: identity records, content-addressed store, verification, OCI transport
  ost-ci/         CI support matrix (openstrata.ci.yaml) + workflow generation (GitHub Actions)
  ost-manifest/   project (openstrata.toml) + lock (strata.lock) models
platforms/        built-in CY manifests, embedded into the binary
profiles/         capability bundles (core / dev / usd / lookdev)
extensions/       controlled extension manifests (openusd / materialx)
templates/        project + plugin scaffolds (usd-fileformat-cpp, usd-schema-codeless, …)
schemas/          JSON schemas for platform / project / lock / plugin-report documents
docs/             this documentation
```

Planned crates from the design (not yet created): `ost-solver`, `ost-session`,
`ost-validation`. They are introduced as their phase lands, not up front.
(`ost-artifact` and `ost-ci` were on this list historically; both now exist and
are shipped, listed above.)

## Crate boundaries

- **`ost-core`** holds vocabulary only — no domain logic. It defines where things
  live (`paths`), what machine we are on (`host`), how a build/runtime variant is
  identified (`variant`), and the shared `Error` type.
- **`ost-platform`** owns the platform manifest: model, a loader that embeds the
  built-in CY manifests and overlays user manifests, and a structured diff.
- **`ost-runtime`** turns a platform + profile into a concrete runtime identity
  and the `EnvSet` that activates it; owns the runtime manifest (`runtime.json`),
  its backend sources (`mock`/`local`/`build`), and structural validation.
- **`ost-build`** decides *what* to build (target id, ABI) and renders the files
  CMake needs (`toolchain.cmake`, presets); also packaging and the Windows MSVC
  bootstrap. It does not replace CMake/Ninja.
- **`ost-extension`** owns the certified extension catalog and the capability
  resolver (capability → providing extension + feature, with transitive pulls).
- **`ost-plugin`** models the OpenUSD plugin *bundle* (`openstrata.plugin.yaml`),
  scaffolds new ones, and runs the verification pyramid (static L0–L1 + executed
  L2–L5 behind a `Probe` seam) into reports.
- **`ost-artifact`** owns the artifact registry: identity records, the local
  content-addressed store, integrity verification, and the `ArtifactTransport`
  seam with a filesystem adapter and a read/write OCI adapter (GHCR-class
  registries) for `ost artifact pull`/`push`.
- **`ost-ci`** owns the CI support matrix (`openstrata.ci.yaml`) — runner
  profiles, lanes, and digest-pinned runtime×plugin support lines — and renders
  it into GitHub Actions workflows (`ost ci plan | validate | generate github`).
- **`ost-manifest`** owns the human-authored `openstrata.toml` and the generated
  `strata.lock` (now populated: runtime digest, variant, Python ABI, validation).
- **`ost-cli`** only parses arguments, calls the libraries, and renders results
  (human or `--json`). It never embeds domain rules.

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
