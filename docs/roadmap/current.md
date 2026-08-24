# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## v0.22.3 - artifact contract hardening

**Status:** 🚧 next milestone · **Depends on:** the v0.22.0-v0.22.2 OpenUSD
artifact, compatibility, provider, distribution and evidence contracts.

This starts the
[v0.22.x runtime-composition series](runtime-composition.md). Before several
artifacts can become one runtime, every needed component must be independently
buildable, packageable, attributable and explicit about its dependencies and
release membership. The milestone is driven by the 2026-08-24 USD VRM
[release-artifact membership report](https://github.com/animu-sphere/usd-vrm-plugins/blob/main/docs/reports/ost/35-2026-08-24-v0.22.2-release-artifact-membership.md).

### P1 acceptance

- `ost library build|test|package` consumes the `requires.libraries` graph it
  validates, so a non-leaf adapter resolves its sibling install prefixes through
  normal CMake package discovery.
- Shared profiles/configuration have a data-only artifact/member contract (or an
  equivalent project-relative install mapping) and ship once under the correct
  owner rather than being copied below one tool.
- Root `ost build` outputs and package members carry coherent managed provenance.

### P2/P3 hardening

- Workspace discovery and aggregate-product membership are separate decisions;
  the project declares or pins the exact release set and packaging prints it.
- OST-upgrade record migrations, missing tool staging and managed-build
  mismatches name their cause and affected member.
- A workstation/CI OST pin mismatch that changes discovery or membership is
  reported before it becomes a release-lane surprise.

### Exit criteria

The USD VRM workspace packages a non-leaf adapter and its shared motion profiles
from declared dependencies, fails on an unexpected aggregate membership change,
and emits member-specific provenance and diagnostics. The complete intake and
the ordered v0.22.4-v0.22.9 slices are in
[runtime-composition.md](runtime-composition.md). DCC host adapters and their
matrix remain deferred to v0.23.0 in the [backlog](backlog.md).

## Active carry-over

- **SEC-002 — symlink escape inside a bundle.** Reject a real in-bundle symlink
  whose canonical target escapes the bundle root.
- **Packaging diagnostic.** Optionally warn when a same-basename PDB is older
  than its DLL; keep it non-fatal until PE/PDB identity can be compared.
