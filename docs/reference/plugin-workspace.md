# Plugin workspace dependency contract

`ost plugin test --workspace` resolves one complete member set, sorts it by
path, validates its dependency graph, and only then resolves a runtime or runs
per-bundle verification. The validated graph supplies each bundle's transitive
runtime/test closure and deterministic bundle and library build order.

Declare the authoritative member roots in the project `openstrata.toml`:

```toml
[workspace]
members = [
  ".",                 # the project root, only when it has a member descriptor
  "plugins/*",         # one path component per * or ? wildcard
  "adapters/*/*",
  "libs/vrmContainer",
  "tools/converter",
]
release_members = ["vrmSchema", "vrmFormat", "vrmTool"]
release_exclude = ["developerFixture"]

[[workspace.install_data]]
source = "profiles/motion"
destination = "share/vrm/motion"
```

`members` controls source discovery; it does not implicitly decide aggregate
release membership. When `release_members` is present, packaging subtracts the
explicit `release_exclude` set from discovered bundle/tool ids and requires the
result to equal the pinned release set. Any newly discovered, missing, or stale
id fails with `AGGREGATE_MEMBERSHIP_MISMATCH` before the aggregate is written.
Human and JSON packaging output print the resolved release members.

`install_data` gives shared project data a product-level owner without attaching
duplicate copies to a plugin or tool. Each source is one regular file or
directory below the project root; packaging expands it to a digest-and-size
inventory, preserves directory structure, and installs it exactly once below
the declared `share/` directory. Product verification checks every file before
installation. Sources, destinations, globs, parent escapes, symlinks, empty
directories, and destination collisions fail closed.

Each directory a **literal** pattern names must contain exactly one of
`openstrata.plugin.yaml`, `openstrata.library.yaml`, or
`openstrata.tool.yaml`; one that does not is
`WORKSPACE_MEMBER_DESCRIPTOR_MISSING`, and the error names the pattern that
selected it. A **wildcard** pattern is a filter over whatever the tree holds
rather than an assertion about every directory it sweeps up, so a matched
directory carrying no descriptor is skipped — a `__pycache__` a test run wrote,
or any other residue, does not refuse the graph. A wildcard that reaches
directories but selects no member is still `WORKSPACE_MEMBER_PATTERN_EMPTY`,
and the error lists what it did reach.

Patterns are portable project-relative paths, use `/`, and may contain `*` and
`?` within a component. Recursive `**`, parent escapes, generated/state
directories, and nesting deeper than eight components are rejected. Every
pattern must match at least one directory.

The declaration is fail-closed where it counts. A bounded scan of the project
(at most eight levels, without following symlinks or entering hidden, `.git`,
`.strata`, `target`, `build`, `out`, `node_modules`, or `__pycache__`
directories) reports any descriptor not covered by `members`. A malformed
selected descriptor also fails loading. Thus a green `--graph-only` result
cannot omit a source member or its dependency edges.

Legacy projects without `[workspace]` use that bounded scan below the project
root as a compatibility fallback; a root descriptor becomes a member only
through an explicit `"."`. Adding the declaration is recommended whenever the
repository has a deliberate multi-member layout, especially nested trees such
as `adapters/*/*`.

The graph check is separately askable:

```bash
ost plugin test --workspace --graph-only     # graph only; exits on its result
```

`--graph-only` runs the dependency-graph checks and stops. It needs no build, no
resolved runtime, and no packaged artifact, so it belongs at the front of a PR
lane rather than behind everything the per-bundle pyramid requires. Without it
the two were welded together: on a fresh checkout the verb validated the graph,
reported it valid, and then failed because nothing had been built yet, leaving a
repository to either build every bundle or parse the graph out of `--json`.

## Versioned manifest extension

Legacy manifests without composition fields remain valid. A manifest that
declares `requires.bundles`, `requires.libraries`, or provides
`schema.contract` must opt into the extension explicitly:

```yaml
manifest:
  schema: openstrata.plugin/v1alpha1
plugin:
  name: vrmFormat
  version: 0.2.0
  kind: usd-fileformat
runtime:
  openusd: ">=25.05,<27.0"
requires:
  capabilities: [usd-stage-read]
  bundles:
    - id: vrmSchema
      version: ">=0.2,<0.3"
      contract: 1
usd:
  plug_info: plugin/resources/vrmFormat/plugInfo.json
```

