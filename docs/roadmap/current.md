# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## Next milestone: v0.16.0 — generated trusted CI

**Status:** not started; release planning · **Depends on:** v0.15.0 workspace
composition and artifact evidence bundles (shipped).

v0.14.0 added artifact trust policy and publisher identity checks. v0.15.0 made
produced artifacts carry SBOM/provenance evidence and made generated source-CI
consume workspace dependency closures. v0.16.0 pushes that trust chain into the
generated CI contract so trusted publication lanes are generated, policy-gated,
and separate from ordinary PR/source validation lanes.

### P0 — trust-aware support matrix

- Add trust requirements to support-matrix targets and lanes: a target-level
  `trust` declaration plus lane minimums such as `pr_min_trust`,
  `main_min_trust`, and `release_min_trust`.
- Keep PR and source-CI lanes publish-free. They may build and test from source,
  but they must not mint trusted runtime or plugin artifacts.
- Make release workflows refuse untrusted artifacts instead of relying on
  hand-authored workflow convention.

### P0 — protected runtime-publish lane

- Generate a distinct trusted runtime-publish lane protected by branch/tag rules,
  GitHub OIDC, required SBOM/provenance/validation reports, and protected
  namespace policy enforcement.
- Bind generated publication steps to the same artifact evidence contract shipped
  in v0.15.0: artifact subject digest, source repository/revision, builder
  identity, and allowed-publisher policy.
- Preserve source-CI ergonomics by deriving publication lanes from the same
  support declaration rather than a parallel hand-maintained workflow.

### P1 — release-lane closure

- Connect cargo-dist release assets, OpenStrata artifact evidence, and release
  policy checks into one generated trusted-release story.
- Keep signing/Sigstore material and installer-side verification as the adjacent
  distribution hardening track unless it is small enough to ship with the lane
  generation work.

## Just shipped: v0.15.0 — workspace composition + artifact evidence bundles

Source commands now consume `requires.bundles` closures for build/test/run and
generated source CI, versioned plugin manifests fail closed below `requires:`,
and plugin/project/runtime artifacts preserve SPDX SBOM plus SLSA/in-toto
provenance sidecars through store, OCI transport, and required-evidence
verification. Full record in [releases/v0.15.0.md](../releases/v0.15.0.md).

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
- **Generated-CI maintenance ergonomics.** Generate a reusable
  bootstrap/runtime-pull fragment for hand-authored workflows from the same
  `openstrata.ci.yaml` pins, and add `ost ci pin bootstrap --version <V>` to
  update the version and exact target checksums coherently. These reduce pin
  drift but do not block the v0.15.0 composition or evidence-bundle gates.
