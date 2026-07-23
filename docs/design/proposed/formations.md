# OpenStrata Formations — cross-repository composition (direction)

> Status: **implemented** for the v0.19.0 `resolve|inspect|lock|run` MVP, with
> `env|doctor` implemented on the v0.20.0 branch. First-party cross-repository
> dogfood remains an acceptance
> task — see the
> [roadmap backlog](../../roadmap/backlog.md) for the milestone entry.

OpenStrata already builds, validates, and distributes individual components:
runtimes, plugin bundles, plugin-workspace products, and renderers. A
**Formation** is the next step — a resolved, reproducible set of those components
assembled for one command or execution purpose. It answers a question the
existing verbs cannot: *given several independently released OpenUSD components,
which exact identities compose into one working environment, and how is a process
launched inside it?*

The motivating case is cross-repository. Today
[`animu-sphere/usd-3dgs-plugins`](https://github.com/animu-sphere/usd-3dgs-plugins)
and [`animu-sphere/usd-vrm-plugins`](https://github.com/animu-sphere/usd-vrm-plugins)
exercise two different plugin-workspace shapes, while
[`animu-sphere/hydra-merlin`](https://github.com/animu-sphere/hydra-merlin)
exercises the renderer boundary. The stronger story is that each independently
released component resolves against a compatible runtime: Gaussian PLY can be
inspected through its packaged file-format bundle and ordinary-library closure,
and the VRM file-format, schema, and resolver bundles can compose with hdMerlin
into a single Vulkan viewport session. Those planned workflows are written up
in [projects/combined-formations.md](../../projects/combined-formations.md); this
document defines the model beneath them.

Formation stays inside OpenStrata's stated boundary: it is orchestration over
existing primitives, not a new package manager or solver (see
[concepts/overview.md](../../concepts/overview.md), "What OpenStrata is not").

## Terminology

A Formation moves through explicit states. Each has a distinct name so
documentation, CLI output, and errors never blur intent with result:

| Term | Meaning |
| --- | --- |
| **Declared Formation** | Human-authored intent: a `formation.toml` naming a runtime target, the components wanted, and the command to launch. |
| **Resolved Formation** | Selected component identities plus their dependency closure and composed environment — deterministic, inspectable, not yet pinned. |
| **Formation Lock** | The immutable, digest-pinned result of resolution (`formation.lock`); reproducible on another machine. |
| **Formation Run** | One execution instance launched from a resolved or locked Formation. |
| **Formation Evidence** | The recorded result and provenance of a run: which runtime, bundles, renderer, and executable were actually used, and the run's outcome. |

A Formation is **not** an environment-variable list. It represents component
identity, version and digest, target and profile compatibility, dependency
closure, environment contributions, conflict detection, a launch command, lock
state, and execution evidence — the same disciplines the runtime, artifact, and
plugin models already enforce, extended across component kinds and repositories.

## Declared components

A component is a typed reference to something OpenStrata already understands:

| Kind | Resolves to | Existing model reused |
| --- | --- | --- |
| `runtime` | one OpenStrata runtime identity (target + profile + digest) | `ost-runtime`, artifact registry |
| `plugin` | a plugin bundle or workspace product | `ost-plugin`, plugin manifest / `dependencies.json` |
| `renderer` | a built/adopted renderer with its evidence overlay | `ost-plugin` renderer model, renderer evidence |
| `tool` | an executable entry point within a resolved component | component manifest |
| `scene` / input | an asset reference passed to the launched command | — (opaque to resolution) |

Runtime and components are declared by full `sha256:<64-hex>` artifact identity,
plus a stable component id and kind. Tags, digest prefixes, repositories, and
source-tree paths are rejected. A Formation selects exactly one runtime; plugin
and renderer components are resolved against it.

## Resolution

Resolution turns a Declared Formation into a Resolved Formation without launching
anything:

1. **Select the runtime** named by `[runtime]` (target + profile), from a pulled
   store entry or a pinned artifact.
2. **Resolve each component** to a concrete identity and read its dependency
   metadata (`dependencies.json` records resolved `bundles` and `libraries`; the
   renderer overlay records its producer session).
3. **Close the dependency graph** — a plugin's declared `requires.bundles` pull
   their providers in, the way `ost plugin test --workspace` already validates a
   source workspace's graph.
4. **Check compatibility** across the closure (next section).
5. **Compose the environment** and detect conflicts.

Resolution is deterministic: the same declared inputs and store state produce the
same resolved model, printable with `--json` for diffing and CI. Every failure
identifies the incompatible component or the missing capability by id — never a
bare non-zero exit.

## Compatibility checks

Before any process launches, a Formation verifies — where the information is
known — that the closure is mutually compatible:

- **target** (VFX Reference Platform calendar year) and **architecture**;
- **OpenUSD version** the runtime provides versus what each bundle/renderer was
  built against;
- **compiler / CRT** identity;
- **Python ABI**;
- **profile** (a `core` component must not silently require a `usd` capability).

These are the same identity axes the runtime, plugin, and renderer models already
record. Formation does not invent new compatibility data; it reads existing
component metadata and refuses a combination whose recorded identities disagree,
with a coded, actionable error.

## Environment composition

A Resolved Formation composes one launch environment from each component's
declared contributions — plugin discovery paths (`PXR_PLUGINPATH_NAME`), loader
paths, runtime `EnvSet`, tool `PATH` entries — reusing the runtime `EnvSet`
machinery so there is **no hidden environment mutation** (a standing quality
bar). The user's ambient `PYTHONPATH` / `PXR_PLUGINPATH_NAME` is not inherited
unless explicitly declared.

When two components contribute conflicting values for the same variable, that is
a **detected conflict**: resolution reports it (and, for order-sensitive path
variables, the composed order) rather than letting last-writer-wins hide it.
`ost formation env --json` makes the whole composed environment inspectable.

## Process launch

`ost formation run` launches the Formation's `[command]` as a **foreground**
process inside the composed environment and propagates its exit code. The v0.19.0
target is foreground execution only; detached, long-running, and forkable
instances belong to the later Session layer (see below). Launch never mutates the
components it composes — a Formation Run reads immutable identities and writes
only its own evidence.

## Lock files

`ost formation lock` writes `formation.lock`: the immutable, digest-pinned
Resolved Formation. A lock records each component's resolved digest, the runtime
identity, the composed environment contract, and the closure — enough to
reproduce the same resolution on another machine without re-deriving selections.
This mirrors `strata.lock` for a single build target, extended to a
multi-component, potentially multi-repository set.

## Execution evidence

A Formation Run records **Formation Evidence**: the runtime id/digest, each
resolved bundle and renderer identity, the launched executable, the composed
environment fingerprint, and the run's outcome. Evidence reuses the artifact and
renderer evidence disciplines corrected in v0.18.0 — a result is bound to the
exact immutable identities that produced it, so "it worked" always names *what*
worked.

## What a Formation is not

Formation is a thin composition layer over models that already exist; the
boundaries matter:

- **Not a plugin workspace.** A [plugin workspace](../../reference/plugin-workspace.md)
  is one repository's set of co-developed, source-tree bundles validated in
  dependency order. A Formation composes *already-built, digest-pinned*
  components that may come from **different repositories and releases**, and adds
  a runtime and a launch command. Workspace graph validation is an input to
  Formation resolution, not the same thing.
- **Not a runtime installation.** `ost runtime pull` materializes one runtime. A
  Formation *selects* one runtime and composes plugins, a renderer, and a command
  around it. Runtime installation is a dependency of a Formation, not a substitute
  for one.
- **Not a generic shell environment.** `ost env` / `ost devshell` print or enter
  an activating environment for one runtime/profile. A Formation resolves a
  digest-pinned multi-component closure, checks its compatibility, records a lock
  and run evidence, and launches a specific command. A shell environment is
  ambient and unpinned; a Formation is a reproducible, inspectable identity set.

## Relationship to Sessions

Formation and the future [Sessions / sandbox phase](../../roadmap/backlog.md)
are distinct layers and must stay that way:

```text
Formation
    ↓ resolves components, composes environment, launches a command
Session
    ↓ may fork, diff, discard, or promote the running instance over time
```

A **Formation** describes *what components are composed and how a command is
launched*. A **Session** describes *a mutable or isolated working instance over
time*. The Session phase is not renamed to Formation; it builds mutable workspace
and sandbox behavior on top of a resolved Formation.

## CLI namespace

```text
ost formation resolve [path]            # Declared -> Resolved, no launch
ost formation inspect [path]            # show the resolved model
ost formation run     [path] -- [cmd…]  # launch the command, foreground
ost formation lock    [path]            # write formation.lock (digest-pinned)
ost formation env     [path]            # export retained composed environment
ost formation doctor  [path]            # diagnose lock/env/command readiness
```

Every subcommand accepts `--json` and emits the shipped
`{ok, schema, data, warnings}` envelope with category exit codes
([reference/json-output.md](../../reference/json-output.md)) — not a bespoke
numeric scheme. A persisted `formation.lock` carries its own schema identifier,
distinct from the envelope `schema`.

A shorthand such as `ost formation formation.toml` aliasing
`ost formation run formation.toml` is deliberately **deferred** until explicit
subcommand behavior is stable, so the shorthand never becomes the contract.

## Manifest examples

The shipped v0.19.0 development schema is strict and digest-pinned. Repeated
digits below are placeholders for complete artifact digests.

VRM inspection (VRM bundles + `usdview`):

```toml
schema = "openstrata.formation/v1alpha1"

[formation]
name = "vrm-inspection"

[runtime]
artifact = "sha256:1111111111111111111111111111111111111111111111111111111111111111"

[[components]]
id = "usd-vrm-product"
kind = "plugin"
artifact = "sha256:2222222222222222222222222222222222222222222222222222222222222222"

[command]
program = "usdview"
args = ["avatar.vrm"]
```

VRM rendered by hdMerlin (VRM bundles + renderer):

```toml
schema = "openstrata.formation/v1alpha1"

[formation]
name = "vrm-merlin"

[runtime]
artifact = "sha256:1111111111111111111111111111111111111111111111111111111111111111"

[[components]]
id = "usd-vrm-product"
kind = "plugin"
artifact = "sha256:2222222222222222222222222222222222222222222222222222222222222222"

[[components]]
id = "hdMerlin"
kind = "renderer"
artifact = "sha256:3333333333333333333333333333333333333333333333333333333333333333"

[command]
program = "usdview"
args = ["avatar.vrm"]
```

## Minimum viable Formation (v0.19.0)

The first implementation must:

- parse a versioned Formation manifest;
- select one runtime;
- resolve plugin and renderer components and read their dependency metadata;
- verify target, architecture, OpenUSD, compiler/CRT, and Python-ABI
  compatibility where known;
- compose plugin discovery and loader paths;
- identify conflicting environment contributions;
- print a deterministic resolved model with `--json`;
- launch a foreground process;
- generate `formation.lock`;
- record Formation Run evidence;
- provide actionable, coded errors that name the incompatible or missing
  component.

### Required first-party dogfood

Acceptance is proven by real cross-repository projects, not fixtures:

1. a Formation using only `usd-3dgs-plugins`;
2. a Formation using only `usd-vrm-plugins`;
3. a Formation using only `hydra-merlin`;
4. a Formation using `usd-vrm-plugins` and `hydra-merlin` together;
5. execution from digest-pinned packaged artifacts, **not** source-tree paths;
6. a clean-machine or isolated-prefix run;
7. evidence showing exactly which runtime, bundles, renderer, and executable were
   used.

## Non-goals (v0.19.0)

Deferred, to keep the first Formation model small and honest:

- DCC discovery (that is the [DCC host milestone](dcc-hosts.md), which will
  *consume* Formation, not fork it);
- Kubernetes execution ([kubernetes.md](kubernetes.md));
- Linux namespace / overlayfs sandboxing;
- detached or long-running session management (the Session layer);
- general-purpose package solving;
- automatic publication of Formation bundles;
- a GUI Formation editor;
- implicit download from arbitrary untrusted sources.

## Reuse principles

- Formation composes **immutable identities** before launching processes.
- It **reuses** the runtime, artifact, plugin, renderer, target, and evidence
  models — it does not fork them or introduce a parallel composition mechanism.
- All resolved identities are inspectable and digest-addressed where artifacts
  exist.
- Every failure identifies the incompatible component or missing capability.
- No hidden environment mutation; the composed environment is fully inspectable.
- DCC host integration builds **on** Formation; Sessions and sandboxing are a
  later layer **above** it.

## Implementation order (v0.19.0)

1. Finalize Formation terminology and the versioned manifest schema.
2. Add an `ost-formation` crate (or an equivalent isolated domain module),
   mirroring `ost-plugin` / `ost-ci`.
3. Implement manifest parsing and deterministic resolution.
4. Reuse the artifact, runtime, plugin, renderer, target, and evidence contracts.
5. Implement environment composition and conflict detection.
6. Implement `resolve`, `inspect`, `env`, and `doctor`.
7. Implement foreground `run`.
8. Implement `lock` and run evidence.
9. Dogfood `usd-3dgs-plugins`, `usd-vrm-plugins`, and `hydra-merlin`, then the
   combined VRM + hdMerlin Formation.
10. Publish an acceptance report before declaring the milestone shipped.

## Positioning

> OpenStrata builds, validates, distributes, and composes independently developed
> OpenUSD components into reproducible Formations. A Formation resolves immutable
> component identities across repositories, checks their compatibility, composes
> one inspectable environment, and launches a command with recorded evidence —
> reusing the runtime, artifact, plugin, and renderer models rather than
> replacing them.
