# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## Next milestone: v0.16.0 — dogfood closure + generated trusted CI

**Status:** 🚧 in progress · **Depends on:** v0.15.0 workspace
composition and artifact evidence bundles (shipped).

v0.14.0 added artifact trust policy and publisher identity checks. v0.15.0 made
produced artifacts carry SBOM/provenance evidence and made generated source-CI
consume workspace dependency closures. v0.16.0 pushes that trust chain into the
generated CI contract so trusted publication lanes are generated, policy-gated,
and separate from ordinary PR/source validation lanes.

The 2026-07-13 `vrmContainer` Phase 2 dogfood adds two release-front acceptance
items. They are shared substrate for independently composed renderer packages as
well as the VRM workspace; they do not prescribe one package per internal
renderer source pack.

### P0 — Windows L5 transport regression ✅ implementation

- ✅ Capture `usdcat --flatten` through `--out <temporary-file>` instead of a
  Windows text-mode stdout pipe.
- ✅ Preserve genuine CR inside USDA strings; do not normalize semantic values.
- ✅ Distinguish a remaining authored/generated CR mismatch from stdout
  translation in the diagnostic and cover authored multiline comments plus the
  generated multiline `doc` shape.
- ⬜ Re-run the concrete `vrmSchema` L5 fixture on Windows, macOS, and Linux and
  restore the temporarily capped Windows hosted cell from L4 to L5.

### P1 — executable plain-library closure 🚧

- ✅ Add `openstrata.library/v1alpha1` with library identity/version, installed
  CMake package and exported target, transitive library edges, and runtime dirs.
- ✅ Accept versioned `requires.libraries`; reject missing, duplicate,
  incompatible, malformed, and cyclic closures before CMake.
- ✅ Build/install libraries deepest-first into the target workspace prefix and
  let consumers continue to use normal `find_package(... CONFIG REQUIRED)`.
- ✅ Add materialized runtime dirs to source test/run/view sessions and expose
  identity, version, descriptor, prefix, package/target, runtime paths, and
  provenance through inspect/test evidence.
- ✅ Materialize selected library runtime directories into plugin packages and
  record the closure; keep digest-pinned multi-bundle package composition in the
  future product layer.
- ✅ Make the `cpp-library` scaffold emit the descriptor and a consumable CMake
  config package.
- ⬜ Dogfood the real `vrmContainer -> usdVrm` producer/consumer on all three
  hosted OSes, then delete the downstream bootstrap/runtime-copy adapter.

### P2 — renderer Slice A 🚧

- ✅ Add `openstrata.renderer/v1alpha1` and
  `openstrata.renderer-report/v1alpha1` with strict parsing and required
  PASS/FAIL/SKIP assertion matching.
- ✅ Add `ost init --template renderer` as one project-level CMake graph with
  host-neutral core, extraction seam, Vulkan capability target, headless runtime
  product, config-package install, and install-tree CTest.
- ✅ Generate `renderer-report.json` and surface each check through generic
  `ost validate`; unavailable GPU work is an explained skip.
- ⬜ Replace the generated GPU-frame skip with a deterministic Vulkan offscreen
  frame, color/depth contract checks, persistent-frame evidence, validation
  callback capture, and the 1,000-frame clean loop.

### P0 — trust-aware support matrix 🚧

- ✅ Add trust requirements to support-matrix targets and lanes: a target-level
  `trust` declaration plus lane minimums such as `pr_min_trust`,
  `main_min_trust`, and `release_min_trust`. `ost ci plan` reports each target
  and its effective floor.
- ✅ Generated support/source jobs enforce the stricter target/lane floor and
  require valid SBOM + provenance evidence; an optional repository policy also
  gates builder identity. `ost artifact verify --minimum-trust` composes with
  (and never weakens) the policy file minimum.
- ✅ Keep PR and source-CI lanes publish-free. They may build and test from
  source, but they do not mint trusted runtime or plugin artifacts.
- ⬜ Make generated release workflows refuse untrusted artifacts instead of relying on
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
