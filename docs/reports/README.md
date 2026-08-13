# Reports

Evidence from real runs — incidents, dogfooding, compatibility, and validation
results. Reports are evidence, not normative specification: they must not be used
as the sole source for current behavior (that is [architecture/](../architecture/)
and [reference/](../reference/)).

| Document | Purpose |
| --- | --- |
| [2026-07-14 v0.17.0 managed renderer view acceptance](2026-07-14-v0.17.0-managed-renderer-view-hydra-merlin.md) | Windows hdMerlin dogfooding of managed view, Hydra host tests, external builds, and report conflict policy. |
| [USD 3DGS report #1 — bootstrap](https://github.com/animu-sphere/usd-3dgs-plugins/blob/main/docs/reports/ost/01-2026-07-18-v0.18.0-bootstrap.md) | Empty repository through scaffold, ordinary-library composition, source L5, package, and package-origin verification. |
| [USD 3DGS report #2 — package provenance and reproducibility](https://github.com/animu-sphere/usd-3dgs-plugins/blob/main/docs/reports/ost/02-2026-07-19-package-provenance-and-reproducibility.md) | Clean extracted-package consumption, Windows reproducibility, and package-time build-provenance feedback. |
| [USD Point Cloud report #1 — PLY FileFormat CI](https://github.com/animu-sphere/usd-pointcloud-plugins/blob/main/docs/reports/ost/01-2026-08-11-v0.22.0-ply-fileformat-ci.md) | Three-platform PLY source CI, standalone dependency closure, strict CRS arguments, and the smoke-fixture format-argument ask. |
| [incident-notes.md](incident-notes.md) | Short debugging notes: incidents, root causes, fixes, and future guardrails. |

Additional dogfooding evidence remains in downstream validation repositories and
is backfilled here as the reorganization proceeds. The two v0.17.0 passes that
drove the v0.18.0 fix-release plan live downstream:
`2026-07-15-v0.17.0-dogfooding-v0.18.0-asks.md` (`animu-sphere/hydra-merlin`)
and `22-2026-07-17-v0.17.0-evidence-gate-v0.18.0-asks.md`
(`animu-sphere/usd-vrm-plugins`).

The adopted `animu-sphere/usd-3dgs-plugins` and the newly adopted
`animu-sphere/usd-pointcloud-plugins` keep their own append-only OST report
series ([3DGS](https://github.com/animu-sphere/usd-3dgs-plugins/tree/main/docs/reports/ost),
[point cloud](https://github.com/animu-sphere/usd-pointcloud-plugins/tree/main/docs/reports/ost)).
Open upstream findings are summarized in their reference-project pages
([3DGS](../projects/usd-3dgs-plugins.md),
[point cloud](../projects/usd-pointcloud-plugins.md)) and tracked in the active
[roadmap](../roadmap/current.md), rather than copied into a second normative
source.
