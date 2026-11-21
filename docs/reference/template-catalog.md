# Template catalog

OST embeds its scaffold catalog in the executable so generation is deterministic
and works offline. Every generated scaffold carries `openstrata.scaffold.yaml`
with the template id/version, generator version, and normalized inputs.

## Project and workspace templates

| Template id | Maturity | Command | Purpose |
| --- | --- | --- | --- |
| `cpp-library` | template | `ost init --template cpp-library` | Minimal installable C++ library. |
| `renderer` | skeleton | `ost init --template renderer` | One-project renderer boundaries, headless evidence, and an optional Hydra 2/usdview pack. |
| `usd-plugin` | skeleton | `ost init --template usd-plugin` | Minimal generic OpenUSD plugin project. |
| `usd-plugin-workspace` | template | `ost init --template usd-plugin-workspace` | Dual-mode root that discovers immediate bundles and `plugins/*`. |

## Plugin templates

`ost plugin new` selects the default template for each plugin kind. When a kind
has more than one implementation, pass `--template <id>`; the stable id is
always recorded in scaffold provenance. `usd-schema-codeless` remains the
backwards-compatible default for `usd-schema`.

| Template id | Maturity | Plugin kind | Required input |
| --- | --- | --- | --- |
| `usd-fileformat-cpp` | template | `usd-fileformat` | `--extension <ext>` |
| `usd-schema-codeless` | template | `usd-schema` | none |
| `usd-schema-cpp` | skeleton | `usd-schema` | `--template usd-schema-cpp` |
| `usd-asset-resolver-cpp` | skeleton | `usd-asset-resolver` | `--scheme <scheme>` |
| `usd-package-resolver-cpp` | skeleton | `usd-package-resolver` | `--extension <ext>` |
| `usd-exec-cpp` | skeleton | `usd-exec` | `--schema-bundle <id> --schema-type <CppType>` |
| `usd-imaging-cpp` | skeleton | `usd-imaging` | `--schema-bundle <id> --schema-type <API-schema-name>` |
| `usdview-plugin-python` | skeleton | `usdview-plugin` | none |

Skeletons have stable generation and lifecycle seams, but their domain
architecture has not met the promotion evidence required of a template.

Every C++ project, workspace, and plugin scaffold emits a root `.clang-format`
with the catalog's shared C++17 baseline. Every template also emits the same
reviewed `.gitignore` baseline for OpenStrata/CMake build trees, package and
install outputs, tool-owned presets, in-source CMake/Ninja spill, Python caches,
compiler indexes, editor state, and OS metadata. Template-specific generated
outputs, such as renderer evidence, extend that baseline explicitly.
Generator-owned compiled-schema baselines carry a nested
`generated/.clang-format` with formatting disabled so recursive editor or CLI
formatting keeps usdGenSchema byte verification authoritative.

The renderer skeleton emits one project-level CMake build/install graph. Its
core, extraction, backend, headless, and optional Hydra 2 and viewport
directories are internal target boundaries, not separate package or plugin
artifacts. Shaders are single-source Slang compiled with the Vulkan SDK's
bundled `slangc`. The generated validator renders a deterministic Vulkan
bootstrap triangle when the capability exists and reports unavailable
GPU/OpenUSD/host work as explained skips. With Hydra enabled, install-tree
tests separate plugin discovery, delegate creation, CPU RenderBuffer
presentation, usdview first frame, and stable update.
`ost renderer view` requests the optional Hydra adapter through the shared build
service, stages its install tree, and opens its smoke scene (or a supplied scene)
in the matching runtime's usdview with the generated renderer selected.
`--build-dir` retains an external/prebuilt escape hatch without claiming OST
build provenance. Developers do not need to hand-compose OpenUSD discovery,
Python, or dynamic-loader paths.
`ost renderer viewport` requests the optional GLFW/Vulkan viewport adapter the
same way and launches it as a native window over the project bootstrap draw;
it needs no OpenUSD runtime and stays outside the validation contract.
The bootstrap proves the seam; generated rendering policy becomes project-owned
source.

The OpenExec skeleton targets OpenUSD 26.05's schema-computation registration
contract. It emits `Info.Exec.Schemas` discovery metadata,
`EXEC_REGISTER_COMPUTATIONS_FOR_SCHEMA`, a deterministic callback seam, and a
versioned dependency on the public schema bundle contract. It deliberately does
not standardize graph construction, scheduling, invalidation, solvers, stage
mutation, or CPU/GPU execution.

The usdview skeleton emits a Python `PluginContainer`, its `plugInfo.json`, and
a smoke stage. It declares `requires.capabilities: [usdview]`; packages retain
the plugin-bundle lifecycle and provenance while advertising
`component.kind: host-addon`. Projects may place an ABI-matched native Python
extension below the generated `python/<module>/` package without hiding the
host integration inside a schema kind.

## Copied CMake helper

Compiled plugin scaffolds include `cmake/OpenStrataPlugin.cmake`. It is a pinned,
self-contained copy of the shared mechanics for:

