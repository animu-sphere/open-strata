# {{name}}

An OpenStrata renderer skeleton generated with
`ost init --template renderer --name {{name}}`.

This is one CMake project with internal target boundaries for the host-neutral
scene model, project-owned extraction seam, Vulkan capability pack, and headless
runtime product. These directories are not separate packages or plugin bundles.

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

## Ownership boundary

Public core headers must remain free of OpenUSD, Hydra, Vulkan, Qt, and DCC SDK
types. Host translation belongs in adapters. Rendering, extraction policy,
materials, residency, batching, and GPU synchronization become project-owned
source as soon as the scaffold is generated.
