# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## Next milestone: v0.13.0 — trust policy foundation

**Status:** planned · **Depends on:** v0.10.0 producer verb + v0.12.0 hosted
source-CI runtime contract (both shipped).

With a producer verb and a stable hosted source-CI runtime contract in place,
close the publish-side trust boundary (future-policy §3.2/§7/§11).

**Scope:**

- `openstrata-artifact-policy.toml` with a protected-namespace + allowed-publisher
  schema and a `local` / `unsigned` / `attested` / `verified` / `trusted`
  trust-level enum.
- A policy parser with stable `ARTIFACT_POLICY_*` codes.
- OIDC publisher verification: match repository / workflow path / git ref / actor
  / event against the allowed-publisher list; reject a protected-namespace publish
  from an untrusted identity; `--allow-untrusted-publisher` as the explicit escape
  hatch.
- `ost artifact verify --policy`.

**Tracks:** SEC-006 and the Phase 6 trust-policy hooks.

## Carry-over follow-ups

Small open items not tied to the milestone ladder:

- **Republish the public macOS runtime (from v0.12.0).** Republish the public
  cy2026 macOS arm64 OpenUSD 26.05 SDK with preserved executable bits and confirm
  a clean GitHub-hosted `macos-15-arm64` source-CI lane reaches
  `ost plugin test --up-to 5` with no `chmod` repair; then remove the temporary
  repo-local `actions/setup-python` + `chmod` repairs in the downstream fixture
  repo. Needs a Mac + live GHCR. See [releases/v0.12.0.md](../releases/v0.12.0.md).
- **GHCR push round-trip (from v0.11.0).** Confirm the `ost artifact push`
  user/password path against GHCR end to end. Needs live credentials.
- **SEC-002 follow-up — symlink escape inside a bundle.** Reject a *real* symlink
  within a bundle that resolves outside the root at read time
  (canonicalize-and-contain), complementing the lexical manifest check.
