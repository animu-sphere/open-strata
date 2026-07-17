# {{name}}

An OpenStrata renderer skeleton generated with
`ost init --template renderer --name {{name}}`.

This is one CMake project with internal target boundaries for the host-neutral
scene model, project-owned extraction seam, Vulkan capability pack, and headless
runtime product. The optional Hydra 2 directory is a co-built runtime module,
not a separate OpenStrata bundle. These directories are not separate packages.

## Build and inspect evidence

```bash
ost runtime pull <platform> --profile core
ost build
ost validate --json
```

The build writes `renderer-report.json`. With Vulkan 1.3 plus `slangc` (the
Vulkan SDK bundles it since 1.3.296; shaders are single-source Slang), the
generated bootstrap path renders a 64x64 offscreen triangle for 1,000 frames,
reads back RGBA8 and depth32 products, captures renderer validation messages,
and records structured device/API/driver identity. Without that capability the
same checks are explicit, explained `SKIP` results; the skeleton never reports a
rendered frame it did not produce.

Run CTest for build-tree and install-tree evidence:

```bash
ctest --test-dir build/<target-id> -C Release --output-on-failure
```

## Standalone viewport

The optional `adapters/viewport` host presents the same bootstrap draw in a
native GLFW window through the backend-owned swapchain. It stays outside the
renderer validation contract — composition evidence remains headless-owned —
and needs no OpenUSD runtime:

```bash
ost renderer viewport
ost renderer viewport -- --frames 8 --hidden --vsync off
```

The command requests the `viewport` intent (`OST_RENDERER_ADAPTERS=viewport`)
from the ordinary managed build service and launches the built executable;
everything after `--` is passed through. GLFW resolves via `find_package` or a
pinned FetchContent fallback, so the first configure may download it. Direct
CMake builds can set `-D<NAME>_ENABLE_VIEWPORT=ON` instead. GLFW types stay
inside the adapter: the backend receives instance extensions plus a
surface-creation callback and owns the `VkSurfaceKHR` and every swapchain
object, which is the boundary a real renderer should keep as input handling,
camera control, and scene hosting grow project-owned.

## Build and view the Hydra 2 adapter

The adapter is opt-in so a default renderer build remains independent of
OpenUSD. Pull or adopt one real imaging/usdview runtime, then let the view loop
select it and incrementally build the adapter through the ordinary OST build
service:

```bash
ost runtime pull cy2026 --profile lookdev --from-usd <openusd-root>
ost renderer view
```

The managed build passes `OST_RENDERER_ADAPTERS=hydra2`, records the selected
runtime fingerprint, and writes atomic build-completion evidence only after
configure, build, and output verification finish.

The Hydra tests independently verify plugin discovery, module loading and
delegate creation, CPU color/depth/id RenderBuffers, and an isolated install-tree
`testusdview` first frame plus USD points update. When the host test passes it
writes `renderer-hydra-report.json` and merges those PASS results into
`renderer-report.json`. It also retains `usdview-first-frame.png` and
`usdview-stable-update.png` under the staged test prefix.

## View in usdview

The command opens the installed smoke scene with the adapter selected:

```bash
ost renderer view
```

`ost renderer view` requests the `hydra2` intent from the common managed build
service, installs the result into a private `.strata/renderer-view/` tree, reads
the renderer display name and discovery directory from the installed
`plugInfo.json`, composes the selected real OpenUSD runtime environment, and
launches that runtime's usdview. The renderer project itself can keep the
host-neutral `core` profile; the command auto-selects a unique pulled real
runtime that provides usdview. Pass `--profile lookdev` or `--profile usd` when
more than one eligible runtime is installed.

Open another scene, configuration, generator, or camera explicitly when needed:

```bash
ost renderer view scenes/shot.usda \
  --config Release --generator Ninja --camera /Camera --profile lookdev
```

An explicit `--build-dir` keeps the external/prebuilt escape hatch. OST does not
rebuild that tree or claim it was produced by `ost build`; it validates the
Hydra option and OpenUSD discovery fingerprint before installing it:

```bash
ost renderer view scenes/shot.usda \
  --build-dir out-hydra --config Release --profile lookdev
```

For manual diagnosis, the equivalent install-tree session is:

```powershell
$env:PXR_PLUGINPATH_NAME = "<stage>/lib/usd/hd{{Name}}/resources"
$env:PYTHONPATH = "<openusd-root>/lib/python"
$env:PATH = "<openusd-root>/bin;<openusd-root>/lib;$env:PATH"
& <openusd-python> <openusd-root>/bin/usdview `
  <stage>/share/{{name}}/tests/usdview-smoke.usda `
  --renderer {{Name}} --camera /Camera
```

The generated adapter intentionally renders the same deterministic bootstrap
triangle used by the headless path. Mesh presence, visibility, and dirty updates
cross the adapter seam, while authored topology/points/camera projection policy
remains an explicit project extension rather than an OST-owned renderer model.

## Ownership boundary

Public core headers must remain free of OpenUSD, Hydra, Vulkan, Qt, and DCC SDK
types. Host translation belongs in adapters. Rendering, extraction policy,
materials, residency, batching, and GPU synchronization become project-owned
source as soon as the scaffold is generated.
