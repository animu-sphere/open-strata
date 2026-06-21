# Architecture

## Workspace layout

OpenStrata is a Rust workspace. The CLI is thin; domain logic lives in libraries so
it can be reused by future surfaces (CI helpers, a daemon, tests).

```text
crates/
  ost-cli/        the `ost` binary: argument parsing + human/JSON rendering
  ost-core/       shared primitives: paths, host, variant, errors
  ost-platform/   VFX Reference Platform CY model, loader, diff
  ost-manifest/   project (openstrata.toml) + lock (strata.lock) models
platforms/        built-in CY manifests, embedded into the binary
schemas/          JSON schemas for platform / project / lock documents
docs/             this documentation
```

Planned crates from the design (not yet created): `ost-solver`, `ost-runtime`,
`ost-build`, `ost-extension`, `ost-plugin`, `ost-session`, `ost-validation`,
`ost-ci`. They are introduced as their phase lands, not up front.

## Crate boundaries

- **`ost-core`** holds vocabulary only — no domain logic. It defines where things
  live (`paths`), what machine we are on (`host`), how a build/runtime variant is
  identified (`variant`), and the shared `Error` type.
- **`ost-platform`** owns the platform manifest: model, a loader that embeds the
  built-in CY manifests and overlays user manifests, and a structured diff.
- **`ost-manifest`** owns the human-authored `openstrata.toml` and the generated
  `strata.lock`. The lock model exists now; later phases populate it.
- **`ost-cli`** only parses arguments, calls the libraries, and renders results
  (human or `--json`). It never embeds domain rules.

## Domain model

| Concept | Meaning | Status |
| --- | --- | --- |
| **Platform** | A VFX Reference Platform calendar year as a machine-readable manifest (`cy2026`). | implemented |
| **Variant** | Concrete artifact identity: OS + arch + ABI + Python ABI, e.g. `linux-x86_64-glibc228-py313`. | implemented |
| **Project** | `openstrata.toml`: the runtime contract a project builds against (platform, profile, capabilities, extensions). | implemented |
| **Lock** | `strata.lock`: pinned runtime digest, variant, Python ABI, validation status. | model only |
| **Profile** | A named bundle of capabilities (`core`, `usd`, `lookdev`, …). | planned |
| **Capability** | A logical feature requested/provided (`usd-materialx`). | planned |
| **Extension** | A controlled VFX-adjacent component (OpenUSD, MaterialX, a USD plugin). | planned |
| **Runtime** | Platform + variant + profile + resolved artifacts, with a digest. | planned |
| **Session** | A mutable workspace over an immutable runtime. | planned |

## On-disk layout

Two roots matter:

```text
~/.ost/                      # user store (override with OST_HOME)
  config.toml
  platforms/                 # user manifests, overlaid over built-ins
  runtimes/ extensions/ artifacts/ cache/ sessions/ logs/

<project>/
  openstrata.toml            # authored project manifest
  .strata/                   # generated state (gitignored)
    targets/<target>/        # toolchain.cmake, env.json, validation.json, *.lock
```

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

The repo currently builds on Rust 1.69. Because several modern crates require a
newer compiler or edition 2024, top-level dependency versions are pinned in
`Cargo.toml` and transitive versions are pinned in `Cargo.lock` to keep the whole
tree on edition ≤ 2021. Bump these deliberately alongside a `rust-version` bump.
