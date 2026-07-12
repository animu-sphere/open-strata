# {{name}} — OpenUSD `{{extension}}` file-format plugin

Scaffolded by `ost plugin new usd-fileformat {{name}} --extension {{extension}}`.

## Layout

```
openstrata.plugin.yaml          bundle contract (identity, runtime range, provides, tests)
CMakeLists.txt                  builds lib{{Name}}FileFormat.so into lib/
cmake/OpenStrataPlugin.cmake    pinned, self-contained build/install mechanics
src/{{Name}}FileFormat.{h,cpp}  the SdfFileFormat implementation
plugin/resources/{{name}}/plugInfo.json   USD plugin registration
tests/fixtures/                 basic (valid) + invalid (negative) fixtures
```

The copied CMake helper is versioned with this scaffold and requires neither an
OpenStrata checkout nor `ost` at build time. Keep bundle-specific targets,
components, resources, and tests in `CMakeLists.txt`; update helper mechanics by
reviewing a newer template rather than linking to the generator source tree.

## Workflow

```sh
ost plugin inspect {{name}}     # Level 0: bundle structure
ost plugin build {{name}}       # build the shared library into lib/
ost plugin doctor {{name}}      # staged diagnostics (L0–L1; L2+ need a real runtime)
ost plugin test {{name}}        # orchestrate the levels and write a report
```

Replace the body of `{{Name}}FileFormat::Read` with your format's parser. The
scaffold emits an empty USDA layer so a `.{{extension}}` file opens as a valid
(if empty) stage out of the box.

## Co-hosting a Typed Schema

To compile a generated API schema into this same plugin, add `schema.usda` at the
bundle root and add `usd-schema:<TypeName>` to `provides` in
`openstrata.plugin.yaml`. `ost plugin build` runs `usdGenSchema` in the resolved
runtime environment, links the generated C++ API sources into this library, and
merges the generated `Types` into `plugin/resources/{{name}}/plugInfo.json`.
