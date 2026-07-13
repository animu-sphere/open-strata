# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## Next milestone: v0.17.0 — DCC host integration

**Status:** ⬜ not started · **Depends on:** v0.16.0 composition, renderer, and
trusted-release contracts (shipped).

Extend the support model beyond runtime-native OpenUSD applications to external
DCC hosts without redistributing their SDKs or inventing one false cross-DCC API.
The milestone begins with read-only discovery and stable host fingerprints, then
uses those records for headless plugin compatibility evidence and explicit DCC
support cells.

### P0 — host records and read-only discovery

- Add an `ost-host` model with a versioned host record: product, version, install
  root, executable/API locations, Python ABI, platform fingerprint, and discovery
  evidence.
- Add `ost host discover|list|inspect` with deterministic Maya and Houdini
  detectors first; discovery must not mutate a host installation or environment.
- Keep operator overrides explicit and evidence-bearing instead of silently
  accepting ambient PATH guesses.

### P0 — headless compatibility probes

- Add a host adapter boundary for launch/session composition, not a shared DCC
  scene API.
- Run a minimal headless plugin load/open/validate probe and preserve stdout,
  stderr, exit status, host fingerprint, runtime/plugin digests, and normalized
  result in one report.
- Separate unavailable license/display/host capability as explained SKIP from a
  plugin or ABI failure.

### P1 — DCC support-matrix integration

- Extend explicit support cells with a pinned host record/capability requirement
  and generate CI annotations without exposing secrets to ordinary PR lanes.
- Define stable/nightly/release/legacy tiers and prove the first Maya/Houdini
  hosted cells before promoting support claims.
- Feed trusted release candidates into headless host verification without
  weakening the v0.16 artifact policy or publisher boundary.

Direction: [dcc-hosts.md](../design/proposed/dcc-hosts.md).

## v0.16 environment-dependent acceptance

The contracts shipped in v0.16.0; these checks require hosted operating systems,
real OpenUSD installations, downstream repositories, or live registry identity:

- Re-run the concrete `vrmSchema` L5 fixture on Windows, macOS, and Linux and
  restore the temporarily capped Windows hosted cell from L4 to L5.
- Dogfood the real `vrmContainer -> usdVrm` library producer/consumer on all
  three hosted OSes, then delete the downstream bootstrap/runtime-copy adapter.
- Run renderer core-only and Vulkan paths across the hosted matrix, apply the
  manifest/report contract to hydra-merlin without changing its ownership, and
  dogfood renderer-owned topology/points/camera translation.
- Generate a downstream `ost-release.yml`, run its OIDC-authorized live GHCR
  round trip, verify the immutable `<version>-<cell>` artifacts, and record the
  protected-environment evidence.

## Carry-over follow-ups

- **Republish the public macOS runtime (from v0.12.0).** Republish the cy2026
  macOS arm64 OpenUSD 26.05 SDK with preserved executable bits and prove a clean
  `macos-15-arm64` source-CI L5 lane before removing downstream repair steps.
- **GHCR push round-trip (from v0.11.0).** Confirm the direct
  `OST_REGISTRY_USER`/`OST_REGISTRY_PASSWORD` path against GHCR; the generated
  v0.16 publisher provides the preferred protected workflow for this evidence.
- **SEC-002 — symlink escape inside a bundle.** Reject a real in-bundle symlink
  whose canonical target escapes the bundle root.
- **Packaging diagnostic.** Optionally warn when a same-basename PDB is older
  than its DLL; keep it non-fatal until PE/PDB identity can be compared.
- **Generated-CI maintenance ergonomics.** Add `ost ci pin bootstrap --version
  <V>` and a reusable bootstrap/runtime-pull fragment derived from the same
  matrix pins.
