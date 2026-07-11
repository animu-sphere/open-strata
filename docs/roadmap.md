# Roadmap

Delivery is phased. Each phase is a usable increment, not a big-bang. Linux x86_64
is the first-class implementation target; other OS targets are modeled from the
start but may be unavailable initially.

Legend: ✅ done · 🚧 in progress · ⬜ not started

## Release milestones

Phases are the long-form structure; releases are the shipped increments cut from
them. Each release is a coherent slice, not a phase boundary.

- ✅ **v0.1.0** — first public release: foundation through OpenUSD/MaterialX
  profiles and the static plugin-verification framework (Phases 0–3, Phase 4a).
- ✅ **v0.2.0** — machine-readable `--json` output + error/exit-code contract,
  build-lifecycle hardening (per-target trees, runtime-env-consistent CMake,
  progress stream), and the security P0/P1 baseline (SEC-001…004).
- ✅ **v0.3.0** — Phase 4b plugin-harness dogfooding round: relative-path
  `plugin build|test`, MSVC bootstrap, loadable `plugInfo.json`, real USD-version
  detection, `plugin package` artifacts, the fmt/clippy/test CI gates, and the
  plugin bundle `license` field.
- ✅ **v0.4.0 — the schema plugin kind.** Where 0.3.0 made the *file-format*
  bundle path solid, 0.4.0 adds `usd-schema` as a first-class kind and closes the
  remaining Phase 4 scaffold/diagnostic gaps. Phase 6-independent. Scope:
  - **Schema bundles (A)** — the
    [Phase 4 — schema-bundle backlog](#phase-4--schema-bundle-backlog-from-downstream-plugin-dogfooding-reports-34)
    below. Done (✅): codeless `usd-schema` template + codeless-aware L0 doctor
    (e2e-hardened so it registers on a real runtime), the schema test contract
    (L2/L4, verified e2e on OpenUSD 26.08), co-hosting a schema in an existing
    bundle, per-variant `cxx_abi`, the `usdGenSchema` regenerate build step, and
    the `usdGenSchema` `Types` *merge* into a co-hosting bundle's existing
    `plugInfo.json` (all verified e2e on OpenUSD 26.08). **Deferred to a later
    release:** the compiled (non-codeless) schema variant — the codeless +
    co-hosting paths cover the data-contract use case; the typed-C++ importer API
    is a heavier, separable increment.
  - **Phase 4 close-out (B)** — P3 repo-shape scaffold and `ost doctor`
    structuring (§14.5), both tagged `(v0.4.0)` in-place below.
  - Out of scope (deferred): `plugin publish` + the runtime×plugin CI matrix
    (blocked on the Phase 6 artifact source) and runtime/extension content
    attribution (lands with the Phase 6 content store).
- ✅ **v0.5.0 — schema authoring hardening + workspace ergonomics.** A
  consolidation release after the schema-kind milestone: make the v0.4 codeless
  and co-hosted schema paths reliable across Windows/macOS, remove the remaining
  "works only if you know the trick" UX, and keep Phase 6-dependent publishing
  work out of this cut. Scope:
  - **Delivered:** UTF-8-forced schema generation, the
    `schema.library_prefix` double-prefix hint, per-target metadata adoption
    nudges, runtime-version drift reporting across `show`/`validate`/doctor JSON
    and human output, the discoverable `usd-plugin-workspace` template alias,
    `plugins/<name>/` workspace discovery, `ost plugin new` workspace guidance,
    macOS `runtime pull --build` notes for the known source-build edges, and the
    schema build-hook groundwork for a compiled co-located flow.
  - **Still out of scope:** `plugin publish`, the runtime×plugin CI matrix, and
    runtime/extension content attribution; those remain tied to the Phase 6
    artifact source/content store. A first-class compiled co-located schema UX
    (`ost plugin schema add` or a documented manifest-driven equivalent) also
    remains a v0.6.0 follow-up from the v0.5.0 dogfooding recheck.
- ✅ **v0.6.0 — artifact registry + publishable plugin CI.** The first practical
  Phase 6 slice: make runtime/plugin/package artifacts addressable by digest,
  publish plugin package outputs into a local registry, and use those artifacts
  as the source of truth for a small runtime×plugin CI matrix. Scope:
  - **Artifact store MVP:** ✅ local content-addressed store and registry index
    (`ost-artifact`), `tar.zst` + manifest + checksums + validation report as
    the canonical bundle, digest-pinned `ost artifact import|export|list|show`,
    artifact integrity verification (`ost artifact verify`), and the
    `RuntimeSource::Artifact` path: `ost runtime export` packs a validated real
    runtime into the registry and `ost runtime pull --from-artifact <digest>`
    materializes it anywhere.
  - **Plugin publish MVP:** ✅ `ost plugin publish` consumes an existing
    `ost plugin package` output, refuses missing validation/provenance/license/
    notices with per-cause stable error codes, requires the frozen concrete
    target ABI (`package` already collapses `cxx_abi: inherit`), and publishes
    by digest rather than by mutable name.
  - **CI matrix MVP:** ✅ GitHub Actions first (Jenkins generator later). Matrix
    cells are explicit support lines (`runtime artifact digest × plugin artifact
    digest × target/profile`) in `openstrata.ci.yaml`, never a naive Cartesian
    product; `ost ci init | validate [--resolve] | generate github` scaffolds,
    gates, and renders them into a scheduled/dispatch workflow. PR CI keeps
    cheap mock/static checks; the generated matrix runs real runtime/plugin
    cells from the registry.
  - **Dogfooding #7 follow-ups:** ✅ the compiled co-located schema path is
    product-shaped — `ost plugin schema add` scaffolds a starter
    `schema/schema.usda` (compiled by default, `--codeless` opt-out) and wires
    the manifest (`provides: usd-schema:<Type>` + the new bundle-relative
    `schema.source`), feeding the existing build flow (usdGenSchema, generated
    C++ linked into the plugin library, `Types` merge, `generatedSchema.usda`
    staging, export define); ✅ adopted-runtime drift repair UX —
    `ost runtime repair` re-adopts a `local` runtime from its recorded USD root
    in one step, and every drift report (`show` human/JSON, `validate`) now
    prints the exact copy-paste fix per source.
  - **Deferred:** macOS source-build ergonomics re-check (needs a Mac), OCI
    layout / ORAS transport, remote hosted registry, Kubernetes execution, full
    Jenkins command surface, and DCC host matrices.
- ✅ **v0.7.0 — CI contract v2 + 0.6.0 CI/lock/package fixes.** Released from
  dogfooding report #8 (2026-07-04) and the CI build-matrix policy notes
  (2026-07-05): first make the 0.6.0 CI surface trustworthy, then promote
  `openstrata.ci.yaml` from an artifact-seeded support matrix into a portable
  CI contract that GitHub Actions merely renders. Details in the
  [Phase 5 — v0.7.0 backlog](#phase-5--v070-backlog-from-dogfooding-report-8--the-ci-build-matrix-policy-notes).
  Delivered:
  - **Correctness first (report #8):** valid + deterministic
    `ost ci generate github` output, `strata.lock` extension versions that
    match `runtime show` (the lockfile must be safe as a CI gate), idempotent
    `ost plugin package` reruns, and placeholder digests that cannot be
    mistaken for a usable matrix.
  - **CI contract v2 (policy notes):** named runner profiles (`github-hosted`
    image vs `self-hosted` labels — repo workflows stop hard-coding raw
    `runs-on`) and lanes (`pull_request` / `main` / `scheduled` /
    `workflow_dispatch`); a generated **source-CI** PR workflow that
    builds+tests a plugin on a GitHub-hosted runner from a digest-pinned
    runtime SDK artifact, keeping the 0.6.0 artifact-seeded workflow as the
    scheduled **support** lane; hosted-runner billing acknowledgement +
    `ost ci plan`; fork-PR safety; CI evidence (profile/lane/digests/outcome)
    in reports.
  - **Workspace + docs:** workspace-level plugin test orchestration
    (`ost plugin test --workspace`) and a documented co-located schema
    migration path for existing (non-scaffold) bundles.
  - **Acceptance shape (from the policy notes):** generated source-CI jobs are
    modeled to build plugin PRs on GitHub-hosted runners from digest-pinned
    runtime artifacts once `ost` is available on the runner; cells reference
    runner profiles, never raw labels; fork PRs cannot publish or reach
    privileged self-hosted runners; a scheduled self-hosted cell revalidates a
    pinned runtime/plugin pair; generated workflows are valid, deterministic
    YAML.
  - **Deferred:** Jenkins renderer, remote/OCI registry transport, macOS
    ergonomics re-check, source-CI bootstrapping beyond the existing
    `ost --version` assertion, dispatch inputs, privileged-runner trust policy,
    and DCC host matrices.
- ✅ **v0.8.0 — packaging reruns survive transient locked stage trees.** A
  narrow follow-up from dogfooding report #9: `ost plugin package` and
  `ost package` now reset staging directories with bounded retry, clear
  read-only entries during cleanup, and fall back to a fresh sibling stage when
  scanners or indexers still hold old files open. The fallback is visible as a
  structured `STAGE_FALLBACK` warning, so CI can keep moving without hiding the
  host condition.
- ✅ **v0.9.0 — remote artifact transport + hosted source-CI closure + macOS
  plugin-build robustness.** *Shipped and demonstrated on real CI* (dogfooding
  report 2026-07-09): the migrated hosted `windows-2022` PR lane goes green end
  to end (~1m51s) — pinned-`ost` bootstrap → anonymous GHCR runtime pull →
  schema-generate → configure → compile/link → verification pyramid → package —
  and the first real OCI runtime publish round-tripped cleanly through
  `ost artifact pull` (produced out-of-band with `oras`, since there is no
  producer verb yet). Planned from dogfooding report #10 (2026-07-05),
  the remote-artifact-transport plan
  ([remote-artifact-transport.md](remote-artifact-transport.md)), and the
  macOS dogfooding report (2026-07-05, `ost 0.8.0` on macOS arm64). Report
  #10 produced real digests for both lanes on 0.8.0, but the rendered PR lane
  still cannot run on a GitHub-hosted runner: nothing installs `ost` there,
  and nothing can seed the ~1.93 GiB runtime SDK artifact into the runner's
  local-only registry. v0.9.0 closes that bootstrap gap end to end — the
  plan's P0 slice. The macOS report showed `ost plugin build` reproducibly
  failing on macOS for the co-hosted schema workspace; v0.9.0 carries those
  fixes as its P1 host-robustness slice. Scope (details in the
  [Phase 6 — v0.9.0 backlog](#phase-6--v090-backlog-from-dogfooding-report-10--the-remote-artifact-transport-plan)
  and the
  [Phase 4 — v0.9.0 macOS backlog](#phase-4--v090-macos-backlog-from-the-macos-dogfooding-report-2026-07-05)):
  - ✅ **Transport abstraction + read-only OCI pull (plan Phase 1):** an
    `ArtifactTransport` contract with the existing filesystem store as one
    adapter and a read-only OCI backend (GHCR-class registries) as the second;
    `ost artifact resolve | pull` with digest-pin enforcement, the full
    verification chain before an atomic local import, JSON pull evidence, and
    stable `ARTIFACT_*` error codes. Landed (#86).
  - ✅ **CI contract + generated hosted bootstrap (plan Phase 2):**
    `openstrata.ci.yaml` support lines gain a `runtime_remote` reference
    (OCI uri + expected digest) beside the artifact digest; `ost ci generate
    github` renders a pinned, checksum-verified `ost` install step and the
    digest-pinned `ost artifact pull`, with `actions/cache` keyed by digest as
    an optional optimization (cache is speed, never correctness). The public
    E2E fixture repository (`snkmcb/_ost_runner_test`) proves fork-PR / push /
    cache-miss runs green end to end — see the Phase 6 P0 item below (#89/#90).
  - ✅ **Runtime export ergonomics (report #10):** a slim/SDK-profile export
    (`include/lib/bin/plugin`-only) cuts the 14.4 GB adopted-tree payload, and
    `export` now packs with multithreaded zstd by default (`--jobs`), takes a
    `--level` knob, and prints throttled progress — replacing the previously
    ~52-minute silent `ost runtime export`.
  - ✅ **macOS plugin-build robustness (macOS report):** the co-hosted schema
    build now resolves Python from the runtime interpreter (never a bare
    `python` on PATH — a precondition error names what was searched);
    scopes the schema-regeneration env to the runtime's plugin registry so
    `usdGenSchema` no longer discovers (and tries to dlopen) the bundle's
    not-yet-built plugin library; fails early with the doctor fix when a
    committed `plugInfo.json` carries another platform's `LibraryPath` suffix;
    and attributes every build failure to a phase (`schema-generate` /
    `configure` / `compile-link` / `schema-merge` / `plugin-discovery`) in
    both human and `--json` output.
  - **Deferred (now sequenced as v0.10.0–v0.16.0 below):** OCI push (v0.10.0),
    producer-side correctness + Linux runtime portability (v0.11.0), hosted
    macOS CI / runtime-contract hardening (v0.12.0), protected publish policy +
    OIDC allowed-publisher (v0.13.0), provenance / SBOM bundle (v0.14.0),
    generated trusted CI + trust levels in the support matrix (v0.15.0), and
    DCC host integration (v0.16.0) — tracking plan
    Phase 3/4, SEC-006, and the Phase 6 trust-policy hooks. The 2026-07-09
    publish dogfooding additionally surfaced three runtime-completeness bugs
    (from-source `--build` version drift, missing `jinja2` in a published
    runtime, `--slim` dropping `resources/`) that block the export→publish arc;
    those land as the **v0.10.0 P0** slice ahead of `ost artifact push`.

- ✅ **v0.10.0 — OCI publish foundation + runtime-completeness closure.** The
  first *produce*-side slice of remote transport, planned from the 2026-07-09
  publish dogfooding report and the future-policy note (`openstrata_future_policy`
  §3.1/§11). v0.9.0 made *pulling* an OCI runtime first-class and enforced it in
  the CI contract, but the image the `runtime_remote` contract demands still had
  to be pushed by hand with `oras`, and standing up that first real publish
  exposed three ways a from-source runtime was not yet publishable. v0.10.0 closes
  both. **Delivered:** all P0/P1/P2 items below, plus the remaining Python/uv
  shadow-dep diagnosis. See the
  [Phase 6 — v0.10.0 backlog](#phase-6--v0100-backlog-from-the-2026-07-09-publish-dogfooding--the-future-policy-note).
  Scope:
  - **P0 — runtime completeness (the publish arc must be usable first):**
    (1) `runtime pull --build` stamps the OpenUSD version read from the built
    `pxr.h` (as the adopt path already does), not the extension catalog default,
    and the drift validator's suggested recovery actually recovers a `build`-source
    runtime (a working `runtime repair`/`--redetect`) instead of being a no-op —
    today the only fix is discarding build provenance via a `--from-usd` re-adopt,
    and `export` is hard-blocked (`EXPORT_VALIDATION_REQUIRED`) until then;
    (2) a runtime that bundles schema tooling (`usdGenSchema` in `bin/`) ships its
    Python deps (`jinja2` + `MarkupSafe`) in `lib/python`, so a published image is
    not silently incomplete for `ost plugin build`'s schema-generate phase on a
    clean runner; (3) `runtime export --slim` retains the top-level `resources/`
    (or at least the MaterialX resources the shipped `cmake/MaterialX` config
    `set_and_check`s), so a slim runtime stays consumable by any `find_package(pxr)`
    plugin; plus an optional build-dep preflight (warn on missing
    `Jinja2`/`PyOpenGL`/`PySide6` before invoking `build_usd.py` when the profile
    implies usdview + schema tooling).
  - **P1 — `ost artifact push` (the producer verb):** a write-capable OCI backend
    behind the existing `ArtifactTransport` contract (filesystem backend
    unchanged) that emits the exact OCI layout / media-types `ost artifact pull`
    already expects, pushes archive + manifest + validation report as layers,
    prints the immutable OCI digest (for pasting straight into
    `runtime_remote.expected_oci_digest`), and is atomic / content-addressed /
    idempotent (re-push of an identical digest succeeds, digest mismatch is a hard
    error). `--json` output carries every digest (manifest / archive / registry).
    Also: use the `oras`/docker credential store (or clearly document
    `OST_REGISTRY_USER`/`_PASSWORD`), and document the WebUI-only GHCR
    visibility-flip step. Spelled `ost runtime publish --to oci://…` may front it.
  - **Deferred (v0.13.0+):** protected-namespace / allowed-publisher policy, OIDC
    publisher verification, provenance / SBOM bundle, trust levels in the support
    matrix, and generated trusted-publish CI — sequenced below (after the v0.11.0
    producer-correctness slice and the v0.12.0 hosted macOS CI/runtime-contract
    hardening slice).
- ✅ **v0.11.0 — producer-side correctness + Linux runtime portability.** The
  first *produce*-side hardening pass, from the 2026-07-10 recheck dogfooding
  report (`2026-07-10-v0.10.0-recheck-v0.11.0-asks.md`). That report re-verified
  every v0.10.0 P0/P1 ask as landed (from-source version stamp, runtime
  schema-gen deps, `--slim` keeps `resources/`, the `ost artifact push` CLI
  surface) and proved public Windows **and** Linux runtime *consumption* by
  digest — but standing up the producer side exposed one hard ABI-labeling
  blocker plus a cluster of producer rough edges that must be boring before the
  trust-policy ladder layers on top. The 2026-07-11 macOS CI dogfood then exposed
  one more runtime-contract slice before trust policy can be boring, so trust
  starts at v0.13.0 and the ladder shifts to v0.13.0–v0.16.0. Details in the
  [Phase 6 — v0.11.0 backlog](#phase-6--v0110-backlog-from-the-2026-07-10-recheck-dogfooding-report).
  Scope:
  - ✅ **P0 — real glibc floor for Linux runtimes (BLOCKER; report ask #7).** The
    WSL/Ubuntu-26.04-built Linux runtime was exported with the ABI label
    `linux-x86_64-glibc228-py313`, but its binaries reference `GLIBC_2.43`, so it
    fails to load on the GitHub-hosted `ubuntu-24.04` runner (glibc 2.39) during
    `usdGenSchema` (`version GLIBC_2.43 not found`, exit 6) — and
    `--require-target` string-matched the *fabricated* label and passed, giving
    false ABI confidence. **Landed:** a streaming ELF scanner (`ost_build::glibc`)
    computes the maximum referenced `GLIBC_x.y` symbol version from the packed
    binaries (no binutils dependency), and `ost runtime export` stamps *that* real
    floor onto the artifact `target`, overriding the fabricated nominal and
    recording the drift as `glibc_floor` evidence. With a truthful label the
    existing `--require-target` string match now rejects a stale `glibc228` pin, so
    the false-pass is closed at its source. Still ⬜ (a P2 doc): a note that portable
    Linux runtimes are built against an old glibc base (manylinux_2_28 /
    older-Ubuntu / container).
  - ✅ **P0 — fix `ost artifact push` against GHCR (report ask #1).** The producer
    verb shipped in v0.10.0 could not actually push to GHCR: `OST_REGISTRY_TOKEN`
    returned 403, and `OST_REGISTRY_USER`/`_PASSWORD` advanced to upload but sent an
    invalid `digest=sha256:sha256:<hex>` request (a double-`sha256:` producer bug)
    and failed with HTTP 400 — while `oras` with the same credentials worked
    immediately. **Landed:** the double-`sha256:` bug is fixed — `push` used
    `digest::sha256_hex` output (already `sha256:<hex>`) verbatim, and a regression
    test asserts every uploaded blob digest is well-formed, closing the upload
    blocker. A write that is denied now gets a push-specific, auth-mode-keyed hint
    (a `static-token` 403 steers to the user/password token exchange for `pull,push`
    scope; a rejected credential points at `write:packages`), and the docs mark
    `OST_REGISTRY_TOKEN` pull-only. Still ⬜: a real GHCR round-trip to confirm the
    user/password path end to end (needs live credentials).
  - ✅ **P1 — Linux symlinks in runtime export (report ask #2).** Linux SDKs
    contain normal shared-library symlink chains (`libFoo.so → libFoo.so.1 →
    libFoo.so.1.39.4`; the built runtime had 48). `runtime export --slim` rejected
    them via the package-staging safety model (`symlink is not allowed in the
    package staging area`), blocking direct export of a build-source Linux runtime
    (the report worked around it with a `cp -aL` dereference + re-adopt).
    **Landed:** the packaging model now preserves a *safe, relative, in-tree*
    symlink as a link entry (never dereferencing it, so 48 soname links stay 48
    links, not 48 full copies), while an absolute target or a `..`-escape past the
    stage root is still a hard error — SEC-001's link-escape protection is intact.
    A validated `link_target` rides through the producer manifest, `walk_archive`
    (which re-checks the target lexically and flags an escaping link as unsafe),
    and the per-file comparison (so a symlink can't be confused with a regular
    file whose bytes equal the target). `Store::extract` gained a pre-extraction
    safety gate that refuses an unsafe entry before unpacking a byte. Unit-tested
    on both the produce and consume sides plus a full CLI `export → artifact
    handoff → pull --from-artifact` round-trip that proves a soname chain survives
    as symlinks.
  - ✅ **P1 — quieter / actionable package-stage fallback (report ask #4).**
    `STAGE_FALLBACK` now names how many stale fallback siblings are still locked
    and points at the `--clean-stage` escape hatch (on both `ost package` and
    `ost plugin package`), which reclaims the stable stage name harder and sweeps
    leftovers deterministically once the holder exits (reported as
    `STAGE_CLEANED`). The sweep returns a real swept/leftover accounting instead
    of being silent best-effort. Unit-tested.
  - ✅ **P2 — first-class repo-specific smoke tests in generated source CI (report
    ask #5).** The support matrix now takes a declarative `source_checks:` list;
    each entry (`name` + bash `run`) renders as a step in the generated source-CI
    workflow after the verification pyramid and before packaging, so regenerating
    never silently drops repo coverage like the `Run corpus CTest smoke` step.
    Both fields are validated (charset-hardened name, control-char-free run, no
    `secrets.` — preserving fork-PR safety). Unit-tested (parse/validate + render
    placement/order).
  - ✅ **P2 — document Linux build prerequisites (report ask #3).** `docs/examples.md`
    now documents the Linux `--build` prerequisites (dev packages incl.
    `libxt-dev`/`unzip`, the source-built `--enable-shared` Python 3.13, the
    Python-package preflight, and the portable-runtime glibc-floor caveat). The
    504-HTML-as-archive flake is documented as an **upstream** `build_usd.py`
    downloader issue with a `codeload.github.com` pre-seed workaround — it lives
    in the script `ost` shells out to, not `ost`'s own digest-verified transport,
    so `ost` cannot repair it.
  - ✅ **P2 — re-test build-host dep preflight on a clean Python (report ask #6).**
    Verified: the preflight probe (`importlib.util.find_spec` over the
    profile-implied imports) correctly reports absent modules, and the warning —
    including the exact `pip install` fix line — was extracted into a pure,
    unit-tested `missing_dep_warning` so the previously-untestable message path is
    now covered without needing a pristine interpreter.
- ⬜ **v0.12.0 — hosted macOS CI + runtime-contract hardening.** The 2026-07-11
  macOS OpenUSD 26.05 PR CI dogfood proved the public GHCR runtime pull,
  artifact verification, and Windows source-CI path, but the hosted macOS lane
  only passed after repo-local temporary repairs (`actions/setup-python` for
  Python 3.13 and `chmod +x` on the materialized runtime `bin/`). v0.12.0 makes
  those requirements first-class so generated source CI does not need hand edits.
  Details in the
  [Phase 6 — v0.12.0 backlog](#phase-6--v0120-backlog-from-the-2026-07-11-macos-openusd-2605-pr-ci-dogfooding-report).
  Scope:
  - **P0 — republish macOS runtimes with executable tool bits preserved and
    verified.** `runtime export` / artifact packaging must preserve Unix execute
    modes for runtime `bin/` tools, `runtime validate` must catch a materialized
    runtime whose tools are not executable, and the public cy2026 macOS arm64
    OpenUSD 26.05 SDK artifact must be republished. Acceptance: a clean
    GitHub-hosted `macos-15-arm64` source-CI lane reaches `ost plugin test
    --up-to 5` with no `chmod` repair step.
  - **P0 — make schema-generation Python an explicit runtime/CI contract.** A
    `py313` runtime that does not bundle `bin/python3.13` cannot leave CI relying
    on an accidental host interpreter. Either the runtime supplies a runnable
    interpreter or generated source CI installs the declared Python ABI before
    `ost plugin build`; local failures name the missing interpreter and the exact
    setup fix before invoking `usdGenSchema`.
  - **P1 — model pre-build requirements directly, not as late `source_checks`.**
    The existing `source_checks:` hook intentionally runs after the verification
    pyramid, so it is the wrong escape hatch for runtime/toolchain repair. Keep
    repo-specific smoke tests there, but add first-class pre-build rendering for
    modeled prerequisites such as host Python setup; avoid a broad arbitrary
    pre-build shell hook unless another dogfood proves it is needed.
  - **P1 — CI evidence for repaired assumptions.** Generated reports should make
    the runtime artifact digest, OCI digest, Python ABI/setup source, runtime-bin
    mode validation, and source-CI lane outcome visible enough that a future
    failure is attributable to the contract, not to a hidden runner state.
  - **P2 — docs and cleanup.** Document the macOS hosted source-CI contract,
    remove the temporary branch-local setup/chmod steps after the runtime is
    republished and CI regenerated, and keep the dogfood recipe as the acceptance
    check for Windows + macOS public-runtime consumption.
- ⬜ **v0.13.0 — trust policy foundation.** With a producer verb and stable hosted
  source-CI runtime contract in place, close the publish-side trust boundary
  (future-policy §3.2/§7/§11). An `openstrata-artifact-policy.toml` with
  protected-namespace + allowed-publisher schema and a `local`/`unsigned`/
  `attested`/`verified`/`trusted` trust-level enum; a policy parser with stable
  `ARTIFACT_POLICY_*` codes; OIDC publisher verification (match repository /
  workflow path / git ref / actor / event against the allowed-publisher list,
  reject protected-namespace publish from an untrusted identity,
  `--allow-untrusted-publisher` as the explicit escape hatch); and
  `ost artifact verify --policy`. Tracks SEC-006 and the Phase 6 trust-policy
  hooks.
- ⬜ **v0.14.0 — provenance / SBOM bundle.** Make the artifact an *evidence
  bundle*, not just an archive (future-policy §5/§6/§11): optional SBOM
  (`sbom.spdx.json`) and SLSA/in-toto provenance (`provenance.intoto.jsonl`)
  layers, `ost artifact push` attaching them and `ost artifact verify
  --require-sbom` / `--require-provenance` checking that the provenance subject
  digest matches the OpenStrata artifact digest, the builder identity matches the
  allowed-publisher policy, and source repo/revision match build metadata. Closes
  the Licensing & attribution "per-artifact SBOM" and Phase 6 "content attribution"
  gaps for published artifacts.
- ⬜ **v0.15.0 — generated trusted CI.** Push the trust chain up into the CI
  contract (future-policy §7/§8/§13): a `trust` field on support-matrix targets, a
  minimum-trust requirement per lane (`pr_min_trust` / `main_min_trust` /
  `release_min_trust`), and lane-specific generated workflows — the PR / source-CI
  lanes stay publish-free, and a separate **trusted runtime-publish lane**
  (protected branch/tag, OIDC, SBOM + provenance + validation report required,
  protected-namespace policy enforced) is generated distinctly from the release
  lane. Release workflows refuse untrusted artifacts.
- ⬜ **v0.16.0 — DCC host integration.** Extend the support matrix beyond
  runtime-native apps to external DCC hosts (future-policy §9/§11; Phase 10
  [dcc-hosts.md](dcc-hosts.md)). Read-only host discovery + fingerprint
  (`ost dcc discover`, host record schema, Maya/Houdini detectors first), headless
  plugin compatibility test, and DCC support-matrix + CI-annotation integration —
  *without* a DCC API abstraction or SDK redistribution (future-policy §13
  non-goals).
- ⬜ **v1.0.0 (after v0.16.0).** The downstream 2026-07-09 report framed its asks
  as "v1.0.0-rc1"; that framing is **superseded** by the finer v0.10.0–v0.16.0
  ladder above (the report's runtime-completeness asks landed as v0.10.0 P0, its
  `ost artifact push` ask as v0.10.0 P1, and the 2026-07-10 recheck's producer
  correctness/portability asks are the v0.11.0 slice). 1.0 is cut once the
  produce→trust→
  provenance→trusted-CI arc and the initial DCC host matrix are shipped and
  dogfooded — i.e. "build it, publish it, verify its provenance, pull it in trusted
  CI, run it against a DCC host" is a single supported, digest-addressed arc.

## Phase 0 — Foundation ✅

Rust workspace + `ost` CLI skeleton, machine-readable platform manifests, project
and lock schemas.

- ✅ `ost-core` / `ost-platform` / `ost-manifest` / `ost-cli` crates
- ✅ Built-in CY2025 / CY2026 / CY2027 manifests (embedded + user overlay)
- ✅ `ost platform list | show | diff`
- ✅ `ost init` (writes `openstrata.toml` + `.strata/`)
- ✅ JSON schemas for platform / project / lock
- ✅ `--json` output and deterministic exit codes — a versioned
  `{ok, schema, data, warnings}` envelope with stable `error.code`/`category` and
  category-based exit codes ([json-schema.md](json-schema.md))

## Phase 1 — Runtime and devshell ✅

Resolve a runtime manifest, lay it out locally, generate environment, enter a shell.

- ✅ Runtime target model + resolver (`ost-runtime`)
- ✅ Profile model + loader (`core`/`dev`/`usd`/`lookdev`)
- ✅ Environment generation (`PATH`, `LD_LIBRARY_PATH`/`DYLD_*`/`PATH`, `PYTHONPATH`,
  `CMAKE_PREFIX_PATH`, `PXR_PLUGINPATH_NAME`)
- ✅ `ost env` and `ost devshell` (bash/pwsh)
- ✅ `ost runtime pull | list | show` against a local/mock backend
- ✅ Digest-bearing runtime manifest (`runtime.json`, deterministic digest)
- ✅ `ost doctor` (host descriptor, host tool detection, runtime report;
  deterministic exit: 0 healthy / precondition code (4) on issues)
- ✅ **(v0.4.0)** `ost doctor` structuring (§14.5): issues are now structured
  `{id, severity, summary, next_action}` (human + `--json`), the runtime report
  surfaces `kind` (mock/adopted/built/downloaded, derived from the manifest
  `source` — no schema change) plus its execution capability (real OpenUSD vs
  static-only), and an active mock runtime emits a `MOCK_RUNTIME_ACTIVE`
  *warning* that does not fail the run (only `error`-severity issues do). Absorbs
  the agent "status" need into `doctor` rather than a new command
- ✅ `ost runtime validate` (schema, digest integrity, layout; records outcome
  in the manifest; deterministic exit)
- ✅ `ost runtime explain` (delivered in Phase 3)
- ✅ Project lockfile `strata.lock` via `ost lock [--check]` and refreshed by
  `ost configure`: pins runtime id/variant/digest, Python ABI + `uv.lock` hash,
  resolved extensions, and validation status; fully deterministic so `--check`
  gates CI
- ✅ Real runtime backends behind `pull` (Phase 4b): `local`/adopt and `build`
  (build_usd.py / CMake-direct) supersede the mock layout; the fetched
  `artifact` source landed with the Phase 6 registry (v0.6.0:
  `runtime export` / `pull --from-artifact`)
- ✅ Richer runtime validation: `runtime validate` asserts `usdcat` + `pxr` on a
  real runtime; native library load + USD stage open are exercised by the plugin
  execution levels (L2–L4, Phase 4b)

## Phase 2 — CMake target build ✅

- ✅ Target model + id (`cy2026-linux-x86_64-py313-usd`) in `ost-build`
- ✅ `ost configure`: `toolchain.cmake`, `env.json`, `target.lock.json`,
  per-target `CMakePresets.json`, and a root `CMakePresets.json` that includes
  each target (verified with `cmake --list-presets`)
- ✅ `ost build`: regenerates the target then runs `cmake --preset` +
  `cmake --build`; locates ninja on PATH / `OST_NINJA` / `--ninja`; `--dry-run`
  and `--jobs`; propagates the build exit code (verified end-to-end: a real
  MSVC+Ninja build of a sample project produced and ran an executable)
- ✅ Windows MSVC-env auto-bootstrap inside `ost build`: locates `vcvars64.bat`
  (vswhere or known paths), captures the env delta, injects it into CMake/Ninja;
  `--no-vcvars` to opt out (verified: a plain shell with no developer prompt
  builds and runs an executable)
- ✅ `ost package`: `cmake --install` into a stage tree, pack to
  `dist/<name>/<version>/<target>/*.tar.zst` with per-file SHA-256, a
  content-addressed `manifest.json` (provenance + runtime digest + validation),
  and `SHA256SUMS` (verified: archive extracts and the binary runs)
- ✅ `ost validate`: checks configured / built / runtime-compatible (digest
  drift) / artifact-integrity (recomputed archive digest); skips the artifact
  check when not packaged; deterministic exit 0/1 (verified: tampering the
  archive fails the check)

## Phase 3 — OpenUSD / MaterialX profiles ✅

- ✅ OpenUSD extension family with feature sets (core/python/imaging/materialx/…)
  and MaterialX, in the new `ost-extension` crate (embedded + overlay loader)
- ✅ Capability resolver: capability → providing extension + feature, pulling in
  transitive extensions (usd-materialx → openusd[materialx] → materialx) and the
  packages each feature needs
- ✅ Compatible range vs certified build point (chosen per resolved feature set)
- ✅ `ost runtime explain` (capability → provider/extension graph, human/--json)
- ✅ `ost extension list | why | add`: list the catalog, trace why an extension
  is required by a profile (direct + transitive), and record it in
  `openstrata.toml` (idempotent, validated against the catalog)

## Phase 4 — OpenUSD plugin verification harness 🚧

Direction: [phase-4-plugin-harness.md](phase-4-plugin-harness.md). Split around
the one hard dependency — a real OpenUSD runtime (today's `runtime pull` is mock).

**4a — framework + static verification (mock backend, no real runtime): ✅**

- ✅ `ost-plugin` crate + Plugin Bundle contract (`openstrata.plugin.yaml`):
  manifest model, bundle loader, dependency-free version-range checks
- ✅ `ost plugin new` scaffold from the embedded `usd-fileformat-cpp` template
  (C++ `SdfFileFormat` + `plugInfo.json` + `CMakeLists` + fixtures + manifest)
- ✅ `ost plugin inspect` (Level 0 structure) and `ost plugin build` (generates a
  toolchain via `ost-build` and drives CMake; `--dry-run`)
- ✅ `ost plugin doctor`: Levels 0–1 (manifest, plugInfo, shared library,
  fixtures; OpenUSD range / ABI / required components) with stable diagnostic ids
  + session-env preview; Levels 2+ reported as `SKIP (needs real runtime)` —
  never a false PASS
- ✅ reports under `.strata/reports/<plugin>/<UTC>/` (`report.json` /
  `summary.txt` / `environment.json`) + published
  [plugin-report JSON schema](../schemas/plugin-report.schema.json);
  human + `--json`, deterministic exit codes

**4b — execution levels (gated on a real OpenUSD runtime backend): 🚧**

- ✅ pluggable runtime backend **sources** behind `pull`
  (`mock|local|build|artifact`), recorded in the manifest (`mock: bool` →
  `source`); `source`-aware validation and provenance everywhere
- ✅ **`local`/adopt source** — `ost runtime pull … --from-usd <path>` (or
  `OST_USD_ROOT`) adopts an existing OpenUSD install in place; `EnvSet` maps
  USD's own layout (`lib/python`, `plugin/usd`); `runtime validate` asserts
  `usdcat` + `pxr`; `plugin doctor` L1 surfaces the source (real but not
  reproducible/certified)
- ✅ `ost plugin run` session launcher (composes the runtime `EnvSet` + bundle
  roots, execs a command, propagates the exit code; no global mutation)
- ✅ Levels 2–5 executed against a real runtime via a `Probe` seam (unit-test
  injectable): L2 discovery (`Sdf.FileFormat.FindByExtension`), L3 `usdcat`
  read, L4 `Usd.Stage.Open`, L5 golden round-trip (`usdcat --flatten` vs
  `<fixture>.golden.usda`); `ost plugin test` orchestrates L0..L5 + report.
  `EnvSet::for_usd_install` probes `lib/python` vs `lib/site-packages`.
  Verified end-to-end against a real OpenUSD 25.05 build.
- ✅ `build` source — `ost runtime pull … --build <usd-src>` builds OpenUSD from
  source into the store (one-time; re-pull is a cache hit), bootstrapping the
  MSVC env on Windows like `ost build`. Two modes:
  - **build_usd.py** (default) — drives the source tree's
    `build_scripts/build_usd.py`, which fetches+builds dependencies itself.
  - **CMake-direct** (`--deps <prefix>…`) — builds OpenUSD directly with CMake
    against pre-provided dependency prefixes (`CMAKE_PREFIX_PATH`), faster and
    aligned with OpenStrata's resolver; sets up deps-as-extensions (Phase 6).

  `--jobs` and `--build-arg` (hyphen-allowed) tune either mode. Both verified by
  building a real OpenUSD 25.05 and running `ost plugin test` against it.
- ✅ Level 6 — `ost plugin view <bundle> <fixture>` opens a fixture in usdview
  inside the runtime session; `ost plugin test-view` (and `test --up-to 6`) runs
  the non-interactive `usdview --quitAfterStartup` launch probe (`usdview.launch`
  diagnostic), SKIPping cleanly when usdview or a display is unavailable.
  Verified against a real usdview-enabled OpenUSD 25.05 build.
- ✅ Multi-plugin sessions (`ost plugin doctor/run/test/view/test-view --with
  <bundle>…`) and bundle-declared `requires.runtime_libs` (extra non-USD runtime
  lib dirs, e.g. a plugin's zlib) — replaces hand-rolled usdview launch batch
  files for the multi-plugin + 3rd-party-dep case. Downstream plugin dogfooding
  (reports #1/#2) surfaced these prerequisites and shapes:
  - **Every bundle path is absolutized at the `ost plugin` boundary** via
    `Bundle::load`, including every `--with <bundle>` arg. Its plugInfo root,
    `lib/`, `python/`, and any `requires.runtime_libs` dir are then composed as
    absolute session env entries, avoiding relative `CMAKE_TOOLCHAIN_FILE` and
    relative `PXR_PLUGINPATH_NAME` failures.
  - **`requires.runtime_libs` prepends to the session's dynamic-loader path**
    (`PATH` / `LD_LIBRARY_PATH` / `DYLD_LIBRARY_PATH`), absolutized and validated
    as bundle-relative. Empty/absent stays the common case: a plugin that
    statically links its 3rd-party deps (e.g. vendoring a parser into one TU,
    exporting no symbols) drags zero extra lib dirs — the opposite of a plugin
    shipping a sibling `zlib.dll`.
  - **`plugInfo.json` `LibraryPath` is generated/validated per target** (suffix +
    lib-dir), since multi-plugin × multi-OS sessions multiply the scaffold's
    cross-platform soft spot. Source bundles may carry templates, but built or
    packaged bundles must carry the concrete `plugInfo.json` for exactly one
    target (`.so` / `.dylib` / `.dll`) and one library layout. See the Phase-4
    fix backlog below.
- ✅ `ost plugin package`: freezes the target-resolved `plugInfo.json`, resolved
  C++/Python ABI, runtime digest/source/validation, static validation report,
  and session environment into a target-specific binary bundle artifact
  (`tar.zst` + `manifest.json` + `SHA256SUMS` under
  `<bundle>/dist/plugins/<name>/<version>/<target>/`)
- ✅ **(v0.6.0)** `ost plugin publish` into the local artifact registry (Phase 6
  MVP; see Phase 6 for the gates). Still ⬜: the runtime×plugin CI matrix and the
  fetched `artifact` runtime source.

### Phase 4 — fix backlog (from downstream plugin dogfooding, reports #1/#2)

A freshly scaffolded `usd-fileformat` bundle did not survive `ost plugin
build`/`test` on Windows out of the box. Ranked, with the implicated code:

Policy from the new cross-platform review: a **source** plugin bundle is not a
universal binary bundle. Source may declare compatibility ranges and generation
templates; `ost plugin build/package` emits a **target-specific** binary bundle
whose `plugInfo.json`, ABI metadata, and provenance are resolved from the CMake
target + runtime variant. `doctor` should validate the resolved files for the
target being tested, not silently accept host-default metadata.

- ✅ **P1 — absolutize `<bundle>` once** in `Bundle::load`
  ([bundle.rs](../crates/ost-plugin/src/bundle.rs)): a single `canonicalize`
  removes *both* the relative-`CMAKE_TOOLCHAIN_FILE` build break and the
  relative-`PXR_PLUGINPATH_NAME` discovery break (single root cause), de-UNCing
  the `\\?\` prefix on Windows (CMake/USD mishandle it). Prerequisite for `--with`
  (above).
- ✅ **P1 — scaffold `plugInfo.json` can't load its own lib.** Was
  `LibraryPath: "lib{{Name}}FileFormat.so"` (wrong suffix off-Windows; beside
  `plugInfo.json` while the lib lands in `lib/`, and USD dlopens the absolutized
  path with no PATH fallback). Now a committed
  [`plugInfo.json.in`](../templates/usd-fileformat-cpp/plugin/resources/{{name}}/plugInfo.json.in)
  (`../../../lib/lib…@CMAKE_SHARED_LIBRARY_SUFFIX@`) that the CMake
  `configure_file` resolves per target; `ost plugin new` also writes a
  host-correct concrete `plugInfo.json` so `doctor`/`test` work before the first
  build (doctor L0 only checks existence + JSON parse, so no collision).
- ✅ **P1 — scaffold `CMakeLists.txt` stages to `${CMAKE_SOURCE_DIR}/lib`**
  ([templates/usd-fileformat-cpp/CMakeLists.txt](../templates/usd-fileformat-cpp/CMakeLists.txt)):
  now uses `CMAKE_CURRENT_SOURCE_DIR` (so an `add_subdirectory()` consumer stages
  the lib in the bundle, not the repo root) and guards `find_package(pxr)` with
  `if(NOT pxr_FOUND)` so a dual-mode project root can resolve it once.
- ✅ **P1 — `ost plugin build` doesn't bootstrap the MSVC env.** `run_step`
  ([commands/plugin.rs](../crates/ost-cli/src/commands/plugin.rs)) now loads the
  MSVC developer environment via `ost_build::msvc::bootstrap()` (Windows, `cl` not
  on PATH), as `ost build`/`runtime pull --build` already do.
- ✅ **P2 — default `CMAKE_BUILD_TYPE=Release` for plugin builds.** `ost plugin
  build`'s configure args now pass `-DCMAKE_BUILD_TYPE=Release`, so Ninja's
  single-config build no longer resolves USD's imported targets to Debug and
  links the missing `tbb12_debug.lib`. The runtimes OpenStrata ships/adopts are
  Release.
- ✅ **P2 — adopted-runtime version was the static placeholder.** `adopt_local`
  ([commands/runtime.rs](../crates/ost-cli/src/commands/runtime.rs)) now reads the
  real `PXR_MINOR/PATCH_VERSION` from the install's `include/pxr/pxr.h` and
  records it (e.g. `26.08`) instead of the catalog's `25.05.01`, so the Level-1
  version gate enforces the actual range. (Python-ABI detection — the `py313` id
  on a py310 install — is still a follow-up; the id parser would need the real
  interpreter version.)
- ✅ **P2 — `runtime show`/`validate` rejected the id `runtime list` prints.**
  Both now accept either `<platform> --profile <profile>` or the full
  `openstrata-cy2026-…-usd` id (the embedded platform/profile win); the variant
  slug is a fixed 3 tokens, so a hyphenated profile stays intact.
- ✅ **P1 — harden target-generated `plugInfo.json` beyond the scaffold.**
  A real downstream bundle with
  `LibraryPath: "../../../lib/lib<Name>FileFormat.dll"` is Windows-only even if
  README/CMake claim cross-platform support. Source commits `plugInfo.json.in`;
  CMake configures the concrete `plugInfo.json` with the target library prefix
  and `CMAKE_SHARED_LIBRARY_SUFFIX`; `ost plugin new` emits a host-concrete
  `plugInfo.json`; `doctor` now has `bundle.plug_info.library_path` to flag
  unresolved templates, non-`lib/` layout, mismatched built lib names, and suffix
  mismatches such as `.dll` on Linux/macOS or `.so` on Windows.
- ✅ **P1 — make source plugin C++ ABI metadata target-aware.**
  The scaffold no longer writes `runtime.cxx_abi: libstdcxx` into a source
  bundle. `ost plugin doctor` derives the runtime ABI from the resolved target
  (`linux → libstdcxx`, `macos → libcxx`, `windows-msvc143 → msvc143`) and still
  compares it when a hand-authored or future packaged manifest records a scalar
  `runtime.cxx_abi`. The binary package step records the one resolved ABI for
  the artifact it emits.
- ✅ **P3 (v0.4.0) — repo-shape scaffold.** `ost init --template plugin-workspace`
  emits a dual-mode root `CMakeLists.txt` + `CMakePresets.json`: it resolves USD
  once (`find_package(pxr)`) and **globs** every immediate subdirectory holding an
  `openstrata.plugin.yaml` + `CMakeLists.txt`, `add_subdirectory()`-ing each — so a
  repo of `ost plugin new` bundles is `cmake -S .`-able by non-`ost` users and new
  bundles are picked up with no edit. Each bundle's `if(NOT pxr_FOUND)` guard lets
  it build standalone (via `ost`) or under this root; the root `CMakePresets.json`
  is the user's own (untouched by `ost configure`, which uses
  `CMakeUserPresets.json`).

### Phase 4 — schema-bundle backlog (from downstream plugin dogfooding, reports #3/#4)

**Targeted for v0.4.0 (scope A).** A second dogfooding pass confirmed the #1/#2 fixes (relative-path
`plugin build|test`, MSVC bootstrap, `CMAKE_BUILD_TYPE=Release`,
`bundle.plug_info.library_path`, full-id `runtime show` all green) and took up the
typed-schema kind (`usd-schema`). `ost plugin new` advertises that kind but ships
no generator, and the harness models only file-format bundles. Ranked:

- ✅ **`usd-schema` (codeless) template + codeless-aware L0 doctor.** The embedded
  `usd-schema-codeless` template (starter `schema.usda` with one single-apply
  `*API` + the `customData` library block and `skipCodeGeneration`, a resource-only
  `plugInfo.json`, a `usdGenSchema` `CMakeLists.txt`, and an apply-the-API
  fixture); `ost plugin new usd-schema` scaffolds it instead of erroring. The
  manifest gained a `schema.codeless` flag (`is_codeless_schema()`), and
  `ost plugin doctor` L0 is now **codeless-aware** — it SKIPs
  `plugin.shared_library` and validates the `Types` block via a new
  `bundle.plug_info.schema_types` check instead of `bundle.plug_info.library_path`,
  so a valid resource-only schema no longer hard-fails. **E2e-hardened against a
  real OpenUSD 26.08:** the scaffold now commits *registerable* resources — a
  correct `Types` entry (`schemaIdentifier`/`schemaKind`/`bases`, no
  self-referential `alias`) plus a flattened `generatedSchema.usda` beside it — so
  a codeless schema registers in `Usd.SchemaRegistry` and applies out of the box
  with no build step.
- ✅ **Schema test contract (L2/L4), verified e2e.** `ost plugin test` runs
  schema-specific execution levels in place of the file-format discovery/read
  levels — **L2 `schema.registration`** (the `provides: usd-schema:<Type>` are
  known to `Usd.SchemaRegistry`) and **L4 `schema.apply_roundtrip`** (the smoke
  fixture applies an `*API` to a prim and its authored attributes survive a flatten
  round-trip), sharing the format-agnostic L5/L6. `ost plugin doctor`'s L2+ SKIP
  placeholders mirror these ids per kind, and the scaffold fixture authors a valid
  USD identifier namespace (`{{ident}}`, e.g. `vrm_schema:`). **Verified end-to-end
  against an adopted OpenUSD 26.08** (both levels PASS); also Probe-unit-tested.
- ✅ **Co-host a schema in an *existing* bundle (the consumable half).** USD lets
  one `plugInfo` provide both an `SdfFileFormat` and schema types; a bundle of any
  kind that declares `usd-schema:<Type>` in `provides` now runs the schema contract
  (L2/L4) *alongside* its primary-kind levels (gated on the explicit `provides`,
  not inferred `Info.Types`, so a file-format's own type is never mistaken for a
  schema). doctor's SKIP placeholders mirror it. Verified e2e: a `usd-fileformat`
  bundle co-hosting a codeless schema passes L2/L4. This is the co-location lean
  realized for the codeless case (commit the `Types` + `generatedSchema.usda` into
  the existing bundle — no second bundle, no `--with`).
- ✅ **`usdGenSchema` build step.** `ost plugin build` on a schema bundle runs the
  template's `usdGenSchema` `CMakeLists.txt` step to regenerate the codeless
  resources (`plugInfo.json` + `generatedSchema.usda`). The fix that made it work:
  the build now composes the runtime **session** env (not just the MSVC delta) so
  usdGenSchema can load `pxr` and resolve the base USD schemas
  (`@usd/schema.usda@`); and ost parses the regenerated `plugInfo.json` as
  JSON-with-comments (usdGenSchema writes a `#` banner). Note `usdGenSchema` itself
  must be present in the runtime and needs `jinja2`/`PyYAML` — OpenUSD skips
  installing it when those are absent at USD build time. **Verified end-to-end
  against an adopted OpenUSD 26.08**: build → regenerate → `ost plugin test`
  L0..L4 PASS.
- ✅ **(v0.5.0) Compiled, co-located schema flow — "add a schema to an existing plugin".**
  `ost plugin build` now recognizes a non-schema bundle that declares
  `usd-schema:<Type>` and ships `schema.usda`, runs `usdGenSchema` in the composed
  runtime/session environment, stages generated typed API sources into the same
  plugin library via a generated CMake fragment, drops Python-module helper files,
  defines the generated `*_EXPORTS` macro, merges the schema `Types` into the
  bundle's existing `plugInfo.json`, copies `generatedSchema.usda`, and also
  merges `Types` into matching `tests/cmake/**/plugInfo.json` registries when a
  bundle's CTest path carries its own plugin registry. If `usdGenSchema` emits no
  C++ files (for example a `skipCodeGeneration` codeless schema), the flow falls
  back to the resource-only merge path.
- ✅ **(v0.6.0) First-class co-located schema UX: `ost plugin schema add`.**
  One command turns an existing non-schema bundle into a schema co-host: it
  scaffolds a starter schema source (default `schema/schema.usda`; compiled by
  default, `--codeless` for resource-only; `--class` picks the source class,
  composed as `<PascalBundleName><Class>` to stay clear of the
  `schema.library_prefix` double-prefix footgun) and wires the manifest
  *textually* — `provides: usd-schema:<Type>` plus the new bundle-relative
  `schema.source` field (validated in-bundle, SEC-002) — preserving the user's
  comments and re-parsing before writing back. The build flow and the
  `schema.library_prefix` doctor hint honor `schema.source`; a
  declared-but-missing source is a configuration error rather than a silent
  no-op.
- ✅ **`usdGenSchema` `Types` merge into a co-hosting bundle.** `ost plugin build`
  on a co-hosting bundle (a non-schema kind shipping a `schema.usda` and declaring
  `usd-schema:<Type>`) runs usdGenSchema to a staging dir and **merges** the
  generated schema `Types` into the bundle's *existing* `plugInfo.json` —
  preserving the `SdfFileFormat` entry usdGenSchema would otherwise clobber — then
  copies `generatedSchema.usda` beside it. Backed by a pure, unit-tested
  `merge_schema_types`. **Verified e2e on OpenUSD 26.08:** the file-format type is
  kept alongside the merged schema, and `ost plugin test` passes L2/L4. A no-op
  (committed resources kept) when there is no `schema.usda` or no usdGenSchema.
- ✅ **Per-variant `cxx_abi` in the source manifest.** `runtime.cxx_abi` now
  accepts a scalar (`msvc143`), a per-OS map
  (`{ windows: msvc143, linux: libstdcxx, macos: libcxx }`), or the `inherit`
  sentinel (defer to the runtime), via a `CxxAbi` enum. The L1 `runtime.cxx_abi`
  check resolves the declared ABI against the target OS before comparing — PASS/FAIL
  on a match/mismatch, SKIP for `inherit` or a target the map doesn't list — so a
  cross-platform source bundle no longer needs hand-editing per target. `ost plugin
  package` freezes the one resolved ABI as a scalar into the artifact. The
  scaffold's file-format template documents the three forms. Unit-tested
  (parse + per-OS/inherit resolution + doctor PASS/FAIL/SKIP).

### Phase 4 — v0.5.0 stabilization backlog (reports #5/#6 + a macOS source-build pass)

A later dogfooding pass on **0.4.0** verified the shipped schema work end-to-end —
`ost plugin new usd-schema` scaffolds a real codeless bundle (asks #1/#3 met),
`ost init --template plugin-workspace` answers the "no root CMake" ask, and a
**macOS arm64 `ost runtime pull --build`** built OpenUSD 25.05.01 from source with
imaging/usdview, then `runtime validate` + `ost plugin test --up-to 6` + CTest all
passed (Phase 4b `build` source confirmed on a second platform). It also took the
Phase 4 schema lean further — building a *compiled, co-located* schema by hand —
and surfaced the v0.5.0 stabilization shape: close correctness/ergonomics gaps
first, keep the compiled schema flow as stretch unless it stays small.

- ✅ **Force UTF-8 for the schema-gen step (locale-encoding bug).** `usdGenSchema`
  writes generated files in the process locale encoding; on a Japanese-locale
  Windows host (cp932) a non-ASCII char (an em-dash) in a `doc=` string aborts with
  `'cp932' codec can't encode`, and the error points at the codec, not the offending
  doc string. The `ost`-owned schema step (the shipped build step and the compiled
  flow above) now sets `PYTHONUTF8=1` / `PYTHONIOENCODING=utf-8` in the composed
  schema build env; the codeless template's own CMake target does the same via
  `cmake -E env` and invokes `python usdGenSchema ...` so direct CMake builds
  are protected on Windows too. The starter `schema.usda` prose is ASCII, while
  edited UTF-8 doc text remains supported.
- ✅ **Schema name-composition guidance (the double-prefix footgun).**
  `usdGenSchema` prepends `libraryPrefix` to the class name for the C++/TfType, so a
  `libraryPrefix` equal to the plugin name plus a class already carrying that name
  doubles it (`Foo` + `FooBarAPI` → `FooFooBarAPI`), while the USD identifier/token
  stays the class name. The codeless scaffold now avoids this by keeping the
  source class unprefixed (`API`) while the generated/public schema type remains
  `<Name>API`; `ost plugin doctor` emits a non-failing `schema.library_prefix`
  hint if edited `schema.usda` reintroduces the repeated leading token shape.
- ✅ **Runtime OpenUSD version truth.** Still reported on 0.4.0: an adopted install
  that is actually 26.x can be recorded as the placeholder `25.05.01`, so the L1
  range check "passes" for the wrong reason. Landed: adopt-time
  `detect_openusd_version` reads `pxr.h`
  ([runtime.rs](../crates/ost-cli/src/commands/runtime.rs)); `ost plugin doctor`
  prefers the install's real `pxr.h` version for L1; `runtime show` flags a
  manifest/install drift in both human output and `--json`; and `runtime validate`
  fails stale manifests with an `openusd-version-drift` check. Repair stays
  explicit: re-pull with `--force --from-usd` so the manifest digest/provenance is
  refreshed deliberately. **(v0.6.0)** one-step repair landed: `ost runtime
  repair` re-adopts a `local` runtime from its recorded USD root (re-reads
  `pxr.h`, re-probes the layout, resets validation to pending), and every drift
  report prints the exact per-source fix command — `repair` for adopted
  runtimes, the pinned `--from-artifact <digest>` re-pull for artifact
  runtimes, a `--build` re-pull for built ones.
- ✅ **`init --template` naming + discoverability.** `plugin-workspace` was hard to
  find: no `ost workspace` command reinforces the term; the `init --template`
  choices mix axes (`cpp-library` = language, `usd-plugin` = domain,
  `plugin-workspace` = repo shape); "plugin" is overloaded (an `init` template *and*
  the `ost plugin` subcommand that populates the repo); and `plugin-workspace` drops
  the `usd-` prefix the other USD templates carry. `usd-plugin-workspace` is now
  the canonical displayed name, `plugin-workspace` remains a compatibility alias,
  `init --help` surfaces the shape, and `ost plugin new` points multi-bundle users
  at `ost init --template usd-plugin-workspace`.
- ✅ **Workspace template: support a nested `plugins/<name>/` layout.** The
  `plugin-workspace` root auto-globs **root-level** bundle dirs; a repo that nests
  bundles under `plugins/` (the "one project → N bundles under `plugins/`"
  convention) isn't found. The workspace root now scans both immediate
  subdirectories and `plugins/*`, so `ost plugin new ... --dir plugins/<name>` is
  picked up by plain CMake without editing the root.
- ✅ **`--build` ergonomics surfaced by the macOS pass (overall a success).** Small
  `ost`-actionable follow-ups: (1) **Apple-Silicon codesign assumes full Xcode** —
  OpenUSD's `apple_utils.py` calls `xcodebuild -version`, which a Command-Line-Tools-
  only host lacks; the build needed a local patch to fall back to ad-hoc
  `codesign -s -`; `ost` now prints a macOS source-build note before `--build`.
  (2) **CMake 4 + bundled oneTBB** needs
  `-DCMAKE_POLICY_VERSION_MINIMUM=3.5`; README/examples/runtime notes document it
  as a known `--build-arg`. (3) **usdview needs Python UI packages** (`PySide6` /
  `PyOpenGL` / `Jinja2`) on `PATH`, and a direct `bin/usdview` without the composed
  env fails (no runtime `lib/python` on `PYTHONPATH`) — already solved by
  `ost plugin view`/`run` / `eval "$(ost env …)"`; the runtime build note now calls
  out the UI package prerequisite.
- ✅ **Doctor nudge: per-target metadata that 0.4.0 already supports but a bundle
  hasn't adopted.** The same pass found a hand-authored bundle still carrying a
  scalar `cxx_abi: msvc143` (fails on macOS `libcxx`) and a Windows `.dll`
  `LibraryPath` (macOS needs `.dylib`) — both already solvable in 0.4.0 (per-OS
  `cxx_abi` map; `plugInfo.json.in` per-target generation). A doctor hint when a
  scalar ABI or fixed-suffix `LibraryPath` mismatches the resolved target, pointing
  at the per-OS forms, now closes the adoption gap.

### Phase 4 — v0.9.0 macOS backlog (from the macOS dogfooding report, 2026-07-05)

**Released in v0.9.0.** The macOS dogfooding report (2026-07-05, `ost 0.8.0`
on macOS arm64, `plugins/usdVrm`, cy2026/usd) found `ost plugin build`
reproducibly failing on a co-hosted schema workspace, with three stacked
blockers — none of them C++ compilation itself: compile/link completes once
the early failures are forced past. The reliability gap is in
toolchain/loader assumptions during schema regeneration and plugin
discovery. Ranked:

- ✅ **P1 — resolve Python from the runtime, never a bare `python` on PATH.**
  The co-hosted schema regeneration step used to die with
  `error[IO_ERROR]: i/o error at run python: No such file or directory` on
  any host without a bare `python` executable (macOS ships `python3` only).
  `ost_build::resolve_run_python` now resolves the interpreter argv from the
  runtime — its bundled `bin/python3` first, then a version-matched host
  `python{ver}` / Windows `py -<ver>` / tool-cache interpreter, then
  `python3`, and only last a bare `python` — probed for runnability
  (`--version`), and `prepare_cohosted_schema`
  ([plugin.rs](../crates/ost-cli/src/commands/plugin.rs)) runs `usdGenSchema`
  through it. When nothing runs, a `PRECONDITION_FAILED` error names every
  candidate searched and the fix (unit-tested ordering; no more `IO_ERROR`).
- ✅ **P1 — schema regeneration must not require a pre-existing plugin
  binary.** `usdGenSchema` previously ran with the bundle's own
  `plugin/resources/…/plugInfo.json` discoverable, so USD tried to load the
  plugin library the build had not produced yet (or an old one with the
  wrong platform suffix) and failed. The schema-generation env is now scoped
  to the **runtime session alone** (`compose_build_env(&msvc_env, &r.env)`)
  — the bundle's `PXR_PLUGINPATH_NAME`/lib entries are left out, so
  `usdGenSchema` resolves the base USD schemas through the runtime registry
  but never discovers the bundle's own not-yet-built plugin. The report's
  temporary `plugInfo.json` move is no longer needed.
- ✅ **P1 — platform-aware `LibraryPath` in the co-hosted build flow.** A
  committed `plugInfo.json` carrying another platform's library suffix
  (`.dll` on a macOS host) used to fail plugin load mid-build. The
  regeneration phase no longer consumes that value (per the isolation item
  above), and where the file *is* consumed `plugin build` now runs
  `verify_target_library_suffix` after the build: a `plugInfo.json.in` source
  bundle has already had the per-target suffix configured, and a committed
  concrete path with the wrong suffix fails early as a `PRECONDITION_FAILED`
  carrying the doctor hint's exact fix — never USD's opaque loader error.
- ✅ **P2 — phase-attributed build diagnostics.** Every `ost plugin build`
  subprocess step now carries a phase — `schema-generate`, `configure`,
  `compile-link`, `schema-merge`, `plugin-discovery` — threaded onto the
  failure via a new `phase` slot on `Error::Coded` (surfaced as `error[CODE]
  (phase: …)` in human output and an `error.phase` field in the `--json`
  envelope). Verified e2e: a failing configure/compile-link build reports its
  phase in both modes; a wrong-suffix `plugInfo.json` reports
  `plugin-discovery`.

## Phase 5 — CI / Jenkins 🚧

- ⬜ CI-safe flags (`--ci`, `--no-interactive`, `--report junit|json`, `--jobs auto`)
- 🚧 Runtime×plugin CI matrix, backed by Phase 6 artifact digests:
  - ✅ **(v0.6.0)** explicit support-cell manifest (`openstrata.ci.yaml`, new
    `ost-ci` crate): each cell pins `runtime_artifact` × `plugin_artifact` by
    **full** registry digest (prefixes rejected — a prefix can silently start
    matching a different artifact) plus platform/profile, verification level
    (`up_to`), and host os/labels. `ost ci init` scaffolds it, `ost ci
    validate` checks structure, `--resolve` additionally requires every pinned
    digest to exist in the local registry.
  - ✅ **(v0.6.0)** GitHub Actions generation: `ost ci generate github` renders
    the matrix into a scheduled/dispatch workflow (`--stdout`/`--out`/
    `--force`), one job per cell via explicit `matrix.include` (never a
    Cartesian product, `fail-fast: false`), SHA-pinned actions (SEC-004). Each
    job re-verifies both artifacts, materializes the runtime
    (`pull --from-artifact`), extracts the plugin (`artifact extract`), runs
    `ost plugin test --up-to <level>`, and uploads the report. Runners need
    `ost` on PATH and the pinned artifacts in their `OST_HOME` registry
    (self-hosted labels are the expected case). e2e:
    [ci_matrix.rs](../crates/ost-cli/tests/ci_matrix.rs).
  - ⬜ JUnit + JSON report upload from `ost plugin test` (the generated
    workflow uploads the existing report dir; a JUnit format is still ahead)
  - ✅ scheduled/release gate for L0..L6 (the generated workflow is
    schedule + dispatch only); PR gate keeps cheap mock/static jobs
- ⬜ Jenkinsfile template + matrix generation (after the GitHub Actions shape is
  proven) — `ost ci generate jenkins` on the same `openstrata.ci.yaml` model

### Phase 5 — v0.7.0 backlog (from dogfooding report #8 + the CI build-matrix policy notes)

**Released in v0.7.0.** A downstream `usd-plugin-workspace` pass on 0.6.0
(report #8, 2026-07-04) exercised `ost ci init|validate|generate`, `ost lock
--check`, and `ost plugin package`, and a follow-up policy read (2026-07-05,
after a self-hosted-labeled PR workflow queued forever on a repo with no
registered runner) settled the CI model: `openstrata.ci.yaml` + named runner
profiles + digest-pinned artifacts is the portable contract; GitHub Actions is
its first renderer, not the source of CI semantics. Ranked:

- ✅ **P0 — `ost ci generate github` emits invalid YAML.** The workflow
  template joined the rendered `matrix.include` block with a `\` string-literal
  continuation ([github.rs](../crates/ost-ci/src/github.rs)); Rust's
  continuation also strips the next line's leading whitespace, so `steps:`
  landed at column 0 instead of under `jobs.cell`. Fixed with an
  `\x20`-protected indent; the unit test and the e2e now assert *placement*
  (`jobs.cell.steps` non-empty, no stray top-level key) — a column-0 `steps:`
  still parses as YAML, so a parse-only assertion misses the regression.
- ✅ **P0 — `strata.lock` extension versions don't match `runtime show`.**
  `build_lock` ([lock.rs](../crates/ost-cli/src/commands/lock.rs)) resolved
  extensions from the static catalog (`ost_extension::resolve`), not from the
  pulled runtime's manifest — an adopted OpenUSD 26.08 runtime locked as the
  catalog's certified `25.05.01`, and `ost lock --check` still reported
  `up_to_date: true` because `--check` re-derived from the same source. The
  lock now pins the pulled runtime manifest's extension records (the same
  source of truth `runtime show` reports); catalog resolution remains only as
  the pre-pull fallback. A lifecycle e2e reproduces the drift and asserts
  `--check` fails until a re-lock records the real version.
- ✅ **P1 — `ost plugin package` reruns are not idempotent.** A second package
  on Windows failed with access-denied (os error 5) at the reused
  `.strata/targets/<id>/package-stage`
  ([plugin.rs](../crates/ost-cli/src/commands/plugin.rs)): staging copies with
  `fs::copy`, which preserves the source's read-only attribute, and Windows
  refuses to delete read-only files. The stage reset now clears the attribute
  recursively and retries once (Windows-only; other platforms delete by
  parent-dir permission), unit-tested with a read-only staged file.
  *Incomplete — report #9 hit the same error on a stage with no read-only
  entries at all; the real second cause was transient scanner-held file
  locks. Superseded by the v0.8.0 staging-fallback fix below.*
- ✅ **P1 — placeholder digests pass validation too quietly.** `ost ci init`
  writes all-zero example digests and `ost ci validate` (without `--resolve`)
  accepted them silently. `validate` now warns per hit (human `WARNING:` lines
  + structured `CI_PLACEHOLDER_DIGEST` warnings in the `--json` envelope's
  `warnings` array — its first real use), and `ci generate github` refuses a
  placeholder matrix with the stable code `CI_PLACEHOLDER_DIGESTS` (exit 5)
  unless `--allow-placeholders` is passed.
- ✅ **P1 — runner profiles + lanes in `openstrata.ci.yaml`.** Cells reference
  named `runners:` profiles — `kind: github-hosted` (fixed image, e.g.
  `windows-2022`, optional `billing.acknowledgement`) or `kind: self-hosted`
  (labels + capability tags) — instead of raw host labels (`host:` stays as
  the legacy fallback; declaring both is a structural error), and declare a
  `lane` (`pull_request` / `main` / `scheduled` / `workflow_dispatch`, default
  `scheduled`) plus a `publish` policy (default `never`; `pull_request` +
  `publish` is rejected outright). The GitHub renderer maps profiles to
  `runs-on` (`image` → the image, `labels` → the list); support cells stay
  explicit support claims, never an inferred Cartesian product. Still ⬜:
  dispatch-input restrictions are moot for now — the generated workflows
  accept no `workflow_dispatch` inputs at all.
- ✅ **P1 — source-CI lane: GitHub-hosted SDK build jobs.**
  `ost ci generate github` renders `pull_request`/`main` cells into a
  second workflow (`ost-source-ci.yml`): checkout (SHA-pinned) →
  `ost ci validate` → `ost artifact verify` + `ost runtime pull
  --from-artifact <digest>` → `ost plugin build <bundle>` → `ost plugin test
  --up-to <level>` → `ost plugin package` (never publish, `contents: read`
  token, no secrets) → upload reports; per-cell `bundle:` selects the bundle
  in a workspace repo. The 0.6.0 artifact-seeded workflow remains the
  scheduled **support** lane. The two gaps report #10 confirmed ("Check ost
  is available" fails on a hosted runner) closed in v0.9.0: hosted cells now
  get a pinned, checksum-verified `ost` bootstrap step and a digest-pinned
  `ost artifact pull` from the cell's `runtime_remote` reference (plus an
  optional digest-keyed registry cache) — see the
  [Phase 6 — v0.9.0 backlog](#phase-6--v090-backlog-from-dogfooding-report-10--the-remote-artifact-transport-plan).
- 🚧 **P2 — hosted-runner cost visibility + fork-PR safety.**
  ✅ `ost ci validate` warns (`CI_HOSTED_BILLING_UNACKNOWLEDGED`) when a
  referenced `github-hosted` profile lacks `billing.acknowledgement:
  required`, and fails (exit 5) when a publish-capable cell sits on such a
  profile; generated hosted jobs print a `::notice` billing annotation before
  work starts; generated PR workflows cannot publish (structural gate +
  no publish step, no secrets, read-only token). Still ⬜: dispatch
  approved-choice inputs (none generated yet) and trust levels for privileged
  self-hosted runners (tracks SEC-006 / Phase 6 trust policy).
- ✅ **P2 — `ost ci plan --json`.** Preflight execution facts without money
  estimates: cells per lane, the workflows `generate` would write, hosted job
  count, metered vs operator-managed runner classes, the hosted profiles still
  missing billing acknowledgement (`requires_billing_acknowledgement`), and
  the publish-capable job count. Facts only — never a currency estimate.
- ✅ **P2 — CI evidence in reports.** Generated workflows export a job-level
  `OST_CI_*` contract (cell, lane, runner profile, `join()`-resolved
  `runs-on`, pinned runtime/plugin digests) from the include entry, and every
  report written inside the job — `report.json` and the `--json` envelope, via
  `ost_plugin::ci_evidence_from_env` — records it as an additive `ci` block,
  so a support claim is reconstructible from its report. Absent outside CI
  (no `OST_CI_CELL`), so local reports are unchanged; the published
  [plugin-report schema](../schemas/plugin-report.schema.json) documents the
  block. Target/profile, verification level, and validation outcome were
  already in the report body; package provenance stays in the package
  `manifest.json`.
- ✅ **P2 — workspace-level plugin testing.** `ost plugin test --workspace`
  discovers the workspace's plugin bundles (immediate subdirectories +
  `plugins/*`, matching the v0.5.0 CMake discovery), runs the verification
  pyramid on each against one resolved runtime session, prints per-bundle
  reports plus an aggregate summary (`--json`: one envelope with every
  bundle's report + `report_dir`), and fails if any bundle fails. `--with`
  bundles compose into every session; a bundle path together with
  `--workspace` is a usage error.
- ✅ **P2 — document the co-located schema migration path for existing
  bundles.** [co-located-schema-migration.md](co-located-schema-migration.md):
  when to co-host vs split a schema bundle, the `ost plugin schema add` fast
  path and the hand-wiring equivalent (`schema.source` + `provides:
  usd-schema:<Type>`), what the next build automates (usdGenSchema in the
  session env, the `OPENSTRATA_SCHEMA_SOURCES_FILE` hook, the `Types` merge
  that preserves the `SdfFileFormat` entry, `generatedSchema.usda` staging),
  the committed-vs-build-tree decision, the `library_prefix` footgun, L2/L4
  verification, and the per-target ABI/`LibraryPath` notes.

### Phase 5 — v0.8.0 backlog (from dogfooding report #9, the v0.7.0 CI policy decision)

Released in v0.8.0. Report #9 (2026-07-05) adopted `openstrata.ci.yaml` as the
downstream repo's CI policy surface and verified the v0.7.0 CI/lock fixes; what
it carried back is the one v0.7.0 fix that didn't hold plus consumer-side
blockers (real artifact digests, a golden L5 fixture) that are theirs, not ours.
Ranked:

- ✅ **P1 — `ost plugin package` rerun still hits access-denied (os error 5).**
  The v0.7.0 read-only fix addressed the wrong (or only half the) cause: the
  failing host's `package-stage` had *no* read-only entries — the reset dies
  when a scanner (Defender, indexer) still holds the previous run's fresh
  files open without `FILE_SHARE_DELETE`, an inherently transient lock the
  old clear-attribute-and-retry-once path never waited out. Staging now goes
  through `ost_core::fs::prepare_staging_dir`
  ([fs.rs](../crates/ost-core/src/fs.rs)): bounded remove retries (~0.4s,
  clearing read-only between attempts), then **fall back to a fresh sibling
  stage** (`package-stage-<16 hex>`) instead of failing — the rerun always
  proceeds; the stuck tree is swept best-effort by every later run once the
  handles close. A fallback surfaces as a `STAGE_FALLBACK` warning (the
  `--json` envelope's `warnings` array / a stderr `warning:` line). Applied to
  both `ost plugin package` and `ost package` (which still had the naked
  `remove_dir_all`); unit-tested on Windows with a genuinely locked file
  (opened without `FILE_SHARE_DELETE`) plus sweep/reset/readonly cases.

## Phase 6 — Artifact registry 🚧

- ✅ **MVP boundary for v0.6.0:** local-first, digest-first artifact registry.
  The registry is a content source for runtimes/plugins/packages, not yet a
  remote service.
- ✅ Artifact identity model (`ost-artifact` crate): `{kind, name, version,
  target, profile, digest, created_unix, producer, source, validation, licenses,
  sbom}` as a fixed-field record with deterministic JSON and a stable schema
  version, always *derived* from a producer `manifest.json` (plugin-bundle,
  project package, or the future `openstrata.runtime` tag) — never hand-authored.
- ✅ Content-addressed artifact store (digest pinning) under `~/.ost/artifacts`:
  `objects/sha256/<hex>/` object dirs staged + renamed atomically, plus a small
  deterministic `index.json` (sorted by digest, rebuildable from the objects)
  before introducing SQLite.
- ✅ `tar.zst` + manifest + checksums as the canonical MVP payload: the store
  keeps the producer manifest byte-for-byte beside the archive and a regenerated
  `SHA256SUMS`; the plugin payload already carries its validation report inside
  the archive (`validation/report.json`).
- ✅ `ost artifact import|export|list|show|verify|extract` for local registry
  operations and CI artifact handoff: import re-hashes the archive and refuses a
  digest/size mismatch (`ARTIFACT_DIGEST_MISMATCH`, exit 5); artifacts resolve
  by full digest or unique hex prefix; `verify` recomputes the archive digest
  *and* re-hashes every tar entry against the manifest `files[]`; `export`
  round-trips to a re-importable directory; `extract` unpacks an artifact's
  archive after re-verifying its digest (the runtime fetch and the CI matrix's
  plugin-under-test step share it). Covered by unit + e2e tests
  ([artifact_registry.rs](../crates/ost-cli/tests/artifact_registry.rs)).
- ✅ `RuntimeSource::Artifact` fetch/use path for prebuilt runtimes.
  `ost runtime export` packs a pulled real runtime (effective prefix, minus the
  store's `runtime.json` — the runtime manifest travels in the producer
  manifest's `provenance.runtime_manifest`) and registers it as a `published`
  `openstrata.runtime` artifact, gated on a real source
  (`EXPORT_REAL_RUNTIME_REQUIRED`), no external `runtime_deps`
  (`EXPORT_DEPS_NOT_PORTABLE` — they would not travel), and passed validation
  (`EXPORT_VALIDATION_REQUIRED`). `ost runtime pull --from-artifact <digest>`
  re-verifies the archive bytes, refuses non-runtime kinds
  (`ARTIFACT_KIND_MISMATCH`) and target/profile mismatches
  (`ARTIFACT_RUNTIME_MISMATCH`), extracts into the store prefix, and restores
  the manifest with `source: artifact` + the registry digest
  (`artifact_digest`), surfaced by `runtime show`/`list` and `doctor`
  (kind `downloaded`). Covered by unit + e2e tests
  ([runtime_artifact.rs](../crates/ost-cli/tests/runtime_artifact.rs)),
  including a two-store export → handoff → fetch round-trip.
- ✅ `ost plugin publish`: consumes existing `ost plugin package` output (never
  re-packages) and registers it by digest as a `published` artifact. Entry is
  gated with per-cause stable codes CI can branch on:
  `PUBLISH_VALIDATION_REQUIRED` (validation must have passed),
  `PUBLISH_LICENSE_REQUIRED` (SPDX license), `PUBLISH_PROVENANCE_INCOMPLETE`
  (runtime id + digest), `PUBLISH_ABI_UNRESOLVED` (a concrete frozen `cxx_abi`,
  not `inherit`/per-OS), and `PUBLISH_NOTICES_MISSING` (declared notices must be
  in the archive). Prints the digest reference CI pins.
- ⬜ Runtime/extension content attribution and per-artifact SBOM generation:
  runtime manifests record upstream licenses/notices; published artifacts include
  complete notices and a generated SPDX or CycloneDX SBOM.
- ⬜ Trust policy hooks: distinguish `local`, `verified`, and `trusted`
  artifacts; allow release CI to require a minimum trust level. Direction now
  settled in [remote-artifact-transport.md](remote-artifact-transport.md)
  (integrity vs trust split, initial `local`/`verified`/`trusted` levels);
  implementation lands with the plan's Phase 4 (post-v0.9.0).
- ✅ OCI layout / registry / oras transport — the read-only pull slice shipped in
  v0.9.0. Direction:
  [remote-artifact-transport.md](remote-artifact-transport.md); ranked backlog
  below. The read-only pull slice (transport contract + OCI backend +
  `ost artifact resolve|pull`) has landed; push and the publish policy stay
  deferred to v0.10.0+.

### Phase 6 — v0.9.0 backlog (from dogfooding report #10 + the remote-artifact-transport plan)

**Released in v0.9.0.** Report #10 (2026-07-05) ran the v0.7.0 decision's
next steps to completion on 0.8.0 — real runtime + plugin digests, a
placeholder-free `openstrata.ci.yaml`, both workflows rendered, L5 golden
gate green — and isolated the one remaining blocker: the generated
`ost-source-ci.yml` fails at "Check ost is available" on any GitHub-hosted
runner, because `ost` install and runtime-artifact transport are both left to
the operator. The remote-artifact-transport plan
([remote-artifact-transport.md](remote-artifact-transport.md)) is the design
contract; this backlog is its P0 slice plus the report's export-ergonomics
asks. Ranked:

- ✅ **P0 — `ArtifactTransport` contract + read-only OCI pull (plan Phase 1).**
  A `resolve / pull` transport trait (`push` declared, refused until the
  publish phase) in front of the registry: the existing filesystem flow is
  one adapter (`file://<dist-dir>`, behavior unchanged — same chain, same
  evidence), and a read-only OCI backend (GHCR-class, ORAS artifact model,
  bearer token exchange, manual cross-host redirects that never replay
  `Authorization`) is the second. `ost artifact resolve <ref>` (tag →
  immutable digest) and `ost artifact pull oci://…@sha256:<digest>` landed —
  pull downloads, runs the full verification chain (OCI blob digests →
  archive digest → manifest schema → pre-extraction safety → per-file
  digests → kind / target / pinned-digest match → trust policy), then
  imports atomically into the local registry; transport success alone is
  never success, and a failed step never leaves a usable artifact. `--json`
  evidence records the remote locator, resolved OCI digest, registry
  identity, auth mode, per-step verification status, and local import path.
  Stable error codes (`ARTIFACT_REFERENCE_MUTABLE`,
  `ARTIFACT_OCI_DIGEST_MISMATCH`, `ARTIFACT_ARCHIVE_DIGEST_MISMATCH`,
  `ARTIFACT_ARCHIVE_UNSAFE`, `ARTIFACT_TRANSPORT_FAILED`, …) so CI can
  branch on cause. Integration-tested against a mock OCI registry
  ([transport_pull.rs](../crates/ost-artifact/tests/transport_pull.rs)):
  corrupt archive, manifest substitution, wrong platform / kind, unsafe
  archive entries, and mutable-only refs all fail.
- ✅ **P0 — digest-pin policy.** Tags are convenience, digests are the
  contract: `ost artifact pull` refuses mutable-only refs
  (`ARTIFACT_REFERENCE_MUTABLE`) and every digest-verification failure is an
  error, never a warning (landed with the transport); `openstrata.ci.yaml`
  `runtime_remote` references must themselves be digest-pinned and matching
  their `expected_oci_digest` (landed with the CI contract below).
- ✅ **P0 — CI contract: remote runtime reference per support line.**
  `openstrata.ci.yaml` runtime pins gain a `runtime_remote` block (`uri:
  oci://…@sha256:<digest>` + `expected_oci_digest`) beside the existing
  OpenStrata `runtime_artifact` digest, and a matrix-level `bootstrap.ost`
  pin (`version`, release `repository`, optional per-triple exact-byte
  `sha256`). Source cells (`pull_request`/`main`) resolving to GitHub-hosted
  runners require both; self-hosted lanes may keep air-gapped local import,
  and CI evidence records the runtime's source either way
  (`.ost-ci/runtime-source.json`). `ost ci validate` enforces the policy,
  `ost ci plan` reports the bootstrap pin, remote-pulling cells, and
  air-gapped source cells.
- ✅ **P0 — generated hosted bootstrap (plan Phase 2).** `ost ci generate
  github` renders, for hosted cells: a bootstrap step that installs the
  version-pinned `ost` release asset with checksum verification (the
  release's published `.sha256` plus the matrix's exact-byte pin when
  declared; bootstrap failure is its own step, never conflated with an
  artifact failure; the observed version is asserted against the pin and
  saved into `.ost-ci/bootstrap.json` / `ost-version.json`); an optional
  `actions/cache` restore of the registry keyed by `{ost-version, os, arch,
  support-line, runtime-digest}` (never branch names or run ids, disableable
  via the `OST_CI_DISABLE_CACHE` repository variable, a poisoned hosted
  cache is wiped and re-pulled); then a digest-pinned `ost artifact pull
  --expect-artifact --require-kind runtime` with `--json` evidence teed to
  `.ost-ci/runtime-pull.json`, falling back cleanly on a cache hit; then the
  existing build → test → package → report-upload chain, with `.ost-ci/`
  evidence uploaded beside the reports. Replaces the "assert `ost
  --version`" placeholder; the generated bootstrap was executed end to end
  against the real v0.8.0 release assets (download → checksum → extract →
  PATH → version assert) as part of verification.
- ✅ **P0 — public E2E fixture repository.**
  [`snkmcb/_ost_runner_test`](https://github.com/snkmcb/_ost_runner_test): a
  tiny `usd-fileformat` plugin (`plugins/toy`) built from source on
  GitHub-hosted `windows-2022`, with the runtime SDK pulled from a public
  GHCR reference
  (`oci://ghcr.io/snkmcb/openstrata-cy2026-windows-x86_64-py313-usd@sha256:39a588fde380…`,
  archive digest `sha256:7b410d92…`) and `ost` bootstrapped from a pinned
  release. **PR source CI, push (main) source CI, and an explicit
  cache-disabled run (`OST_CI_DISABLE_CACHE=true`) all green**, full pyramid
  L0–L5 passing on the runner; workflow verified read-only (no `secrets.`,
  no publish command, no self-hosted labels, `permissions: contents: read`).
  Standing this up end to end surfaced — and drove fixes for — six ways an
  **adopted USD build-tree runtime is not relocatable** to a clean host
  (landed v0.9.0, all with a stale-only guard so a developer's in-place tree
  is never mutated):
  1. `pxrConfig.cmake` bakes the export machine's Python behind
     `if(NOT DEFINED)` guards → `ost` resolves a matching host interpreter
     and pins `Python3_*` in the generated toolchain (required version read
     from pxrConfig, not `runtime.json` — the runtime was labeled `py313`
     but its USD linked Python 3.10).
  2. `pxrTargets.cmake` bakes the Python include into imported targets'
     `INTERFACE_INCLUDE_DIRECTORIES` → relocated to the host include.
  3. `pxrConfig.cmake` bakes the build-tree's own absolute prefix into the
     external-dependency imported targets (TBB/MaterialX) → relocated to the
     runtime's on-host store prefix (old prefix discovered from the baked
     files).
  4. adopted build trees don't bundle `pythonXY.dll`, so `usdcat`/`usdview`
     couldn't start → a matching host interpreter's dir is put on the
     session loader path.
  5. Windows Python 3.8+ doesn't search PATH for an extension's dependent
     DLLs, so `import pxr` failed on `_tf` → `os.add_dll_directory` preamble
     over the session PATH in the Python level scripts.
  6. `usdcat --flatten` stamps the absolute root-layer path into the golden's
     `doc`, so a committed golden never matched off its origin host → the L5
     comparison normalizes that line. The fixture is now a product-level
     contract in continuous CI for renderer/transport changes.
- ✅ **P1 — slim/SDK-profile `runtime export`.** `ost runtime export --slim`
  ships only the SDK layout — `include`, `lib`, `bin`, `plugin`, `cmake`,
  `libraries` (MaterialX standard defs), plus the top-level CMake package
  config and attribution files — dropping the source/`build` tree and sample
  `resources/` a runtime adopted from a full USD build carries. The predicate
  (`ost_build::is_sdk_path`) is pure and unit-tested; the excluded top-level
  entries are reported in the human and `--json` output, and the producer
  manifest records `layout_profile: sdk|full` so a fetch can tell a slim
  artifact from a full one (distinct digests of the same runtime). Measured on
  report #10's runtime: **1.93 GiB → 27 MiB archive (~73×), 18,029 → 3,818
  files, ~30 s vs ~52 min**, and the slim artifact — materialized into a clean
  `OST_HOME` — builds and runs the toy fixture's full L0–L5 pyramid green
  (12 pass / 0 fail / 3 skip). This is the clean-install answer to the adopted
  build-tree relocatability issues above (the `build/` tree that carried the
  stale absolute paths is simply gone).
- ✅ **P1 — `runtime export` performance + progress.** `export` now packs with
  multithreaded zstd by default (`--jobs`, defaulting to the host's available
  parallelism; `--jobs 1` forces the single-threaded encoder) and takes a
  `--level` knob (1–22, default 19). Throttled in-place progress prints to
  stderr (`N/M files, <bytes> in <secs>s`) so a long pack shows liveness
  instead of looking hung, and the finished archive is stream-hashed rather
  than read whole into memory. Small artifacts (`ost package`/`ost plugin`)
  keep the byte-stable single-threaded default via `pack_dir`.
- ✅ **P2 — L5 golden skip-message clarity (Phase 4 harness UX).** The L5
  `golden.roundtrip` SKIP now names the concrete expected file
  (`<fixture-filename>.golden.usda`, fixture extension retained — e.g.
  `basic.toy.golden.usda`), states it must be the *flattened* stage, and
  carries the generation recipe as a suggested action
  (`ost plugin run <bundle> -- usdcat --flatten <fixture> --out <golden>`),
  rendered under the diagnostic in human and `--json` output
  ([levels.rs](../crates/ost-plugin/src/levels.rs) `level5_golden`).
- **Deferred to v0.10.0+:** `ost artifact push` + plugin publish over OCI +
  protected publish policy + OIDC federation (plan Phase 3); trust levels in
  manifest/CI contract, publisher identity/provenance, SBOM attach, trusted
  runtime allowlist (plan Phase 4 — tracks SEC-006 and the trust-policy hooks
  above); registry mirroring / air-gapped sync / multi-registry failover.

### Phase 6 — v0.10.0 backlog (from the 2026-07-09 publish dogfooding + the future-policy note)

**Released in v0.10.0.** The 2026-07-09 report re-verified every v0.8.0→v0.9.0
ask as landed and *enforced* (the old matrix now fails `ost ci validate`), then
did the first real end-to-end OpenUSD-from-source build → export → GHCR publish →
anonymous CI pull. The consume side is mechanically complete; the two remaining
rough edges are both on the *produce* side, and three runtime-completeness bugs
sit between "build a runtime" and "publish a usable one." The future-policy note
(`openstrata_future_policy.md`) is the forward design contract; this backlog is
its v0.10.0 slice. Delivered:

- ✅ **P0 — `--build` OpenUSD version detection + recoverable drift (report
  Finding A).** `runtime pull --build` used to stamp the manifest's `openusd`
  extension from the **catalog default** (e.g. `25.05.01`) even though the freshly
  built `include/pxr/pxr.h` reported the real version; `runtime validate` failed
  with `openusd-version-drift` and both documented recoveries were dead ends.
  Landed: the `--build` path now reads the real version from the built `pxr.h`
  (shared `stamp_openusd_version`, as adopt already did) and stamps it; `runtime
  repair` re-derives a `build`-source runtime **in place** from the store tree
  (no rebuild — the built bits are correct, only the version field drifted), and
  the drift/repair suggested-fix text points at `ost runtime repair` instead of
  the no-op `--build … --force`. Unit-tested (stamp correction, per-source repair
  command).
- ✅ **P0 — ship schema-gen Python deps inside the runtime (report Finding D).**
  A from-source runtime bundled `usdGenSchema` in `bin/` but not its runtime
  imports (`jinja2` + `MarkupSafe`), so a published image died deep in `ost plugin
  build`'s schema-generate phase with a bare `ModuleNotFoundError`. Landed: after
  a `--build`, when `usdGenSchema` is bundled, `ost` provisions `jinja2` (+
  transitive `MarkupSafe`) into the runtime's `lib/python` via `pip install
  --target` (best effort, warns with the manual fix on failure); and `runtime
  validate` now **gates on** a `schema-gen-deps` check, so `export` (which requires
  a passing validation) refuses to publish a schema runtime missing `jinja2`.
  Unit-tested in `ost-build` (detection/idempotency) and `ost-runtime` (the gate).
- ✅ **P0 — `--slim` retains `resources/` (report Finding E).** `runtime export
  --slim` dropped the SDK's top-level `resources/`, but `pxrConfig.cmake` chains
  into `MaterialXConfig.cmake`, which `set_and_check`s `MATERIALX_RESOURCES_DIR =
  <prefix>/resources` at `find_package(pxr)` time — so a slim MaterialX runtime
  was unconsumable. Landed: `resources` is now an SDK-layout dir kept by the slim
  export (a no-op for a runtime without one). Test + `--slim` help/docs updated.
- ✅ **P1 — `ost artifact push` (report Finding B; future-policy §3.1).** OCI
  transport was consume-only. Landed: a write-capable OCI backend behind the
  existing `ArtifactTransport` contract (filesystem backend unchanged) that emits
  the exact pull-compatible OCI layout / media-types (archive + producer
  `manifest.json` layers with title annotations, empty config), uploads blobs
  content-addressed (HEAD-then-PUT, monolithic), prints the immutable OCI digest
  to pin into `runtime_remote.expected_oci_digest`, and is idempotent (identical
  re-push transfers nothing) with a pinned-digest mismatch a hard `ARTIFACT_OCI_
  DIGEST_MISMATCH`. `--json` carries oci/artifact digests. `push` re-hashes the
  stored archive first so store corruption is never published. Tested end to end
  against an in-process mock registry (upload sequence + idempotency) plus the
  build/round-trip/mismatch pure checks.
- ✅ **P2 — build-dep preflight (report §Dogfood).** Before invoking
  `build_usd.py`, `ost` probes the host interpreter for the Python deps the
  resolved profile implies (Jinja2 for schema tooling; PySide6 + PyOpenGL for a
  Hydra/usdview profile; PySide6 for a Qt profile) and warns (never fails) on the
  missing ones with the `pip install` line, rather than letting the build abort
  late. Pure capability→requirements mapping unit-tested.
- ✅ **P2 — publish-flow documentation.** `docs/examples.md` gained a "Publishing
  to and pulling from a remote OCI registry" section covering `ost artifact push`
  / `resolve` / `pull`, the credential env vars (`ost` does **not** read the
  `oras`/docker credential store — set `OST_REGISTRY_USER`/`_PASSWORD` or
  `OST_REGISTRY_TOKEN`), the WebUI-only GHCR visibility flip, and the anonymous CI
  pull path once public.

### Phase 6 — v0.11.0 backlog (from the 2026-07-10 recheck dogfooding report)

**Released in v0.11.0.** The 2026-07-10 recheck report
(`2026-07-10-v0.10.0-recheck-v0.11.0-asks.md`, `ost 0.10.0`) re-verified every
v0.10.0 P0/P1 ask as landed — `--build` stamps the built OpenUSD version
(`26.05`) and validates without re-adopting, the runtime ships `jinja2` +
`MarkupSafe` in `lib/python` and `runtime validate` gates on `schema-gen-deps`,
`--slim` keeps top-level `resources/` (MaterialX config consumable again), and
`ost artifact push` exists as the producer verb — and drove the full local
dogfood plus a public GHCR round-trip proving Windows **and** Linux runtime
*consumption* by digest (anonymous `resolve` + digest-pinned `pull` with
`--expect-artifact` / `--require-kind` / `--require-target`). The consume side is
excellent; every remaining edge is on the *produce* side, and one is a hard
blocker. Delivered:

- ✅ **P0 (BLOCKER) — measure and record the real glibc floor for Linux runtimes
  (report ask #7).** The Linux runtime was built in WSL on Ubuntu 26.04 (glibc
  2.43), so its binaries (`libusd_gf.so`, …) carry `GLIBC_2.43` versioned symbols,
  but `ost runtime export` stamped the catalog-default ABI label
  `linux-x86_64-glibc228-py313`. The hosted `usdvrm-pr-linux` lane on
  `ubuntu-24.04` (glibc 2.39) then died in `ost plugin build` → `schema-generate`:
  `ImportError: /lib/x86_64-linux-gnu/libm.so.6: version 'GLIBC_2.43' not found
  (required by …/lib/libusd_gf.so)`, exit 6. Two compounding defects: (1) the
  `glibc228` variant label was **fabricated, not measured**; (2) `--require-target`
  gave **false confidence** by string-matching the fabricated label. **Landed:**
  `ost_build::glibc` streams each ELF binary and computes the maximum referenced
  `GLIBC_x.y` symbol version (the value `readelf -V | grep GLIBC_` surfaces,
  without a binutils dependency; non-ELF and non-Linux inputs are a no-op). `ost
  runtime export` measures the floor from the packed binaries and stamps it onto
  the artifact `target` — a runtime referencing `GLIBC_2.43` is now labeled
  `glibc243`, never `glibc228` — overriding the fabricated nominal and recording
  the measured-vs-recorded drift as a `glibc_floor` evidence field (producer
  manifest + `--json`). The embedded build provenance is left faithful. Because the
  label is now truthful, the existing `--require-target` string match rejects a
  stale `glibc228` pin instead of false-passing, so the consumer defect is closed
  at its source without a separate floor-comparison path. Unit-tested (scanner:
  cross-file max, chunk-boundary matches, non-ELF exclusion, numeric ordering;
  relabel: higher/lower floor wins, non-glibc untouched) + a Linux-gated e2e export
  test. Still ⬜ (folded into the P2 Linux-build docs below): document that portable
  Linux runtimes are built against an old glibc base so the measured floor is
  genuinely low.
- 🚧 **P0 — fix `ost artifact push` against GHCR (report ask #1).** The producer
  verb's CLI surface is right, but the actual upload failed two ways: with
  `OST_REGISTRY_TOKEN`, GHCR returned `403`; with `OST_REGISTRY_USER=snkmcb` +
  `OST_REGISTRY_PASSWORD=<PAT>`, auth advanced but the upload sent
  `digest=sha256:sha256:44136fa355… → HTTP 400` — a real double-`sha256:` producer
  bug in the upload URL. **Landed:** the double-`sha256:` bug is fixed. `push`
  wrapped `digest::sha256_hex` output (already `sha256:<hex>`) in another
  `sha256:` for the config, producer-manifest, and OCI-manifest digests; the config
  blob (uploaded first) hit GHCR as `sha256:sha256:44136…` → HTTP 400. The values
  are now used verbatim, and the push idempotency test asserts every uploaded blob
  digest is a single well-formed `sha256:<hex>` (via `is_sha256_ref`) — the mock
  registry had masked the bug by matching the same double-prefixed string. **Also
  landed:** the credential story is now honest about push. A write that the
  registry answers 401/403 gets a push-specific, auth-mode-keyed hint (via
  `write_auth_hint`) instead of the generic pull hint: a `static-token` 403 (the
  report's `OST_REGISTRY_TOKEN` case — a bearer presented verbatim is accepted for
  reads but cannot carry `push` scope, and a 403 triggers no exchange retry) steers
  to `OST_REGISTRY_USER` + `OST_REGISTRY_PASSWORD` so `ost` runs the token exchange
  for `pull,push`; a rejected credential (`token-exchange-basic`) points at the
  `write:packages` scope and package write permission; an anonymous write says a push
  cannot be anonymous. `docs/examples.md` now documents `OST_REGISTRY_TOKEN` as
  pull-only and the credential path as the push requirement, and the module doc
  records the same. Unit-tested per auth mode. Still ⬜: a real GHCR round-trip to
  confirm the user/password path end to end (needs live credentials).
- ✅ **P1 — support Linux symlinks in runtime export (report ask #2).** Direct
  `runtime export --slim` of the build-source Linux runtime failed with `symlink
  is not allowed in the package staging area:
  …/lib/libMaterialXGenMsl.so`. The built Linux SDK carries routine shared-library
  symlink chains (`libFoo.so → libFoo.so.1 → libFoo.so.1.39.4`; 48 counted); the
  report shipped only via a `cp -aL` dereference-and-re-adopt workaround.
  **Landed:** the strategy is *preserve safe in-tree links* (not dereference —
  48 links stay 48 links, not full copies). `ost_build::validate_symlink` keeps a
  symlink only when its target is relative and, resolved lexically against the
  link's own directory, stays inside the stage root; an absolute target or a `..`
  that climbs above the root is rejected, so SEC-001's link-escape protection is
  untouched (no out-of-tree or absolute link can enter the artifact, and the link
  is never dereferenced). A kept link is written as a tar link entry carrying its
  target; a `link_target` field flows through `FileEntry` → the producer manifest
  (`ManifestFile`) → `walk_archive` (which re-validates the target string with the
  purely-lexical `unsafe_symlink_target` and flags an escaping link as unsafe) →
  `compare_archive_files` (so a symlink and a regular file whose contents equal
  the target string stay distinct identities). `Store::extract` now walks the
  digest-verified archive and refuses any unsafe entry (escaping symlink,
  hardlink, `..` path, special file) *before* unpacking a byte, so a local artifact
  that skips the transport verify gate still cannot escape on extraction.
  Unit-tested on the produce side (real-filesystem soname chain packs as links;
  absolute + `..` links rejected), the consume side (walk keeps/flags, compare
  type-confusion, extract restores a link and refuses an escaper), plus a full CLI
  `runtime export → artifact export/import → runtime pull --from-artifact`
  round-trip asserting the chain materializes as symlinks pointing at the one real
  object.
- ✅ **P1 — quieter / more actionable package-stage fallback (report ask #4).**
  `ost plugin package` (and `ost package`) succeeded but repeated runs emitted an
  opaque, recurring `STAGE_FALLBACK`: the prior `package-stage` was "held open by
  another process", so `ost` staged into `package-stage-<id>` and said only that a
  later run would sweep it. **Landed:** `prepare_staging_dir` now returns a
  `StagingOutcome` with a real swept/leftover accounting (the sweep was previously
  silent best-effort), and the shared `prepare_package_stage` CLI helper turns that
  into an *actionable* warning — `STAGE_FALLBACK` names how many stale fallback
  siblings are still locked and points at the new `--clean-stage` escape hatch.
  `--clean-stage` (on both commands) reclaims the stable stage name with extra
  bounded retry rounds so it deterministically recovers once the holding process
  exits (rather than accumulating another sibling), and reports what it swept as
  `STAGE_CLEANED`. The JSON warnings carry `leftover`/`swept` for tooling. Chose the
  `--clean-stage` + accounting route over OS-specific locking-process identification
  (fragile, non-portable; the ask's "and/or"). Unit-tested: `fs` sweep counts +
  clean reclaim, the Windows-gated locked-stage fallback→reclaim, and the CLI
  helper's warning shape. Documented in `docs/examples.md`.
- ✅ **P2 — first-class way to keep repo-specific smoke tests in generated source
  CI (report ask #5).** Regenerating the workflow with `ost 0.10.0` silently
  removed the repo's custom `Run corpus CTest smoke` step, so the hosted PR
  workflow ran the `ost plugin test` pyramid + package but lost standalone corpus
  coverage. **Landed:** the declarative route. `openstrata.ci.yaml` takes an
  optional matrix-level `source_checks:` list; each entry (`name` + bash `run`)
  renders as a workflow step in the generated **source-CI** job(s), spliced in
  after the verification pyramid and before packaging (so the built plugin is
  present and the package step still runs last). Regeneration re-emits the checks
  every time, so renderer output no longer drops repo-declared coverage. Chose
  declarative post-build steps over teaching `ost plugin test` about corpus assets
  (keeps the pyramid's contract fixed; a repo's smoke command stays the repo's).
  Both fields are validated as injection/fork-PR hardening — `name` to a plain
  step-title charset (no `:`/`#`/newline breakout), `run` rejects control chars
  and (like the rest of source CI) the GitHub Actions `secrets` context. The `run` renders as
  a literal block scalar with every line re-indented, so a multi-line script
  cannot escape its step. Support lanes (which re-verify pinned artifacts and
  never build from source) are unaffected. Unit-tested: parse + default-empty,
  name/secrets/empty-run rejection, and render placement/order + determinism +
  fork-PR safety. Documented in the `ost ci init` scaffold comment and
  `docs/examples.md`.
- ✅ **P2 — document Linux build prerequisites (report ask #3).** **Landed:**
  `docs/examples.md` gains a "Linux `--build` prerequisites" section — the dev
  packages `build_usd.py`'s deps link against (incl. `libxt-dev` for MaterialX's
  `MaterialXRenderGlsl`/`Xt` and `unzip`), why the interpreter must be a
  source-built `--enable-shared` Python 3.13 (`python3.13` is not an apt package),
  a pointer to `ost`'s Python-package preflight (Jinja2 for schema gen, PySide6 +
  PyOpenGL for usdview), and the portable-runtime glibc-floor caveat (build
  against an old glibc base; `ost` measures and stamps the floor and
  `--require-target` enforces it). The 504-HTML-body-as-archive flake is
  documented honestly as an **upstream `build_usd.py` downloader** issue with a
  `codeload.github.com` pre-seed workaround: `build_usd.py`'s fetch is a
  third-party script `ost` shells out to, not `ost`'s own (digest-verified)
  transport, so `ost` has no seam to detect the bad body there. (The
  hardening-the-downloader half of the original ask is therefore out of `ost`'s
  reach; recorded as documented rather than fixed.)
- ✅ **P2 — re-test build-host dependency preflight on a clean Python (report ask
  #6).** v0.10.0's runtime-side `jinja2`/`MarkupSafe` provisioning was already
  confirmed; the open question was whether the *host-side* preflight warns clearly
  on a pristine Python missing `Jinja2`/`PyOpenGL`/`PySide6` before invoking
  `build_usd.py`. **Verified:** the probe — `python -c "import importlib.util as
  u; …find_spec(m) is None"` over the profile-implied imports — was exercised
  directly and correctly reports absent modules (a present host reports only the
  genuinely-missing one). The warning body (including the `-m pip install <pip
  names>` fix line, mapped from import name → pip name) was extracted into a pure
  `missing_dep_warning` and unit-tested, so the message path is covered without
  needing a clean interpreter on the CI host. The `build_dep_requirements`
  capability→dep mapping was already unit-tested.

### Phase 6 — v0.12.0 backlog (from the 2026-07-11 macOS OpenUSD 26.05 PR CI dogfooding report)

The 2026-07-11 report
(`2026-07-11-macos-openusd-26.05-pr-ci-dogfooding.md`, `ost 0.11.0`) exercised a
public macOS arm64 OpenUSD 26.05 runtime in the generated `ost source ci` PR lane
for `plugins/usdVrm`. The important result is mixed but useful: GHCR access,
digest-pinned `ost artifact pull`, artifact verification, and `runtime pull
--from-artifact` all worked on the hosted `macos-15-arm64` runner; Windows passed
the same source-CI contract. The first macOS job failed earlier, during `ost
plugin build` → `schema-generate`, because two assumptions were still implicit:
the `py313` runtime did not provide `bin/python3.13`, and the published macOS
runtime materialized `bin/` tools without executable bits. A follow-up commit made
the PR green by adding repo-local temporary repairs (`actions/setup-python@v6`
for Python 3.13 and `chmod +x <runtime>/bin/*` before build). v0.12.0 turns those
repairs into product behavior so generated CI remains regenerable.

- 🚧 **P0 — preserve and validate executable modes for runtime tools.** The public
  macOS runtime artifact passed OpenStrata artifact verification, but local
  `ost plugin test --up-to 5` could not run `usdcat` until `chmod +x` was applied
  to the materialized runtime `bin/` tree. That means the artifact identity and
  verification path still miss a runnable-runtime invariant. Durable fix:
  `runtime export` / artifact packaging preserves Unix execute bits, extraction
  restores them, `runtime validate` fails a runtime whose declared tools are not
  executable, and the public cy2026 macOS arm64 OpenUSD 26.05 SDK is republished.
  Acceptance: clean local materialization and a GitHub-hosted macOS source-CI lane
  reach L5 without a workflow-level chmod repair. **Landed (code):** `pack_dir`
  reads each staged file's real mode and packs an executable file as `0o755`
  (everything else stays the canonical `0o644` for a deterministic archive),
  recording the runnable bit as `files[].executable` in the producer manifest;
  `tar` extraction restores the mode; `walk_archive`/`verify` carry and compare
  `executable`, so a tool that lost `+x` is a per-file manifest mismatch, not a
  silent pass; and `runtime validate` emits a Unix-only `bin-tools-executable`
  check that fails any real runtime whose top-level `bin/` tools are not
  executable (transitively gating `runtime export`, which requires a passing
  validation). Unit-tested: pack→extract mode round-trip, walk/verify mismatch,
  and the validate failure/repair. **Still ⬜:** republish the public cy2026 macOS
  arm64 OpenUSD 26.05 SDK with preserved bits and confirm the hosted macOS
  source-CI lane reaches L5 with no `chmod` repair (needs a Mac + live GHCR).
- ⬜ **P0 — make schema-generation Python explicit.** `ost plugin build` correctly
  runs USD's script-style `usdGenSchema` through an interpreter, but for this
  runtime the interpreter was found only on the host (`python3.13`) rather than in
  the runtime. Local macOS had it; hosted macOS did not. Durable fix: the runtime
  manifest / target contract exposes the Python ABI required by schema tooling,
  and either the runtime provides a runnable interpreter or generated source CI
  installs that exact Python before the build. If neither is true, `ost plugin
  build` should fail before `usdGenSchema` with a precondition error naming every
  interpreter candidate and the exact setup action.
- ⬜ **P1 — pre-build requirements belong in the CI model, not `source_checks`.**
  The existing `source_checks:` feature is deliberately post-pyramid, so it is too
  late for Python setup, runtime-bin validation, or any other prerequisite needed
  before `ost plugin build`. Keep `source_checks` for repo-specific smoke tests,
  and render modeled pre-build prerequisites in a first-class section between
  runtime materialization and plugin build. Do not add a broad arbitrary
  pre-build shell hook unless a separate dogfood proves a repo-specific
  prerequisite that cannot be modeled.
- ⬜ **P1 — renderer and validation coverage for hosted macOS assumptions.** Add
  tests that generated GitHub source-CI places Python setup / runtime validation
  before the build step and leaves repo smoke tests after the pyramid; add runtime
  validation coverage for non-executable tool bits; and keep the Windows +
  macOS public-runtime PR lane as the acceptance fixture.
- ⬜ **P2 — documentation and cleanup.** Document the macOS hosted source-CI
  contract in `docs/examples.md` / runtime-publishing notes, record the incident
  shape for future debugging, then remove the temporary repo-local
  `actions/setup-python` and `chmod` repairs after the runtime is republished and
  CI is regenerated from `openstrata.ci.yaml`.

## Phase 7 — Sessions / sandbox ⬜

- ⬜ Session metadata; `ost session start | fork | diff | discard | promote`
- ⬜ Workspace isolation; optional Linux namespace / overlayfs

## Phase 8 — AI / GPU profiles ⬜

- ⬜ GPU host detection; driver requirement checks (`ost doctor gpu`)
- ⬜ AI runtime profiles (`ai-cuda124`, `ai-rocm`, `ai-mps`, hybrid `cy2026-lookdev-ai`)
- ⬜ Jenkins GPU routing labels; smoke tests

## Phase 9 — Kubernetes execution backend ⬜

Direction: [kubernetes.md](kubernetes.md). OpenStrata owns the runtime contract,
artifacts, and validation; Kubernetes is a pluggable **execution backend** that
runs those contracts on a cluster. `local` stays first-class; Kubernetes is
opt-in. Start narrow — generate → submit → monitor → collect a `batch/v1 Job` via
`kubectl` — not an Operator.

- ⬜ `ost-execution` crate: `ExecutionBackend` trait (`local` + `kubernetes`),
  domain `ResolvedTask` → `KubernetesJobRequest` → Job YAML separation
- ⬜ `ost submit build|validate|plugin-test|ai-validate|matrix --backend
  kubernetes` and `ost jobs list|show|logs|wait|artifacts|cancel`
  (logical `ostj_…` ids; `--output json` contract)
- ⬜ Phased: manifest export (`--dry-run --output yaml`) → kubectl submit/status/
  logs → artifact collection + provenance → matrix (`--max-parallel`,
  `--fail-fast`) → GPU tasks (with Phase 8) → Jenkins bridge (with Phase 5) →
  optional native `kube` client → CRD/Operator only if Jobs prove insufficient
- ⬜ Digest-pinned runtime/extension/source per Job (`latest` rejected);
  safe-by-default manifests; `ost doctor kubernetes`

## Phase 10 — DCC host support ⬜

Direction: [dcc-hosts.md](dcc-hosts.md). Runtime-native apps stay first-class;
existing DCCs (Maya/Houdini/Nuke) are supported as **third-party external hosts**
behind a host adapter boundary — discovered, fingerprinted, driven headlessly,
packaged for, and checked for cross-DCC USD compatibility. No DCC API
abstraction, install, license, or GUI-required path (§2.2).

- ⬜ `ost-host` crate: host record model, selectors, inventory, discovery
  providers (explicit/configured/known/env/PATH/registry/custom rules),
  `HostValidator` / `HostAdapter` traits; reuses the `--json` envelope + exit
  codes and the runtime `EnvSet`
- ⬜ Discovery + validation (candidate→validated→rejected, read-only/bounded/no
  GUI) and standard/deep fingerprints; Maya first, then Houdini + Nuke
- ⬜ `ost host discover|list|inspect|probe|run|test`; headless run with a composed
  env; host-standard packaging (Maya `.mod`, Houdini package JSON)
- ⬜ Matrix cells / support lines / tiers and cross-DCC USD compatibility edges
  (reusing the plugin-harness levels); `ost matrix …` / `ost compat …`
- ⬜ Fleet inventory export/import, `ost compat diff` / `ost reproduce`, optional
  Blender adapter

## Python / uv (§9)

- ✅ `ost uv <args>`: runs `uv` pinned to the project's runtime Python — applies
  the runtime environment and sets `UV_PYTHON` to the runtime interpreter, so uv
  never silently substitutes a different Python (§9.3, §20.3). No-args prints the
  pinning; `uv` is located via `OST_UV` or PATH. `uv.lock` is already hashed into
  `strata.lock`.
- ✅ **(v0.10.0)** Diagnose/refuse app-local `uv` deps that shadow ABI-sensitive
  runtime packages. `ost uv` reads the project's resolved packages (`uv.lock`, else
  the declared `pyproject.toml` deps), normalizes them (PEP 503), and flags any that
  duplicate a native family the runtime provides — OpenUSD (`usd-core`/`openusd`/
  `pxr`), Qt (`PySide6`/`shiboken`/`PyQt`), OpenColorIO, OpenEXR/OpenImageIO,
  MaterialX — since a duplicated binding sits ahead of the runtime's ABI-matched
  build on `sys.path` and crashes at import. It **warns** on every command and
  **refuses** an install-shaped subcommand (`sync`/`add`/`install`/`pip`/`lock`)
  that would materialize them, with `OST_UV_ALLOW_SHADOWED` as the escape hatch;
  the diagnosis is surfaced in the no-arg human + `--json` output too. Pure
  parse/normalize/shadow-match logic unit-tested.

## Distribution — `ost` binary releases 🚧

The `ost` CLI is a single self-contained binary (no Python/USD dependency), so it
ships independently of the heavy runtime artifacts. Publish tagged builds to
GitHub Releases. Implemented with **cargo-dist** (`dist-workspace.toml`,
`release.yml`); the generated workflow is hand-pinned to commit SHAs (SEC-004),
so a dist version bump needs it regenerated and re-pinned (`allow-dirty = ["ci"]`).

- ✅ **Tag convention.** Releases are cut from a tag `v<semver>` on `main`;
  cargo-dist parses the version from the tag and errors unless it matches the
  workspace `Cargo.toml` `version`. A `-rc.N` / prerelease suffix is marked
  "pre-release" automatically.
- ✅ **Release workflow** (GitHub Actions, triggered on `v*`/semver tags via
  cargo-dist). Builds a binary per target, each packaged with checksums:
  - `x86_64-unknown-linux-musl` (first-class, fully static for old-glibc
    portability), `aarch64-apple-darwin`, `x86_64-apple-darwin`,
    `x86_64-pc-windows-msvc`.
  - Per-archive `SHA256SUMS`, a `dist-manifest.json`, and `NOTICE` +
    `THIRD_PARTY_NOTICES.md` bundled into every archive; attached to the GitHub
    Release with generated notes. Built on the pinned toolchain.
- ✅ **Install ergonomics.** cargo-dist generates `shell` + `powershell`
  installers (fetch the right asset for the host, verify the checksum) hosted on
  the Release; `cargo binstall` works against the `dist-manifest.json`. Document
  `cargo install --path crates/ost-cli` as the from-source fallback.
- 🚧 **Provenance.** GitHub build provenance attestations (SLSA) are attached to
  release artifacts (`github-attestations = true`). Still ⬜: explicit
  signature/Sigstore key material and `ost`-side verification of it (tracks with
  Security baseline SEC-005).

This covers the **`ost` tool** itself; runtime/extension/plugin *content*
artifacts are distributed via the content-addressed store and the artifact
registry (Phase 6).

## Licensing & third-party attribution 🚧

OpenStrata must ship with a clear license of its own and **complete** attribution
for everything it bundles, links, or distributes. The project license, SPDX
headers, Rust-dependency attribution (CI-gated), and the plugin bundle license
field have landed; runtime/extension content attribution and per-artifact SBOM
remain (the latter with the Phase 6 content store).

- ✅ **OpenStrata's own license.** Top-level `LICENSE` (Apache-2.0, matching the
  manifests) and `NOTICE`; SPDX headers
  (`// SPDX-License-Identifier: Apache-2.0`) on all source files; `README` License
  section.
- ✅ **Rust dependency attribution.** `THIRD_PARTY_NOTICES.md` is generated for
  the crate tree with `cargo-about` (`about.toml`/`about.hbs`, host targets only)
  and committed; `licenses.yml` gates every PR with `cargo-deny` (SPDX allowlist
  in `deny.toml`, deny copyleft/unknown) and fails if `THIRD_PARTY_NOTICES.md` is
  stale (CRLF-normalized diff).
- ⬜ **Runtime/extension content attribution.** Anything OpenStrata builds or
  distributes (OpenUSD, MaterialX, TBB, OpenSubdiv, OpenEXR, OCIO, …, and their
  transitive deps) carries its upstream license. Each runtime/extension manifest
  records license metadata; built/adopted runtimes collect the upstream
  `LICENSE`/`NOTICE` files, and a runtime's licenses are inspectable
  (e.g. `ost runtime licenses <cy> --profile <p>`).
- 🚧 **Per-artifact notices + SBOM.** Notices: the `ost` binary archives bundle
  `LICENSE`/`NOTICE`/`THIRD_PARTY_NOTICES` (cargo-dist `include`), and plugin
  packages copy their `notices` files and record the bundle `license`. Still ⬜:
  a generated SBOM (SPDX or CycloneDX) per artifact and a package
  manifest/provenance that lists component licenses by digest (lands with the
  Phase 6 content store). **No artifact ships without complete third-party
  attribution** — this is a release gate.
- ✅ **Plugin bundle license field.** `openstrata.plugin.yaml` carries a `license`
  (SPDX) and optional bundle-relative `notices`. `ost plugin inspect` surfaces the
  license (human + `--json`/report.json), `ost plugin package` records it in the
  artifact `manifest.json` and copies the `notices` files into the package. The
  scaffold seeds `license: Apache-2.0`; `notices` paths are validated as
  bundle-relative (SEC-002).

## Security baseline 🚧

Shrinking the attack surface across build, runtime, plugins, CI, and the
distribution path before OpenStrata is used in production. IDs track the
security baseline document. P0 lands first; P1 next; P2 is continuous.

- ✅ **SEC-001 (P0) — package staging rejects unsafe files.** `ost package`
  classifies each entry by the entry itself (no symlink-following) and errors on
  a symlink, FIFO, socket, or device anywhere in the stage tree (including the
  root), so an artifact cannot absorb a link target's bytes or recurse outside
  the tree.
- ✅ **SEC-002 (P0) — plugin manifest paths stay in the bundle.** `Bundle::load`
  validates `usd.plug_info` and every fixture up front and rejects `..`,
  absolute, drive, and UNC paths (host-independent), so a malicious
  `openstrata.plugin.yaml` cannot steer reads outside the bundle.
- ✅ **SEC-003 (P1) — safe atomic writes.** `write_atomic` creates its temp file
  with `O_EXCL` and an unpredictable name, refuses to write over a symlinked
  destination, and fsyncs the parent directory (mode follows the umask, as a
  plain write would, since the current outputs are shared project config).
- ✅ **SEC-004 (P1) — CI supply-chain pinning.** Every third-party GitHub Action
  is pinned to a full commit SHA (with a `# vN` comment), and Dependabot manages
  SHA/dependency bumps as reviewable PRs. Release retains workflow-level
  `contents: read` with job-scoped grants and build provenance attestation.
- ⬜ **SEC-002 follow-up — symlink escape inside a bundle.** Reject a *real*
  symlink within a bundle that resolves outside the root at read time
  (canonicalize-and-contain), complementing the lexical manifest check.
- ⬜ **SEC-005 (P1) — installer & release-asset verification.** Publish per-release
  SHA-256 checksums, signature/Sigstore material, SBOM, and build provenance; the
  installer pins a version, verifies the checksum, and aborts on mismatch. Tracks
  with **Distribution → Install ergonomics / Provenance**.
- ⬜ **SEC-006 (P2) — runtime trust policy.** Introduce runtime trust levels
  (`local` / `verified` / `trusted`), record runtime source / version / platform
  / binary & plugin hashes / trust level in the manifest and lock, warn on
  world-writable runtime roots, and let `ost build` / `ost plugin test` require a
  minimum trust level (release/production CI refuses `local`).
- ✅ **CI test gate.** `.github/workflows/ci.yml` runs `fmt`
  (`cargo fmt --all -- --check`), `clippy`
  (`cargo clippy --workspace --all-targets --locked -- -D warnings`), and `test`
  (`cargo test --workspace --locked`) on every push to `main` and every PR, so the
  security regression tests above now run in CI. Linux-only / mock-runtime only
  (no real DCC, no OS matrix). Marked as required status checks (with `licenses`)
  on protected `main`. Actions are SHA-pinned (SEC-004).

## Quality bar (applies to every phase)

- CLI errors must be actionable.
- All generated manifests must be deterministic.
- Runtime and extension identities always include version + target + digest.
- No hidden environment mutation outside `ost devshell` / `ost env`.
- Every published artifact includes provenance and validation result.
- Every published artifact carries complete third-party attribution (no missing
  upstream licenses/notices).
- OpenStrata must work without a preinstalled Python environment.
