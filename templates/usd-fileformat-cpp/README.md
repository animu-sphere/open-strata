# {{name}} — OpenUSD `{{extension}}` file-format plugin

Scaffolded by `ost plugin new usd-fileformat {{name}} --extension {{extension}}`.

## Layout

```
openstrata.plugin.yaml          bundle contract (identity, runtime range, provides, tests)
CMakeLists.txt                  builds lib{{Name}}FileFormat.so into lib/
src/{{Name}}FileFormat.{h,cpp}  the SdfFileFormat implementation
plugin/resources/{{name}}/plugInfo.json   USD plugin registration
tests/fixtures/                 basic (valid) + invalid (negative) fixtures
```

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
