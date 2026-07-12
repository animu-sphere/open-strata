# {{name}} — OpenStrata plugin workspace

A repository that holds one or more OpenUSD plugin bundles. Scaffolded by
`ost init --template usd-plugin-workspace`.

## Add a bundle

```sh
ost plugin new usd-fileformat myfmt --extension myfmt
ost plugin new usd-schema     myschema
ost plugin new usd-fileformat mynested --extension mynested --dir plugins/mynested
```

Each command creates a self-contained bundle directory (with its own
`openstrata.plugin.yaml` and `CMakeLists.txt`). The root `CMakeLists.txt` picks
up root-level bundles and `plugins/<name>/` bundles automatically.

## Declare bundle dependencies

Composition stays in each consumer's `openstrata.plugin.yaml`. For example, a
file-format bundle that consumes the generated `myschema` contract declares:

```yaml
manifest:
  schema: openstrata.plugin/v1alpha1
requires:
  capabilities: [usd-stage-read]
  bundles:
    - id: myschema
      version: ">=0.1,<0.2"
      contract: 1
```

`ost plugin test --workspace` validates bundle ids, semantic version ranges,
schema contracts, dependency directions, and cycles before resolving a runtime
or running CMake. The check is read-only: CMake still discovers bundles without
inferring or changing their build order.

## Build with `ost` (recommended)

`ost` composes the resolved runtime's environment and toolchain per bundle:

```sh
ost runtime pull cy2026 --profile usd       # or adopt one: --from-usd <path>
ost plugin build myfmt
ost plugin test  myfmt
ost plugin test --workspace --up-to 1
```

## Build without `ost` (plain CMake)

The root is dual-mode: it resolves OpenUSD once and `add_subdirectory()`s each
bundle, so a plain CMake user can build the whole repo:

```sh
cmake -S . -B build -DCMAKE_PREFIX_PATH=<your-openusd-install>
cmake --build build
# or: cmake --preset default -DCMAKE_PREFIX_PATH=<your-openusd-install>
```

New root-level bundles and `plugins/<name>/` bundles are discovered
automatically — no edit to the root `CMakeLists.txt` is needed.
