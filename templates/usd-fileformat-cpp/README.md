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

## plugInfo.json and the library name

Edit `plugin/resources/{{name}}/plugInfo.json.in`; `plugInfo.json` beside it is
generated. Its `LibraryPath` is `@OPENSTRATA_PLUGIN_LIBRARY_PATH@`, which the
helper expands to `../../../lib/$<TARGET_FILE_NAME:{{Name}}FileFormat>` and
CMake evaluates at generate time. The registered filename is therefore always
the one the target actually builds, including any `OUTPUT_NAME`, `PREFIX`,
`SUFFIX` or `DEBUG_POSTFIX` you set on the target.

- `ost plugin new` writes a host-specific preview (`lib{{Name}}FileFormat` plus
  this host's suffix) so `inspect` and `doctor` work before the first build.
  CMake overwrites it at the first configure.
- Each configuration also gets
  `<build>/openstrata/{{Name}}FileFormat/<config>/plugInfo.json`. The
  source-tree file holds the primary configuration (the build type, or
  `Release` for multi-config generators; set `OPENSTRATA_PLUGIN_PRIMARY_CONFIG`
  to change it). `cmake --install --config <config>` installs the matching
  copy.
- The installed `plugin/resources/{{name}}/` must stay at the same depth as the
  source one, so the same relative `LibraryPath` resolves in both trees.
  Configure fails if it does not.

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
