# Template catalog

OST embeds its scaffold catalog in the executable so generation is deterministic
and works offline. Every generated scaffold carries `openstrata.scaffold.yaml`
with the template id/version, generator version, and normalized inputs.

## Project and workspace templates

| Template id | Maturity | Command | Purpose |
| --- | --- | --- | --- |
| `cpp-library` | template | `ost init --template cpp-library` | Minimal installable C++ library. |
| `renderer` | skeleton | `ost init --template renderer` | One-project renderer boundaries and headless PASS/FAIL/SKIP evidence. |
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

Skeletons have stable generation and lifecycle seams, but their domain
architecture has not met the promotion evidence required of a template.

The renderer skeleton emits one project-level CMake build/install graph. Its
core, extraction, backend, and headless directories are internal target
boundaries, not separate package or plugin artifacts. The generated validator
passes only checks it actually executes and leaves GPU frame/product assertions
as explained skips until project-owned rendering code replaces the seam.

The OpenExec skeleton targets OpenUSD 26.05's schema-computation registration
contract. It emits `Info.Exec.Schemas` discovery metadata,
`EXEC_REGISTER_COMPUTATIONS_FOR_SCHEMA`, a deterministic callback seam, and a
versioned dependency on the public schema bundle contract. It deliberately does
not standardize graph construction, scheduling, invalidation, solvers, stage
mutation, or CPU/GPU execution.

## Copied CMake helper

Compiled plugin scaffolds include `cmake/OpenStrataPlugin.cmake`. It is a pinned,
self-contained copy of the shared mechanics for:

- selecting a default single-config build type;
- linking the OpenUSD monolith or declared discrete components;
- using a bundle-local, multi-config-safe shared-library output directory;
- configuring `plugInfo.json` with the target platform's library name; and
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
