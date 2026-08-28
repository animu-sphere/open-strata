# Adopting a plugin workspace

This guide is for a repository that contains **several OpenUSD plugin bundles
developed together** — schemas, file formats, resolvers, and shared libraries —
and wants OpenStrata to discover, validate, test, and package them as one
dependency-ordered workspace. It is transferable to any such repository. The
[USD 3DGS Plugins](../projects/usd-3dgs-plugins.md) single-bundle-plus-library
workspace, the [USD Point Cloud Plugins](../projects/usd-pointcloud-plugins.md)
multi-format workspace, and the [USD VRM Plugins](../projects/usd-vrm-plugins.md)
multi-bundle workspace are complementary worked examples, not required layouts.

The factual contract behind everything here is
[reference/plugin-workspace.md](../reference/plugin-workspace.md); this page is
the procedure.

## 1. Lay out the workspace

Place each bundle in its own directory with a plugin manifest. Ordinary CMake
libraries the bundles link live alongside them. A typical shape:

```text
plugins/
  vrmSchema/            # USD schema bundle
  usdVrmFileFormat/     # SdfFileFormat bundle
  usdVrmPackageResolver/# ArPackageResolver bundle
vrmContainer/           # ordinary CMake library the bundles link
```

You do **not** restructure your project into an OpenStrata package abstraction.
OpenStrata adopts your existing CMake target boundaries.

Declare those roots in `openstrata.toml`; globs can express nested layouts
without hard-coding every leaf:

```toml
[workspace]
members = ["vrmContainer", "plugins/*", "adapters/*/*", "tools/*"]
```

Use `"."` when the project root itself carries a library, plugin, or tool
descriptor. The list is authoritative: `ost` reports a descriptor found outside
it, an empty pattern, or a selected directory without exactly one member
descriptor. Projects without the table retain bounded recursive discovery below
the root for compatibility; use an explicit `"."` to opt the root descriptor
in. An explicit list makes review and CI graph coverage clear.

## 2. Declare dependencies between bundles

In each bundle manifest, declare what it needs from other bundles and libraries.
Separate link-time dependencies from runtime-only ones so a consumer sees the real
closure:

```yaml
requires:
  bundles:
    - vrmSchema          # this bundle depends on the schema bundle
  libraries:
    - vrmContainer       # ordinary CMake library dependency
```

Declared bundle dependencies define the workspace graph OpenStrata tests in order.

## 3. Validate the graph, then test every bundle

```sh
ost plugin test --workspace --graph-only
```

This validates discovery and the dependency graph without requiring a runtime or
build. A cycle, missing provider, omitted descriptor, or unreadable member fails
fast. Then run every bundle in dependency order, scoping the pyramid with
`--up-to <level>` while iterating:

```sh
ost plugin test --workspace             # full configured pyramid
ost plugin test --workspace --up-to 1   # graph + cheap levels only
```

Individual bundles still build and test on their own — the workspace does not
replace `ost plugin build <bundle>` / `ost plugin test <bundle>`.

## 4. Package and re-validate on a clean install

Package a bundle to an immutable, digest-addressed artifact, then prove the
packaged output — not the build tree — still discovers and opens:

```sh
ost plugin package plugins/usdVrmFileFormat
ost plugin test    plugins/usdVrmFileFormat --from-package
```

`--from-package` extracts the artifact to a clean directory and runs discovery /
open / validate against it, catching a build-tree path baked into
`plugInfo`/`LibraryPath` that source-tree testing cannot see.

## 4b. Ship a workspace-built executable

A CLI tool your workspace builds is a user-facing deliverable that no bundle
requires, so nothing in the dependency graph reaches it. Describe it with an
`openstrata.tool.yaml` beside its CMake project (v0.21.0):

```yaml
schema: openstrata.tool/v1alpha1
tool:
  id: motion_retarget
  version: 0.4.0
  license: Apache-2.0
executables: [motion_retarget]   # no platform extension; packaging adds it
directories: [bin]               # defaults to [bin, lib]
```

`ost plugin package --workspace` then packages it after the bundles, with the
same dist shape a bundle package has (archive, `manifest.json`, `SHA256SUMS`,
SBOM), and `--product` composes it into the aggregate as a `tool` member.
`ost plugin product install` puts it under `tools/<id>/` and joins its
directories to the aggregate loader path.

A root `ost build` may leave `add_executable()` output in its target build tree
instead of the source member. OST snapshots matching files before CMake runs;
after success it selects only executables created or changed by that invocation,
using the member path and build configuration (or a globally unique filename),
then stages the selected set transactionally below the descriptor's first
`directories` entry. Stale and ambiguous matches are reported and never
guessed. The root completion records the staged digest, so a subsequent
workspace package proves it consumed the exact managed bytes.

Packaging fails if a declared executable is not there, so a release cannot ship
a tool package with no tool in it. `ost build` records the executables it
produced, so `plugin package` reports the same `matched` / `untracked` /
`mismatched` provenance it reports for a bundle.

## 5. Pin a runtime and generate CI

Pin the OpenUSD runtime your cells build against by digest, then generate the
support-matrix workflow instead of hand-maintaining it:

