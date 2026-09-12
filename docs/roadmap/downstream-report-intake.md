---
title: Downstream OST report intake
status: active
owners:
  - openstrata-maintainers
created: 2026-09-12
updated: 2026-09-12
applies_to: v0.22.10-v0.23.0
---

# Downstream OST report intake

This plan records the reusable OpenStrata work that remains after auditing every
OST report in the reference repositories. The 2026-09-12 audit covered 71
reports, excluding each repository's report index. A request was treated as
closed only when the current source, tests or a release record supplied the
contract; repeated observations were merged into one item. Repository-only
fixes, observations that explicitly requested no OpenStrata change, and
superseded failures are not carried forward.

## Audit coverage

| Repository | Reports | Result |
| --- | ---: | --- |
| [USD VRM Plugins](https://github.com/animu-sphere/usd-vrm-plugins/tree/main/docs/reports/ost) | 43 | Earlier high-priority asks are closed; reports 37-39 leave runtime-export, CI/release and test-attribution work scheduled below. |
| [hdMerlin](https://github.com/animu-sphere/hydra-merlin/tree/main/docs/reports/ost) | 12 | No open carryover: managed renderer diagnostics and resilient OCI transfer shipped in v0.22.0, with idle-timeout semantics hardened again in v0.22.9. |
| [USD Point Cloud Plugins](https://github.com/animu-sphere/usd-pointcloud-plugins/tree/main/docs/reports/ost) | 4 | No open carryover: structured file-format arguments and managed-output provenance are implemented; the preimplementation report requested no change. |
| [USD 3DGS Plugins](https://github.com/animu-sphere/usd-3dgs-plugins/tree/main/docs/reports/ost) | 3 | No open carryover: package provenance and capability-based profile selection are implemented; the golden-output report requested no roadmap item. |
| [USD HTTP Resolver](https://github.com/animu-sphere/usd-http-resolver/tree/main/docs/reports/ost) | 3 | Runtime-free CI, resolver probing, external dependency inputs and generated/hand-authored lane alignment remain. |
| [USD Stage Runner](https://github.com/animu-sphere/usd-stage-runner/tree/main/docs/reports/ost) | 3 | Stall diagnostics, relocatable Python metadata and a first-class host add-on remain. |
| [USD Vector Plugins](https://github.com/animu-sphere/usd-vector-plugins/tree/main/docs/reports/ost) | 2 | Package/runtime provenance and lock/lifecycle correctness remain. |
| [USD Raster Plugins](https://github.com/animu-sphere/usd-raster-plugins/tree/main/docs/reports/ost) | 1 | Its bundle-free workspace request is shared with the resolver work. |

## v0.22.10 - correctness and diagnostics

### Runtime and package identity (P1)

- Make exported OpenUSD CMake metadata relocatable: `pxrConfig.cmake` and
  `pxrTargets.cmake` must not retain producer-absolute Python paths. This merges
  [VRM report 37](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/37-2026-08-30-v0.22.6-runtime-python-paths-from-the-producer.md)
  and [Stage Runner report 02](https://github.com/animu-sphere/usd-stage-runner/blob/main/docs/reports/ost/02-2026-08-30-v0.22.6-runtime-python-paths.md).
- Before plugin or product packaging and validation, compare the selected and
  packaged runtime digest/source with the project's `strata.lock`; reject a
  mismatch with both identities and remediation. This closes the release gap in
  [Vector report 02](https://github.com/animu-sphere/usd-vector-plugins/blob/main/docs/reports/ost/02-2026-09-06-v0.22.8-release-provenance-dogfooding.md).

### Lock and lifecycle truth (P2/P3)

- Make `ost lock --check` compare the parsed, normalized lock contract rather
  than raw file bytes, while still detecting semantic drift.
- Align `ost lock --check` help and generated reference text with the validation
  exit category (`5`), or deliberately change the implementation and document
  the resulting compatibility impact.
- Reconcile provenance when bundle-scoped `ost plugin build` is followed by a
  root `ost build` that reuses unchanged outputs; the recorded producer must
  describe the lifecycle that actually supplied the package input.

### Workspace and probe diagnostics (P1/P2)

- Allow `ost plugin test --workspace --graph-only` to validate libraries and
  tools when a workspace has zero plugin bundles. This merges
  [HTTP report 01](https://github.com/animu-sphere/usd-http-resolver/blob/main/docs/reports/ost/01-2026-08-16-v0.1.0-ci-without-a-support-matrix.md)
  and [Raster report 01](https://github.com/animu-sphere/usd-raster-plugins/blob/main/docs/reports/ost/01-2026-08-23-v0.22.2-workspace-cells-bundle-free-repository.md).
- Give resolver Level 2 a registration/dispatch assertion that does not require
  a live remote asset. On probe failure, preserve status plus bounded stdout,
  stderr and probe detail even when stderr is empty. This merges
  [HTTP report 02](https://github.com/animu-sphere/usd-http-resolver/blob/main/docs/reports/ost/02-2026-08-18-resolver-bundle-under-the-pyramid.md)
  and [report 03](https://github.com/animu-sphere/usd-http-resolver/blob/main/docs/reports/ost/03-2026-08-18-a-support-matrix-with-one-hand-authored-lane.md).
- Add declarative third-party CMake prefix/toolchain/cache inputs to root and
  scoped library/plugin builds and generated CI, so external dependency lanes
  do not require hand-authored setup.
- At the first managed-build stall, write one bounded diagnostic snapshot with
  the descendant process tree, tool/runtime versions, watched-file metadata,
  log tails and active thresholds. This carries
  [Stage Runner report 01](https://github.com/animu-sphere/usd-stage-runner/blob/main/docs/reports/ost/01-2026-08-30-v0.22.8-windows-ninja-wait.md).
- Document that `.data.product` is `null` unless `--product` is selected, as
  requested by
  [VRM report 39](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/39-2026-09-01-v0.22.8-release-lane-first-execution.md).

## v0.23.0 - CI, release and host integration

- Model a truthful first-class `usdview` host add-on component that participates
  in workspace build, package, product composition and capability-aware
  `plugin view` selection. Do not disguise Python/native host integration as a
  codeless schema bundle. Source: [Stage Runner report 03](https://github.com/animu-sphere/usd-stage-runner/blob/main/docs/reports/ost/03-2026-09-02-v0.22.8-usdview-host-plugin-composition.md).
- Support runtime-free workspace build/test CI cells and let workspace cells
  select project `[build.intents.*]`, carrying
  [HTTP report 01](https://github.com/animu-sphere/usd-http-resolver/blob/main/docs/reports/ost/01-2026-08-16-v0.1.0-ci-without-a-support-matrix.md).
- Extend workspace CI verification beyond graph/build/test with `pyramid` and
  `package`, or generate an equivalent release contract/lane. Source:
  [VRM report 38](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/38-2026-08-30-v0.22.8-workspace-cell-verbs-and-orphaned-lanes.md)
  and [report 39](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/39-2026-09-01-v0.22.8-release-lane-first-execution.md).
- Add an externally managed (`generate = false` or equivalent) cell/mirror
  declaration so hand-authored workflow CLI pins and runtime digests are checked
  against `bootstrap.ost.version` and the support contract. Warn when the local
  developer CLI and CI pin exercise meaningfully different behavior. This
  merges HTTP report 03 with VRM reports 36 and 39.
- Detect stale generated workflow files that are no longer emitted by the
  current support matrix, as reported in VRM report 38.
- Attribute `ost test --json` results to each discovered workspace member and
  warn when a testable member contributes zero tests, also from VRM report 38.

## Closed in v0.22.9

The audit also verified report-driven work already present on `main`: required
normalized OpenUSD CI cells and versions; `[[workspace.install_data]].include`;
symlink-escape and stale-PDB protections; exact consumer/runtime identity;
deterministic wheel/npm archives and private loaders; clean-consumer native,
Python and JavaScript probes; installed-library consumer verification; and OCI
body-idle timeout semantics. These are release facts in
[v0.22.9](../releases/v0.22.9.md), not open roadmap entries.