Each dependency requires a portable bundle `id` and a numeric dotted-version
range. `contract` is allowed only when the provider is a `usd-schema` bundle.
Dependency entries reject unknown keys.

## Smoke fixtures with file-format arguments

`tests.smoke` keeps its path-only form for existing bundles and also accepts a
structured entry for formats that require explicit `SdfFileFormat` arguments:

```yaml
tests:
  smoke:
    - tests/fixtures/basic.las
    - path: tests/fixtures/strict.ply
      file_format_arguments:
        epsg: "4978"
```

The first smoke entry drives the execution pyramid. OST passes one normalized
layer identifier to L3 `usdcat`, L5's smoke-to-roundtrip fallback, and L6
`usdview`; L4 builds the same identity with
`Sdf.Layer.CreateIdentifier(path, arguments)` before `Usd.Stage.Open()`. Keys
are ordered deterministically. Paths remain bundle-relative, and keys/values
that OpenUSD's current unescaped identifier syntax cannot represent are rejected
when the bundle loads. `tests.roundtrip` and `tests.negative` remain path-only.

A schema provider declares its authored-data contract separately from its
semantic implementation version:

```yaml
manifest:
  schema: openstrata.plugin/v1alpha1
plugin:
  name: vrmSchema
  version: 0.2.4
  kind: usd-schema
schema:
  codeless: true
  contract: 1
```

Compatible implementation releases keep `contract` unchanged. A breaking
type, property, or token surface increments it and requires authored-data
migration notes. Consumers of a versioned schema contract must select it
explicitly.

Versioned manifests recursively reject unknown keys below `requires:`. Plain
libraries use a separate producer descriptor while the plugin consumer names
only identity and compatible version:

```yaml
# plugins/usdVrm/openstrata.plugin.yaml
requires:
  libraries:
    - id: vrmContainer
      version: ">=0.1,<0.2"
```

```yaml
# libs/vrmContainer/openstrata.library.yaml
schema: openstrata.library/v1alpha1
library:
  id: vrmContainer
  version: 0.1.0
cmake:
  package: vrmContainer
  target: vrmContainer::vrmContainer
runtime:
  directories: [bin, lib]
```

The library may itself declare `requires.libraries` for a transitive closure.
OST validates missing, duplicate, incompatible, malformed, and cyclic library
edges. It never infers identity from CMake target names; consumers continue to
use `find_package(vrmContainer CONFIG REQUIRED)`. A library descriptor carries
no plugin kind, `plugInfo.json`, registration, or OpenUSD dependency. Legacy
plugin manifests retain their previous permissive parsing for compatibility,
but using either composition field requires the versioned plugin header.

## Dependency directions

- A public schema bundle has no dependency on a file format, resolver, or other
  plugin implementation.
- An asset resolver cannot depend on a file-format bundle.
- A file-format bundle may consume schema and resolver bundles.
- Every cycle, including a self-cycle, is invalid.

These checks preserve standalone bundle ownership. Composition does not
synthesize `add_subdirectory` links or link targets.

## Source-workspace composition

After graph validation succeeds, source-workspace commands consume the same
graph rather than asking each caller to restate it:

- `plugin test --workspace` composes each primary bundle with its transitive
  dependency closure before running L2 and above;
- a selected `plugin doctor|test|run <bundle>` resolves the same closure when
  its containing workspace is unambiguous;
- `plugin build <bundle>` builds source dependencies in deterministic
  topological order, installs them to an OST-owned target-specific prefix, and
  passes that prefix through normal CMake package discovery;
- `library build <library>` resolves the selected library's transitive sibling
  closure from this same validated graph, builds each prerequisite deepest
  first into its own managed prefix, and exposes those prefixes through
  `CMAKE_PREFIX_PATH` while configuring the consumer;
- `library test|package <library>` re-resolves that closure and refuses stale or
  missing prerequisite build evidence; the library package records dependency
  identities and evidence digests but contains only the selected library's
  install tree;
- plain-library runtime directories materialized below that prefix are added to
  the loader environment for selected test/run/view sessions;
- `plugin inspect --json` and test report `dependencies.json` expose selected
  library identity, version, descriptor, CMake package/target, prefix, runtime
  paths, and source-workspace provenance;
- generated source-CI cells use the manifest closure selected by `bundle:` and
  do not gain a second, manually maintained `with:` list;
