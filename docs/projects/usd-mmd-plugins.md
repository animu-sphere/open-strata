# USD MMD Plugins

[`animu-sphere/usd-mmd-plugins`](https://github.com/animu-sphere/usd-mmd-plugins)
is the reference MikuMikuDance format workspace. Its PMX file-format bundle
authors a conventional OpenUSD stage; separate libraries read PMX and VMD,
bind motion to a model and keep MMD-specific control semantics. Shared motion
transforms belong to [USD Motion Plugins](usd-motion-plugins.md).

> The project's [capability matrix](https://github.com/animu-sphere/usd-mmd-plugins/blob/main/docs/reference/CAPABILITY_MATRIX.md)
> records PMX import and VMD reading/binding through Phase 7. MMD control
> evaluation, the shared motion adapter and avatar-runtime composition remain
> future work in its [roadmap](https://github.com/animu-sphere/usd-mmd-plugins/blob/main/docs/roadmap/README.md).

## What it tests in OpenStrata

- A mixed workspace of ordinary C++ libraries, inspection tools and an
  `SdfFileFormat` bundle, with plain CMake and OpenStrata build paths.
- Plugin discovery and installed-prefix consumption across the supported CI
  cells, with fixture-backed stage and parser claims.
- The coming cross-repository dependency on `usd-motion-plugins`. That edge
  needs the same version, artifact-digest and provenance contract surfaced by
  the [VRM migration report](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/41-2026-09-19-v0.22.10-a-library-from-another-repository.md).

The [local model and motion reports](https://github.com/animu-sphere/usd-mmd-plugins/tree/main/docs/reports)
validate MMD behavior. They are not v0.22.10 OST dogfooding reports; the
[downstream intake](../roadmap/downstream-report-intake.md) tracks only the
reusable OpenStrata work they or a later OST pass establish.

## Related documentation

- [Repository](https://github.com/animu-sphere/usd-mmd-plugins)
- [Workspace contract](https://github.com/animu-sphere/usd-mmd-plugins/blob/main/docs/architecture/WORKSPACE.md)
- [Project roadmap](https://github.com/animu-sphere/usd-mmd-plugins/blob/main/docs/roadmap/README.md)
- [Reference projects](README.md)
