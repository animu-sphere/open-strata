# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## Next milestone: v0.18.0 - DCC host integration

**Status:** ⬜ not started · **Depends on:** v0.17.0 renderer build/session,
runtime fingerprint, installed Hydra product, and evidence chain (shipped).

v0.18.0 extends OpenStrata beyond runtime-native OpenUSD applications without
redistributing DCC SDKs or inventing one false cross-DCC API. Host integration
must consume the renderer identity and evidence model established in v0.17.0
rather than introduce a parallel DCC renderer contract.

### P0 - host records and read-only discovery

- Add an `ost-host` model with a versioned host record: product, version,
  install root, executable/API locations, Python ABI, platform fingerprint, and
  discovery evidence.
- Add `ost host discover|list|inspect` with deterministic Maya and Houdini
  detectors first.
- Discovery must not mutate a host installation or silently accept ambient PATH
  guesses.

### P0 - headless compatibility probes

- Add a host adapter boundary for launch/session composition, not a shared DCC
  scene API.
- Run a minimal headless plugin load/open/validate probe and preserve stdout,
  stderr, exit status, host fingerprint, renderer/runtime/plugin digests, and
  normalized evidence.
- Treat unavailable licenses, display, or host capability as explained SKIP
  rather than plugin/ABI failure.

### P1 - DCC support-matrix integration

- Extend explicit support cells with pinned host records/capabilities.
- Define stable/nightly/release/legacy tiers and prove the first Maya/Houdini
  hosted cells.
- Feed trusted release candidates into host verification without weakening the
  artifact publisher boundary.

Direction: [dcc-hosts.md](../design/proposed/dcc-hosts.md).

## v0.17 environment-dependent acceptance

The lifecycle, managed-view, adoption, and renderer evidence contracts shipped
in v0.17.0; these checks require hosted operating systems, real OpenUSD
installations, or downstream renderer repositories:

- Apply the managed `ost renderer view` loop to hdMerlin without changing its
  ownership boundaries.
- Prove renderer core-only, Vulkan, and Hydra paths across the hosted
  OS/OpenUSD matrix.
- Validate external/prebuilt `--build-dir` evidence with a real adopted renderer
  project.
- Exercise renderer report merge conflict policy against independently produced
  CPU/GPU/runtime evidence.

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