- explicit `--with` remains additive for external or ad-hoc bundles and keeps
  its existing caller-defined ordering.

A selected primary bundle that declares neither `requires.bundles` nor
`requires.libraries` has an empty
closure and skips workspace discovery entirely: unrelated sibling bundles (a
broken manifest, a stale copy) cannot fail its commands. Once a bundle declares
dependencies, an unloadable or invalid workspace graph fails closed.

Dependency builds install, deepest dependency first, into
`.strata/targets/<target-id>/workspace-prefix`. OpenStrata prepends that private
prefix to `CMAKE_PREFIX_PATH`, so consumers use normal installed CMake package
discovery. The prefix is target-specific and rebuilt for a composed build; it
is not part of a bundle's installed interface.

The descriptor-scoped `ost library` lifecycle instead uses one
`library-prefix` below each member's target state directory. A non-leaf build
rebuilds its declared closure into those owner-specific prefixes and prepends
the exact transitive set for each configure. This keeps dependency payloads out
of the selected library's archive: dependency edges and their build-record
digests are recorded in its build/package manifests rather than flattened into
one install tree.

The primary bundle keeps priority in the plugin and loader search paths;
resolved dependencies follow in a stable order, then the runtime. Duplicate
bundle identities are rejected or deduplicated only after identity/version/
contract agreement—path order must not silently pick a provider.

A plugin package materializes its selected plain-library runtime under
`runtime/libraries/`, adds those directories to the packaged manifest's loader
paths, and records the library closure in `dependencies.json` and the artifact
manifest.

Every package also carries
[`openstrata.activation.json`](../../schemas/plugin-activation.schema.json), `activate.ps1`,
`activate.sh`, and `openstrata_activate.py`. The JSON document is the portable
consumer contract: it names the package-relative USD plugin, dynamic-library,
and Python roots plus the target OS loader variable. Dot-source `activate.ps1`
or source `activate.sh` to prepend the existing roots without requiring `ost`.
On Windows with Python 3.8 or newer, import `openstrata_activate` before `pxr`;
the module calls `os.add_dll_directory()` for every packaged library root and
retains the handles for the life of the process. This is the supported bridge
from `requires.runtime_libs` to non-`ost` consumers; parsing the plugin YAML and
guessing loader behavior is not.

For example, after the OpenUSD host itself is active:

```powershell
# OpenUSD command-line host
. .\activate.ps1
usdcat tests/fixtures/minimal.vrm

# Python 3.8+ on Windows: retain package DLL-directory handles before pxr loads.
python -c "import openstrata_activate; from pxr import Usd; assert Usd.Stage.Open('tests/fixtures/minimal.vrm')"
```

The OpenUSD installation remains responsible for activating its own
`bin`/`lib`/Python directories; the package entrypoint adds and retains the
package's staged dependency directories. A vendor/runtime Python launcher
normally provides the first half. When embedding OpenUSD into a stock Windows
Python process, register the host's DLL directories with
`os.add_dll_directory()` before importing `openstrata_activate`.

Package-origin verification carries its oracle too. For every declared
`tests.roundtrip` fixture that has an adjacent `<fixture>.golden.usda`,
`ost plugin package` stages both files and emits
[`openstrata.verification.json`](../../schemas/plugin-verification.schema.json).
That versioned contract records the fixture/oracle pair and both SHA-256
digests; the artifact `manifest.json` points to it and includes both files in
its hashed `files[]` inventory. `ost plugin test --from-package --up-to 5`
verifies the contract before flattening. An oracle absent from source remains an
optional L5 SKIP, but an oracle declared by the packaged contract that is
missing or has changed is a validation failure.

Managed plugin builds also bind packages to the bytes they produced. After a
successful `ost plugin build`, the target's `.ost-build-complete.json` records
the target/runtime/compiler/generator build fingerprint and the path, size, and
SHA-256 of the primary bundle's package-relevant registration, library, and
Python outputs. A successful root `ost build` records the same output set for
every workspace bundle in its project completion, prefixing each path with the
member's project-relative directory. It also stages a declared workspace-tool
executable found in the root target build tree below the first directory in the
tool descriptor. Selection considers only files created or changed since the
pre-build snapshot, prefers the member-relative build path and requested
configuration, permits a globally unique filename fallback, and reports rather
than guesses when candidates are stale or ambiguous. The selected set is
committed transactionally. `ost plugin package` recomputes the staged set and
compares it with both applicable managed producers; an exact match is
accepted even when an older completion from the other build path is stale.
Root candidates include the default and every currently declared named build
intent; their recorded generator is validated against the target lock rather
than a package-time generator default. It reports one of:

