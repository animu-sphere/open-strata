# USD Physics Plugins

[`animu-sphere/usd-physics-plugins`](https://github.com/animu-sphere/usd-physics-plugins)
is an OpenStrata reference **ordinary-library workspace** for backend-neutral
physics. It separates the `physicsCore` contract from the `physicsJolt` backend;
OpenUSD, source-format semantics and application policy stay outside the core.

## Current evidence

The repository builds and installs both libraries. The Jolt-required named
intent has local and hosted Windows/Linux evidence, including clean-prefix
consumers. The
[Phase 3 artifact report](https://github.com/animu-sphere/usd-physics-plugins/blob/main/docs/reports/2026-09-23-phase3-linux-artifacts.md)
records digest-pinned Linux packages, empty-store OCI retrieval with SBOM and
provenance checks, and a subsequent Stage Runner hosted consumer run with
46 passing tests on each of Windows and Linux.

These are consumer inputs, not a tagged release. `physicsUsd`, secondary-motion
components and a schema bundle are not present. The downstream
[capability matrix](https://github.com/animu-sphere/usd-physics-plugins/blob/main/docs/reference/CAPABILITY_MATRIX.md)
owns the supported physics surface; dated reports qualify newer consumer results.

## What it proves about OpenStrata

- A bundle-free workspace with ordinary CMake package dependencies.
- Root `ost build` / `ost test` and the `jolt` build intent.
- Isolated library build/test/package and installed-consumer checks.
- Artifact identity, attribution and cross-repository library consumption.

The Phase 3 report also records a remaining acceptance gap: the generated
`physicsJolt` consumer cannot find its external Jolt SDK after
`CMAKE_PREFIX_PATH` is cleared. The root installed-consumer test supplies that
evidence today; it does not close the isolated-consumer gap. This remains in
the [downstream intake](../roadmap/downstream-report-intake.md).

## Related documentation

- Downstream [workspace contract](https://github.com/animu-sphere/usd-physics-plugins/blob/main/docs/architecture/WORKSPACE.md)
  and [reports](https://github.com/animu-sphere/usd-physics-plugins/tree/main/docs/reports).
- [USD Stage Runner](usd-stage-runner.md), the first application consumer.
- [USD MMD Plugins](usd-mmd-plugins.md) and [USD VRM Plugins](usd-vrm-plugins.md),
  which retain their format-owned physics semantics.
- [Reference projects](README.md).
