---
title: Downstream OST report intake
status: active
owners:
  - openstrata-maintainers
created: 2026-09-12
updated: 2026-09-13
applies_to: v0.23.0
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
| [USD VRM Plugins](https://github.com/animu-sphere/usd-vrm-plugins/tree/main/docs/reports/ost) | 43 | Runtime-export correctness shipped in v0.22.10; CI/release and test-attribution work remains below. |
| [hdMerlin](https://github.com/animu-sphere/hydra-merlin/tree/main/docs/reports/ost) | 12 | No open carryover: managed renderer diagnostics and resilient OCI transfer shipped in v0.22.0, with idle-timeout semantics hardened again in v0.22.9. |
| [USD Point Cloud Plugins](https://github.com/animu-sphere/usd-pointcloud-plugins/tree/main/docs/reports/ost) | 4 | No open carryover: structured file-format arguments and managed-output provenance are implemented; the preimplementation report requested no change. |
| [USD 3DGS Plugins](https://github.com/animu-sphere/usd-3dgs-plugins/tree/main/docs/reports/ost) | 3 | No open carryover: package provenance and capability-based profile selection are implemented; the golden-output report requested no roadmap item. |
| [USD HTTP Resolver](https://github.com/animu-sphere/usd-http-resolver/tree/main/docs/reports/ost) | 3 | Offline resolver probing and shared external inputs shipped in v0.22.10; runtime-free CI is complete during v0.23.0 development and lane alignment remains. |
| [USD Stage Runner](https://github.com/animu-sphere/usd-stage-runner/tree/main/docs/reports/ost) | 3 | Stall diagnostics and relocatable Python metadata shipped in v0.22.10; a first-class host add-on remains. |
| [USD Vector Plugins](https://github.com/animu-sphere/usd-vector-plugins/tree/main/docs/reports/ost) | 2 | Package/runtime provenance and semantic lock/lifecycle correctness shipped in v0.22.10. |
| [USD Raster Plugins](https://github.com/animu-sphere/usd-raster-plugins/tree/main/docs/reports/ost) | 1 | Bundle-free workspace graph validation shipped in v0.22.10. |

## v0.23.0 - CI, release and host integration

- Model a truthful first-class `usdview` host add-on component that participates
  in workspace build, package, product composition and capability-aware
  `plugin view` selection. Do not disguise Python/native host integration as a
  codeless schema bundle. Source: [Stage Runner report 03](https://github.com/animu-sphere/usd-stage-runner/blob/main/docs/reports/ost/03-2026-09-02-v0.22.8-usdview-host-plugin-composition.md).
- Extend workspace CI verification beyond graph/build/test with `pyramid` and
  `package`, or generate an equivalent release contract/lane. Source:
  [VRM report 38](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/38-2026-08-30-v0.22.8-workspace-cell-verbs-and-orphaned-lanes.md)
  and [report 39](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/39-2026-09-01-v0.22.8-release-lane-first-execution.md).
- Add an externally managed (`generate = false` or equivalent) cell/mirror
  declaration so hand-authored workflow CLI pins and runtime digests are checked
  against `bootstrap.ost.version` and the support contract. Warn when the local
  developer CLI and CI pin exercise meaningfully different behavior. This
  merges HTTP report 03 with VRM reports 36 and 39.
- Attribute `ost test --json` results to each discovered workspace member and
  warn when a testable member contributes zero tests, also from VRM report 38.

### Completed during v0.23.0 development

- Source workspace cells may omit `runtime_artifact` and select a project
  `[build.intents.*]` declaration. Generated runtime-free jobs are isolated
  from runtime-backed jobs, carry no runtime cache/pull/validation/evidence
  contract, and pass the same intent to `ost build --without-runtime` and
  `ost test --without-runtime`. Source: [HTTP report 01](https://github.com/animu-sphere/usd-http-resolver/blob/main/docs/reports/ost/01-2026-08-16-v0.1.0-ci-without-a-support-matrix.md).
- `ost ci validate`, `ost ci plan`, and `ost ci generate github` now detect an
  OST-generated default workflow that the current matrix no longer emits and
  report `CI_STALE_GENERATED_WORKFLOW` without deleting it. Hand-authored files
  at the same paths are not claimed. Source: VRM report 38.

## Closed through v0.22.10

v0.22.10 closed relocatable exported OpenUSD CMake metadata; package/runtime
lock enforcement; semantic lock checking; mixed root/scoped provenance;
bundle-free graphs; offline bounded resolver evidence; shared declarative CMake
inputs; first-stall snapshots; and the documented nullable product field. These
are release facts in [v0.22.10](../releases/v0.22.10.md).

The audit also verified report-driven work already present before v0.22.10:
required
normalized OpenUSD CI cells and versions; `[[workspace.install_data]].include`;
symlink-escape and stale-PDB protections; exact consumer/runtime identity;
deterministic wheel/npm archives and private loaders; clean-consumer native,
Python and JavaScript probes; installed-library consumer verification; and OCI
body-idle timeout semantics. These are release facts in
[v0.22.9](../releases/v0.22.9.md), not open roadmap entries.