```sh
ost ci validate                 # check the openstrata.ci.yaml matrix
ost ci generate github          # render the runtime × bundle workflow
```

Pinning a `runtime_artifact` by digest keeps every cell reproducible. If a cell
pins a runtime that lacks the evidence a generated gate demands, `ost ci
generate` warns and `ost ci validate` fails fast (v0.18.0).

A digest pins the runtime bytes, but it does not state which OpenUSD variant
those bytes must provide. OpenUSD-consuming cells should also declare the
normalized consumer cell and, when the project supports one exact upstream
release, its version:

```yaml
cells:
  - name: plugin-pr-linux-vulkan
    lane: pull_request
    runtime_artifact: sha256:<runtime SDK digest>
    require_openusd: cy2026/linux/x86_64/vulkan
    require_openusd_version: "26.08"
    platform: cy2026
    profile: usd
    # runner/runtime_remote/bundle omitted here
```

`ost ci validate` checks that the selector is an approved platform cell and
agrees with the cell's platform and runner OS. `ost ci validate --resolve`
also checks the pinned local artifact's compiler/runtime, Python, TBB, graphics,
ABI, provider and version identity. Generated source, support and release jobs
pass the same requirements to both remote pull and local artifact verification,
so a wrong re-pin fails before runtime materialization or CMake configure.
Self-hosted runner profiles must therefore include a `linux`, `windows`, or
`macos` label; an opaque label set cannot prove the selector's OS contract.

Not every workspace member is a bundle. A plain library that no bundle requires,
and a CLI executable built from the workspace, are invisible to a cell that names
a `bundle:` — so declare `kind: workspace` for a cell that builds the workspace
CMake tree instead (v0.21.0):

```yaml
cells:
  - name: workspace-pr-linux
    kind: workspace
    lane: pull_request
    runtime_artifact: sha256:<runtime SDK digest>
    require_openusd: cy2026/linux/x86_64/gl
    require_openusd_version: "26.08"
    platform: cy2026
    profile: usd
    verify: test          # graph | build | test (default test)
```

It validates the dependency graph, runs `ost build`, then runs the workspace's
own CTest suite — the members the bundle verbs never reach. Source lanes only; a
workspace cell names no bundle and publishes nothing.

When one plain-library member must ship independently (for example, an optional
input adapter), use its descriptor-scoped lifecycle instead of packaging the
aggregate workspace:

```sh
ost library build adapters/ply
ost library test adapters/ply
ost library package adapters/ply
```

For a non-leaf library, the build resolves the same `requires.libraries` graph
accepted by workspace validation, rebuilds prerequisites deepest first into
owner-specific target prefixes, and exposes those prefixes through normal
CMake package discovery. The selected descriptor is installed separately and
its record binds the resolved dependency identities and build evidence along
with the descriptor, runtime, and every installed byte. `test` consumes that
exact closure and record. `package` refuses dependency/descriptor/runtime/
install-tree drift and writes
`dist/<id>/<version>/<target>/<id>-<version>-<target>.tar.zst`; the archive
contains only the selected library while its manifest records the dependency
evidence, checksums, SBOM, and available provenance. Re-run `ost library build`
after changing any member of the declared closure, the runtime, or installed
output.

`verify: graph` is the cheap early PR gate, and it gets a job of its own
(`pr-workspace-graph`) that stops after the checkout: the graph alone, with
nothing built and no runtime fetched, verified, or materialized. `verify: build`
and `verify: test` share one job, since both need the same runtime.

## 6. Keep OpenStrata and plain CMake both working

A workspace stays dual-mode: the same tree builds with `ost` and with plain
CMake. Do not let OpenStrata-specific files break a direct `cmake` build; the
reference projects build both ways in CI. When both modes write to a bundle's
staged `lib/`, `ost build` records the package-relevant workspace bundle bytes
in its managed completion. `ost plugin package` accepts those root-build digests
or a matching bundle-local `ost plugin build` completion; a later plain CMake
overwrite is still reported as `mismatched` and needs the explicit
`--allow-unmanaged-output` override.

## Reference implementations

- [USD 3DGS Plugins](../projects/usd-3dgs-plugins.md) demonstrates an empty-repo
  scaffold becoming a real `SdfFileFormat`, a versioned ordinary-library edge,
  clean extracted-package consumption, and the need to verify that every
  requested package-origin test level really executes rather than skips.
- [USD Point Cloud Plugins](../projects/usd-pointcloud-plugins.md) demonstrates
  four file-format bundles sharing native authoring and tiling code, a
  digest-pinned 24-cell source-CI matrix, and a strict file format whose smoke
  fixture needs arguments unavailable in the current string-only manifest form.
- [USD VRM Plugins](../projects/usd-vrm-plugins.md) demonstrates several
  bundles, bundle-to-bundle runtime and link dependencies, a shared library, and
  workspace packaging.

## Where to go next

- Command details: [reference/plugin-workspace.md](../reference/plugin-workspace.md),
  [reference/cli.md](../reference/cli.md).
- A full command tour: [examples.md](examples.md).
- Composing this workspace with other repositories' components is the planned
  [Formation](../design/proposed/formations.md) model — see
  [compose a formation](compose-a-formation.md) and
  [combined-formations.md](../projects/combined-formations.md).
