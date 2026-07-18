# Composing a formation (v0.19.0-oriented)

> **Design preview, not a current procedure.** `ost formation` is **planned for
> v0.19.0 and is not available today.** This page describes the intended user
> experience so the design has a stable target; the commands here do not run in
> v0.18.0. The model is defined in
> [design/proposed/formations.md](../design/proposed/formations.md); the
> milestone is in the [roadmap backlog](../roadmap/backlog.md).

A **Formation** composes independently released, digest-pinned components — one
runtime, some plugin bundles, an optional renderer, a command — into one
reproducible execution environment. This guide previews the intended flow for
authoring and running one.

## 1. Declare the Formation

Author a `formation.toml`: name it, pick one runtime target and profile, list the
components by `source` and `id`, and give the command to launch.

```toml
[formation]
name = "vrm-inspection"

[runtime]
target = "cy2026"
profile = "usd"

[[components]]
kind = "plugin"
source = "animu-sphere/usd-vrm-plugins"
id = "usdVrmFileFormat"

# … additional [[components]] …

[command]
program = "usdview"
args = ["avatar.vrm"]
```

The schema is illustrative and may change; see the
[Formation design](../design/proposed/formations.md) for the current shape.

## 2. Resolve and inspect (no launch)

Turn the declared intent into a resolved, deterministic model without launching
anything, and inspect it:

```sh
ost formation resolve formation.toml         # planned, v0.19.0
ost formation inspect formation.toml --json
```

Resolution selects the runtime, resolves each component to a concrete digest,
closes the dependency graph, and checks compatibility (target, architecture,
OpenUSD version, compiler/CRT, Python ABI). A mismatch fails with a coded error
that names the incompatible component.

## 3. Check the composed environment

The composed environment is fully inspectable — nothing is mutated silently:

```sh
ost formation env formation.toml --json
```

Conflicting contributions to the same variable are reported (with order, for path
variables), not hidden by last-writer-wins.

## 4. Lock for reproducibility

Pin every resolved identity to a digest so the same Formation resolves the same
way on another machine:

```sh
ost formation lock formation.toml            # writes formation.lock
```

## 5. Run and record evidence

Launch the command as a foreground process inside the composed environment; its
exit code is propagated, and Formation Run evidence records which exact runtime,
bundles, renderer, and executable ran:

```sh
ost formation run formation.toml -- usdview avatar.vrm
```

## Notes

- Every subcommand emits the shipped `{ok, schema, data, warnings}` envelope with
  category exit codes ([reference/json-output.md](../reference/json-output.md)).
- Foreground execution only in v0.19.0. Mutable/forkable instances belong to the
  later Sessions layer.
- Worked cross-repository cases (VRM inspection, hdMerlin view, VRM rendered by
  hdMerlin) are in [combined-formations.md](../projects/combined-formations.md),
  built from the [USD VRM Plugins](../projects/usd-vrm-plugins.md) and
  [hdMerlin](../projects/hydra-merlin.md) reference projects.
