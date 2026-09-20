# USD Motion Plugins

[`animu-sphere/usd-motion-plugins`](https://github.com/animu-sphere/usd-motion-plugins)
owns the motion values and reusable sampling, recording, retargeting and
`UsdSkelAnimation` bridge shared by avatar formats and live sources. VRMA stays
in [USD VRM Plugins](usd-vrm-plugins.md), VMD in
[USD MMD Plugins](usd-mmd-plugins.md), and device protocols in
[Motion Connectors](motion-connectors.md).

> Its [capability matrix](https://github.com/animu-sphere/usd-motion-plugins/blob/main/docs/reference/CAPABILITY_MATRIX.md)
> is authoritative for implemented behavior. The repository has imported
> `motionCore`, `motionSampling`, `motionRecording`, `motionUsd`, `motionSource`
> and `motionBvh`; retargeting and OpenExec motion remain planned. Imported
> code and a green source CI run do not by themselves establish a tagged,
> independently consumable release.

## What it tests in OpenStrata

- Ordinary native library and tool members, including a library that links
  OpenUSD, built and tested on Windows, macOS and Linux.
- An installed-package consumer that uses a pulled OpenUSD runtime. Its first
  hosted run encountered producer-local Python paths in the existing runtime
  artifact; the project supplied explicit `Python3_*` values to continue.
- A library artifact intended to be consumed by other repositories through a
  declared, versioned, digest-pinned dependency. OpenStrata v0.23.0 adds that
  edge; the VRM migration report records the original blockage, and the
  [post-release intake](../roadmap/downstream-report-intake.md) tracks its
  downstream acceptance.

The source of truth for component identities, support and release timing stays
in the [workspace contract](https://github.com/animu-sphere/usd-motion-plugins/blob/main/docs/architecture/WORKSPACE.md),
[capability matrix](https://github.com/animu-sphere/usd-motion-plugins/blob/main/docs/reference/CAPABILITY_MATRIX.md)
and [project roadmap](https://github.com/animu-sphere/usd-motion-plugins/blob/main/docs/roadmap/README.md).

## Related documentation

- [Repository](https://github.com/animu-sphere/usd-motion-plugins)
- [Cross-repository VRM migration report](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/41-2026-09-19-v0.22.10-a-library-from-another-repository.md)
- [OpenStrata plugin workspace contract](../reference/plugin-workspace.md)
- [Reference projects](README.md)