| Status | Meaning |
| --- | --- |
| `matched` | Every current package-relevant output matches the last managed build. |
| `untracked` | No output-bearing managed completion exists; the bytes may come from plain CMake or another external producer. |
| `mismatched` | A managed output is missing, changed, or newly present relative to the completion. |

`mismatched` fails packaging by default with the changed path, expected and
observed digests, and last build fingerprint. If an external/plain-CMake output
is intentional, `--allow-unmanaged-output` permits packaging while recording
the origin as `external-or-unmanaged-override`; it does not rewrite the status
to `matched`. The matched origin is `ost-managed` for a bundle-local build and
`ost-managed-root` for a root workspace build. The same object is emitted in
human/JSON package output and under `provenance.build_outputs` in the artifact
manifest. An artifact with no managed completion remains packageable as
`untracked`, preserving plain CMake as a supported producer without presenting
it as an `ost` build. A mismatch names the matching producer's rebuild command,
including `--intent <name>` for a named root build.

`ost plugin package --workspace --product` additionally emits one aggregate
`openstrata.plugin-product` artifact. Its archive has this fixed layout:

```text
openstrata.product.json
members/<bundle-id>/<bundle archive>.tar.zst
members/<bundle-id>/manifest.json
members/<bundle-id>/SHA256SUMS
members/<bundle-id>/sbom.spdx.json
members/<bundle-id>/provenance.intoto.jsonl  # when the member has provenance
```

[`openstrata.product.json`](../../schemas/plugin-product.schema.json) records the validated dependency order and each
member's archive digest, manifest, checksums, evidence, optional debug archive,
and dependency closure. The product is built from the exact per-bundle package
outputs—not from sibling source paths—so every member remains independently
verifiable after a single product download. Product identity is the enclosing
project's `project.name`, effective project version, and target; its archive is
named `<name>-<version>-<target>-plugin-product.tar.zst`. Member bundle names and
versions stay independent and are pinned by the product contract.

Use the product commands instead of unpacking members manually:

```sh
# A producer dist directory or its manifest.json carries the expected digest.
ost plugin product verify dist/products/<name>/<version>/<target>
ost plugin product install dist/products/<name>/<version>/<target> \
  --prefix ./installed-product

# A standalone downloaded archive can be pinned explicitly.
ost plugin product verify product.tar.zst --expect-digest sha256:<64-hex>
ost plugin product install product.tar.zst --expect-digest sha256:<64-hex> \
  --prefix ./installed-product
```

Verification covers the product digest, strict contract and order, member
archive digests/sizes, each member `SHA256SUMS`, evidence presence, extracted
file inventory, and bundle validity. Installation refuses to replace an
existing prefix, expands members under `bundles/<id>/` in dependency order, and
emits aggregate `activate.ps1`, `activate.sh`,
`openstrata.activation.json`, and `openstrata_activate.py` entrypoints. The
aggregate itself also has a producer manifest, SBOM, digest, and registry kind
`product`, so `ost artifact import` / `verify` / transport remain available for
digest-addressed registry workflows.

A `requires.bundles` provider travels as **both halves**. Its link half is staged
under `runtime/bundles/<id>/lib`, beside the provider-relative path its
`plugInfo.json` already names; ordinary `requires.libraries` runtime files stay
under `runtime/libraries/`. Its USD *registration* half is staged under
`runtime/bundles/<id>/<provider plugInfo root>` and declared in the packaged
manifest's `requires.runtime_plugin_paths`, which the session adds to
`PXR_PLUGINPATH_NAME` behind the bundle's own root. Both are required for the
package to be independently installable: the link half satisfies the loader, and
only the registration half lets USD find a `kind: usd-schema` provider's
`plugInfo.json` and `generatedSchema.usda` and apply its schemas. Staging one
without the other produces an artifact that records a resolved closure, resolves
its own file format, and then fails at `Usd.Stage.Open()`.

