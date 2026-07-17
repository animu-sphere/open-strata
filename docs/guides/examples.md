# Command examples

A practical, copy-pasteable tour of the `ost` CLI. Every command also accepts
`--json` for machine-readable output and uses deterministic, category-based exit
codes, so the same invocations drive both interactive use and CI. Under `--json`
each command prints a single `{ok, schema, data, warnings}` envelope on stdout
(failures carry `error.code` / `error.category` instead of `data`). The full
contract — envelope, codes, exit codes, and compatibility policy — is in
[json-schema.md](../reference/json-output.md):

```bash
ost platform show cy2099 --json   # {"ok":false,"schema":1,"error":{"code":"PLATFORM_NOT_FOUND",…}}
echo $?                           # 2  (exit: 2 usage·3 config·4 precondition·5 validation·6 external_tool·7 io·70 internal)
```

In the examples `ost` is the built binary. To run from a checkout instead:

```bash
ost() { cargo run -q -p ost-cli -- "$@"; }
```

State lives under `~/.ost` (override with `OST_HOME`). See
[architecture.md](../architecture/overview.md#on-disk-layout) for the on-disk layout.

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
ost init --template renderer --name my-renderer # renderer skeleton + headless evidence
ost init --template usd-plugin                # scaffold a USD plugin project
ost init --template usd-plugin-workspace      # root for one repo with many plugin bundles
ost init --name my-show --platform cy2026     # explicit name + platform
ost init --bare                               # manifest only — adopt an existing CMake project
ost init --force                              # overwrite an existing manifest / template files
```

Writes `openstrata.toml`, a `.strata/` state directory, and (unless `--bare`) a
minimal, buildable CMake project — so `ost build` works straight after
`ost runtime pull`.

## runtime — manage runtimes in the store

`runtime pull` materializes a runtime from a backend **source**. Pick the source
by flag; precedence is artifact > build > adopt > mock.

```bash
# mock — placeholder layout, no real OpenUSD (works offline, for the static checks)
ost runtime pull cy2026 --profile usd

# local / adopt — register an existing OpenUSD install in place (fastest real USD)
ost runtime pull cy2026 --profile usd --from-usd /opt/usd
OST_USD_ROOT=/opt/usd ost runtime pull cy2026 --profile usd

# build (build_usd.py) — build OpenUSD + deps from source into the store
ost runtime pull cy2026 --profile usd --build /src/OpenUSD --jobs 8
ost runtime pull cy2026 --profile usd --build /src/OpenUSD --build-arg --no-imaging
# macOS + CMake 4 bundled deps: retry with --build-arg -DCMAKE_POLICY_VERSION_MINIMUM=3.5

# build (CMake-direct) — build OpenUSD against pre-provided dependency prefixes
ost runtime pull cy2026 --profile usd \
  --build /src/OpenUSD --deps /opt/usd-deps \
  --build-arg -DPXR_BUILD_IMAGING=OFF

# artifact — materialize a prebuilt runtime from the local artifact registry
# (an `ost runtime export`ed runtime; see the artifact section below)
ost runtime pull cy2026 --profile usd --from-artifact sha256:3fa9c1d2…
```

### Linux `--build` prerequisites

`--build` runs OpenUSD's own `build_scripts/build_usd.py`, which fetches and
compiles the dependency tree itself. On a fresh Linux host (this list is from a
WSL/Ubuntu-26.04 dogfood) it needs, beyond a C/C++ toolchain, CMake and Ninja:

```bash
# Debian/Ubuntu — dev packages build_usd.py's deps link against
sudo apt install -y build-essential cmake ninja-build git unzip \
  libx11-dev libxt-dev libgl1-mesa-dev libglu1-mesa-dev   # libxt-dev: MaterialX GLSL render
```

- **A shared-library Python 3.13.** `build_usd.py` links usdview/schema tooling
  against `libpython`, so the interpreter must be built `--enable-shared`. On
  Ubuntu there is no `python3.13` apt package even on very new releases; build
  CPython from source with `./configure --enable-shared` (and put its `lib` on
  `LD_LIBRARY_PATH`). `ost` preflights the *Python packages* the profile implies
  (Jinja2 for schema generation; PySide6 + PyOpenGL for usdview) and prints the
  exact `pip install` fix before `build_usd.py` runs — install them into that
  interpreter first.
- **Portable-runtime note (glibc floor).** A Linux runtime you intend to
  `ost runtime export` and ship should be built against an *old* glibc base:
  `ost` measures the real glibc floor from the packed ELF binaries and stamps it
  onto the artifact target (e.g. `glibc243`), and `--require-target` rejects a
  consumer whose host glibc is older. Building on a bleeding-edge distro bakes in
  a high floor that narrows where the artifact can run.
- **Known upstream downloader flake.** `build_usd.py`'s fetch is upstream code
  `ost` shells out to, not `ost`'s own (digest-verified) transport — so `ost`
  cannot repair it. If a dependency download returns an HTML error page (a
  `github.com/.../vX.Y.Z.zip` 504 has been seen) `build_usd.py` fails with an
  opaque *"unrecognized archive file type"*. Work around it by pre-seeding that
  archive from `codeload.github.com/<owner>/<repo>/zip/refs/tags/<tag>` (or a
  mirror) into build_usd.py's dependency source/download directory and retrying.

A pulled **real, validated** runtime can go the other way — into the registry —
so other machines fetch it by digest instead of rebuilding:

```bash
ost runtime validate cy2026 --profile usd   # export requires a passed validation
ost runtime export   cy2026 --profile usd   # pack + register; prints the digest
ost runtime export   cy2026 --profile usd --slim   # SDK layout only (much smaller)
# --slim keeps include/lib/bin/plugin/cmake/libraries/resources + CMake config
# and drops the source/build tree of a runtime adopted from a full USD build
# (e.g. 1.93 GiB -> ~27 MiB); the slim artifact still builds and runs plugins.
# (resources/ is retained because MaterialXConfig.cmake set_and_checks it at
# find_package(pxr) time — dropping it broke slim MaterialX runtimes.)
# packing is multithreaded by default and prints progress to stderr; tune it with:
ost runtime export   cy2026 --profile usd --jobs 8    # zstd worker threads (default: host CPUs)
ost runtime export   cy2026 --profile usd --level 12  # faster pack, larger archive (default 19)
# refused for: a mock runtime, external --deps prefixes (they would not travel),
# or a runtime that has not passed validation
```

```bash
ost runtime list                          # what's in the store (+ SOURCE column)
ost runtime show cy2026 --profile usd      # manifest: source, prefix, deps, digest
ost runtime validate cy2026 --profile usd  # schema/digest/layout (+ usdcat/pxr if real)
ost runtime explain cy2026 --profile lookdev   # capability -> extension graph
ost runtime pull cy2026 --profile usd --force  # re-pull / rebuild

# When an adopted install moved underneath its manifest (`show`/`validate`
# report openusd-version-drift), one step re-adopts from the recorded USD root:
ost runtime repair cy2026 --profile usd
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
ost build --progress json              # one JSON event per line (a stable stream for tools)
ost build --quiet                      # silence progress; output to .strata/targets/<id>/build.log
ost build --notify                     # desktop toast on finish (no-op over SSH / in CI)
ost package                            # install + dist/<name>/<ver>/<target>/*.tar.zst
ost package --allow-empty              # permit a metadata-only artifact (empty install tree)
ost package --clean-stage              # reclaim a stuck stage + sweep stale fallback stages
ost validate                           # configured / built / runtime / artifact checks
```

### renderer — inspect a Hydra adapter in usdview

The renderer template's default build is host-neutral. Pull or adopt one real
OpenUSD imaging/usdview runtime, then let the view command drive the optional
Hydra adapter through the shared build service and compose the interactive
session:

```bash
ost runtime pull cy2026 --profile lookdev --from-usd <openusd-root>
ost renderer view                       # managed incremental build + smoke scene
ost renderer view scenes/shot.usda      # project-relative or absolute scene
ost renderer view --config Debug --generator Ninja --profile lookdev
```

The view command records the selected runtime and `hydra2` build intent,
refreshes a private install tree, discovers the generated `HdRendererPlugin`
metadata there, selects its display name in usdview, and overlays the runtime
environment only on the child process. It does not mutate the current shell or
imply that the co-built Hydra adapter is a standalone OST plugin bundle.

Use `--build-dir other-build` for an external/prebuilt CMake tree. That mode is
validated and labeled as manual evidence; OST installs it but does not rebuild
it or claim `ost build` completion.

### renderer — launch the standalone viewport

The optional viewport adapter presents the project bootstrap draw in a native
GLFW window and needs no OpenUSD runtime — the project's host-neutral profile
is enough:

```bash
ost runtime pull cy2026 --profile core
ost renderer viewport                          # managed build + native window
ost renderer viewport -- --frames 8 --hidden   # smoke run; args pass through
```

The command records the `viewport` build intent, builds through the same
managed service, and launches the executable mapped under
`composition.adapters.viewport`. Exit code 77 from the viewport reports an
environment that cannot present (no display or Vulkan device) as an explicit
precondition rather than a failure.

Adopt an existing renderer without overwriting its CMake or source tree. The
first command is a read-only plan; add `--write` only after reviewing target
mappings. `--version-file` keeps an existing release version source
authoritative instead of copying it into `openstrata.toml`:

```bash
ost renderer adopt --name merlin --platform cy2026 \
  --core merlin-core --extraction merlin-extraction \
  --backend vulkan=merlin-vulkan --headless merlin-headless \
  --hydra2 hdMerlin --version-file VERSION
ost renderer adopt --name merlin --platform cy2026 \
  --core merlin-core --extraction merlin-extraction \
  --backend vulkan=merlin-vulkan --headless merlin-headless \
  --hydra2 hdMerlin --version-file VERSION --write
```

An existing standalone viewport executable joins the adapter map with
`--viewport <target>`, which enables `ost renderer viewport` for the adopted
project.

Independently produced renderer reports merge only under explicit conflict
rules: duplicate assertion ids need `--replace`, and an existing FAIL cannot be
downgraded.

```bash
ost renderer merge --base cpu.json --overlay gpu.json --out combined.json
```

If a previous run's stage tree is briefly held open (a scanner, or another
`ost`), packaging stages into a fresh `stage-<hex>` sibling and warns
(`STAGE_FALLBACK`) instead of failing. The warning names how many stale
siblings are still locked; once the holding process exits, rerun with
`--clean-stage` to reclaim the stable stage name and sweep the leftovers
(reported as `STAGE_CLEANED`). Same flag on `ost plugin package`.

On Windows, `ost build` auto-loads the MSVC developer environment
(`vcvars64.bat`) unless `--no-vcvars` is given.

With `--progress json`, each line is one event; child build output is kept off
stdout (in the log) so the stream stays pure:

```json
{"event":"phase_started","phase":"configuring-cmake","index":2,"total":4,"timestamp":1782541330}
{"event":"heartbeat","phase":"building-targets","elapsed_ms":120000,"last_output_ms":90000,"timestamp":1782541450}
{"event":"phase_completed","phase":"building-targets","duration_ms":240000,"timestamp":1782541570}
{"event":"phase_failed","phase":"building-targets","exit_code":2,"duration_ms":202,"log":"…/build.log","timestamp":1782541371}
{"event":"completed","duration_ms":2573,"timestamp":1782541332}
```

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

# scaffold OpenExec computations against an independently versioned schema contract
ost plugin new usd-exec pose-eval \
  --schema-bundle rig-schema --schema-type RigContractAPI

ost plugin inspect toy                 # Level 0 structure (human + --json)
ost plugin build toy --target cy2026 --profile usd       # build the .so via ost-build
ost plugin build toy --dry-run         # show the cmake commands only

# static diagnostics (L0–L1) + session-env preview; writes .strata/reports/...
ost plugin doctor toy --target cy2026 --profile usd
ost plugin doctor toy --with ./plugins/other --target cy2026 --profile usd

# full pyramid L0..L5 against a real runtime; writes a report
ost plugin test toy --target cy2026 --profile usd
ost plugin test toy --up-to 3          # stop after usdcat read
ost plugin test toy --json
ost plugin test --workspace --up-to 1  # graph preflight, then every discovered bundle

# launch any command inside the composed runtime session (real runtime)
ost plugin run toy --target cy2026 --profile usd -- usdcat tests/fixtures/basic.toy
ost plugin run toy --with ./plugins/other --target cy2026 --profile usd -- usdcat tests/fixtures/basic.toy

# Level 6: open a fixture in usdview, or verify it launches (needs usdview + display)
ost plugin view      toy tests/fixtures/basic.toy --target cy2026 --profile usd
ost plugin test-view toy tests/fixtures/basic.toy --target cy2026 --profile usd
ost plugin test toy --up-to 6 --target cy2026 --profile usd   # full pyramid incl. L6

# co-locate a USD schema in the existing bundle: scaffolds schema/schema.usda
# and wires the manifest (provides: usd-schema:ToyAPI + schema.source); the next
# `ost plugin build` runs usdGenSchema, links the generated C++ into the same
# plugin library, and merges the schema Types into plugInfo.json
ost plugin schema add toy
ost plugin schema add toy --class MetadataAPI       # public type ToyMetadataAPI
ost plugin schema add toy --codeless                # resource-only (no C++)

# package a built plugin as a target-specific binary bundle artifact
ost plugin package toy --target cy2026 --profile usd

# publish the packaged artifact into the local registry, addressed by digest.
# Refused (exit 5, stable per-cause codes) unless the package records a passed
# validation, complete runtime provenance, a concrete frozen cxx_abi, a license,
# and every declared notices file.
ost plugin publish toy --target cy2026 --profile usd
```

Reports land under `<bundle>/.strata/reports/<plugin>/<UTC>/`
(`report.json`, `summary.txt`, `environment.json`); see the
[plugin-report schema](../../schemas/plugin-report.schema.json).
Plugin package artifacts land under
`<bundle>/dist/plugins/<name>/<version>/<target>/` with a `tar.zst`,
`manifest.json`, `SHA256SUMS`, `sbom.spdx.json`, and—when complete build
metadata is available—`provenance.intoto.jsonl`. See
[artifact evidence](../reference/artifact-evidence.md).

### Golden USDA line endings

Keep committed USDA fixtures and goldens at LF even on Windows:

```gitattributes
*.usda text eol=lf
```

This is semantic for triple-quoted USDA strings. A CRLF checkout can put a
literal carriage return into the parsed string value; `usdcat --flatten` then
preserves it and L5 correctly reports a real golden mismatch. Do not fix that by
normalizing carriage returns inside string values. Re-normalize the working tree
after adding the attribute, regenerate the golden only if authored data changed,
and inspect the flattened diff before accepting it. OST captures the flatten
payload with `usdcat --out` rather than stdout, so a remaining semantic-CRLF
diagnostic is authored/generated tool output, not Windows pipe translation.

## artifact — the local digest-addressed registry

Runtimes, plugin bundles, and project packages become registry **artifacts**:
`tar.zst` + producer `manifest.json` + checksums, addressed by the archive's
SHA-256 digest (never by mutable name). The store lives under
`~/.ost/artifacts` (`OST_HOME`-aware). Runtimes enter via `ost runtime export`
and come back out via `ost runtime pull --from-artifact`; plugin bundles enter
via `ost plugin publish`.

```bash
# register a package output (a dist dir, or its manifest.json directly);
# the archive is re-hashed and a digest/size mismatch is refused (exit 5)
ost artifact import toy/dist/plugins/toy/0.1.0/<target>/
ost artifact import dist/my-show/1.0.0/<target>/manifest.json

ost artifact list                       # everything in the registry
ost artifact list --kind plugin         # filter: runtime | plugin | package

# digests resolve in full or by unique hex prefix (>= 6 chars)
ost artifact show sha256:3fa9c1d2…
ost artifact show 3fa9c1

# integrity: recompute the archive digest AND re-hash every tar entry
# against the producer manifest's files[] (exit 5 on any mismatch)
ost artifact verify 3fa9c1
ost artifact verify 3fa9c1 --minimum-trust verified \
  --require-sbom --require-provenance \
  --policy openstrata-artifact-policy.toml

# CI handoff: copy archive + manifest + SHA256SUMS + record out;
# the exported directory is re-importable on another machine
ost artifact export 3fa9c1 ./handoff/

# unpack an artifact's archive (digest re-verified first) — e.g. a plugin
# bundle under test in a CI matrix cell
ost artifact extract 3fa9c1 ./plugin-under-test/
```

### Publishing to and pulling from a remote OCI registry

A stored artifact publishes to any OCI registry (GHCR, Harbor, ECR, GAR, ACR,
`registry:2`). `push` emits the exact layout `pull` consumes, prints the
**immutable OCI manifest digest**, and is content-addressed + idempotent
(re-pushing the same bytes transfers nothing):

```bash
# publish a stored runtime by digest; the OCI digest it prints is what a
# support line's runtime_remote.expected_oci_digest pins
ost artifact push 3fa9c1 oci://ghcr.io/<owner>/openstrata-runtime:usd-26.05-linux-x86_64
ost artifact push 3fa9c1 oci://ghcr.io/<owner>/openstrata-runtime --json   # every digest as data

# pull one back anywhere: resolve the tag to a digest, then pull the pin
ost artifact resolve oci://ghcr.io/<owner>/openstrata-runtime:usd-26.05-linux-x86_64
ost artifact pull    oci://ghcr.io/<owner>/openstrata-runtime@sha256:<oci-digest>
```

When `openstrata-artifact-policy.toml` protects the destination, `push`
auto-discovers it from the current directory or a parent and verifies the
GitHub Actions OIDC publisher before contacting the registry. CI jobs need
`id-token: write`; use `--policy <FILE>` when running outside the project tree.
`--allow-untrusted-publisher` is an explicit, output-recorded break-glass
override.

Credentials and visibility (the two footguns standing up a first publish):

- **`ost` does not read the `oras`/docker credential store.** For a private
  registry — including pushing to a *new* GHCR package, which is private until
  you flip it — set `OST_REGISTRY_USER` + `OST_REGISTRY_PASSWORD` (a GitHub PAT
  with `write:packages` for GHCR). Without credentials a private pull/resolve
  returns `ARTIFACT_AUTH_DENIED` even after `oras login`.
- **Push needs the credential path, not `OST_REGISTRY_TOKEN`.** `push` runs the
  registry token exchange (`OST_REGISTRY_USER` + `OST_REGISTRY_PASSWORD`) to
  request `pull,push` scope. `OST_REGISTRY_TOKEN` is a pull-only convenience: a
  bearer presented verbatim is accepted for reads but cannot carry push scope on
  GHCR-class registries, so a push with it authenticates and then fails with a
  403 → `ARTIFACT_AUTH_DENIED`. If a push is refused, the hint names which fix
  applies: `static-token` → **unset `OST_REGISTRY_TOKEN`** (while it is set `ost`
  prefers it and never runs the exchange) and set user/password; a rejected
  credential → confirm the PAT has `write:packages` and may publish to that
  package.
- **GHCR package visibility is a WebUI-only flip.** A freshly pushed package is
  private; make it public under the package's *Package settings → Change
  visibility* (not settable via the packages API, and gated by the org's
  "Package creation" policy). Once public, the CI pull path works **fully
  anonymously** — no token on the runner.
- Fixture registries / air-gapped mirrors that speak plain HTTP take
  `--plain-http` on `push`/`pull`/`resolve`.

## ci — the runtime×plugin support matrix

Support cells are explicit claims — *this* runtime artifact × *this* plugin
artifact × *this* platform/profile, verified up to a level — pinned by **full**
registry digest in `openstrata.ci.yaml`. Generators render that one file into
CI configuration (GitHub Actions today, Jenkins later).

```bash
ost ci init                       # scaffold openstrata.ci.yaml (commented starter)
# …publish artifacts (ost runtime export / ost plugin publish), pin the digests…

ost ci validate                   # structural checks (schema, names, digests, levels)
ost ci validate --resolve         # + every pinned digest must exist in the local registry
ost ci validate --support support/platforms.toml

ost ci generate github            # write .github/workflows/ost-support-matrix.yml
ost ci generate github --support support/platforms.toml  # gate public claims first
ost ci generate github --stdout   # print instead (inspect / pipe)
ost ci generate github --force    # regenerate over the existing workflow
```

The generated workflow is scheduled/dispatch CI (PR CI should keep its cheap
static checks): one job per cell (`fail-fast: false`), which re-verifies both
artifacts, materializes the runtime (`runtime pull --from-artifact`), extracts
the plugin (`artifact extract`), runs `ost plugin test --up-to <level>`, and
uploads the report. Runners need `ost` on PATH and the pinned artifacts in
their `OST_HOME` registry — self-hosted labels are the expected case.

Cells can also opt into source CI with `lane: pull_request` or `lane: main`.
Those jobs render to `.github/workflows/ost-source-ci.yml`, check out the repo,
materialize the pinned runtime SDK, build/test/package the bundle from source,
and never publish or use secrets. Keep repo-specific post-build smoke coverage
in the generated workflow with matrix-level `source_checks`:

```yaml
trust:
  policy: openstrata-artifact-policy.toml
  pr_min_trust: unsigned
  main_min_trust: attested
  release_min_trust: verified

cells:
  - name: plugin-pr-linux
    lane: pull_request
    trust: unsigned              # target floor; lane floor may be stricter
    support:
      platform: linux_x86_64     # id from support/platforms.toml
      features: [plugin_build, plugin_test]
    # runtime_artifact, runtime_remote, bundle, platform, profile, runner, ...
```

Generated jobs use the stricter of a cell's target `trust` and its lane floor.
Every consumed runtime/plugin artifact is verified with that minimum plus
required SBOM and provenance; `trust.policy` additionally gates the provenance
builder identity. PR and ordinary source-CI jobs remain publish-free and retain
read-only repository permissions.

Repo-specific checks remain separate from trust policy:

```yaml
source_checks:
  - name: Run corpus CTest smoke
    run: |
      set -euo pipefail
      ctest --test-dir build/corpus --output-on-failure
```

`source_checks` are deliberately **post-pyramid**: they run after
`ost plugin test`, with the built plugin present. They are the wrong place for
anything the *build* depends on. Build prerequisites are modeled separately and
render in a first-class section between runtime materialization and
`ost plugin build`.

### Generated trusted releases

A release lane is not a source-CI step with credentials added. It is a separate,
tag-triggered workflow generated from a typed `release:` block. Candidate cells
must use `lane: main`, opt in with `publish: candidate`, meet
`release_min_trust` (at least `verified`), and use exact per-target
`bootstrap.ost.sha256` pins on hosted runners.

```yaml
trust:
  policy: openstrata-artifact-policy.toml
  main_min_trust: verified
  release_min_trust: trusted

bootstrap:
  ost:
    version: "0.16.0"
    sha256:
      x86_64-unknown-linux-musl: <64-hex release-asset checksum>

release:
  version: 1.2.3                 # only tag v1.2.3 is accepted
  mode: publish                  # draft stops after verified handoffs
  destination: oci://ghcr.io/example/my-plugin
  publisher_runner: linux-hosted # key under runners:
  environment: release
  reproducible: true
  from_package: true
  checks:
    - name: Run release corpus smoke
      run: ctest --test-dir build/corpus --output-on-failure

cells:
  - name: linux-release
    lane: main
    publish: candidate
    trust: trusted
    runner: linux-hosted
    bundle: plugins/myPlugin
    # runtime_artifact, runtime_remote, platform, profile, up_to, ...
```

`ost ci generate github` adds `.github/workflows/ost-release.yml`. Its read-only
ref gate requires the exact `v<release.version>` tag. Each candidate job checks
the bundle version, materializes and verifies the pinned runtime, builds/tests,
runs the declared checks, packages twice when `reproducible: true`, exercises the
clean archive when `from_package: true`, and uploads an immutable artifact
handoff containing checksums, SBOM, and provenance.

Only the final publisher job has `id-token: write`, `packages: write`, and the
registry credential. It downloads each candidate into a fresh local store,
re-verifies the artifact and its provenance against `trust.policy`, then pushes
to `<destination>:<version>-<cell>`. A fresh import remains `local` in the stored
record; verification derives a non-sticky effective trust from subject-bound
provenance, a valid SBOM, and the matched publisher policy. This keeps handoff
trust reproducible without treating an imported `record.json` as authority.

### Hosted source CI: the runtime/toolchain contract (macOS + Windows)

A GitHub-hosted source cell (e.g. `macos-15-arm64`, `windows-2022`) pulls the
runtime SDK by digest onto a clean runner, so two assumptions that hold on a
developer's machine must become explicit — otherwise the lane only passes with
hand-added `chmod`/`setup-python` repairs that the next `ost ci generate github`
silently drops:

1. **Runnable runtime tools.** `ost runtime export` / artifact packaging preserve
   Unix execute bits (`bin/` tools pack as `0o755`, everything else stays the
   canonical `0o644`), extraction restores them, and `ost runtime validate` fails
   any runtime whose top-level `bin/` tools are not executable. Generated source
   CI runs `ost runtime validate` right after materialization, so a runtime that
   lost its execute bits fails there — with `.ost-ci/runtime-validate.json`
   evidence — instead of deep inside `usdGenSchema`.

2. **Schema-tooling Python.** If the pinned runtime does not bundle a runnable
   interpreter under `bin/` but its profile needs `usdGenSchema`, declare the
   CPython ABI the tooling expects with `host_python` on the source cell. On a
   hosted runner the generator installs exactly that Python (a SHA-pinned
   `actions/setup-python`) before `ost plugin build`, and records the resolved
   Python source in `.ost-ci/python-setup.json`. Locally, `ost plugin build`
   fails before `usdGenSchema` with a precondition naming every interpreter it
   searched when none is runnable.

```yaml
cells:
  - name: mac-usd-pr
    lane: pull_request
    runner: mac-hosted            # kind: github-hosted, image: macos-15-arm64
    host_python: "3.13"           # runtime bundles no bin/python3.13
    bundle: plugins/usdVrm
    runtime_artifact: sha256:<runtime SDK digest>
    runtime_remote:
      uri: oci://ghcr.io/<owner>/<runtime-repo>@sha256:<oci-digest>
      expected_oci_digest: sha256:<oci-digest>
    platform: cy2026
    profile: usd
    up_to: 5
```

Self-hosted runners keep their operator-provisioned Python, so the `setup-python`
step is gated on `matrix.hosted` and skips there. Omit `host_python` entirely
when the runtime ships its own interpreter.

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
ost plugin package toy --target cy2026 --profile usd
```

## Tool overrides

For tools not on `PATH`: `OST_NINJA` (ninja), `OST_UV` (uv). Runtime-source env
fallbacks: `OST_USD_ROOT`, `OST_USD_SRC`, `OST_USD_DEPS`. Store location:
`OST_HOME` (defaults to `~/.ost`).
