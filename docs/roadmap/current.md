# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## Next milestone: v0.18.0 - evidence integrity fix release

**Status:** ⬜ not started · **Depends on:** v0.17.0 renderer lifecycle,
artifact evidence, and generated-CI contracts (shipped).

v0.18.0 is a corrective release, re-planned from the previously scheduled DCC
host milestone (now v0.19.0 in the [backlog ladder](backlog.md)). Two v0.17.0
dogfooding passes surfaced the same defect class at two layers: a PASS or a
success report that is not bound to a completed, owning producer.

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
