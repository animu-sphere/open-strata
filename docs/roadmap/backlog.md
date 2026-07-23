# Backlog

Ordered but unscheduled work. The next milestone and active carry-overs are in
[current.md](current.md); shipped detail is in [releases/](../releases/) and the
[delivery history](../reports/delivery-history.md).

Legend: ⬜ not started

## Milestone ladder (beyond next)

The v0.19.0 composition and reach milestone is shipped in
[v0.19.0](../releases/v0.19.0.md). The active v0.20.0 dogfood-closure and
renderer-workflow milestone is in [current.md](current.md). DCC host integration
follows it in v0.21.0.

The Formation scope below is **Half B** of v0.19.0, narrowed to
`resolve|inspect|run|lock`. It is gated on Half A (artifact closure, staged-byte
reach, external-provenance reach, producer-session publication) because
Formation's own acceptance criteria — three dogfoods run from packaged,
digest-pinned artifacts on a clean machine — cannot pass while a packaged bundle
from a split workspace is not independently installable. If Half A consumes the
milestone, Formation ships in v0.20.0 and DCC host integration moves to v0.21.0.

- ✅ **v0.19.0 Half B - Formation composition.** Turn the reference-project
  ecosystem documented in v0.18.0 into an executable contract. A **Formation** is a
  resolved, reproducible set of OpenStrata-managed components — runtime, plugin
  bundles, plugin-workspace products, renderer, tool, and scene/input asset
  references — assembled for one command or execution purpose. Direction:
  [formations.md](../design/proposed/formations.md). Minimum viable scope: parse
  a versioned `formation.toml`; select one runtime and resolve plugin and
  renderer components from their dependency metadata; verify target,
  architecture, OpenUSD, compiler/CRT, and Python-ABI compatibility where known;
  compose plugin discovery and loader paths and identify conflicting environment
  contributions; print a deterministic resolved model with `--json`; launch a
  foreground process; emit a digest-pinned `formation.lock`; and record Formation
  Run evidence. CLI: `ost formation resolve|inspect|run|lock` for this
  milestone, using the shipped `{ok, schema, data, warnings}` envelope and
  category exit codes; `formation env|doctor` deferred to and are implemented
  on the v0.20.0 branch.
  Formation must **reuse** the runtime, artifact, plugin, renderer, target, and
  evidence contracts rather than fork them, and introduce no DCC-specific logic
  in the core model. Acceptance requires four first-party dogfoods run from
  packaged, digest-pinned artifacts (not source-tree paths): a
  `usd-3dgs-plugins`-only Formation, a `usd-vrm-plugins`-only Formation, a
  `hydra-merlin`-only Formation, and a combined VRM-rendered-by-hdMerlin
  Formation — each on a clean machine or isolated prefix, with evidence naming
  the exact runtime, bundles, renderer, and executable used. Non-goals: DCC
  discovery, Kubernetes execution, Linux
  namespace / overlayfs sandboxing, detached session management, general-purpose
  package solving, automatic Formation-bundle publication, and implicit download
  from untrusted sources.
- ⬜ **v0.21.0 - DCC host integration (Phase 10).** Deferred from v0.20.0 by
  the v0.19.0 package/release and renderer dogfooding findings. Extends
  OpenStrata beyond runtime-native OpenUSD
  applications without redistributing DCC SDKs or inventing one false cross-DCC
  API: an `ost-host` model with a versioned host record (product, version,
  install root, executable/API locations, Python ABI, platform fingerprint,
  discovery evidence); `ost host discover|list|inspect` with deterministic Maya
  and Houdini detectors that never mutate a host install or accept ambient PATH
  guesses; a host adapter boundary running minimal headless load/open/validate
  probes with preserved output and explained SKIP for unavailable
  licenses/display/capability; and support-matrix cells with pinned host records,
  stable/nightly/release/legacy tiers, and trusted release candidates fed in
  without weakening the artifact publisher boundary. Host integration **consumes
  Formation** as its environment and component-assembly layer — a host record is
  a Formation component, and a host launch binds a runtime, plugins, renderer,
  and host executable through the same resolved model — rather than introducing a
  parallel DCC composition or environment mechanism. It must also consume the
  renderer identity and evidence model established in v0.17.0 and corrected in
  v0.18.0. Direction: [dcc-hosts.md](../design/proposed/dcc-hosts.md).
