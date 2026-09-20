---
title: Downstream OST report intake
status: active
owners:
  - openstrata-maintainers
created: 2026-09-12
updated: 2026-09-20
applies_to: v0.23.0
---

# Downstream OST report intake

This plan records the reusable OpenStrata work that remains after auditing the
OST reports in the reference repositories. The 2026-09-12 baseline covered 71
reports. The 2026-09-19 refresh covers 75 reports in nine repositories,
excluding each repository's report index; the four additions exercise
v0.22.10. The three new motion/MMD repositories have no OST report series yet.
MMD's dated model and motion reports test its own format behavior and are not
counted as OST reports. A request was treated as
closed only when the current source, tests or a release record supplied the
contract; repeated observations were merged into one item. Repository-only
fixes, observations that explicitly requested no OpenStrata change, and
superseded failures are not carried forward.

## Audit coverage

| Repository | Reports | Result |
| --- | ---: | --- |
| [USD VRM Plugins](https://github.com/animu-sphere/usd-vrm-plugins/tree/main/docs/reports/ost) | 45 | New reports 40–41 expose per-bundle packaging from a shared prefix and missing cross-repository library dependencies; details below. |
| [hdMerlin](https://github.com/animu-sphere/hydra-merlin/tree/main/docs/reports/ost) | 12 | No open carryover: managed renderer diagnostics and resilient OCI transfer shipped in v0.22.0, with idle-timeout semantics hardened again in v0.22.9. |
| [USD Point Cloud Plugins](https://github.com/animu-sphere/usd-pointcloud-plugins/tree/main/docs/reports/ost) | 4 | No open carryover: structured file-format arguments and managed-output provenance are implemented; the preimplementation report requested no change. |
| [USD 3DGS Plugins](https://github.com/animu-sphere/usd-3dgs-plugins/tree/main/docs/reports/ost) | 4 | New report 04 finds a generated Bash empty-array failure on macOS when optional OpenUSD selectors are absent. |
| [USD HTTP Resolver](https://github.com/animu-sphere/usd-http-resolver/tree/main/docs/reports/ost) | 3 | Offline resolver probing and shared external inputs shipped in v0.22.10; runtime-free CI and externally managed lane alignment are complete during v0.23.0 development. |
| [USD Stage Runner](https://github.com/animu-sphere/usd-stage-runner/tree/main/docs/reports/ost) | 3 | Stall diagnostics and relocatable Python metadata shipped in v0.22.10; the first-class usdview host add-on is complete during v0.23.0 development. |
| [USD Vector Plugins](https://github.com/animu-sphere/usd-vector-plugins/tree/main/docs/reports/ost) | 2 | Package/runtime provenance and semantic lock/lifecycle correctness shipped in v0.22.10. |
| [USD Raster Plugins](https://github.com/animu-sphere/usd-raster-plugins/tree/main/docs/reports/ost) | 1 | Bundle-free workspace graph validation shipped in v0.22.10. |
| [USD Geospatial Runtime](https://github.com/animu-sphere/usd-geospatial-runtime/tree/main/docs/reports/ost) | 1 | Its first hosted SDK lane cannot consume the existing pinned OpenUSD runtime's producer-local Python paths. |
| [USD Motion Plugins](https://github.com/animu-sphere/usd-motion-plugins) | 0 | No OST report series; VRM report 41 covers its installed-consumer result and dependency need. |
| [USD MMD Plugins](https://github.com/animu-sphere/usd-mmd-plugins) | 0 | Local PMX/VMD reports are domain evidence; cross-repository motion dependency awaits a dedicated OST pass. |
| [Motion Connectors](https://github.com/animu-sphere/motion-connectors) | 0 | Empty scaffold; VRM report 41 records the generated-CI blockage. |

## v0.23.0 - CI, release and host integration

### New v0.22.10 dogfooding acceptance

- **P1 — compose and verify each bundle's own library closure.** Workspace
  package and test must not read a shared prefix left by the last individual
  bundle build. Package only the selected library's installed files, verify
  each recorded runtime file exists before writing `dependencies.json`, and
  make package and product verification fail when a declared shared library is
  absent. The same workspace must produce identical valid packages regardless
  of bundle build order; run the packaged consumer and a negative missing-file
  case. [VRM report 40](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/40-2026-09-13-v0.22.10-one-workspace-prefix-for-every-bundle.md)
  measured a product with no `vrmContainer` binary even though packaging
  exited successfully. Its release lane has an ordering workaround and a
  three-OS installed-product check, which do not close the OST defect.
- **P1 — declare external library artifacts in the workspace graph.** Extend
  `requires.libraries` with a versioned, digest-pinned provider from another
  repository. Materialize its declared closure for builds and installed
  consumers; validate version and runtime identity; record its digest in
  dependency/product provenance; and render the pull in CI. A missing or wrong
  artifact must fail the graph or materialization, without relying on ambient
  `CMAKE_PREFIX_PATH`. Prove the `usd-motion-plugins` `motionCore` →
  `usd-vrm-plugins` migration and one independent consumer.
  [VRM report 41](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/41-2026-09-19-v0.22.10-a-library-from-another-repository.md)
  records nine blocked VRM members; MMD and connectors have the same future
  boundary.
- **P1 — prove relocatable OpenUSD CMake consumption for the actual pinned
  artifact.** v0.22.10 added relocation to newly exported SDK artifacts; it
  cannot rewrite already published, digest-pinned runtime bytes. A hosted
  external consumer must configure, link and test against a republished
  artifact with both `pxrConfig.cmake` and `pxrTargets.cmake` free of
  producer-local Python paths. Runtime validation must distinguish a
  configure-only check from this linkable consumer claim. The geospatial
  `sdk-usd` lane remains disabled until that artifact or a fully validated
  materialized-prefix repair passes.
  [Geospatial report 01](https://github.com/animu-sphere/usd-geospatial-runtime/blob/main/docs/reports/ost/01-2026-09-18-v0.22.10-openusd-runtime-python-paths.md),
  [VRM report 37](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/37-2026-08-30-v0.22.6-runtime-python-paths-from-the-producer.md),
  and [VRM report 41](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/41-2026-09-19-v0.22.10-a-library-from-another-repository.md)
  establish the two exported-CMake layers and the new consumer failures.
- **P2 — prove optional OpenUSD flags on hosted macOS.** Generated jobs now
  build optional selectors with positional parameters. Exercise the empty and
  populated paths under the hosted macOS default Bash, including cache-verify
  and remote-pull paths; explicit selectors in the 3DGS matrix are a repository
  workaround until that evidence is available.
  [3DGS report 04](https://github.com/animu-sphere/usd-3dgs-plugins/blob/main/docs/reports/ost/04-2026-09-15-v0.22.10-macos-empty-openusd-args.md).
- **P3 — product path identity.** In a product activation, emit each bundle
  identity once after checking identity/version/contract agreement; a package
  must not gain another bundle's runtime files through a shared directory.
  Sources: [VRM report 41](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/41-2026-09-19-v0.22.10-a-library-from-another-repository.md)
  and [report 40](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/40-2026-09-13-v0.22.10-one-workspace-prefix-for-every-bundle.md).

### Completed during v0.23.0 development

- Aggregate product verification checks that an internal bundle dependency's
  recorded version, kind and schema contract match the product member. Product
  activation emits that member's plugin and library paths once while preserving
  the individual package's standalone dependency paths. This closes the
  product-path identity item above.
- Workspace library builds retain member-specific install snapshots. Packaging
  reads each selected library's own recorded files and fails if an inventoried
  file is missing. A library may declare OS-specific `runtime.required_files`;
  library build and plugin packaging then fail if its own install omits a required shared
  library. Packaged dependency evidence records each library's exact files and
  required runtime files, and product verification checks those paths against
  the member archive and package inventory. Clean installed-product loadability
  remains part of the bundle-closure acceptance above.
- Runtime validation now records a separate `consumer-link` result after
  `consumer-configure`: a scratch C++ consumer builds against the selected
  OpenUSD CMake export and runs under the runtime loader environment. Generated
  source CI invokes the same validation and retains its JSON report. A hosted
  clean-host run against a republished, digest-pinned artifact remains open.
- Generated source and release lanes now pass optional OpenUSD selectors through
  Bash positional parameters, which work when the selectors are empty under
  `set -u`. The source cache verification, remote pull and release candidate
  gates use the same form. Local empty and populated shell cases pass; hosted
  macOS execution remains an open acceptance check.
- An explicitly empty `[workspace]` (`members = []`) validates as a zero-member
  graph. The graph-only source CI rung can omit the runtime artifact and does
  not run build, test or package. Undeclared descriptors still fail discovery.
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

v0.22.10 closed relocatable OpenUSD CMake metadata for newly exported SDK
artifacts; previously published pinned artifacts still require separate
consumer evidence as above. The release also closed package/runtime lock
enforcement; semantic lock checking; mixed root/scoped provenance;
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
