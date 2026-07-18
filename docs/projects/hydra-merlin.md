# hdMerlin

[`animu-sphere/hydra-merlin`](https://github.com/animu-sphere/hydra-merlin) is
OpenStrata's reference **renderer project**: a host-neutral renderer that is *not*
a plugin workspace, proving OpenStrata manages project types beyond OpenUSD
bundles.

> This page summarizes what the project proves about OpenStrata and links to the
> project for everything else. The repository is authoritative for its renderer
> architecture, feature support, backend details, and release notes. See the
> [cross-repository link policy](README.md#cross-repository-link-policy).

## Overview

hydra-merlin contains a host-neutral scene and rendering core with
backend-neutral render contracts, Vulkan execution, native viewport and headless
tools, an optional Hydra 2 adapter, and renderer benchmarks and evidence
production. hdMerlin is its Hydra render delegate.

## Why it is an OpenStrata reference project

It proves OpenStrata's renderer contracts on a real renderer whose shape is
nothing like a plugin workspace:

- renderer projects that are **not** plugin workspaces;
- preservation of project-owned CMake target boundaries;
- managed CMake configure/build execution with build fingerprints;
- atomic renderer evidence bound to a completed producer session;
- renderer validation and core-versus-Hydra build intents;
- optional GPU capability jobs;
- adoption of digest-pinned OpenUSD runtimes;
- install-tree renderer discovery and managed `usdview` execution;
- renderer-lifecycle dogfooding against real downstream code.

## Renderer architecture boundary

The core public API intentionally avoids OpenUSD, Hydra, DCC, Qt, Vulkan, and
Metal types. Host and Hydra integrations are adapters around the project-owned
renderer architecture.

OpenStrata respects that boundary: it **does not split Merlin's internal CMake
targets into artificial plugin bundles.** It drives the project's own configure
and build, discovers the renderer in the install tree, and records evidence —
without reshaping the renderer into a bundle graph. This is the counterpart to
the [plugin workspace](usd-vrm-plugins.md) case: OpenStrata adopts each project's
architecture instead of forcing one abstraction.

## OpenStrata integration

- **Managed build** — `ost build` runs the project's CMake configure/build under
  a recorded runtime, generator intent, and configuration, and records a build
  fingerprint. External build trees can be validated without OpenStrata claiming
  it configured them.
- **Renderer evidence** — a renderer PASS is bound to a completed producer
  session (id, kind, target, start/completion, outcome); `ost renderer merge`
  refuses assertions from failed, incomplete, or superseded sessions.
- **Runtime adoption** — CI adopts a digest-pinned OpenUSD runtime rather than a
  local install, so the renderer is exercised against a known build.
- **Managed view** — `ost renderer view` opens a scene in the matching `usdview`
  session with the built Hydra renderer selected; `ost renderer viewport` builds
  and launches the standalone native viewport adapter.
- **Capability-aware validation** — GPU/Vulkan checks are separate capability
  jobs, and validation explains SKIPs (no display, no GPU) rather than failing
  blindly.

## Workflows demonstrated

These are **current** commands available today (v0.17 / v0.18); the repository's
guide is authoritative.

Pull a runtime, preflight, build, and validate:

```sh
ost runtime pull cy2026 --profile core
ost build --check
ost build --jobs auto
ost validate --json
```

Open the built Hydra renderer in the matching `usdview` session:

```sh
ost renderer view --profile usd
```

## Dogfooding and evidence

hdMerlin's v0.17.0 renderer-lifecycle pass (downstream report
`2026-07-15-v0.17.0-dogfooding-v0.18.0-asks.md`, findings OST18-RND-001..006)
found a renderer assertion becoming PASS from a CTest that later timed out, and
two concurrent invocations writing the same managed target. Those findings are
half of the [v0.18.0 evidence-integrity milestone](../roadmap/current.md) — one
completed producer behind every renderer PASS, and an exclusive target lease.

The Windows managed-view acceptance is recorded in the
[v0.17.0 managed renderer view report](../reports/2026-07-14-v0.17.0-managed-renderer-view-hydra-merlin.md).
Reports are linked as evidence, not copied; the downstream index is in the
[delivery reports](../reports/README.md).

## Current limitations

- Renderer skeleton promotion and applying the contract to a second independent
  renderer are still ahead (tracked in the
  [roadmap backlog](../roadmap/backlog.md); direction in
  [renderer-templates.md](../design/proposed/renderer-templates.md)).
- Full hosted OS/OpenUSD renderer acceptance is environment-dependent and
  ongoing.
- Composing hdMerlin with plugins from another repository (VRM rendered by
  hdMerlin) is a **planned** Formation workflow, not a current capability — see
  [combined-formations.md](combined-formations.md).
- The authoritative renderer feature/backend support matrix lives in the
  repository, not here.

## Related documentation

- Repository:
  [`animu-sphere/hydra-merlin`](https://github.com/animu-sphere/hydra-merlin)
  (authoritative for renderer architecture, backends, support matrix, and release
  notes).
- Transferable procedure:
  [Adopt a renderer project](../guides/adopt-a-renderer-project.md).
- Renderer direction:
  [renderer-templates.md](../design/proposed/renderer-templates.md).
- Planned cross-repository composition:
  [combined-formations.md](combined-formations.md),
  [Formation design](../design/proposed/formations.md).
- Reference Projects overview: [README.md](README.md).
