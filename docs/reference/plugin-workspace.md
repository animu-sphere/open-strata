# Plugin workspace dependency contract

`ost plugin test --workspace` discovers bundle manifests in immediate child
directories and `plugins/*`, sorts them by path, validates their dependency
graph, and only then resolves a runtime or runs per-bundle verification. Graph
validation is read-only and does not control CMake build order.

## Versioned manifest extension

Legacy manifests without bundle composition fields remain valid. A manifest
that declares `requires.bundles` or provides `schema.contract` must opt into the
extension explicitly:

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

The proposed `requires.libraries` class is not accepted yet. Library identity
and discovery need a separate portable contract before workspace validation can
distinguish a missing package from an externally installed CMake dependency.

## Dependency directions

- A public schema bundle has no dependency on a file format, resolver, or other
  plugin implementation.
- An asset resolver cannot depend on a file-format bundle.
- A file-format bundle may consume schema and resolver bundles.
- Every cycle, including a self-cycle, is invalid.

These checks preserve standalone bundle ownership. They do not synthesize
`add_subdirectory` order or link targets.

## Graph result

With `--json`, the normal workspace result includes `data.graph`:

```json
{
  "passed": true,
  "nodes": [{"id":"vrmFormat","version":"0.2.0","kind":"usd-fileformat"}],
  "edges": [{"from":"vrmFormat","to":"vrmSchema","version":">=0.2,<0.3","contract":1}],
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
