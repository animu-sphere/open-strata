# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## Next milestone: v0.15.0 — provenance / SBOM bundle

**Status:** not started · **Depends on:** v0.14.0 trust policy foundation
(shipped).

Make the artifact an *evidence bundle*, not just an archive (future-policy
§5/§6/§11): optional SBOM (`sbom.spdx.json`) and SLSA/in-toto provenance
(`provenance.intoto.jsonl`) layers, `ost artifact push` attaching them, and
`ost artifact verify --require-sbom` / `--require-provenance` checking that the
provenance subject digest matches the OpenStrata artifact digest, the builder
identity matches the allowed-publisher policy, and source repo/revision match
build metadata.

**Tracks:** licensing per-artifact SBOM, Phase 6 content attribution, and the
future-policy provenance requirements for trusted release lanes.

## Just shipped: v0.14.0 — artifact trust policy foundation

Strict `openstrata-artifact-policy.toml`, stable `ARTIFACT_POLICY_*` errors,
protected publisher enforcement for `ost artifact push`, `ost artifact verify
--policy`, and the release-lane dogfood hardening (strict CI manifest keys,
native Windows bootstrap PATH, ABI-matched bare Python under `ost plugin run`).
Full record in [releases/v0.14.0.md](../releases/v0.14.0.md).

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
