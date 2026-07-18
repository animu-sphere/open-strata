# Combined Formations (planned)

> **`ost formation` is planned for v0.19.0 and is not available in v0.18.0.**
> Every workflow on this page is a *planned* cross-repository composition. The
> manifests are illustrative and the schema may change; nothing here ships today.
> The model is defined in
> [design/proposed/formations.md](../design/proposed/formations.md); the
> milestone is in the [roadmap backlog](../roadmap/backlog.md).

The reference projects — [USD VRM Plugins](usd-vrm-plugins.md) (a plugin
workspace) and [hdMerlin](hydra-merlin.md) (a renderer) — are released and pinned
independently. A **Formation** resolves such independently released components
into one reproducible, digest-pinned execution environment and launches a
command inside it. This page shows the three conceptual cases that motivate the
v0.19.0 Formation milestone.

For each case, "today" describes what is possible now with existing commands, and
"planned" describes the declarative Formation equivalent.

## Case 1 — VRM inspection

Inspect a `.vrm` file in `usdview` using the VRM schema, file-format, and
resolver bundles.

**Today** (v0.18) — compose the bundles into a `usdview` session by hand:

```sh
ost plugin view plugins/usdVrmFileFormat avatar.vrm \
    --with plugins/vrmSchema \
    --with plugins/usdVrmPackageResolver
```

**Planned** (v0.19.0) — declare the components and let Formation resolve, pin,
and launch:

```toml
[formation]
name = "vrm-inspection"

[runtime]
target = "cy2026"
profile = "usd"

[[components]]
kind = "plugin"
source = "animu-sphere/usd-vrm-plugins"
id = "vrmSchema"

[[components]]
kind = "plugin"
source = "animu-sphere/usd-vrm-plugins"
id = "usdVrmFileFormat"

[[components]]
kind = "plugin"
source = "animu-sphere/usd-vrm-plugins"
id = "usdVrmPackageResolver"

[command]
program = "usdview"
args = ["avatar.vrm"]
```

```sh
ost formation run vrm-inspection.toml   # planned, v0.19.0
```

## Case 2 — hdMerlin inspection

Open a scene with the hdMerlin renderer selected, using an OpenUSD runtime and
the renderer.

**Today** (v0.18) — open the built renderer in its matching session:

```sh
ost renderer view scene.usda --profile usd
```

**Planned** (v0.19.0) — declare the runtime and renderer as a Formation:

```toml
[formation]
name = "merlin-usdview"

[runtime]
target = "cy2026"
profile = "usd"

[[components]]
kind = "renderer"
source = "animu-sphere/hydra-merlin"
id = "hdMerlin"

[command]
program = "usdview"
args = ["scene.usda"]
```

## Case 3 — VRM rendered by hdMerlin

The case that has **no single-command equivalent today**: a VRM file, opened
through the VRM bundles, rendered by hdMerlin, in one Vulkan viewport.

```text
VRM file
   ↓ usdVrmFileFormat
USD stage
   ↓ vrmSchema and package resolution
Hydra scene
   ↓ hdMerlin
Vulkan viewport
```

**Planned** (v0.19.0) — one Formation composes plugins from `usd-vrm-plugins` and
a renderer from `hydra-merlin` against one runtime:

```toml
[formation]
name = "vrm-merlin"

[runtime]
target = "cy2026"
profile = "usd"

[[components]]
kind = "plugin"
source = "animu-sphere/usd-vrm-plugins"
id = "vrmSchema"

[[components]]
kind = "plugin"
source = "animu-sphere/usd-vrm-plugins"
id = "usdVrmFileFormat"

[[components]]
kind = "plugin"
source = "animu-sphere/usd-vrm-plugins"
id = "usdVrmPackageResolver"

[[components]]
kind = "renderer"
source = "animu-sphere/hydra-merlin"
id = "hdMerlin"

[command]
program = "usdview"
args = ["avatar.vrm"]
```

Resolving this Formation checks that the VRM bundles and hdMerlin agree on the
runtime's OpenUSD version, compiler/CRT, and Python ABI before launch, composes
one conflict-checked environment, pins every component in `formation.lock`, and
records which exact runtime, bundles, renderer, and executable ran. This combined
case is the required first-party dogfood for the v0.19.0 milestone.

## What is planned versus shipped

| Capability | Status |
| --- | --- |
| Compose bundles into a `usdview` session by hand (`plugin view --with`) | shipped (v0.17+) |
| Open a built renderer in `usdview` (`renderer view`) | shipped (v0.17+) |
| Declarative `formation.toml` and `ost formation run` | **planned, v0.19.0** |
| Cross-repository resolution + compatibility checks + `formation.lock` | **planned, v0.19.0** |
| VRM-rendered-by-hdMerlin in one command | **planned, v0.19.0** |

## Related documentation

- Formation model and CLI:
  [design/proposed/formations.md](../design/proposed/formations.md).
- v0.19.0-oriented procedure:
  [Compose a formation](../guides/compose-a-formation.md).
- The projects: [USD VRM Plugins](usd-vrm-plugins.md),
  [hdMerlin](hydra-merlin.md).
- Reference Projects overview: [README.md](README.md).
