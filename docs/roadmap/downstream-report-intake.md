---
title: Downstream OST report intake
status: active
owners:
  - openstrata-maintainers
created: 2026-09-12
updated: 2026-09-26
applies_to: post-v0.23.10
---

# Downstream OST report intake

This plan records the reusable OpenStrata work that remains after auditing the
OST reports in the reference repositories. The 2026-09-12 baseline covered 71
reports. The 2026-09-19 refresh covers 75 reports in nine repositories,
excluding each repository's report index; the four additions exercise
v0.22.10. VRM report 42, received on 2026-09-20, brings the total to 76 and
exposes a premature `consumer-link` claim repaired in v0.23.1. The three new
motion/MMD repositories had no OST report series then. MMD report 01, received
on 2026-09-24, brings the total to 77 and exercises target-local output staging.
VRM report 46 then brings the total to 78 and exposes root CTest's missing
external bundle and tool paths. Report 47 brings the total to 79: v0.23.5
restored the root suite, but a workspace-installed schema bundle's package
failed because its build and package recorded different output trees.
hydra-toon report 01, received on 2026-09-26, brings this OST-series intake to
80 reports and drives the v0.23.7 renderer fixes. Report 02 brings the total
to 81, confirms those fixes and requests the core install-tree evidence repair
shipped in v0.23.8. VRM reports 48–49 bring the total to 83: Windows
packaging is confirmed, and UsdImaging bundle support ships in v0.23.9.
Report 50 brings the total to 84, confirms native registration on Windows and
exposes the profile gate repaired in v0.23.10. Physics has no numbered OST
series; its separate dated adoption and artifact reports are linked below.
MMD's dated model and motion reports test its own format behavior and are not
counted as OST reports. A request was treated as
closed only when the current source, tests or a release record supplied the
contract; repeated observations were merged into one item. Repository-only
fixes, observations that explicitly requested no OpenStrata change, and
superseded failures are not carried forward.

## Audit coverage

