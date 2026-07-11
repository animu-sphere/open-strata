# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## Next milestone: v0.14.0 — trust policy foundation

**Status:** in progress · **Depends on:** v0.10.0 producer verb + v0.13.0
release-quality artifacts (reproducible, lean, testable-from-package; all
shipped).

With a producer verb and reproducible, lean artifacts in place, close the
publish-side trust boundary (future-policy §3.2/§7/§11) — trust-level enforcement
on top of the v0.13.0 artifacts.

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

### Release-lane dogfood hardening

The usd-vrm-plugins v0.13.0 release-lane report found three small correctness
gaps that should close with v0.14 without changing its trust-policy objective:

- recursively reject unknown `openstrata.ci.yaml` keys instead of silently
  dropping a misspelled verification/publication control;
- emit a native Windows directory to `GITHUB_PATH` from the generated Bash
  bootstrap and prove a native child process can launch `ost`;
- make `ost plugin run -- python ...` resolve an absolute interpreter matching
  the runtime Python ABI, or fail before launch with an actionable diagnostic.

An atomic helper for updating `bootstrap.ost.version` plus exact per-target
checksums is a v0.14 non-blocking target. Tag-triggered release-lane generation
waits for the trust + provenance foundations and is not part of v0.14. Direction:
[release-lane-ci.md](../design/proposed/release-lane-ci.md).

## Just shipped: v0.13.0 — release-quality packaging

Reproducible, lean, testable-from-package artifacts: `SOURCE_DATE_EPOCH`-honoring
deterministic packaging, symbol-split (lean default + sibling `*-debug` /
`--with-debug`), `ost plugin run --plugin-path`/`--no-inject`, `ost plugin test
--from-package`, and the `ost artifact extract --into` alias. Full record in
[releases/v0.13.0.md](../releases/v0.13.0.md).

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
- **Packaging diagnostic — stale debug sidecar candidate.** Optionally warn when
  a same-basename PDB is older than its DLL; keep it non-fatal until PE/PDB
  identity can be compared reliably. See
  [release-lane-ci.md](../design/proposed/release-lane-ci.md#debug-sidecar-diagnostics).