- selecting a default single-config build type;
- linking the OpenUSD monolith or declared discrete components;
- using a bundle-local, multi-config-safe shared-library output directory;
- generating `plugInfo.json` whose `LibraryPath` names the file the CMake target
  actually builds (see [plugInfo.json layout and library naming](#pluginfojson-layout-and-library-naming));
  and
- installing the library, plugin resources, manifests, provenance, and fixtures;
- optionally exporting a compiled target for installed C++ consumers.

The compiled-schema skeleton additionally exposes `AUTO`, `GENERATE`,
`PREGENERATED`, and `VERIFY` CMake generation modes, commits its OpenUSD 26.05
baseline, exports `<Name>::Schema`, and includes an install-only consumer
fixture. `VERIFY` is the release/CI mode; Python bindings are not claimed by the
initial skeleton.

The generated bundle never loads this helper from the OST source tree and does
not require `ost` at CMake configure/build time. Bundle-specific targets,
OpenUSD components, resource paths, and fixtures remain explicit in the
generated top-level `CMakeLists.txt`.

The canonical helper lives at `templates/_shared/cmake/OpenStrataPlugin.cmake`.
Changing it requires a template-version bump for every catalog entry that copies
the new bytes. Existing generated projects keep their pinned copy; updates are
reviewed as template migrations rather than applied silently.

## plugInfo.json layout and library naming

The manifest's `usd.plug_info` is the only path contract. `ost` puts the
directory that contains it on `PXR_PLUGINPATH_NAME`, so `ost` itself does not
fix a resource layout. C++ templates generate
`plugin/resources/<name>/plugInfo.json`. A bundle may move it, for example to
OpenUSD's per-plugin `plugin/<name>/resources/plugInfo.json`, by changing
`usd.plug_info` and its CMake paths together. Both layouts are three
directories below the bundle root, so the same `LibraryPath` works for either.

Native OpenUSD discovery was checked without `ost` on an extracted
`usd-fileformat-cpp` package, with the source tree removed. `PlugRegistry`
registered and loaded the library, `SdfFileFormat` resolved the extension, and
`UsdStage::Open` read the fixture. Both layouts behaved the same when
`PXR_PLUGINPATH_NAME` named the `plugInfo.json` directory (with or without a
trailing `/`), the file itself, a wildcard, or a parent `plugInfo.json` with
`Includes` (`resources/*/` or `*/resources/`). Neither layout is found from a
bare parent directory with no `Includes`. The current layout has no defect, so
the templates keep it.

`plugInfo.json.in` spells the library as `@OPENSTRATA_PLUGIN_LIBRARY_PATH@`
(helper 1.2.0, used by `usd-fileformat-cpp` 1.3.0). The helper expands that to
the relative path from the `plugInfo.json` directory to the bundle `lib/`, plus
`$<TARGET_FILE_NAME:target>`. `file(GENERATE)` evaluates the result at generate
time, which comes after configure. `OUTPUT_NAME`, `PREFIX`, `SUFFIX` and
`DEBUG_POSTFIX` set on the target are therefore always reflected. On Windows
the name is the DLL, never the import `.lib`.

- **Before the first build.** `ost plugin new` writes a host-specific preview
  (`lib<Name>` plus the host suffix, with `<Name>` taken from the plugin entry)
  so `inspect` and L0 `doctor` work. The first configure overwrites it.
- **Per configuration.** Every configuration gets
  `<build>/openstrata/<target>/<config>/plugInfo.json`. The source-tree file
  that `usd.plug_info` names holds only the primary configuration: the build
  type, or `Release` (overridable with `OPENSTRATA_PLUGIN_PRIMARY_CONFIG`) for
  multi-config generators. Two configurations therefore never write different
  content to one path.
- **Schema Types.** `ost plugin build` still merges co-hosted schema `Types`
  into the source-tree file.
- **Install.** `cmake --install --config <config>` installs the source-tree file
  for the primary configuration and the matching build-tree copy for any other
  configuration. Configure fails if the install `RESOURCE_DESTINATION` is at a
  different depth than the source `plugInfo.json` directory, because then one
  relative `LibraryPath` cannot resolve in both trees.

Inputs that use the legacy `@OPENSTRATA_PLUGIN_LIBRARY_PREFIX@` and
`@CMAKE_SHARED_LIBRARY_SUFFIX@` tokens resolve exactly as before.
`crates/ost-plugin/tests/cmake_helper.rs` tests the helper with real CMake.

**Migrating an existing bundle** is opt-in; `ost` never rewrites a scaffold.

1. Copy helper 1.2.0 over `cmake/OpenStrataPlugin.cmake`.
2. Replace the hand-assembled `LibraryPath` in `plugInfo.json.in` with
   `@OPENSTRATA_PLUGIN_LIBRARY_PATH@`.
3. Reconfigure.

The `plugInfo.json` path, the manifest and the install layout stay the same.
Bundles that keep the 1.1.0 helper keep working unchanged.

## UsdImaging adapters

`usd-imaging-cpp` registers a `UsdImagingAPISchemaAdapter` and declares its schema
provider through `requires.bundles`. The `--schema-type` value is the registered
API schema name, not necessarily its C++ type. Implement the adapter's data
sources and invalidation methods; the skeleton contributes no material data.

Provide `usd-imaging:<apiSchemaName>` or, for a hand-authored prim adapter,
`usd-imaging-prim:<primTypeName>`. L0 requires matching metadata and unique keys;
workspace/session composition rejects multiple owners. Select an imaging SDK
with usdImaging headers and library; the canonical imaging variants of the
`usd` profile work without a viewer or a `lookdev` runtime. Template 0.1.1
requires only `usd-stage-read`; remove the old template's explicit
`hydra-preview` requirement when migrating, unless your plugin needs it.

L2 builds a native registry checker with CMake, Ninja and a C++ compiler, even
when testing an extracted package. It checks schema presence, registry lookup
and adapter construction. Disabled external plugins fail. Scene-index behavior
and rendered images need the plugin's own tests. The skeleton avoids
`hd/retainedDataSource.h`, whose OpenUSD 26.08 constructor spelling fails under
GCC 13 with C++20.
