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

OpenStrata's artifact strategy is intentionally aligned with artifact-forge tooling
(e.g. `vitrakiln`): immutable, versioned artifacts identified by digest, each with a
manifest and validation report, with generated binaries kept out of Git and only
small manifest/lock records tracked. OpenStrata can consume such artifacts as
runtime/extension inputs; it does not duplicate the forge's build pipelines.

## First vertical slice

The first meaningful end-to-end demonstration (§21 of the design):

```bash
ost platform show cy2026                          # inspect the target
ost runtime pull cy2026 --profile usd             # certified USD runtime
ost devshell cy2026 --profile usd                 # enter it
ost plugin new usd-fileformat toy-cache --extension toy
ost plugin build  --target cy2026
ost plugin validate --target cy2026               # discovery + open a fixture
ost doctor usd                                    # diagnose the USD environment
ost plugin package --target cy2026                # immutable artifact + manifest
```

Success = the `toy` extension is discovered by OpenUSD, a `.toy` fixture opens as
an `SdfLayer` and `UsdStage`, and the artifact manifest records target, runtime
digest, OpenUSD features, and validation result.
