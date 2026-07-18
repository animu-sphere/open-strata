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
| [incident-notes.md](incident-notes.md) | Short debugging notes: incidents, root causes, fixes, and future guardrails. |

Additional dogfooding evidence remains in downstream validation repositories and
is backfilled here as the reorganization proceeds. The two v0.17.0 passes that
drove the v0.18.0 fix-release plan live downstream:
`2026-07-15-v0.17.0-dogfooding-v0.18.0-asks.md` (`animu-sphere/hydra-merlin`)
and `22-2026-07-17-v0.17.0-evidence-gate-v0.18.0-asks.md`
(`animu-sphere/usd-vrm-plugins`).

The newly adopted `animu-sphere/usd-3dgs-plugins` keeps its own append-only
[OST report series](https://github.com/animu-sphere/usd-3dgs-plugins/tree/main/docs/reports/ost).
Its open upstream findings are summarized in the
[reference-project page](../projects/usd-3dgs-plugins.md) and tracked in the
[v0.19.0 roadmap](../roadmap/current.md), rather than copied into a second
normative source.