| Repository | Reports | Result |
| --- | ---: | --- |
| [USD VRM Plugins](https://github.com/animu-sphere/usd-vrm-plugins/tree/main/docs/reports/ost) | 50 | Reports 40–41 expose per-bundle packaging and cross-repository library dependencies; report 42's early consumer claim is repaired in v0.23.1; report 44's tool edges shipped in v0.23.3; report 45's cache repair and external bundle/tool pins ship in v0.23.4; report 46's root CTest paths ship in v0.23.5; report 48 confirms the v0.23.6 Windows package repair; report 49's imaging kind ships in v0.23.9; report 50's usd profile gate is repaired in v0.23.10. |
| [hdMerlin](https://github.com/animu-sphere/hydra-merlin/tree/main/docs/reports/ost) | 12 | No open carryover: managed renderer diagnostics and resilient OCI transfer shipped in v0.22.0, with idle-timeout semantics hardened again in v0.22.9. |
| [USD Point Cloud Plugins](https://github.com/animu-sphere/usd-pointcloud-plugins/tree/main/docs/reports/ost) | 4 | No open carryover: structured file-format arguments and managed-output provenance are implemented; the preimplementation report requested no change. |
| [USD 3DGS Plugins](https://github.com/animu-sphere/usd-3dgs-plugins/tree/main/docs/reports/ost) | 4 | New report 04 finds a generated Bash empty-array failure on macOS when optional OpenUSD selectors are absent. |
| [USD HTTP Resolver](https://github.com/animu-sphere/usd-http-resolver/tree/main/docs/reports/ost) | 3 | Offline resolver probing and shared external inputs shipped in v0.22.10; runtime-free CI and externally managed lane alignment are complete during v0.23.0 development. |
| [USD Stage Runner](https://github.com/animu-sphere/usd-stage-runner/tree/main/docs/reports/ost) | 3 | Stall diagnostics and relocatable Python metadata shipped in v0.22.10; the first-class usdview host add-on is complete during v0.23.0 development. |
| [USD Vector Plugins](https://github.com/animu-sphere/usd-vector-plugins/tree/main/docs/reports/ost) | 2 | Package/runtime provenance and semantic lock/lifecycle correctness shipped in v0.22.10. |
| [USD Raster Plugins](https://github.com/animu-sphere/usd-raster-plugins/tree/main/docs/reports/ost) | 1 | Bundle-free workspace graph validation shipped in v0.22.10. |
| [USD Geospatial Runtime](https://github.com/animu-sphere/usd-geospatial-runtime/tree/main/docs/reports/ost) | 1 | Its first hosted SDK lane cannot consume the existing pinned OpenUSD runtime's producer-local Python paths. |
| [USD Motion Plugins](https://github.com/animu-sphere/usd-motion-plugins) | 0 | No OST report series; VRM report 41 covers its installed-consumer result and dependency need. |
| [USD MMD Plugins](https://github.com/animu-sphere/usd-mmd-plugins/tree/main/docs/reports/ost) | 1 | Report 01 finds that bundle registration/libraries and tool directories are read from the source tree. Target-local staging ships in v0.23.5 and awaits a downstream rerun. |
| [Motion Connectors](https://github.com/animu-sphere/motion-connectors) | 0 | Empty scaffold; VRM report 41 records the generated-CI blockage. |
| [hydra-toon](https://github.com/animu-sphere/hydra-toon/tree/main/docs/reports/ost) | 2 | Report 02 confirms all five report 01 fixes in published v0.23.7. Its core install-tree verdict repair ships in v0.23.8; downstream rerun remains. |
| [USD Physics Plugins](https://github.com/animu-sphere/usd-physics-plugins/tree/main/docs/reports) | 0 | No numbered OST series. Dated reports prove library packaging and Stage Runner consumption, and expose the external Jolt SDK consumer gap below. |

## Post-v0.23.10 - downstream acceptance

- **P3 — verify core renderer install-tree evidence downstream.**
  [Report 02](https://github.com/animu-sphere/hydra-toon/blob/main/docs/reports/ost/02-2026-09-26-v0.23.7-report-01-reverified.md)
  confirms report 01's five fixes, but the core install-tree CTest writes its
  verdict outside the primary report consumed by validation. Apply template
  0.5.3's script change, then repeat build/test/validate and a no-op build with
  v0.23.8. `renderer.install_tree` must pass with the managed test producer.
  The report's lockfile question is answered in the
  [lock guide](../guides/examples.md#lock--reproducibility); profile override
  behavior is unchanged.
- **P2 — prove isolated physicsJolt consumption with its external SDK closure.**
  The [Physics Phase 3 report](https://github.com/animu-sphere/usd-physics-plugins/blob/main/docs/reports/2026-09-23-phase3-linux-artifacts.md)
  records that generated consumer verification clears `CMAKE_PREFIX_PATH` and
  loses the external Jolt SDK. Its root installed-consumer suite and later
  Stage Runner hosted runs pass, but do not close the isolated consumer gap.
  Model and verify the declared SDK closure without inheriting arbitrary host
  paths; continue this in the [package-contract plan](component-package-contracts.md).

- **P2 — verify target-local bundle and tool stages downstream.**
  [MMD report 01](https://github.com/animu-sphere/usd-mmd-plugins/blob/main/docs/reports/ost/01-2026-09-24-v0.23.3-a-bundle-is-staged-in-its-source-tree.md)
  reproduced a successful bundle build followed by L0–L5 failures when its
  generated `plugInfo.json` and library were moved out of the source tree.
  v0.23.5 stages the installed bundle and tool outputs per target. Run source
  tests and packaging without source-generated outputs, then repeat on another
  target to close downstream acceptance.
- **P2 — adopt and verify UsdImaging bundles downstream.**
  [VRM report 49](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/49-2026-09-26-v0.23.8-no-plugin-kind-for-a-usdimaging-adapter.md)
  drives the `usd-imaging` kind, ownership checks, imaging SDK gate and native
  registry verification in [v0.23.9](../releases/v0.23.9.md). Add the descriptor
  and product membership downstream, then verify the real adapter's scene-index
  behavior and the native Linux/macOS lanes.
  [Report 50](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/50-2026-09-26-v0.23.9-the-imaging-kind-arrives-and-the-usd-profile-cannot-select-it.md)
  confirms Windows native registry construction but finds the intrinsic
  `hydra-preview` requirement blocks canonical `usd` runtimes.
  [v0.23.10](../releases/v0.23.10.md) checks the resolved SDK instead. Remove the
  old explicit capability requirement and update the downstream baseline session
  before repeating the locked workspace product lane.
- **P3 — remove producer-local ELF RUNPATH entries.**
  [VRM report 48](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/48-2026-09-24-v0.23.6-the-product-packages-again.md)
  confirms the Windows product-packaging repair and repeatable digests. Its new
  request is to strip producer paths or rewrite only package-valid `$ORIGIN`
  entries; runtime activation remains responsible for external SDK libraries.
  The old packaging failure still needs Linux/macOS acceptance evidence.

### New v0.22.10 dogfooding acceptance

- **P1 — prove each bundle's own library closure downstream.** v0.23.0
  packages the selected library's installed files and rejects missing declared
  runtime files. Run the same workspace in another bundle build order, the
  packaged consumer and a negative missing-file case across target operating
  systems. [VRM report 40](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/40-2026-09-13-v0.22.10-one-workspace-prefix-for-every-bundle.md)
  measured a product with no `vrmContainer` binary even though packaging
  exited successfully. Its release lane has an ordering workaround and a
  three-OS installed-product check, which do not close the OST defect.
- **P1 — prove the external library artifact edge downstream.** v0.23.0 adds a
  versioned, digest-pinned provider to `requires.libraries`, checks its runtime
  identity and selected closure, records its digest in package/product
  provenance, and pulls it in generated CI. Migrate `usd-motion-plugins`
  `motionCore` → `usd-vrm-plugins` and run one independent installed consumer.
  [VRM report 41](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/41-2026-09-19-v0.22.10-a-library-from-another-repository.md)
  records nine blocked VRM members; MMD and connectors have the same future
  boundary.

  **First exercised 2026-09-20/21.** `usd-motion-plugins` v0.5.0 published
  seven library artifacts on three targets, and `motion-connectors` consumed
  `motionCore` through the declared edge — the declaration, `ost library pull`,
  the archive/identity/runtime checks, the graph edge, the link and the tests
  all held, and an anonymous pull needed no credential. Two defects came out of
  it and are fixed: the consumer-link claim running before the materialized
  prefix is repaired (v0.23.1, VRM report 42) and the root build not composing
  the external prefixes at all (v0.23.2,
  [VRM report 43](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/43-2026-09-20-v0.23.1-the-root-build-cannot-see-an-external-library.md)).
  What remains of this item is the migration itself: nine VRM members switched
  at once, and one independent installed consumer. v0.23.3 also fixes the
  tool-only external edges exposed by
  [VRM report 44](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/44-2026-09-23-v0.23.2-a-tool-edge-reaches-nothing-and-a-tree-keeps-its-runtime.md);
  the downstream rerun remains to be measured.
- **P2 — prove published bundles and tools across repositories.**
  [VRM report 45](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/45-2026-09-23-v0.23.3-a-tool-edge-arrives-and-a-bundle-tree-keeps-its-runtime.md)
  requests artifact pins for `requires.bundles` so `execVrm` can consume the
  published `execMotion` bundle, and a way for tests to run published tools
  instead of storing their output as fixtures. v0.23.4 adds the descriptor,
  graph, artifact, execution and packaging paths. The remaining acceptance is
  a downstream migration and installed-consumer result against those pins.
- **P1 — prove relocatable OpenUSD CMake consumption for the actual pinned
  artifact.** v0.22.10 added relocation to newly exported SDK artifacts; it
  cannot rewrite already published, digest-pinned runtime bytes. A hosted
  external consumer must configure, link and test against a republished
  artifact with both `pxrConfig.cmake` and `pxrTargets.cmake` free of
  producer-local Python paths. Runtime validation must distinguish a
  configure-only check from this linkable consumer claim. v0.23.1 applies
  materialized-prefix repair before both consumer claims; its hosted result
  remains to be measured. The geospatial `sdk-usd` lane remains disabled until
  the artifact or a fully validated materialized-prefix repair passes.
  [Geospatial report 01](https://github.com/animu-sphere/usd-geospatial-runtime/blob/main/docs/reports/ost/01-2026-09-18-v0.22.10-openusd-runtime-python-paths.md),
  [VRM report 37](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/37-2026-08-30-v0.22.6-runtime-python-paths-from-the-producer.md),
  [VRM report 41](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/41-2026-09-19-v0.22.10-a-library-from-another-repository.md),
  and [VRM report 42](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/42-2026-09-20-v0.23.0-a-claim-measured-before-the-repair.md)
  establish the two exported-CMake layers and the new consumer failures.
- **P2 — prove optional OpenUSD flags on hosted macOS.** Generated jobs now
  build optional selectors with positional parameters. Exercise the empty and
  populated paths under the hosted macOS default Bash, including cache-verify
  and remote-pull paths; explicit selectors in the 3DGS matrix are a repository
  workaround until that evidence is available.
  [3DGS report 04](https://github.com/animu-sphere/usd-3dgs-plugins/blob/main/docs/reports/ost/04-2026-09-15-v0.22.10-macos-empty-openusd-args.md).

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