`requires.runtime_plugin_paths` is written by `ost plugin package` from the
resolved workspace graph; authored bundles do not normally set it. The L0
`bundle.runtime_plugin_paths` diagnostic fails when a declared path is missing,
is not a directory, or contains no `plugInfo.json` — a staged tree that
registers nothing is indistinguishable at discovery time from one that was never
staged.

## Graph result

With `--json`, the normal workspace result includes `data.graph`:

```json
{
  "passed": true,
  "nodes": [{"id":"vrmFormat","version":"0.2.0","kind":"usd-fileformat"}],
  "edges": [{"from":"vrmFormat","to":"vrmSchema","version":">=0.2,<0.3","contract":1}],
  "libraries": [{"id":"vrmContainer","version":"0.1.0","package":"vrmContainer","target":"vrmContainer::vrmContainer"}],
  "library_edges": [{"from":"vrmFormat","from_kind":"bundle","to":"vrmContainer","version":">=0.1,<0.2"}],
  "issues": []
}
```

An invalid graph exits with validation status `5` before bundle reports are
written. Issues use stable codes:

| Code | Meaning |
| --- | --- |
| `WORKSPACE_MEMBER_PATTERN_EMPTY` | A declared member pattern matches no directory, or reaches only directories carrying no descriptor. |
| `WORKSPACE_MEMBER_DESCRIPTOR_MISSING` | A directory named by a *literal* member pattern has no OpenStrata member descriptor. |
| `WORKSPACE_MEMBER_DESCRIPTOR_AMBIGUOUS` | One member directory contains more than one member descriptor kind. |
| `WORKSPACE_DESCRIPTOR_NOT_DECLARED` | A bounded scan found a descriptor outside the authoritative member list. |
| `WORKSPACE_BUNDLE_ID_INVALID` | A discovered plugin identity is not portable. |
| `WORKSPACE_DUPLICATE_BUNDLE_ID` | More than one discovered bundle has the same identity. |
| `WORKSPACE_DEPENDENCY_ID_INVALID` | A dependency id is not portable. |
| `WORKSPACE_DUPLICATE_DEPENDENCY` | A consumer repeats the same dependency id. |
| `WORKSPACE_DEPENDENCY_MISSING` | No unique discovered bundle provides the required id. |
| `WORKSPACE_DEPENDENCY_VERSION_INVALID` | A dependency version range cannot be parsed. |
| `WORKSPACE_DEPENDENCY_VERSION_MISMATCH` | The provider version does not satisfy the range. |
| `WORKSPACE_SCHEMA_CONTRACT_INVALID` | A schema provider or consumer declares contract `0`. |
| `WORKSPACE_SCHEMA_CONTRACT_REQUIRED` | A consumer did not select the provider's contract. |
| `WORKSPACE_SCHEMA_CONTRACT_MISSING` | A consumer selects a contract that the schema does not provide. |
| `WORKSPACE_SCHEMA_CONTRACT_MISMATCH` | Required and provided schema contracts differ. |
| `WORKSPACE_SCHEMA_CONTRACT_NOT_APPLICABLE` | A contract is attached to a non-schema dependency or bundle. |
| `WORKSPACE_DEPENDENCY_DIRECTION_FORBIDDEN` | The dependency violates the bundle ownership direction. |
| `WORKSPACE_DEPENDENCY_CYCLE` | The directed bundle graph contains a cycle. |
| `WORKSPACE_DUPLICATE_LIBRARY_ID` | More than one descriptor provides the same library id. |
| `WORKSPACE_DUPLICATE_LIBRARY_DEPENDENCY` | A bundle or library repeats one library edge. |
| `WORKSPACE_LIBRARY_DEPENDENCY_ID_INVALID` | A library dependency id is not portable. |
| `WORKSPACE_LIBRARY_DEPENDENCY_MISSING` | No unique descriptor provides the required library. |
| `WORKSPACE_LIBRARY_DEPENDENCY_VERSION_INVALID` | A library version range cannot be parsed. |
| `WORKSPACE_LIBRARY_DEPENDENCY_VERSION_MISMATCH` | The provider version does not satisfy the range. |
| `WORKSPACE_LIBRARY_DEPENDENCY_CYCLE` | The directed plain-library graph contains a cycle. |
| `WORKSPACE_LIBRARY_RUNTIME_MISSING` | Build/package/test/run needs an installed library runtime directory which is absent. |
