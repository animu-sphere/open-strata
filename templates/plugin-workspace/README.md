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

## Build with `ost` (recommended)

`ost` composes the resolved runtime's environment and toolchain per bundle:

```sh
ost runtime pull {{name}} --profile usd     # or adopt one: --from-usd <path>
ost plugin build myfmt
ost plugin test  myfmt
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
