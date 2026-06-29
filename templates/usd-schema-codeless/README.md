# {{name}} — codeless OpenUSD schema (`{{Name}}API`)

Scaffolded by `ost plugin new usd-schema {{name}}`.

This is a **codeless** schema: it ships no compiled C++ and no shared library.
USD registers the classes entirely from `plugInfo.json`'s `Types` block plus the
flattened `generatedSchema.usda`. Both are committed and correct out of the box,
so the schema registers on a real runtime **without** a build step — `usdGenSchema`
only *re*-generates them from `schema.usda` when you change the contract.

## Layout

```
openstrata.plugin.yaml                          bundle contract (identity, runtime range, codeless flag)
schema.usda                                     authored class definitions (the source of truth)
CMakeLists.txt                                  re-runs usdGenSchema to regenerate the resources
plugin/resources/{{name}}/plugInfo.json         USD plugin registration (Types block)
plugin/resources/{{name}}/generatedSchema.usda  flattened schema definition (registration needs it)
tests/fixtures/basic.usda                       applies {{Name}}API to a prim
```

## Workflow

```sh
ost plugin inspect {{name}}     # Level 0: bundle structure (codeless: validates the Types block)
ost plugin build {{name}}       # runs usdGenSchema against the resolved runtime
ost plugin doctor {{name}}      # staged diagnostics (L0–L1; L2+ need a real runtime)
ost plugin test {{name}}        # orchestrate the levels and write a report
```

Edit `schema.usda` to define the schema's real properties (and add more
`class` definitions as needed), then rebuild to regenerate `plugInfo.json`.
To ship a *compiled* schema instead (typed C++ getters/setters for an importer
to call), set `schema.codeless: false` and remove `skipCodeGeneration` — doctor
will then expect a built library like a file-format plugin.
