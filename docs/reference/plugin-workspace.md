# Plugin workspace dependency contract

`ost plugin test --workspace` discovers bundle manifests in immediate child
directories and `plugins/*`, plus plain-library descriptors in immediate child
directories and `libs/*`. It sorts them by path, validates their dependency
graph, and only then resolves a runtime or runs per-bundle verification. The
validated graph supplies each bundle's transitive runtime/test closure and
deterministic bundle and library build order.

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
verifiable after a single product download. Verify the product `SHA256SUMS`,
then each member `SHA256SUMS`, and extract members in `install.order`. The
aggregate itself has a producer manifest, SBOM, digest, and registry kind
`product`, so `ost artifact import` / `verify` / transport treat it as a
first-class artifact.

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
