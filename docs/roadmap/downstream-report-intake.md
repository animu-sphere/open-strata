---
title: Downstream OST report intake
status: active
owners:
  - openstrata-maintainers
created: 2026-09-12
updated: 2026-09-15
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
| [USD HTTP Resolver](https://github.com/animu-sphere/usd-http-resolver/tree/main/docs/reports/ost) | 3 | Offline resolver probing and shared external inputs shipped in v0.22.10; runtime-free CI and externally managed lane alignment are complete during v0.23.0 development. |
| [USD Stage Runner](https://github.com/animu-sphere/usd-stage-runner/tree/main/docs/reports/ost) | 3 | Stall diagnostics and relocatable Python metadata shipped in v0.22.10; the first-class usdview host add-on is complete during v0.23.0 development. |
| [USD Vector Plugins](https://github.com/animu-sphere/usd-vector-plugins/tree/main/docs/reports/ost) | 2 | Package/runtime provenance and semantic lock/lifecycle correctness shipped in v0.22.10. |
| [USD Raster Plugins](https://github.com/animu-sphere/usd-raster-plugins/tree/main/docs/reports/ost) | 1 | Bundle-free workspace graph validation shipped in v0.22.10. |

## v0.23.0 - CI, release and host integration

### Completed during v0.23.0 development

- `usdview-plugin` is a truthful source bundle kind and packages as the
  first-class `host-addon` component kind. It participates in workspace build,
  package/product composition, activation, managed-output provenance and
  Formation resolution. The `usdview` capability drives `lookdev` selection;
  `plugin view`, `test-view`, and Level 6 include the full composed bundle
  closure. Static validation checks the Python `PluginContainer` registration,
  L2 imports it through the real OpenUSD registry, and L6 launches the host.
  The embedded `usdview-plugin-python` scaffold replaces the codeless-schema
  workaround. Source: [Stage Runner report 03](https://github.com/animu-sphere/usd-stage-runner/blob/main/docs/reports/ost/03-2026-09-02-v0.22.8-usdview-host-plugin-composition.md).

- `ost test --json` attributes selected CTest case counts to each discovered,
  testable workspace member by project-relative root. Unfiltered runs emit
  `WORKSPACE_MEMBER_NO_TESTS` for zero-count members, and the managed completion
  record retains the same attribution. Source: VRM report 38.
- `external_workflows` binds each declared hand-authored workflow to named
  matrix cells. `ost ci validate` checks its `OST_VERSION` against
  `bootstrap.ost.version` and requires each projected or exact-literal
  `OST_CI_*` binding to be consumed by a workflow step; `--support` applies the
  same public declaration to those cells. Local/CI CLI skew is reported as
  `CI_BOOTSTRAP_VERSION_SKEW`. This merges HTTP report 03 with VRM reports 36
  and 39.
- Workspace source cells accept cumulative `verify: pyramid` and
  `verify: package` rungs. They run the whole-workspace plugin pyramid at the
  declared `up_to` level, then package every member and the aggregate product,
  so generated lanes can exercise the same verbs as a release without
  bundles-times-platforms duplicate cells. Source: VRM reports 38 and 39.
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
