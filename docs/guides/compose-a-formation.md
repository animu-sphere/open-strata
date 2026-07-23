# Composing a Formation

> `ost formation resolve|inspect|lock|run` shipped in v0.19.0;
> `env|doctor` are implemented for v0.20.0. The model is defined in
> [design/proposed/formations.md](../design/proposed/formations.md); the
> milestone is in the [roadmap backlog](../roadmap/backlog.md).

A **Formation** composes independently released, digest-pinned components — one
runtime, some plugin bundles, an optional renderer, a command — into one
reproducible execution environment. This guide describes the implemented flow for
authoring and running one.

## 1. Declare the Formation

Author a versioned `formation.toml`: name it, pin one runtime artifact and each
packaged component by its full SHA-256 identity, and give the command to launch.
Tags, digest prefixes, repository names, and source-tree paths are deliberately
not accepted.

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

The machine-readable contract is
[`schemas/formation.schema.json`](../../schemas/formation.schema.json).

## 2. Resolve and inspect (no launch)

Turn the declared intent into a resolved, deterministic model without launching
anything, and inspect it:

```sh
ost formation resolve formation.toml
ost formation inspect formation.toml --json
```

Resolution reads only exact identities already present in the local artifact
store; it never follows a mutable tag or downloads implicitly. Aggregate plugin
products are expanded in their declared install order. The resolver checks
artifact integrity, target/architecture, runtime identity/digest, profile,
capabilities, OpenUSD version, compiler/CRT, and Python ABI where recorded. A
mismatch fails with a coded error naming the incompatible component.

## 3. Inspect the composed environment

The composed environment is fully inspectable — nothing is mutated silently:

```sh
ost formation env formation.toml --shell bash
ost formation env formation.toml --json
ost formation doctor formation.toml
```

`resolve --json` and `inspect --json` include the portable, component-relative
contract. `env` materializes the verified artifacts, retains that
materialization so the exported paths remain usable by the caller, and prints
either evaluable shell or ordered JSON variables. `doctor` checks resolution,
lock freshness, environment conflicts, and command reachability. Duplicate
plugin identities are rejected instead of allowing two ambiguous discovery
roots.

## 4. Lock for reproducibility

Pin every resolved identity to a digest so the same Formation resolves the same
way on another machine:

```sh
ost formation lock formation.toml            # writes formation.lock
```

The lock contains no machine-local materialization paths. `run` refuses a stale
or drifting lock.

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
- The local artifact store must already contain every pinned digest. Pulling from
  a remote remains an explicit `ost artifact pull ... --expect-artifact ...`
  step.
- Worked cross-repository cases (VRM inspection, Gaussian PLY stage inspection,
  hdMerlin view, and VRM rendered by hdMerlin) are in
  [combined-formations.md](../projects/combined-formations.md), built from the
  [USD 3DGS Plugins](../projects/usd-3dgs-plugins.md),
  [USD VRM Plugins](../projects/usd-vrm-plugins.md), and
  [hdMerlin](../projects/hydra-merlin.md) reference projects.
