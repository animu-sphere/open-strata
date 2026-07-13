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

The build writes `renderer-report.json`. The generated skeleton proves its
commit/extraction seam and reports Vulkan availability. GPU frame, color/depth,
and persistence checks remain explicit `SKIP` results until this project adds a
real backend implementation; the skeleton never reports a rendered frame it did
not produce.

Run CTest for build-tree and install-tree evidence:

```bash
ctest --test-dir build/<target-id> -C Release --output-on-failure
```

## Ownership boundary

Public core headers must remain free of OpenUSD, Hydra, Vulkan, Qt, and DCC SDK
types. Host translation belongs in adapters. Rendering, extraction policy,
materials, residency, batching, and GPU synchronization become project-owned
source as soon as the scaffold is generated.
