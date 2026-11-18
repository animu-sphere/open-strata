# Migrating an existing bundle to a co-located USD schema

This guide is for an **existing** plugin bundle — typically a `usd-fileformat`
or `usd-asset-resolver` bundle that predates the schema tooling — that wants to
co-host USD schema types in the same bundle: one `plugInfo.json` providing both
the primary plugin type and the schema `Types`, one library (for a compiled
schema), one artifact. Freshly scaffolded bundles don't need this page:
`ost plugin new usd-schema` and `ost plugin schema add` set everything up.

Why co-locate instead of splitting a separate `usd-schema` bundle: if the
plugin's C++ calls the typed schema API directly (e.g. a file format populating
`*API` attributes), a separate schema bundle makes standalone
`ost plugin build` and the CI artifact model harder for no benefit. Split only
when other plugins consume the schema *without* the primary plugin.

## The fast path

```sh
ost plugin schema add <bundle>              # compiled (default)
ost plugin schema add <bundle> --codeless   # resource-only
```

This scaffolds a starter source (default `schema/schema.usda`), and wires the
manifest *textually* (your comments survive): each schema type is added to
`provides:` as `usd-schema:<Type>`, and the bundle-relative source path is
recorded as `schema.source`. The public type name is composed as
`<PascalBundleName><Class>` (`--class`, default `API`): bundle `toy` + class
`MetadataAPI` → `ToyMetadataAPI`.

The scaffold writes that name twice, for two different usdGenSchema rules:

- **The class prim** (`class "ToyMetadataAPI"`) is the schema *identifier*.
  Fixtures list it in `apiSchemas`, and OpenUSD requires it to be unique across
  every loaded plugin.
- **`customData = { string className = "MetadataAPI" }`** sets the C++/TfType
  name, which usdGenSchema composes as `libraryPrefix + className`. That makes
  the TfType name match the identifier.
- **`string fileName = "toyMetadataAPI"`** names the generated `.h`/`.cpp`.
  Without it, usdGenSchema names them after `className` with its first letter
  lowercased. For class `API` that gives `aPI.h`, which is the same file as
  usdGenSchema's export header `api.h` on Windows and default macOS. The export
  macros are then lost and the compile fails. `ost plugin doctor`
  (`schema.generated_file_names`) and `ost plugin build` reject such names up
  front.

A bare `class "API"` would register the identifier `API`. Any two bundles built
that way then fail to load together: OpenUSD rejects duplicate schema
identifiers. Keep `className` free of the bundle name. usdGenSchema prepends
`libraryPrefix` itself, so a `className` that already carries it doubles up
(`ToyToyMetadataAPI`). `ost plugin doctor` emits a non-failing
`schema.library_prefix` hint for that shape.

`schema add` also writes `tests/fixtures/<Type>.usda`, which applies the
schema, and appends it to `tests.smoke`. L4 searches every smoke fixture for a
prim that applies a declared schema, because a file format's own fixture
usually cannot carry `apiSchemas`.

If the bundle already has a hand-authored `schema.usda`, skip the scaffold and
wire the manifest by hand:

```yaml
plugin:
  name: usdVrm
  kind: usd-fileformat
provides:
  - usd-fileformat:vrm
  - usd-schema:VrmHumanoidAPI
  - usd-schema:VrmExpressionAPI
schema:
  source: schema/schema.usda   # bundle-relative; validated in-bundle (SEC-002)
```

A declared-but-missing `schema.source` is a configuration error, never a
silent no-op.

## What the next `ost plugin build` does

On a non-schema bundle that declares `usd-schema:<Type>` and ships a schema
source, `ost plugin build`:

1. runs `usdGenSchema` in the composed runtime/session environment (so `pxr`
   imports and `@usd/schema.usda@` resolves), forcing UTF-8
   (`PYTHONUTF8=1`) so non-ASCII `doc=` strings survive non-UTF-8 locales;
2. stages the generated typed C++ into the *same* plugin library via a
   generated CMake fragment, and defines the generated `*_EXPORTS` macro;
3. **merges** the generated schema `Types` into the bundle's existing
   `plugInfo.json` — preserving the `SdfFileFormat` (or resolver) entry that a
   raw `usdGenSchema` run would clobber — and copies `generatedSchema.usda`
   beside it, then beside the installed `plugInfo.json` in the bundle stage
   (and therefore the package). The bundle's own install rules do not list
   that file. Without it OpenUSD still registers the schema type, but with no
   properties;
4. falls back to the resource-only merge when the schema is codeless
   (`skipCodeGeneration`) or emits no C++.

For the compiled flow your `CMakeLists.txt` needs the hook (scaffolded bundles
already carry it):

```cmake
if(DEFINED OPENSTRATA_SCHEMA_SOURCES_FILE AND EXISTS "${OPENSTRATA_SCHEMA_SOURCES_FILE}")
    include("${OPENSTRATA_SCHEMA_SOURCES_FILE}")
endif()
```

`usdGenSchema` must exist in the runtime (OpenUSD skips installing it when
`jinja2`/`PyYAML` were absent at USD build time).

## Committed vs build-tree generated sources

Decide one policy per repo:

- **Build-tree only** (recommended with `ost`): generated sources live under
  the build tree; `ost plugin build` regenerates them. Plain-CMake consumers
  need `ost` (or a manual `usdGenSchema` step).
- **Committed fallback**: commit the generated `.h/.cpp`, `plugInfo.json`
  `Types`, and `generatedSchema.usda` so a plain `cmake -S .` build works
  without `ost`. Regenerate deliberately after editing `schema.usda`.

Either way, `generatedSchema.usda` must be staged next to the same
`plugInfo.json` that carries the merged `Types` — schema registration reads it
from there.

## Verify

```sh
ost plugin test <bundle> --up-to 4    # or: ost plugin test --workspace
```

A bundle declaring `usd-schema:<Type>` runs the schema contract *alongside*
its primary-kind levels:

- **L2 `schema.registration`:** each provided TfType is known to
  `Usd.SchemaRegistry`, resolved through its schema identifier.
- **L4 `schema.apply_roundtrip`:** some smoke fixture applies an `*API`, and
  its authored attributes survive a flatten round-trip.

The gate is
the explicit `provides:` entry — a file format's own type is never mistaken
for a schema.

## Cross-platform notes

- `runtime.cxx_abi`: keep `inherit` (or the per-OS map) in the source
  manifest; `ost plugin package` freezes the one resolved ABI into the
  artifact.
- `plugInfo.json` `LibraryPath`: commit the `plugInfo.json.in` template and
  let CMake configure the per-target concrete file; a fixed `.dll`/`.so`
  suffix is single-platform even if the README claims otherwise. `ost plugin
  doctor` flags both (`runtime.cxx_abi`, `bundle.plug_info.library_path`).
