# Backlog

Ordered but unscheduled work. The next milestone and active carry-overs are in
[current.md](current.md); shipped detail is in [releases/](../releases/) and the
[delivery history](../reports/delivery-history.md).

Legend: ⬜ not started

## Milestone ladder (beyond next)

The next milestone (v0.14.0 — trust policy foundation) is detailed in
[current.md](current.md).

- ⬜ **v0.15.0 — provenance / SBOM bundle.** Make the artifact an *evidence
  bundle*, not just an archive (future-policy §5/§6/§11): optional SBOM
  (`sbom.spdx.json`) and SLSA/in-toto provenance (`provenance.intoto.jsonl`)
  layers, `ost artifact push` attaching them, and `ost artifact verify
  --require-sbom` / `--require-provenance` checking that the provenance subject
  digest matches the OpenStrata artifact digest, the builder identity matches the
  allowed-publisher policy, and source repo/revision match build metadata. Closes
  the licensing "per-artifact SBOM" and Phase 6 "content attribution" gaps for
  published artifacts.
- ⬜ **v0.16.0 — generated trusted CI.** Push the trust chain up into the CI
  contract (future-policy §7/§8/§13): a `trust` field on support-matrix targets, a
  minimum-trust requirement per lane (`pr_min_trust` / `main_min_trust` /
  `release_min_trust`), and lane-specific generated workflows — the PR / source-CI
  lanes stay publish-free, and a separate **trusted runtime-publish lane**
  (protected branch/tag, OIDC, SBOM + provenance + validation report required,
  protected-namespace policy enforced) is generated distinctly from the release
  lane. Release workflows refuse untrusted artifacts.
- ⬜ **v0.17.0 — DCC host integration.** Extend the support matrix beyond
  runtime-native apps to external DCC hosts (future-policy §9/§11; Phase 10
  [dcc-hosts.md](../design/proposed/dcc-hosts.md)). Read-only host discovery +
  fingerprint (`ost dcc discover`, host record schema, Maya/Houdini detectors
  first), headless plugin compatibility test, and DCC support-matrix + CI-annotation
  integration — *without* a DCC API abstraction or SDK redistribution
  (future-policy §13 non-goals).
- ⬜ **v1.0.0 (after v0.17.0).** Cut once the produce → trust → provenance →
  trusted-CI arc and the initial DCC host matrix are shipped and dogfooded — i.e.
  "build it, publish it, verify its provenance, pull it in trusted CI, run it
  against a DCC host" is a single supported, digest-addressed arc.

## Future phases

- ⬜ **OpenUSD template catalog maturity and expansion.** Direction:
  [openusd-plugin-templates.md](../design/proposed/openusd-plugin-templates.md).
  Add versioned template descriptors, deterministic scaffold provenance,
  clean-room generation/install/discovery gates, and self-contained copied CMake
  helpers around the templates already shipped. Extend the existing
  `ost plugin new` lifecycle—do not create a parallel template repository, CLI,
  renderer, bundle model, or artifact path. Add asset/package resolver, Exec,
  Hydra, and tool candidates only at evidence-appropriate maturity.
- ⬜ **Renderer project skeleton and Level 7 verification.** Direction:
  [renderer-templates.md](../design/proposed/renderer-templates.md). Add one
  composable `renderer` project scaffold (host-neutral core, project-owned
  extraction seam, Vulkan offscreen backend, validation pack, optional Hydra 2
  adapter), a committed renderer composition manifest, and independently
  reported discovery/delegate/GPU/RenderBuffer/usdview stable-update evidence.
  Keep it at skeleton maturity until a second independent implementation;
  instancing, materials, upload policy, and zero-copy interop remain
  renderer-owned until separately proven.
- ⬜ **Phase 7 — Sessions / sandbox.** Session metadata; `ost session start | fork
  | diff | discard | promote`. Workspace isolation; optional Linux namespace /
  overlayfs.
- ⬜ **Phase 8 — AI / GPU profiles.** GPU host detection + driver checks
  (`ost doctor gpu`); AI runtime profiles (`ai-cuda124`, `ai-rocm`, `ai-mps`,
  hybrid `cy2026-lookdev-ai`); Jenkins GPU routing labels + smoke tests.
- ⬜ **Phase 9 — Kubernetes execution backend.** Direction:
  [kubernetes.md](../design/proposed/kubernetes.md). An `ost-execution` crate with
  an `ExecutionBackend` trait (`local` + `kubernetes`); `ost submit …` /
  `ost jobs …`; phased manifest-export → kubectl submit/status/logs → artifact
  collection → matrix → GPU → Jenkins bridge; digest-pinned tasks, safe-by-default
  manifests, `ost doctor kubernetes`. `local` stays first-class; Kubernetes is
  opt-in, starting from `batch/v1 Job`, not an Operator.
