# Crates

*Last verified against: v0.12.0 (workspace version 0.12.0).*

OpenStrata is a Rust workspace. The CLI is thin; domain logic lives in libraries
so it can be reused by future surfaces (CI helpers, a daemon, tests). The
authoritative crate list is the `members` array in the root
[`Cargo.toml`](../../Cargo.toml); this document must stay in sync with it.

## Workspace members

| Crate | Responsibility |
| --- | --- |
| `ost-cli` | The `ost` binary: argument parsing + human/JSON rendering. Embeds no domain rules. |
| `ost-core` | Shared primitives: catalog loader, paths, host, variant, digest, tools, errors. |
| `ost-platform` | VFX Reference Platform calendar-year model, loader, diff. |
| `ost-manifest` | Project (`openstrata.toml`) + lock (`strata.lock`) models. |
| `ost-runtime` | Runtime identity, profiles, env generation, runtime manifest + validation. |
| `ost-build` | Build target model, toolchain/preset generation, packaging, MSVC bootstrap. |
| `ost-extension` | Controlled extensions: model, loader, capability resolver. |
| `ost-plugin` | OpenUSD plugin bundles: model, scaffold, verification levels, reports. |
| `ost-artifact` | Artifact registry: identity records, content-addressed store, verification, OCI transport. |
| `ost-ci` | CI support matrix (`openstrata.ci.yaml`) + workflow generation (GitHub Actions). |
| `ost-formation` | Digest-pinned Formation manifest, compatibility resolution, portable environment, and lock model. |

## Crate boundaries

- **`ost-core`** holds vocabulary only — no domain logic. It defines where things
  live (`paths`), what machine we are on (`host`), how a build/runtime variant is
  identified (`variant`), and the shared `Error` type.
- **`ost-platform`** owns the platform manifest: model, a loader that embeds the
  built-in CY manifests and overlays user manifests, and a structured diff.
- **`ost-manifest`** owns the human-authored `openstrata.toml` and the generated
  `strata.lock` (runtime digest, variant, Python ABI, validation).
- **`ost-runtime`** turns a platform + profile into a concrete runtime identity
  and the `EnvSet` that activates it; owns the runtime manifest (`runtime.json`),
  its backend sources (`mock`/`local`/`build`/`artifact`), and structural
  validation.
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
  profiles, lanes, and digest-pinned runtime×plugin support lines — and renders it
  into GitHub Actions workflows (`ost ci plan | validate | generate github`).
- **`ost-formation`** owns strict Formation declarations, the deterministic
  resolved/lock model, artifact/runtime/plugin compatibility checks, and portable
  environment contributions. Materialization and foreground process launch stay
  at the CLI boundary.
- **`ost-cli`** only parses arguments, calls the libraries, and renders results
  (human or `--json`). It never embeds domain rules.

## Planned crates

Not yet created; introduced as their phase lands, not up front:

- `ost-solver`, `ost-session`, `ost-validation`.

(`ost-artifact` and `ost-ci` were on this list historically; both now exist and
are shipped, listed above. `ost-execution` and `ost-host` are proposed for the
Kubernetes and DCC-host phases — see [roadmap/backlog.md](../roadmap/backlog.md).)
