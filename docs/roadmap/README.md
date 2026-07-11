# Roadmap

The roadmap holds only **incomplete** work. Shipped work lives in
[releases/](../releases/) (per-version records) and, in granular form, in the
[delivery history](../reports/delivery-history.md). Design rationale lives in
[design/](../design/).

Legend: 🚧 in progress · ⬜ not started

| Document | Contents |
| --- | --- |
| [current.md](current.md) | The next milestone and active carry-over work. |
| [backlog.md](backlog.md) | Ordered but unscheduled work: the milestone ladder beyond next, future phases, and cross-cutting open items. |

Delivery is phased and each release is a coherent slice cut from the phases.
Linux x86_64 is the first-class implementation target; other OS targets are
modeled from the start and may be unavailable or partial initially.

When a milestone ships, its detail moves to a [release record](../releases/) and
is removed from here — the roadmap is not a second changelog.

## Quality bar (applies to every phase)

- CLI errors must be actionable.
- All generated manifests must be deterministic.
- Runtime and extension identities always include version + target + digest.
- No hidden environment mutation outside `ost devshell` / `ost env`.
- Every published artifact includes provenance and validation result.
- Every published artifact carries complete third-party attribution (no missing
  upstream licenses/notices).
- OpenStrata must work without a preinstalled Python environment.
