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

The build writes `renderer-report.json`. With Vulkan 1.3 plus `glslc`, the
generated bootstrap path renders a 64x64 offscreen triangle for 1,000 frames,
reads back RGBA8 and depth32 products, captures renderer validation messages,
and records structured device/API/driver identity. Without that capability the
same checks are explicit, explained `SKIP` results; the skeleton never reports a
rendered frame it did not produce.

Run CTest for build-tree and install-tree evidence:

```bash
ctest --test-dir build/<target-id> -C Release --output-on-failure
```

## Build the Hydra 2 adapter

The adapter is opt-in so a default renderer build remains independent of
OpenUSD. Point `CMAKE_PREFIX_PATH` at a matching OpenUSD imaging/usdview SDK:

```bash
cmake -S . -B out-hydra \
  -D{{NAME}}_ENABLE_HYDRA2=ON \
  -DCMAKE_PREFIX_PATH=<openusd-root> \
  -DPython3_EXECUTABLE=<openusd-python>
cmake --build out-hydra --config Release
ctest --test-dir out-hydra -C Release --output-on-failure
```

The Hydra tests independently verify plugin discovery, module loading and
delegate creation, CPU color/depth/id RenderBuffers, and an isolated install-tree
`testusdview` first frame plus USD points update. When the host test passes it
writes `renderer-hydra-report.json` and merges those PASS results into
`renderer-report.json`. It also retains `usdview-first-frame.png` and
`usdview-stable-update.png` under the staged test prefix.

## View in usdview

After the Hydra build above succeeds, open its installed smoke scene with the
adapter selected:

```bash
ost renderer view
```

`ost renderer view` installs the current `out-hydra` build into a private
`.strata/renderer-view/` tree, reads the renderer display name and discovery
directory from the installed `plugInfo.json`, composes the selected real
OpenUSD runtime environment, and launches that runtime's usdview. The renderer
project itself can keep the host-neutral `core` profile; this command defaults
to the same platform with the Hydra-capable `lookdev` profile. Pass
`--profile usd` when a full imaging SDK was adopted under that profile instead.

Open another scene, build tree, configuration, or camera explicitly when needed:

```bash
ost renderer view scenes/shot.usda \
  --build-dir out-hydra --config Release --camera /Camera --profile lookdev
```

The command expects a prior Hydra-enabled CMake build, just as
`ost plugin view` expects a prior plugin build. If the runtime has not been
adopted yet, register the OpenUSD installation used for the build first:

```bash
ost runtime pull <platform> --profile lookdev --from-usd <openusd-root>
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
