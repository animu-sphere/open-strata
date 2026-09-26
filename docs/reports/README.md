# Reports

Evidence from real runs — incidents, dogfooding, compatibility, and validation
results. Reports are evidence, not normative specification: they must not be used
as the sole source for current behavior (that is [architecture/](../architecture/)
and [reference/](../reference/)).

| Document | Purpose |
| --- | --- |
| [2026-09-26 Imaging on the usd profile](2026-09-26-v0.23.10-imaging-usd-profile.md) | Locked published usd runtime: native SDK checks, workspace product and extracted-package verification. |
| [2026-09-26 UsdImaging verification](2026-09-26-v0.23.9-usd-imaging.md) | Native adapter registration, schema closure, disabled-plugin rejection and package evidence. |
| [2026-09-26 v0.23.7 renderer bootstrap](2026-09-26-v0.23.7-renderer-bootstrap.md) | Fresh template on Windows/MSVC and RTX A5000: repeated-build evidence, isolated viewport validation and OpenUSD 26.08 Hydra host tests. |
| [hydra-toon report #1 — v0.23.6 renderer bootstrap](https://github.com/animu-sphere/hydra-toon/blob/main/docs/reports/ost/01-2026-09-26-v0.23.6-renderer-template-bootstrap.md) | Real Windows GPU/Hydra bootstrap, no-op evidence loss, viewport validation and template corrections; primary v0.23.7 intake. |
| [USD Physics Phase 3 — Linux artifacts](https://github.com/animu-sphere/usd-physics-plugins/blob/main/docs/reports/2026-09-23-phase3-linux-artifacts.md) | Published library artifacts, empty-store verification, Stage Runner consumption and the external SDK consumer gap. |
| [2026-08-26 USD VRM CI render prerequisites](2026-08-26-usd-vrm-ci-render-prerequisites.md) | OST 0.22.5 hosted Windows/macOS failures, Qt prerequisite skips, legacy WGL fallback, and local before/after evidence. |
| [2026-07-14 v0.17.0 managed renderer view acceptance](2026-07-14-v0.17.0-managed-renderer-view-hydra-merlin.md) | Windows hdMerlin dogfooding of managed view, Hydra host tests, external builds, and report conflict policy. |
| [USD 3DGS report #1 — bootstrap](https://github.com/animu-sphere/usd-3dgs-plugins/blob/main/docs/reports/ost/01-2026-07-18-v0.18.0-bootstrap.md) | Empty repository through scaffold, ordinary-library composition, source L5, package, and package-origin verification. |
| [USD 3DGS report #2 — package provenance and reproducibility](https://github.com/animu-sphere/usd-3dgs-plugins/blob/main/docs/reports/ost/02-2026-07-19-package-provenance-and-reproducibility.md) | Clean extracted-package consumption, Windows reproducibility, and package-time build-provenance feedback. |
| [USD Point Cloud report #1 — PLY FileFormat CI](https://github.com/animu-sphere/usd-pointcloud-plugins/blob/main/docs/reports/ost/01-2026-08-11-v0.22.0-ply-fileformat-ci.md) | Three-platform PLY source CI, standalone dependency closure, strict CRS arguments, and the smoke-fixture format-argument ask. |
| [USD Point Cloud report #4 — HTTP resolver pre-implementation](https://github.com/animu-sphere/usd-pointcloud-plugins/blob/main/docs/reports/ost/04-2026-08-16-usd-http-resolver-preimplementation.md) | Historical resolver-skeleton evidence; the missing bundle/backend/CI/tests later shipped downstream and the report requested no OST change. |
| [USD VRM report #35 — v0.22.2 release artifact membership](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/35-2026-08-24-v0.22.2-release-artifact-membership.md) | Non-leaf library composition, shared data ownership, explicit product membership, managed provenance, and workstation/CI OST-pin drift; primary v0.22.3 intake. |
| [USD VRM report #40 — v0.22.10 workspace prefix](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/40-2026-09-13-v0.22.10-one-workspace-prefix-for-every-bundle.md) | Build-order-dependent bundle package closure, missing shared library binary, and installed-product evidence. |
| [USD VRM report #41 — v0.22.10 external library](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/41-2026-09-19-v0.22.10-a-library-from-another-repository.md) | Missing cross-repository library edge, empty-scaffold CI, and the installed OpenUSD consumer result. |
| [USD 3DGS report #4 — v0.22.10 macOS generated CI](https://github.com/animu-sphere/usd-3dgs-plugins/blob/main/docs/reports/ost/04-2026-09-15-v0.22.10-macos-empty-openusd-args.md) | Empty optional OpenUSD Bash array under `set -u`, and the repository-side selector workaround. |
| [USD Geospatial Runtime report #1 — v0.22.10 SDK consumer](https://github.com/animu-sphere/usd-geospatial-runtime/blob/main/docs/reports/ost/01-2026-09-18-v0.22.10-openusd-runtime-python-paths.md) | A hosted SDK lane exposes producer-local Python paths in its existing pinned OpenUSD runtime. |
| [USD MMD Plugins report #1 — v0.23.3 bundle staging](https://github.com/animu-sphere/usd-mmd-plugins/blob/main/docs/reports/ost/01-2026-09-24-v0.23.3-a-bundle-is-staged-in-its-source-tree.md) | A standalone bundle and workspace tools need target-local build outputs for test and package. |
| [incident-notes.md](incident-notes.md) | Short debugging notes: incidents, root causes, fixes, and future guardrails. |

Additional dogfooding evidence remains in downstream validation repositories and
is backfilled here as the reorganization proceeds. The two v0.17.0 passes that
drove the v0.18.0 fix-release plan live downstream:
`2026-07-15-v0.17.0-dogfooding-v0.18.0-asks.md` (`animu-sphere/hydra-merlin`)
and `22-2026-07-17-v0.17.0-evidence-gate-v0.18.0-asks.md`
(`animu-sphere/usd-vrm-plugins`).

Reference repositories retain their own dated evidence; the
[downstream report intake](../roadmap/downstream-report-intake.md#audit-coverage)
lists every audited OST report series and repositories with no numbered series.
Open upstream findings are summarized in the [reference-project pages](../projects/)
and tracked in the active [roadmap](../roadmap/current.md), rather than copied
into a second normative source. The full report count and deduplicated
v0.22.10 intake are in [downstream-report-intake.md](../roadmap/downstream-report-intake.md).
