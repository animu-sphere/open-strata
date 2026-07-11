# Overview

OpenStrata (`ost`) turns VFX Reference Platform compatibility into **executable,
validated, distributable runtime layers**. It is a runtime / build / extension /
validation manager for VFX and OpenUSD work.

```text
VFX Reference Platform
  -> machine-readable platform manifest
  -> immutable runtime artifact
  -> controlled extensions
  -> validated capability graph
  -> reproducible build / extension / session
```

OpenStrata is **runtime-centric, not DCC-centric**. The long-term goal is a small,
immutable, guaranteed runtime on top of which task-specific VFX / AI applications
can be grown — instead of bolting tools onto a host DCC.

## What OpenStrata is not

It is an orchestration and certification layer that integrates proven primitives.
It is **not** a new package manager, build system, container runtime, DCC, render
farm, or GPU driver manager. It reuses CMake/Ninja (build execution), `uv` (Python
deps), Jenkins (CI orchestration), OCI (transport), and Git (workspace history).

## Core principles

1. **The `ost` CLI is light; the runtime is heavy.** The single Rust binary has no
   Python, USD, or VFX-library dependency. Weight lives in managed runtime artifacts.
2. **Workflows are portable; artifacts are explicit.** User commands stay
   OS-agnostic, but every lockfile and diagnostic records the concrete variant,
   e.g. `linux-x86_64-glibc228-py313`.
3. **Controlled extensibility, not arbitrary dependency freedom.** Tier 0 = VFX
   Reference Platform core; Tier 1 = Strata certified extensions (OpenUSD,
   MaterialX, …); Tier 2 = app-local deps managed by `uv`.
4. **Runtime contract and validation are first-class.** "Installed" is not enough;
   a certified runtime carries a feature set, capability graph, validation result,
   and digest.
5. **Resolve from capability, not package name.** A project requests
   `usd-materialx`, not `materialx`; the resolver derives packages, extensions,
   environment, and validation from capabilities.

## Relationship to other projects

OpenStrata's artifact strategy is intentionally aligned with external
artifact-forge tooling: immutable, versioned artifacts identified by digest, each
with a manifest and validation report, with generated binaries kept out of Git and
only small manifest/lock records tracked. OpenStrata can consume such artifacts as
runtime/extension inputs; it does not duplicate a forge's build pipelines.

## First vertical slice

The first meaningful end-to-end demonstration (§21 of the design):

```bash
ost platform show cy2026                                  # inspect the target
ost runtime pull cy2026 --profile usd --from-usd /opt/usd # a real USD runtime
ost runtime validate cy2026 --profile usd                 # usdcat + pxr present
ost plugin new usd-fileformat toy --extension toy         # scaffold a bundle
ost plugin build toy --target cy2026 --profile usd        # build the .so
ost plugin test  toy --target cy2026 --profile usd        # L0..L5 + report
```

Success = the `toy` plugin is discovered by OpenUSD (L2), a `.toy` fixture is read
by `usdcat` (L3) and opens as a `UsdStage` (L4), and the run's `report.json`
records the runtime source, OpenUSD version, and each level's result.
`ost plugin package` (an immutable artifact + manifest) follows with Phase 6.
