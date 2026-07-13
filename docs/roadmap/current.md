# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## Next milestone: v0.15.0 — workspace composition + provenance / SBOM bundle

**Status:** not started · **Depends on:** v0.14.0 trust policy foundation
(shipped).

The v0.14.0 workspace graph and schema-contract model held through the first
real bundle split, but the run also exposed the missing second half of that
contract: `requires.bundles` validates a dependency and then build/test/run do
not consume it. v0.15.0 therefore has two release tracks. Source-workspace
composition is the correctness gate; the planned evidence-bundle work remains
the trust-chain deliverable.

### P0 — source-workspace composition

- Resolve the transitive `requires.bundles` closure from the validated graph.
  `plugin test --workspace` must test each bundle with its own closure, and a
  selected `plugin doctor|test|run <bundle>` must do the same when its workspace
  can be identified unambiguously. Keep `--with` as an explicit additive escape
  hatch for external/ad-hoc bundles, not as required duplication of the
  manifest.
- Build source dependencies in deterministic topological order, install them to
  an OST-owned target-specific private prefix, and expose that prefix through
  normal CMake package discovery. Do not synthesize `add_subdirectory` links or
  encode sibling source paths in a bundle's install interface.
- Make generated source-CI cells consume the same manifest closure from their
  existing `bundle:` selection. Do not add a cell-level `with:` field. Packaged
  support cells remain single-artifact until a product/artifact-closure contract
  can pin every dependency by digest.
- Make versioned plugin manifests strict below `requires:`. Until a portable
  library identity/discovery contract exists, `requires.libraries` is reserved
  and must fail explicitly instead of being silently ignored.

The dogfood acceptance gate is a file-format consumer that builds without
repo-owned sibling bootstrap glue, passes L0–L5 without `--with`, passes the
same full pyramid under `--workspace` and generated source CI, and leaves the
schema provider standalone-buildable/packageable. Its pre-split authored-stage
baseline must remain byte-identical.

### P0/P1 — workspace hardening found by the split

- Fix `usd-schema-cpp`'s installed consumer on MSVC (`NOMINMAX`) and avoid
  re-entering non-idempotent `pxrConfig` when `pxr` is already resolved.
- Keep L5 comparison semantic: do not normalize carriage returns embedded in
  USDA string values. Emit an actionable CRLF/`.gitattributes` hint when that is
  the only difference, and record a bounded diff or diff-artifact path in the
  JSON report.
- Do not render `OPENSTRATA_SCHEMA_SOURCES_FILE` for bundles that no longer
  own/co-host a schema, and clear any fragment left by an earlier target shape.

### Evidence-bundle track

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
- **Generated-CI maintenance ergonomics.** Generate a reusable
  bootstrap/runtime-pull fragment for hand-authored workflows from the same
  `openstrata.ci.yaml` pins, and add `ost ci pin bootstrap --version <V>` to
  update the version and exact target checksums coherently. These reduce pin
  drift but do not block the v0.15.0 composition or evidence-bundle gates.
