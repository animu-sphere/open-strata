---
title: Downstream OST report intake
status: active
owners:
  - openstrata-maintainers
created: 2026-09-12
updated: 2026-09-21
applies_to: post-v0.23.3
---

# Downstream OST report intake

This plan records the reusable OpenStrata work that remains after auditing the
OST reports in the reference repositories. The 2026-09-12 baseline covered 71
reports. The 2026-09-19 refresh covers 75 reports in nine repositories,
excluding each repository's report index; the four additions exercise
v0.22.10. VRM report 42, received on 2026-09-20, brings the total to 76 and
exposes a premature `consumer-link` claim repaired in v0.23.1. The three new
motion/MMD repositories have no OST report series yet.
MMD's dated model and motion reports test its own format behavior and are not
counted as OST reports. A request was treated as
closed only when the current source, tests or a release record supplied the
contract; repeated observations were merged into one item. Repository-only
fixes, observations that explicitly requested no OpenStrata change, and
superseded failures are not carried forward.

## Audit coverage

| Repository | Reports | Result |
| --- | ---: | --- |
| [USD VRM Plugins](https://github.com/animu-sphere/usd-vrm-plugins/tree/main/docs/reports/ost) | 46 | Reports 40–41 expose per-bundle packaging and cross-repository library dependencies; report 42's early consumer claim is repaired in v0.23.1, and report 44's tool edges and runtime cache are repaired in v0.23.3. |
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

## Post-v0.23.0 - downstream acceptance

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
