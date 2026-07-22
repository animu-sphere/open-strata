# Combined Formations (acceptance plan)

> The `ost formation resolve|inspect|lock|run` MVP is implemented on the v0.19.0
> development branch and is not available in v0.18.0. The four real downstream
> runs on this page remain the milestone's cross-repository acceptance work.
> The model is defined in
> [design/proposed/formations.md](../design/proposed/formations.md); the
> milestone is in the [roadmap backlog](../roadmap/backlog.md).

The reference projects — [USD 3DGS Plugins](usd-3dgs-plugins.md) (a file-format
bundle with an ordinary-library dependency),
[USD VRM Plugins](usd-vrm-plugins.md) (a multi-bundle plugin workspace), and
[hdMerlin](hydra-merlin.md) (a renderer) — are released and pinned independently.
A **Formation** resolves such independently released components into one
reproducible, digest-pinned execution environment and launches a command inside
it. This page shows the four conceptual cases that motivate the v0.19.0
Formation milestone.

For each case, "v0.18" describes the shipped source-tree procedure and
"Formation" describes the digest-pinned v0.19 branch procedure awaiting its
recorded downstream run.

## Case 1 — Gaussian PLY stage inspection

Open a Gaussian PLY through `gaussian-ply` and flatten the resulting standard
OpenUSD 26.05 Gaussian schema to USDC. This verifies import and stage inspection;
it does not require or claim a renderer that draws Gaussian splats.

**Today** (v0.18) — run `usdcat` inside the bundle's resolved runtime session:

```sh
ost plugin run plugins/gaussian-ply -- \
  usdcat --flatten --usdFormat usdc --out scene.usd scene.ply
```

**Formation** (v0.19.0 branch) — resolve a packaged `gaussian-ply` component and its
ordinary-library closure, then launch the same tool:

```toml
schema = "openstrata.formation/v1alpha1"

[formation]
name = "gaussian-ply-inspection"

[runtime]
artifact = "sha256:1111111111111111111111111111111111111111111111111111111111111111"

[[components]]
id = "gaussian-ply"
kind = "plugin"
artifact = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

[command]
program = "usdcat"
args = ["--flatten", "--usdFormat", "usdc", "--out", "scene.usd", "scene.ply"]
```

Resolution must enforce the bundle's OpenUSD `>=26.05,<27.0` requirement and
make the packaged `gaussianCore` dependency reachable without a source-workspace
path.

## Case 2 — VRM inspection

Inspect a `.vrm` file in `usdview` using the VRM schema, file-format, and
resolver bundles.

**Today** (v0.18) — compose the bundles into a `usdview` session by hand:

```sh
ost plugin view plugins/usdVrmFileFormat avatar.vrm \
    --with plugins/vrmSchema \
    --with plugins/usdVrmPackageResolver
```

**Formation** (v0.19.0 branch) — declare the aggregate product and let Formation resolve, pin,
and launch:

```toml
schema = "openstrata.formation/v1alpha1"

[formation]
name = "vrm-inspection"

[runtime]
artifact = "sha256:1111111111111111111111111111111111111111111111111111111111111111"

[[components]]
id = "usd-vrm-product"
kind = "plugin"
artifact = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"

[command]
program = "usdview"
args = ["avatar.vrm"]
```

```sh
ost formation lock vrm-inspection.toml
ost formation run vrm-inspection.toml
```

## Case 3 — hdMerlin inspection

Open a scene with the hdMerlin renderer selected, using an OpenUSD runtime and
the renderer.

**Today** (v0.18) — open the built renderer in its matching session:

```sh
ost renderer view scene.usda --profile usd
```

**Formation** (v0.19.0 branch) — declare the runtime and renderer as a Formation:

```toml
schema = "openstrata.formation/v1alpha1"

[formation]
name = "merlin-usdview"

[runtime]
artifact = "sha256:1111111111111111111111111111111111111111111111111111111111111111"

[[components]]
id = "hdMerlin"
kind = "renderer"
artifact = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"

[command]
program = "usdview"
args = ["scene.usda"]
```

## Case 4 — VRM rendered by hdMerlin

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

**Formation acceptance** — one Formation composes plugins from `usd-vrm-plugins` and
a renderer from `hydra-merlin` against one runtime:

```toml
schema = "openstrata.formation/v1alpha1"

[formation]
name = "vrm-merlin"

[runtime]
artifact = "sha256:1111111111111111111111111111111111111111111111111111111111111111"

[[components]]
id = "usd-vrm-product"
kind = "plugin"
artifact = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"

[[components]]
id = "hdMerlin"
kind = "renderer"
artifact = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"

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
| Open or flatten a Gaussian PLY through `gaussian-ply` (`plugin run`) | shipped (v0.18) |
| Open a built renderer in `usdview` (`renderer view`) | shipped (v0.17+) |
| Declarative `formation.toml` and `ost formation run` | implemented on v0.19.0 branch |
| Cross-repository resolution + compatibility checks + `formation.lock` | implemented on v0.19.0 branch |
| VRM-rendered-by-hdMerlin in one command | acceptance run pending |

## Related documentation

- Formation model and CLI:
  [design/proposed/formations.md](../design/proposed/formations.md).
- v0.19.0-oriented procedure:
  [Compose a formation](../guides/compose-a-formation.md).
- The projects: [USD 3DGS Plugins](usd-3dgs-plugins.md),
  [USD VRM Plugins](usd-vrm-plugins.md), and [hdMerlin](hydra-merlin.md).
- Reference Projects overview: [README.md](README.md).
