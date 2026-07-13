# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## Next milestone: v0.17.0 — renderer build truth and one-command Hydra inspection

**Status:** 🚧 active · **Depends on:** v0.16.0 renderer composition,
runtime-session, evidence, and trusted-release contracts (shipped). The local
implementation is underway; real hdMerlin/OpenUSD acceptance remains a release
gate.

The hdMerlin adoption pass proved the renderer ownership and report contracts,
then exposed lifecycle seams around them. v0.17.0 makes build completion fail
closed, makes external CMake failures bounded and diagnosable, and turns
`ost renderer view` into the normal configure → build → install → usdview loop
without inventing a second renderer build lifecycle. Explicit external build
trees remain supported and are labeled as manual evidence.

### P0 — atomic build completion and recoverable external tools

- Replace build-directory existence as the generic `built` check with an atomic
  completion record written only after configure, build, and output verification
  succeed. Bind it to the target id, runtime digest, compiler fingerprint,
  generator, source project, build directory, and effective build intent.
- Invalidate the previous completion record before starting a new build; an
  interrupted configure/compile, stale cache, or copied renderer report must not
  pass `ost validate` as a completed OST build.
- Report active child phase, elapsed time, PID, log path, and bounded output tail.
  Add configurable configure/build timeouts, process-tree cancellation/cleanup,
  and an explicit generator escape hatch while keeping Ninja the default.
- Make dry-run and execution share one generated-file plan; the default build
  must name tool-owned `CMakeUserPresets.json`, not root `CMakePresets.json`.

### P0 — managed Hydra adapter view loop

- Make `ost renderer view [scene]` resolve a compatible pulled real OpenUSD
  runtime, request the `hydra2` build intent through the common build service,
  incrementally build a fingerprinted managed tree, stage its install tree, and
  launch usdview with the installed renderer selected.
- Define one project-facing CMake intent (for example
  `OST_RENDERER_ADAPTERS=hydra2`) that generated and adopted renderers map to
  their project-owned adapter option. Do not serialize a project's complete
  target graph or private CMake policy into the renderer manifest.
- Keep `--build-dir` as the opt-in external/prebuilt escape hatch. Validate and
  label that tree without claiming `ost build` produced it, and retain explicit
  `--profile`, `--config`, renderer, scene, and camera overrides.
- Fingerprint the OpenUSD SDK/runtime relationship beyond optional `pxr_DIR`
  cache entries so CMake discovery through `CMAKE_PREFIX_PATH`, imported targets,
  or package registries cannot silently launch against a different runtime.

### P1 — honest renderer adoption and portable evidence

- Add a dry-run-first renderer adoption flow that writes the common project and
  strict renderer manifests without overwriting project source/CMake files.
  Record adopted/migrated provenance rather than generated scaffold provenance.
- Allow validation against an explicit external build/evidence directory, or
  import it only after schema, renderer identity, target identity, runtime, and
  provenance checks pass.
- Provide deterministic renderer-report merge semantics: duplicate assertion
  ids require explicit replacement, FAIL cannot be silently downgraded, and
  conflicting device/runtime identities are errors.
- Preserve project-owned release version sources through an explicit version-file
  or synchronization policy instead of requiring an unqualified second static
  source of truth.

### P2 — bounded backlog closures

- Generate `docs/reference/environment-variables.md` from one structured source
  for supported `OST_*` inputs and generated session/CI variables; keep the
  reference freshness check in the existing docs-generation CI gate.
- Reuse `support/platforms.toml` to validate that generated hosted CI cells do
  not exceed declared feature/platform support. Report omissions and unsupported
  claims before workflow generation.
- Publish a first-class portable-Linux runtime build guide, including a reference
  container recipe, Python 3.13/venv and X/GL prerequisites, and the rule that the
  build container's glibc must not exceed the oldest target runner.

The DCC host milestone now follows as v0.18.0. It must consume the same renderer
identity, installed Hydra product, runtime fingerprint, and renderer evidence
chain established here rather than introduce a parallel DCC renderer contract.

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
