# Reports

Evidence from real runs — incidents, dogfooding, compatibility, and validation
results. Reports are evidence, not normative specification: they must not be used
as the sole source for current behavior (that is [architecture/](../architecture/)
and [reference/](../reference/)).

| Document | Purpose |
| --- | --- |
| [2026-07-14 v0.17.0 managed renderer view acceptance](2026-07-14-v0.17.0-managed-renderer-view-hydra-merlin.md) | Windows hdMerlin dogfooding of managed view, Hydra host tests, external builds, and report conflict policy. |
| [incident-notes.md](incident-notes.md) | Short debugging notes: incidents, root causes, fixes, and future guardrails. |

Additional dogfooding evidence remains in downstream validation repositories and
is backfilled here as the reorganization proceeds. The two v0.17.0 passes that
drove the v0.18.0 fix-release plan live downstream:
`2026-07-15-v0.17.0-dogfooding-v0.18.0-asks.md` (`animu-sphere/hydra-merlin`)
and `22-2026-07-17-v0.17.0-evidence-gate-v0.18.0-asks.md`
(`animu-sphere/usd-vrm-plugins`).
