# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## Next milestone: v0.19.0 - composition and reach

**Status:** 🚧 in progress · **Depends on:** the v0.18.0 evidence-integrity,
target-lease, test-lifecycle and workspace-closure contracts (shipped).

v0.19.0 was scheduled as the Formation composition milestone. Three v0.18.0
dogfooding passes on 2026-07-18 — two of them from a **real published release** —
re-planned it into two ordered halves. Two further v0.18.0 passes from the newly
adopted `usd-3dgs-plugins` workspace on 2026-07-18 and 2026-07-19 refine the same
reach boundary. The findings are not scope creep on Formation; they are the
foundation it was already standing on:

> The [Formation acceptance criteria](backlog.md) require *four first-party
> dogfoods run from packaged, digest-pinned artifacts, each on a clean machine
> or isolated prefix*. As of v0.18.0 **no packaged bundle from a split workspace
> is independently installable**, so none of those four dogfoods can pass. The
> reach work below is what makes the Formation milestone acceptance-testable.

The dogfooding trail:

- usd-3dgs-plugins
  ([`animu-sphere/usd-3dgs-plugins`](../projects/usd-3dgs-plugins.md),
  [report 01](https://github.com/animu-sphere/usd-3dgs-plugins/blob/main/docs/reports/ost/01-2026-07-18-v0.18.0-bootstrap.md)
  and
  [report 02](https://github.com/animu-sphere/usd-3dgs-plugins/blob/main/docs/reports/ost/02-2026-07-19-package-provenance-and-reproducibility.md)):
  an empty repository reached a real PLY importer, ordinary-library composition,
  source L5, three hosted OS cells, and clean extracted-package consumption.
  The smaller workspace then exposed two last-mile package gaps: the packaged
  test loses its golden oracle, and packaging cannot tell which build flavor
  last overwrote the bundle output.
- usd-vrm-plugins (`animu-sphere/usd-vrm-plugins`,
  `23-2026-07-18-…-workspace-packaging-v0.19.0-asks.md` and
  `24-2026-07-18-…-first-workspace-release-v0.19.0-asks.md`): v0.2.0 shipped for
  real — three bundles × three OS, 20 assets — off the v0.18.0 workspace verbs.
  The verbs held. The artifacts did not: a packaged bundle records a dependency
  closure it does not actually carry, and a consumer outside `ost` cannot reach
  the bytes that *are* carried.
- hdMerlin (`animu-sphere/hydra-merlin`,
  `07-2026-07-18-v0.18.0-recheck-v0.19.0-asks.md`, findings OST19-RND-001..008):
  v0.18.0's evidence and lifecycle model is right, but its *implementation
  cannot see* the build trees and producer sessions that actually exist — so
  `runtime-compatible` and `renderer-evidence` are stuck at an honest SKIP/FAIL
  with no reachable exit.

All three repositories surfaced the same shape, which is the theme of this
milestone: **v0.18.0's models are correct and its reach is short.** The artifact
is closed under `ost` and open under everything else; a package test can carry
its input but not its oracle; the provenance model is right but cannot bind the
staged bytes to the build that produced them; and the producer-session rule is
right and unreadable by any producer.

### Half A - reach (ships first, gates Half B)

Priorities P0/P1/P2 below. This half is corrective and must land before the
Formation slice is started, because it is Formation's own precondition.

### Half B - Formation composition (narrowed)

The Formation MVP as scoped in the [backlog ladder](backlog.md), narrowed to
`ost formation resolve|inspect|run|lock` — `env` and `doctor` move to v0.20.0.
Design target: [formations.md](../design/proposed/formations.md). The aggregate
product artifact (Half A, P1) is the input Formation resolves against, which is
why it moves out of the backlog and into this milestone.

If Half A consumes the milestone, Formation ships in v0.20.0 and the DCC host
milestone moves to v0.21.0 — but Half A does **not** get deferred to protect the
Formation date. A composition layer over artifacts that cannot be installed is
not worth having.

### P0 - a packaged bundle carries the closure it claims

From usd-vrm-plugins report 23 §2/§6(1) — the one correctness regression in the
set, and the reason v0.2.0 needs a repo-side workaround to verify what it ships.

v0.18.0 records `bundles` in `dependencies.json` and stages the dependency's
*link* half (`libvrmSchema.dll` + its CMake package). It does not stage the
*registration* half (`plugInfo.json`, `generatedSchema.usda`) — for a bundle
declared `kind: usd-schema`, which is precisely the kind whose entire value is
runtime registration. The result is an artifact that asserts a closure it does
not have: `Usd.Stage.Open()` still fails, and the package now looks *more*
complete than it is. v0.17.0 omitted the bundle honestly, with no `bundles` key
at all; v0.18.0 is confidently wrong where it used to be honestly silent.

- For a `requires.bundles` edge, stage the dependency's USD resource tree into
  the package (`runtime/bundles/<id>/…`), not only its link artifacts.
- Declare the staged tree in the packaged manifest so the session plugin path
  includes it — `L0 session.plugin_path` must see it the same way
  `requires.runtime_libs` directories are already declared and activated.
- `ost plugin test --from-package` on a single packaged bundle then passes
  without any workspace flag, because the package is genuinely closed. That is
  the acceptance test, and it is the test that fails today.

### P1 - staged bytes are reachable outside `ost`

From usd-vrm-plugins report 24 §2/§3(4). Same shape as the P0, one layer out:
the package contains the right bytes and a consumer has no supported way to
reach them.

`ost` stages dependency libraries into `runtime/libraries/{lib,bin}` and knows
to activate them at `plugin run`. Nothing in the artifact tells a consumer
installing from a GitHub release how to do the same, and the failure blames the
wrong component — a missing transitive DLL surfaces as
`Cannot determine file format for @….vrm@`. `requires.runtime_libs` is a
*description*, not an activation mechanism: using it means parsing the staged
manifest, resolving a non-portable path, and knowing that Python 3.8+ removed
`PATH` from the DLL search used for dynamically loaded modules.

Any of: co-locate dependency libraries beside the plugin so the platform's
default search finds them; emit a per-platform activation snippet into the
package; or publish the staged layout as a stable, portable, documented
contract. Today it is a real, load-bearing, `ost`-internal convention.

**Implemented on the v0.19.0 branch:** each package now emits a versioned
`openstrata.activation.json` plus PowerShell, Bash, and Python entrypoints. The
Python entrypoint retains `os.add_dll_directory()` handles on Windows, closing
the Python 3.8+ DLL-search gap rather than documenting `PATH` as sufficient.

### P1 - package-origin verification carries its golden oracle

From usd-3dgs-plugins
[report 01](https://github.com/animu-sphere/usd-3dgs-plugins/blob/main/docs/reports/ost/01-2026-07-18-v0.18.0-bootstrap.md).
The source bundle declares `one-gaussian-ascii.ply` as its roundtrip fixture and
keeps `one-gaussian-ascii.ply.golden.usda` beside it. Source L5 passes. Packaging
copies the declared fixture but neither discovers the adjacent golden nor offers
a manifest field to declare it, so an explicitly requested package-origin L5
returns SKIP. The package is runnable but cannot reproduce the verification
claim made by its source.

- Define a versioned roundtrip fixture + oracle contract: either an explicit
  fixture/golden pair or a deterministic adjacent-golden convention recorded in
  the packaged manifest.
- Stage and hash the oracle as verification content, preserving its association
  with the roundtrip fixture after extraction.
- Treat a requested verification level whose declared oracle was omitted as a
  packaging/validation failure, not a successful package with a silent gap.
- Acceptance: `ost plugin test <bundle> --from-package --up-to 5` reports
  `golden.roundtrip` PASS for the extracted package, with no source-tree path.

**Implemented on the v0.19.0 branch:** packaging now discovers the deterministic
`<roundtrip fixture>.golden.usda` neighbor, stages and hashes both inputs, and
records their association in versioned `openstrata.verification.json` plus the
artifact manifest. Package-origin L5 verifies those digests before running
`usdcat`; a declared oracle that is missing or changed is FAIL, not SKIP.

### P1 - package output is bound to managed build provenance

From usd-3dgs-plugins
[report 02](https://github.com/animu-sphere/usd-3dgs-plugins/blob/main/docs/reports/ost/02-2026-07-19-package-provenance-and-reproducibility.md).
The dual-mode root CMake build and `ost plugin build` intentionally produce the
same discoverable bundle layout. `ost plugin package` stages whichever binary is
currently in `lib/`; a plain Visual Studio build therefore replaced the managed
Ninja build and the July-18 package carried that different flavor without a
signal. This is the package equivalent of the managed-test lifecycle rule: the
consumer needs to know which build produced the bytes being asserted.

- `ost plugin build` records the target/runtime/build fingerprint and digests of
  the package-relevant output set it completed.
- `ost plugin package` compares every staged managed output with that record and
  reports `matched`, `untracked`, or `mismatched` provenance in human, JSON, and
  package metadata.
- A release/reproducibility lane fails on `mismatched` output unless an explicit
  override records the external/unmanaged origin honestly; plain CMake remains a
  supported producer, not an invisible replacement.
- Acceptance: overwrite the bundle output with `cmake --build build/plain`, then
  package it; OpenStrata warns or refuses with the changed file, expected digest,
  observed digest, and last managed build identity.

### P1 - external provenance can see the trees that exist

From the hdMerlin report OST19-RND-001/OST19-RND-002. `ost external import`
rejects both build-tree flavors that project produces, and `ost validate` names
`external import` as the next action for `runtime-compatible` — a closed
guidance loop with no reachable exit.

- **Generator-aware identity.** Visual Studio generators never write a top-level
  `CMAKE_CXX_COMPILER` cache entry; every VS tree is rejected for a variable
  that generator does not emit. The identity is fully recorded in the same tree
  at `CMakeFiles/<version>/CMakeCXXCompiler.cmake`, alongside
  `CMAKE_GENERATOR{,_INSTANCE,_PLATFORM,_TOOLSET}`. Resolve identity from the
  generator's actual sources; model multi-config generators first-class rather
  than assuming a single `CMAKE_BUILD_TYPE`; name the detected generator flavor
  and the unresolved identity source in the diagnostic. Cover Ninja, Ninja
  Multi-Config, Visual Studio and Xcode trees in tests.
- **Capability-scoped requirements.** `external import` demands `pxr_DIR` even
  for a `core` profile that v0.18.0's own `doctor` says "exercises no
  OpenUSD-dependent capability". Derive import requirements from the resolved
  profile and requested capabilities, accept `--capability` the way `doctor`
  now does, and record which requirements were applied versus skipped as
  not-applicable so a later `validate` can tell "not required" from "not
  checked".
- **Applicable remediation.** A hint is emitted only when following it can
  change the outcome. A `pxr_ROOT` hint does not belong on a compiler-identity
  failure, and when `validate` recommends a command it either verifies the
  command applies to that tree or explains the precondition instead.

### P1 - the producer-session contract is readable by producers

From the hdMerlin report OST19-RND-007. A build **`ost` itself performed**,
through the new first-class `ost renderer viewport`, produced a report that
`ost validate` then rejected. The rejection is correct — the report records no
producer session — but the enforcement message is the only description of the
requirement that exists anywhere. It names no field, no schema version, and no
required shape, and no CLI surface emits or attaches one. A producer cannot
conform to a contract it cannot read.

- Publish and version the renderer-report schema, including the producer-session
  shape, rather than implying it from an error.
- **When `ost` owns the producing session (`ost build`, `ost test`,
  `ost renderer viewport`), `ost` records the session outcome itself.** If the
  producing project must self-assert success, an unreliable producer simply
  asserts it — which is the exact class of problem the v0.18.0 P0 removed.
- For genuinely external producers, a supported command attaches a producer
  session to an existing report, recording the external/unverified origin
  honestly.
- The rejection diagnostic names the missing field and the schema version it was
  evaluated against, and a schema mismatch is distinguishable from a well-formed
  report recording a failed session.

### P1 - aggregate product artifact

From usd-vrm-plugins reports 23 §6(3) and 24 §3(3), and promoted out of the
[backlog](backlog.md) where v0.18.0 deliberately parked it. v0.2.0 publishes
three assets per target plus a documented install order; an aggregate collapses
that to one. Defining it means deciding how a consumer installs and pins a *set*
rather than a bundle — which is the Formation model, and why this now sits at
the Half A / Half B seam. Preserve member bundle digests, member manifests and
provenance; define the extraction layout and aggregate evidence; do not fall
back to workspace source paths or a hand-maintained per-bundle loop.

**Implemented on the v0.19.0 branch:** `plugin package --workspace --product`
wraps the exact member archives, manifests, checksums, SBOM/provenance and debug
sidecars under `members/<id>/`, records the graph-derived install order, and
emits its own digest, manifest and aggregate evidence as artifact kind
`product`.

### P1 carry - named build intents

From the hdMerlin report OST19-RND-003 (carried unchanged from OST18-RND-007).
`ost build` still exposes no `--intent` and the manifest accepts no intent
declaration, so typed CMake cache inputs cannot be expressed and the MaterialX
configuration stays manual CMake.

v0.18.0 made this reproducible on the *default* path with no optional dependency
involved: `ost renderer viewport` builds with `BuildIntent::default()`, so with
`MERLIN_ENABLE_HYDRA2` defaulting to `OFF` the binary it produces cannot open a
stage. The refusal correctly names the cache variable to set — and neither
`ost build` nor `ost renderer viewport` can set it. A user following in-product
advice accurately arrives at a configuration `ost` cannot express. Note that
manifest strictness (P2 below) is a prerequisite: the manifest cannot fail closed
on a malformed intent declaration while it fails closed on nothing.

### P2 - contract and diagnostic consistency

- **Print the exact immutable evidence-gap recovery command** (usd-3dgs-plugins
  [report 01](https://github.com/animu-sphere/usd-3dgs-plugins/blob/main/docs/reports/ost/01-2026-07-18-v0.18.0-bootstrap.md)).
  When `ost ci validate` knows `runtime_remote`, its expected OCI digest, and
  `runtime_artifact`, the diagnostic names the exact safe
  `ost artifact pull ... --expect-artifact ...` command that refreshes missing
  SBOM/provenance evidence without changing the pinned artifact identity.
- **Resolve package sessions from required capabilities, or name the failed
  profile choice** (usd-3dgs-plugins
  [report 02](https://github.com/animu-sphere/usd-3dgs-plugins/blob/main/docs/reports/ost/02-2026-07-19-package-provenance-and-reproducibility.md)).
  Outside a project, `plugin run <extracted-root> --target cy2026` currently
  defaults to `core` and then fails `REAL_RUNTIME_REQUIRED` even though the
  package declares `requires.capabilities: [usd-stage-read]`. Resolve a unique
  satisfying profile from the capability graph; if none or several qualify,
  fail with the selected/defaulted profile and an exact `--profile` correction.
- **Offer an explicit across-build reproducibility check** (usd-3dgs-plugins
  report 02 observation). The package-twice gate correctly proves archive
  determinism for one build but cannot observe compiler/linker timestamps across
  clean builds. Add an opt-in release-lane mode that builds in two isolated
  roots, packages both, compares artifact digests, and identifies the earliest
  differing output. Keep it opt-in because it deliberately doubles native build
  cost.
- **Normalize staged paths to `/`** (usd-vrm-plugins reports 22 §11.5, 23 §5,
  24 §3(5) — filed three times, and no longer cosmetic). A Windows-produced
  package writes `runtime/libraries\bin` into portable, digest-addressed data.
  It is the exact string a consumer must turn into a loader path to implement
  the P1 activation contract above, and splitting it on `/` yields
  `libraries\bin`. A Windows-produced and a Linux-produced package must not
  differ in a field describing the same layout.
- **Fail closed on unknown manifest keys** (hdMerlin OST19-RND-004). Unknown
  `openstrata.toml` tables — a plausible-but-unsupported `[build.intents.*]` and
  an outright `[nonsense_table]` alike — are accepted with `ok: true` and an
  empty `warnings` array. Low-impact today; a correctness problem the moment
  intents ship, when a typo'd cache key silently produces a build with the
  feature disabled and evidence that looks legitimate. Reject unknown top-level
  tables and unknown keys in known tables, naming the offending path and the
  closest valid key; distinguish "unknown to this `ost` version" from "invalid
  anywhere"; fail closed on duplicate tables. An off-by-default
  `--allow-unknown-manifest-keys` escape hatch may exist. **Open decision:**
  whether this ships as a breaking change in v0.19.0 or needs a warning-only
  deprecation window first.
- **Honour `--json` on the viewport success path** (hdMerlin OST19-RND-008, and
  where the still-open half of OST18-RND-005 now lives).
  `ost renderer viewport --json` emits a well-formed envelope on failure and
  *raw child output* on success — so the success case, the one carrying the
  launch outcome, resolved executable, backend and device, is the unparseable
  one. Those values are exactly the durable launch record OST18-RND-005 asked
  for, and today they are printed and discarded. One envelope on both paths with
  child output captured to a field or log path; a launch/readiness record that
  persists after exit; the same contract for `renderer view`.
- **Warn on conflicting `plugin run` flags** (usd-vrm-plugins report 24 §2.4).
  `--no-inject` sounds like it makes the bundle argument inert; it does not — the
  bundle argument still selects whose `requires` get staged. This cost the
  downstream three invalid experiments before the harness was corrected. Warn
  when `--plugin-path` roots exclude the bundle argument's own tree.
  **Implemented on the v0.19.0 branch:** the warning carries stable code
  `PLUGIN_RUN_PLUGIN_PATH_MISMATCH` and stays quiet when an extracted root has
  the selected bundle's identity/version/kind.
- **Say something when no debug package is produced** (usd-vrm-plugins report 24
  §1.1). `debug_archive: null` on every package, every cell, all three OS, while
  `plugin package --help` documents lean/split as the default — which reads as
  though a sibling `*-debug` package is the normal outcome. If splitting requires
  something of the build profile, say so at `package` time.
  **Implemented on the v0.19.0 branch:** human and JSON output plus the producer
  manifest distinguish `split`, `included`, and `not-produced`, with the latter
  naming the absence of separate `.pdb`/`.dwo` files and the embedded-debug
  limitation.
- **Redacted diagnostic export** (hdMerlin OST19-RND-005, carried unchanged from
  OST18-RND-008). No `--redact-paths`, no `ost report` subcommand; machine JSON
  still carries the absolute project root, absolute rendered command paths, and
  a `runtime_env` array of user-profile runtime-store paths.
- **Correct the `--from-package` help text.** `ost plugin test --from-package`
  still documents itself as "incompatible with `--workspace`". The composition
  shipped in v0.18.0 and works; only the help text is stale. usd-vrm-plugins
  report 23 §3 read the help, believed it, reused the already-existing
  `scripts/clean_install_smoke.py`, and re-filed a capability that had shipped.
  Report 25 corrects the narrower cost: one wrong downstream conclusion and a
  duplicate ask, not the creation of that smoke harness.

### P3 - cosmetic

- `ost doctor` reports `env_keys` with `PATH` listed twice (hdMerlin
  OST19-RND-006). Deduplicate, or model the entries as ordered key/value pairs.
- Observe localized MSVC `/showIncludes` output in the usd-3dgs-plugins hosted
  Windows lane before filing a stronger suppression/change. Japanese local
  output is very noisy during `ost plugin build --json`, but the build succeeds
  and stdout still ends in the correct JSON contract; this is log ergonomics,
  not a correctness blocker.

### Answers owed to hdMerlin

Report 07 §"Requested maintainer decisions" asks six questions directly. The
positions taken above: (1) resolve compiler identity from the generator's own
sources *and* model multi-config explicitly; (2) derive requirements from
profile capabilities *and* accept an explicit `--capability`; (3) yes — a
recommended command is verified applicable or replaced by an explanation;
(4) open, and called out in the P2 item; (5) yes — `ost` stamps the session when
it owns the build, and the schema version ships published; (6) both carried asks
stay on v0.19.0 (OST19-RND-003 as P1 carry, OST19-RND-005 as P2).

## Shipped: v0.18.0 - evidence integrity and ecosystem documentation

**Status:** ✅ released 2026-07-18 — see the
[v0.18.0 release record](../releases/v0.18.0.md). Retained below for the
dogfooding trail that drove it.

v0.18.0 is a corrective release, re-planned from the previously scheduled DCC
host milestone (now v0.20.0 in the [backlog ladder](backlog.md), behind the new
v0.19.0 Formation composition milestone). Two v0.17.0 dogfooding passes surfaced
the same defect class at two layers: a PASS or a success report that is not bound
to a completed, owning producer.

- hdMerlin (`animu-sphere/hydra-merlin`,
  `2026-07-15-v0.17.0-dogfooding-v0.18.0-asks.md`): a renderer assertion became
  PASS from a CTest that later timed out, and two concurrent invocations wrote
  the same managed target (findings OST18-RND-001..006).
- usd-vrm-plugins (`animu-sphere/usd-vrm-plugins`,
  `22-2026-07-17-v0.17.0-evidence-gate-v0.18.0-asks.md`): `ost ci generate`
  emits an SBOM/provenance gate no existing artifact can satisfy, while
  `ArtifactStore::import` silently drops the evidence that would satisfy it —
  adopting 0.17.0 turned all nine hosted PR cells red with no repo-side cause.

v0.18.0 extends v0.17.0's file-level build truth to whole operations: evidence
import, generated CI gates, renderer producer sessions, and target ownership.
No new DCC or host surface ships in this release.

Alongside the evidence-integrity fixes, v0.18.0 lands an ecosystem documentation
slice. The two real downstream repositories that already exercise these
contracts — `animu-sphere/usd-vrm-plugins` and `animu-sphere/hydra-merlin` — are
documented as **reference projects**, and the cross-repository **Formation**
model they motivate is written up as a v0.19.0 design target. This is
documentation only: it must not weaken or defer the P0 evidence-integrity work
below and ships no `ost formation` surface (see the
[documentation priority](#p1---reference-projects-and-formation-design-documentation)).

### P0 - artifact evidence attaches and gates honestly

From the usd-vrm-plugins report §2/§3 (both P0 there):

- Move the evidence attach in `ArtifactStore::import` out of the
  producer-manifest equality guard: sidecar SBOM/provenance that verifies
  against the stored archive digest attaches even when the digest is already
  in the registry, whatever the producer manifest says.
- Never drop caller-supplied evidence silently. A refusal is a coded error with
  a non-zero exit; import outcomes surface `evidence_attached` /
  `evidence_skipped` in `--json`.
- Make the generated evidence gate migratable: `require_evidence` is
  expressible in `openstrata.ci.yaml` (per cell and/or globally),
  `ost ci generate` warns when a pinned `runtime_artifact` lacks the evidence
  the rendered gate will demand, and `ost ci validate` fails fast on the same
  condition.
- The generated cache-hit short-circuit uses the gate's own verify predicate,
  so a cached evidence-less record cannot wedge a lane permanently.
- Add a supported registry reset — `ost artifact rm <digest>` or an equivalent
  forced re-import — so a machine that already holds a pre-evidence digest can
  obtain its evidence without hand-deleting `$OST_HOME/artifacts` internals.

### P0 - one completed producer behind every renderer PASS

From the hdMerlin report OST18-RND-001/002 (both P0 there); absorbs the
"renderer host evidence capture" carry-over:

- Renderer report overlays record a producer session: id, kind, target, start
  and completion times, and final success/failure state. A producer writes a
  successful overlay atomically only after its containing command or declared
  check completes; interrupted temporary overlays are not mergeable.
- `ost renderer merge` preserves producer provenance and refuses PASS
  assertions from failed, incomplete, mismatched, or superseded sessions.
  `ost validate` explains which producer contributed each assertion.
- Configure, build, output verification, and completion publication hold one
  OS-backed exclusive target lease. A second writer deterministically fails
  busy, waits with a timeout, or attaches read-only — the behavior is explicit,
  never implicit. Stale-owner recovery verifies process identity before
  takeover; build logs and completion records name the owning invocation.

### P1 - managed test lifecycle and external build provenance

From the hdMerlin report OST18-RND-003/004:

- Add a deliberate `ost test` command (or an explicit `ost build --test` mode —
  plain build semantics do not change silently) that propagates the selected
  runtime, compiler/vcvars environment, generator intent, configuration, and
  build fingerprint; provides overall and per-test timeouts with Windows
  process-tree cleanup; and records `tested` as distinct from `built`,
  `packaged`, and `host-tested`.
- Import exact external build provenance: a configure-time record or explicit
  import binding source root, build directory, runtime id/digest/root, OpenUSD
  version, compiler/CRT, Python ABI, generator, and configuration, hashed
  against the inspected CMake cache. `validate --build-dir` upgrades runtime
  compatibility only on a full identity match and never claims `ost build`
  configured or built the external tree.

### P1 - workspace packaging closure

From the usd-vrm-plugins report §11; schedules the "packaged workspace/product
composition" backlog item and the carried `requires.bundles` closure gap:

- `ost plugin package --workspace` packages every discovered bundle in
  dependency order — the same graph `plugin test --workspace` already
  validates.
- `dependencies.json` records resolved `bundles` the way it already records
  `libraries` (id, version, contract, provenance), so a consumer can detect a
  missing closure instead of hitting a runtime schema-application failure.
- Let `--from-package` compose with `--workspace` so a packaged workspace is
  verifiable by the same pyramid as its source tree; define the aggregate
  product artifact or defer it explicitly.
- Normalize staged manifest and `dependencies.json` relative paths to `/`;
  host-shaped separators must not ship inside portable artifacts.

### P1 - truthful artifact records

From the usd-vrm-plugins report §4: `record.producer` is stamped by the
importing ost, not the producing tool, so the same image reads `ost 0.10.0` on
one machine and `ost 0.17.0` on another. Carry the producer from the dist
manifest (preferred) or rename the field so it cannot be read as origin.

### P2 - provenance and pin ergonomics

From the usd-vrm-plugins report §5/§6/§7/§8:

- Accept explicit build metadata (for example
  `ost runtime export --build-metadata <file>`) validated under the same
  required-fields rule, so non-GitHub-Actions producers can emit provenance.
- Document that `runtime_remote.expected_oci_digest` changes on every republish
  while `runtime_artifact` does not (evidence layers embed the producing
  commit); state it in `artifact push --json` or add a repin helper.
- Mirror the JSON error report (at least the `error.code`) to stderr so a
  redirected `--json` stdout cannot hide a failure.
- Unify or document together the `runtime export --json` and
  `artifact show --json` digest/file-count shapes; warn when
  `runtime export --slim` would drop a layout referenced by the SDK's own
  CMake package config.

### P2 - session-aware renderer view and doctor advice

From the hdMerlin report OST18-RND-005/006:

- `renderer view` defaults to automatic camera selection: use the named camera
  only when valid, otherwise omit it and report free-camera selection. An
  opt-in detached mode records a durable session — process identity, runtime,
  renderer, scene, staging prefix, log paths, readiness, and exit state — and
  classifies optional host warnings separately from plugin-discovery,
  renderer-selection, scene-open, and first-frame failures.
- `ost doctor` next actions depend on the selected profile and the capability
  being exercised; the core profile does not recommend `--from-usd` unless an
  OpenUSD-dependent action was requested, and JSON explains why a real runtime
  would change a result.

### P1 - Reference Projects and Formation design documentation

An ecosystem documentation slice, added without touching the evidence-integrity
code above. It names the real downstream repositories as **reference projects**,
records which OpenStrata contract each one proves, and specifies the
cross-repository **Formation** model as a v0.19.0 design target — so the v0.19.0
implementation has a stable public target and no reader mistakes a planned
capability for a shipped one.

- Add a `docs/projects/` category: a
  [Reference Projects overview](../projects/README.md) (ecosystem map, project
  comparison, cross-repository link policy) plus a project page for
  [usd-vrm-plugins](../projects/usd-vrm-plugins.md) (multi-bundle OpenUSD plugin
  workspace) and one for [hydra-merlin](../projects/hydra-merlin.md)
  (host-neutral renderer). Each page states which OpenStrata capability the
  project validates and links to the authoritative downstream documentation
  instead of copying command references or support tables.
- Write the Formation concept and design as
  [design/proposed/formations.md](../design/proposed/formations.md):
  terminology (declared / resolved / lock / run / evidence), the
  declared-versus-shipped boundary, and how Formation reuses the runtime,
  artifact, plugin, renderer, and evidence models rather than introducing a
  parallel composition mechanism. The planned cross-repository workflows (VRM
  inspection, hdMerlin view, VRM rendered by hdMerlin) are documented in
  [projects/combined-formations.md](../projects/combined-formations.md) as
  **planned**, not shipped.
- Add transferable adoption guides —
  [adopt a plugin workspace](../guides/adopt-a-plugin-workspace.md),
  [adopt a renderer project](../guides/adopt-a-renderer-project.md) — and a
  v0.19.0-oriented [compose a formation](../guides/compose-a-formation.md)
  guide, plus a concise **Reference projects** section in the root README that
  links to the overview.
- Every Formation reference is explicitly labeled available in **v0.19.0, not
  v0.18.0**; the documentation link and consistency checks
  (`scripts/check_doc_links.py`, `scripts/check_docs_consistency.py`) stay
  green.

This priority ships documentation only. It does not implement `ost formation`,
Formation resolution, lock files, or any cross-repository artifact composition —
those are the [v0.19.0 Formation composition milestone](backlog.md).

## v0.17 environment-dependent acceptance

The lifecycle, managed-view, adoption, and renderer evidence contracts shipped
in v0.17.0; these checks require hosted operating systems, real OpenUSD
installations, or downstream renderer repositories:

- Repeat renderer core-only, Vulkan, and Hydra acceptance across the remaining
  hosted OS/OpenUSD matrix. The Windows cy2026 cell, managed hdMerlin view,
  external build validation, and report merge policy are recorded in the
  [v0.17.0 acceptance report](../reports/2026-07-14-v0.17.0-managed-renderer-view-hydra-merlin.md).
- Downstream v0.17.0 dogfooding is recorded in
  `2026-07-15-v0.17.0-dogfooding-v0.18.0-asks.md` (hydra-merlin) and
  `22-2026-07-17-v0.17.0-evidence-gate-v0.18.0-asks.md` (usd-vrm-plugins);
  their findings drive the v0.18.0 milestone above.

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
  Note (from the 2026-07-17 usd-vrm-plugins report §3.1): the macOS packing
  change is why macOS re-exports land on new digests while Linux/Windows
  re-exports are byte-reproducible.
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
- **Evidence for already-published plugin artifacts.** A pinned
  `plugin_artifact` published before evidence existed cannot satisfy the
  evidence gate and has no republish story equivalent to the runtime one
  (usd-vrm-plugins report §8); define the plugin-package republish/attach path
  once the v0.18.0 import fix lands.
