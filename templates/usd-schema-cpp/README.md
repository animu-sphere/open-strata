# {{name}} — compiled OpenUSD schema (`{{Name}}ContractAPI`)

Scaffolded by:

```sh
ost plugin new usd-schema {{name}} --template usd-schema-cpp
```

This is an experimental compiled-schema skeleton. `schema.usda` is the only
hand-edited schema source. The typed C++ API and registration resources are
committed so source archives remain buildable without `usdGenSchema`; generated
files must not be edited by hand.

`schema.contract: 1` versions the authored type/property/token surface
independently from `plugin.version`. Increment it only for a breaking data-model
change, with migration notes and updated consumer evidence.

## Generation modes

Set `OPENSTRATA_SCHEMA_GENERATION_MODE` at CMake configure time:

- `AUTO` (default) regenerates when compatible `usdGenSchema` and Python are
  available, otherwise uses committed outputs.
- `GENERATE` requires the generator and builds from fresh outputs.
- `PREGENERATED` never invokes the generator.
- `VERIFY` regenerates and fails if C++ or registration outputs differ from the
  committed baseline. Use this mode for promotion and release CI.

The initial generated baseline was produced with OpenUSD 26.05 and the skeleton
claims generator compatibility only for the manifest range `>=25.05,<27.0`.
Review and recommit generated deltas when changing that range.

## Build and verify

```sh
ost plugin inspect {{name}}
ost plugin build {{name}}
ost plugin doctor {{name}}
ost plugin test {{name}}

# Plain CMake, committed outputs only
cmake -S {{name}} -B {{name}}/build \
  -DCMAKE_PREFIX_PATH=/path/to/openusd \
  -DOPENSTRATA_SCHEMA_GENERATION_MODE=PREGENERATED
cmake --build {{name}}/build
cmake --install {{name}}/build --prefix {{name}}/install
```

The install exports `{{Name}}::Schema` and headers under
`include/{{name}}`. `tests/consumer` is a downstream CMake fixture that must
configure from the install prefix alone; it must not use a sibling source path.

The authored schema identifier includes the bundle prefix
(`{{Name}}ContractAPI`), while `customData.className = "ContractAPI"` keeps the
C++ class name from receiving that prefix twice. `ContractAPI` also avoids a
source class named only `API`: that makes `usdGenSchema` emit both `api.h` and
`aPI.h`, which collide on case-insensitive filesystems. Add real schema classes
with unique identifiers and portable, case-distinct generated filenames.