- ⬜ **Phase 10 — DCC host support.** Direction:
  [dcc-hosts.md](../design/proposed/dcc-hosts.md). An `ost-host` crate (host record
  model, discovery providers, `HostValidator` / `HostAdapter`); discovery +
  validation + fingerprints (Maya, then Houdini + Nuke); `ost host
  discover|list|inspect|probe|run|test`; host-standard packaging; matrix cells /
  tiers and cross-DCC USD compatibility; fleet inventory and `ost compat` /
  `ost reproduce`. (Delivered incrementally via the v0.17.0 milestone.)

## Cross-cutting open items

Shipped context for each area is in the
[delivery history](../reports/delivery-history.md).

- ⬜ **Distribution — release-asset signing.** Build-provenance attestations
  (SLSA) already attach to release artifacts; still open is explicit
  signature/Sigstore key material and `ost`-side verification of it (tracks
  SEC-005).
- ⬜ **Licensing — runtime/extension content attribution.** Runtime/extension
  manifests record upstream license metadata; built/adopted runtimes collect
  upstream `LICENSE`/`NOTICE` files; a runtime's licenses are inspectable
  (e.g. `ost runtime licenses <cy> --profile <p>`). Per-artifact SBOM lands with
  v0.15.0. No artifact ships without complete third-party attribution.
- ⬜ **SEC-005 (P1) — installer & release-asset verification.** Publish per-release
  checksums, signature/Sigstore material, SBOM, and provenance; the installer pins
  a version, verifies the checksum, and aborts on mismatch. Tracks Distribution →
  signing/provenance. Reproducible packaging + stable checksums land with v0.13.0;
  SBOM/provenance with v0.15.0.
- ⬜ **SEC-006 (P2) — runtime trust policy.** Runtime trust levels (`local` /
  `verified` / `trusted`) recorded in the manifest and lock; warn on
  world-writable runtime roots; `ost build` / `ost plugin test` can require a
  minimum trust level (release/production CI refuses `local`). Foundation lands
  with v0.14.0.
- ⬜ **Runtime distribution — glibc-floor ergonomics & OCI producer parity.**
  From v0.12.0 Linux dogfooding: (a) surface the measured glibc floor earlier — in
  `ost runtime show` / `validate`, not only at export; (b) at pull time, fail or
  loudly warn when an artifact's `glibcNNN` floor exceeds the current host's glibc
  even without `--require-target`, catching an ABI mismatch before the first
  `dlopen`; (c) reconcile `ost artifact push` vs `oras push` OCI manifests —
  document the canonical producer path (prefer `ost artifact push`) or reproduce
  the `oras` manifest byte-for-byte so CI pins don't drift.

## Documentation & tooling

Shipped documentation infrastructure is in the
[delivery history](../reports/delivery-history.md); these are the remaining
pieces of the documentation reorganization.

- ⬜ **Documentation website.** Render the repository's Markdown (concepts,
  guides, generated reference, release records) as a static site with search and
  pull-request previews, treating the site as a *renderer* of repo-owned Markdown
  — no manually duplicated CLI/schema content. A framework (Astro/Starlight,
  Docusaurus, MkDocs, …) will be chosen when this is picked up; framework choice
  is secondary to content ownership, which is already in place.
- ⬜ **Generated `environment-variables.md`.** Centralize the scattered `OST_*`
  environment variables into a single source and generate the reference page from
  it (the last reference page not yet generated).
- ⬜ **CI matrix validation from `support/platforms.toml`.** Reuse the support
  declaration that drives the support matrix to validate the generated CI matrix
  against declared support levels (§10).
- ⬜ **Portable-Linux runtime build guide.** Document, as a first-class recipe,
  building Linux runtimes in a container whose glibc ≤ the lowest target runner
  (e.g. `ubuntu:24.04` for hosted `ubuntu-24.04`, or `manylinux_2_28` for broader
  reach), including the deadsnakes-py3.13 + venv + `libxt-dev` / X-GL dev
  prerequisites; consider shipping a reference Dockerfile. (The `glibc228`-vs-real-floor
  trap from v0.10.0/v0.12.0 is easy to fall into.)
- ⬜ **Generated release lanes.** Matrix-level `source_checks` already preserves
  repo-specific corpus smoke in generated pull-request/main lanes. The remaining
  gap is a typed, tag-triggered release model: version/ref agreement,
  package-twice reproducibility, `--from-package`, artifact staging, trust and
  provenance gates, and a separately permissioned draft/publish job. Do not use
  raw per-cell `extra_steps` as a substitute. Direction:
  [release-lane-ci.md](../design/proposed/release-lane-ci.md).