- ⬜ **v1.0.0 (after the DCC host milestone).** Cut once the produce → trust →
  provenance → trusted-CI arc, cross-repository Formation composition, and the
  initial DCC host matrix are shipped and dogfooded — i.e. "build it, publish it,
  verify its provenance, pull it in trusted CI, compose it into a reproducible
  Formation, and run that against a DCC host" is a single supported,
  digest-addressed arc.

## Future phases

- ⬜ **OpenUSD template catalog maturity and expansion.** Direction:
  [openusd-plugin-templates.md](../design/proposed/openusd-plugin-templates.md).
  Versioned descriptors, deterministic provenance, the asset-resolver and
  compiled-schema skeletons, copied CMake helpers, read-only bundle graph
  validation, and source-workspace dependency composition are in. Next, automate
  clean-install consumer gates, prove the
  schema skeleton on a second supported platform/OpenUSD line, and harden the
  asset resolver. The OpenExec schema-computation skeleton is now in the
  embedded catalog; its next evidence is a schema-specific applied fixture and
  `ExecUsdSystem` dogfood. Extend the existing `ost plugin new` lifecycle—do not create
  a parallel template repository, CLI, renderer, bundle model, or artifact
  path. Add Hydra and tool candidates only at
  evidence-appropriate maturity.
- ⬜ **Renderer skeleton promotion after the v0.17 lifecycle slice.** Direction:
  [renderer-templates.md](../design/proposed/renderer-templates.md). The optional
  co-built Hydra 2 bootstrap now separates discovery, delegate creation, CPU
  RenderBuffer, and install-tree usdview first-frame/stable-update evidence.
  v0.17 owns build truth, managed view, adoption, and evidence transport. After
  that, finish the hosted OS/OpenUSD matrix and apply the contract to a second
  independent renderer. Keep skeleton maturity until that evidence exists;
  instancing, materials, upload policy, and zero-copy interop remain
  renderer-owned until separately proven.
- ⬜ **Phase 7 — Sessions / sandbox.** Session metadata; `ost session start | fork
  | diff | discard | promote`. Workspace isolation; optional Linux namespace /
  overlayfs. A Session is a mutable or isolated working instance *over time* and
  builds **on top of** a resolved Formation (Formation composes components and
  launches; a Session then forks, diffs, discards, or promotes the running
  instance). It is a distinct later layer, not a rename of the
  [Formation milestone](#milestone-ladder-beyond-next).
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
  (e.g. `ost runtime licenses <cy> --profile <p>`). Per-artifact SBOM evidence
  landed with v0.15.0. No artifact ships without complete third-party
  attribution.
- ⬜ **SEC-005 (P1) — installer & release-asset verification.** Publish per-release
  checksums, signature/Sigstore material, SBOM, and provenance; the installer pins
  a version, verifies the checksum, and aborts on mismatch. Tracks Distribution →
  signing/provenance. Reproducible packaging + stable checksums landed with
  v0.13.0, and artifact SBOM/provenance evidence landed with v0.15.0; remaining
  work is release-asset signing material plus installer-side verification.
- ⬜ **SEC-006 (P2) — runtime trust policy.** Runtime trust levels (`local` /
  `verified` / `trusted`) recorded in the manifest and lock; warn on
  world-writable runtime roots; `ost build` / `ost plugin test` can require a
  minimum trust level (release/production CI refuses `local`). Artifact policy
  foundation landed with v0.14.0; runtime minimum-trust hooks remain future work.
- ⬜ **Runtime distribution — glibc-floor ergonomics & OCI producer parity.**
  From v0.12.0 Linux dogfooding: (a) surface the measured glibc floor earlier — in
  `ost runtime show` / `validate`, not only at export; (b) at pull time, fail or
  loudly warn when an artifact's `glibcNNN` floor exceeds the current host's glibc
  even without `--require-target`, catching an ABI mismatch before the first
  `dlopen`; (c) reconcile `ost artifact push` vs `oras push` OCI manifests —
  document the canonical producer path (prefer `ost artifact push`) or reproduce
  the `oras` manifest byte-for-byte so CI pins don't drift.
- ✅ **Packaged workspace/product composition — aggregate product artifact.**
  **Scheduled into [v0.19.0 Half A](current.md) as P1** by the 2026-07-18
  usd-vrm-plugins reports, which re-filed it off a real release that now
  publishes three assets per target plus a documented install order. The
  per-bundle half (`--workspace` packaging, resolved `bundles` in
  `dependencies.json`, `--from-package` composition) shipped in
  [v0.18.0](../releases/v0.18.0.md). No longer tracked here.

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
