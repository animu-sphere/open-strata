# hydra-toon

[`animu-sphere/hydra-toon`](https://github.com/animu-sphere/hydra-toon) is an
OpenStrata reference **renderer project**, bootstrapped from the `renderer`
template. Its intended role is low-latency avatar rendering through `hdToon`,
with a host-neutral core and a Vulkan backend.

## Current evidence

The [first OST report](https://github.com/animu-sphere/hydra-toon/blob/main/docs/reports/ost/01-2026-09-26-v0.23.6-renderer-template-bootstrap.md)
records Windows 11/MSVC, NVIDIA RTX A5000 and OpenUSD 26.08 bootstrap evidence:
headless GPU checks, managed CTest, Hydra discovery, `testusdview` first-frame
and update checks, and a hidden eight-frame standalone viewport. The Hydra
build required a local API correction, and the Japanese Windows host required
UTF-8 compiler flags. Those template fixes ship in [v0.23.7](../releases/v0.23.7.md),
alongside no-op build evidence retention and viewport validation isolation.

This is a working bootstrap triangle, not implemented avatar rendering.
Mesh rendering, skinning, MToon/MMD materials, late motion latching and WebGPU
remain downstream work. The repository's
[capability matrix](https://github.com/animu-sphere/hydra-toon/blob/main/docs/reference/CAPABILITY_MATRIX.md)
and [roadmap](https://github.com/animu-sphere/hydra-toon/blob/main/docs/roadmap/current.md)
own that distinction.

## What it proves about OpenStrata

- A second renderer repository can adopt the project template and preserve
  its own core, backend and adapter CMake boundaries.
- Managed build/test evidence must survive unchanged incremental builds while
  retaining the original producer and digest.
- A viewport build needs its own durable launch record and a selectable
  validation target: `ost validate --intent renderer-viewport`.
- Template compatibility includes current OpenUSD APIs and non-English MSVC
  hosts, not only a successful core build.

The integration boundary is the composed USD stage. Source formats and motion
semantics remain with VRM, MMD and Motion Plugins; device input belongs to
Motion Connectors. Avatar renderer features are not OpenStrata responsibilities.
See the downstream
[integration policy](https://github.com/animu-sphere/hydra-toon/blob/main/docs/design/INTEGRATION_SCOPE_POLICY.md).

## Related documentation

- [Adopt a renderer project](../guides/adopt-a-renderer-project.md).
- [hdMerlin](hydra-merlin.md), the general renderer reference.
- [Reference projects](README.md) and [downstream intake](../roadmap/downstream-report-intake.md).
- Downstream [building guide](https://github.com/animu-sphere/hydra-toon/blob/main/docs/guides/BUILDING.md).
