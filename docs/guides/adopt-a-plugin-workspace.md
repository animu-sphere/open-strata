# Adopting a plugin workspace

This guide is for a repository that contains **several OpenUSD plugin bundles
developed together** — schemas, file formats, resolvers, and shared libraries —
and wants OpenStrata to discover, validate, test, and package them as one
dependency-ordered workspace. It is transferable to any such repository; the
[USD VRM Plugins](../projects/usd-vrm-plugins.md) reference project is used as a
worked example, not a required layout.

The factual contract behind everything here is
[reference/plugin-workspace.md](../reference/plugin-workspace.md); this page is
the procedure.

## 1. Lay out the workspace

Place each bundle in its own directory with a plugin manifest. OpenStrata
discovers immediate subdirectories and `plugins/*` that carry a manifest. Ordinary
CMake libraries the bundles link live alongside them. A typical shape:

```text
plugins/
  vrmSchema/            # USD schema bundle
  usdVrmFileFormat/     # SdfFileFormat bundle
  usdVrmPackageResolver/# ArPackageResolver bundle
vrmContainer/           # ordinary CMake library the bundles link
```

You do **not** restructure your project into an OpenStrata package abstraction.
OpenStrata adopts your existing CMake target boundaries.

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
ost plugin test --workspace
```

This validates the dependency graph first (a cycle or a missing provider fails
fast), then tests every discovered bundle in dependency order. Scope the pyramid
with `--up-to <level>` while iterating:

```sh
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

## 6. Keep OpenStrata and plain CMake both working

A workspace stays dual-mode: the same tree builds with `ost` and with plain
CMake. Do not let OpenStrata-specific files break a direct `cmake` build; the
reference project builds both ways in CI.

## Where to go next

- Command details: [reference/plugin-workspace.md](../reference/plugin-workspace.md),
  [reference/cli.md](../reference/cli.md).
- A full command tour: [examples.md](examples.md).
- Composing this workspace with other repositories' components is the planned
  [Formation](../design/proposed/formations.md) model — see
  [compose a formation](compose-a-formation.md) and
  [combined-formations.md](../projects/combined-formations.md).
