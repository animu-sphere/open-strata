# Combined Formations (reference cases)

> `ost formation resolve|inspect|lock|run` shipped in v0.19.0, followed by
> `formation env|doctor` and downstream closure fixes in v0.20.0. This page
> preserves the four cross-repository cases that shaped that work; it is not an
> active milestone plan. See the [Formation design](../design/proposed/formations.md),
> [v0.19.0](../releases/v0.19.0.md) and
> [v0.20.0](../releases/v0.20.0.md).

Four reference projects are released and pinned independently:
[USD 3DGS Plugins](usd-3dgs-plugins.md) (a file-format bundle with an
ordinary-library dependency),
[USD Point Cloud Plugins](usd-pointcloud-plugins.md) (a multi-format geospatial
workspace), [USD VRM Plugins](usd-vrm-plugins.md) (a multi-bundle plugin
workspace), and [hdMerlin](hydra-merlin.md) (a renderer). This acceptance plan
retains the four original Formation cases built from 3DGS, VRM, and hdMerlin;
the point-cloud workspace is an additional plugin-workspace dogfood, not a
retroactive v0.19.0 acceptance requirement.
A **Formation** resolves such independently released components into one
reproducible, digest-pinned execution environment and launches a command inside
it. This page shows the four conceptual cases that motivated the Formation
contract and remain useful as downstream evidence scenarios.

For each case, "source-tree workflow" preserves the earlier per-project command,
while "digest-pinned Formation" shows the cross-repository form available since
v0.19.0. Placeholder digests remain illustrative and must be replaced by exact
published artifacts before execution.

## Case 1 — Gaussian PLY stage inspection

Open a Gaussian PLY through `gaussian-ply` and flatten the resulting standard
OpenUSD 26.05 Gaussian schema to USDC. This verifies import and stage inspection;
it does not require or claim a renderer that draws Gaussian splats.

**Source-tree workflow** (available since v0.18) — run `usdcat` inside the
bundle's resolved runtime session:

```sh
ost plugin run plugins/gaussian-ply -- \
  usdcat --flatten --usdFormat usdc --out scene.usd scene.ply
```

**Digest-pinned Formation** — resolve a packaged `gaussian-ply` component and
its ordinary-library closure, then launch the same tool:

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

**Source-tree workflow** (available since v0.17) — compose the bundles into a
`usdview` session by hand:

```sh
ost plugin view plugins/usdVrmFileFormat avatar.vrm \
    --with plugins/vrmSchema \
    --with plugins/usdVrmPackageResolver
```

**Digest-pinned Formation** — declare the aggregate product and let Formation
resolve, pin and launch:

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

**Source-tree workflow** (available since v0.17) — open the built renderer in
its matching session:

```sh
ost renderer view scene.usda --profile usd
```

**Digest-pinned Formation** — declare the runtime and renderer as a Formation:

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

This is the strongest combined scenario: a VRM file opened through the VRM
bundles and rendered by hdMerlin in one Vulkan viewport. Formation can express
and launch the graph; a real pass still depends on compatible published VRM,
renderer and runtime artifacts and should retain its own execution evidence.

```text
VRM file
   ↓ usdVrmFileFormat
USD stage
   ↓ vrmSchema and package resolution
Hydra scene
   ↓ hdMerlin
Vulkan viewport
```

**Digest-pinned Formation** — one Formation composes plugins from
`usd-vrm-plugins` and a renderer from `hydra-merlin` against one runtime:

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
case was the original strongest first-party dogfood proposal. v0.19.0 shipped the
generic Formation contract and v0.20.0 closed its package/product/renderer gaps;
a retained run of this exact project pairing remains downstream evidence rather
than an open v0.19.0 roadmap condition.

## Contract status versus project evidence

| Capability | Status |
| --- | --- |
| Compose bundles into a `usdview` session by hand (`plugin view --with`) | shipped (v0.17+) |
| Open or flatten a Gaussian PLY through `gaussian-ply` (`plugin run`) | shipped (v0.18) |
| Open a built renderer in `usdview` (`renderer view`) | shipped (v0.17+) |
| Declarative `formation.toml` and `ost formation run` | shipped (v0.19.0) |
| Cross-repository resolution + compatibility checks + `formation.lock` | shipped (v0.19.0) |
| Formation environment export and diagnostics | shipped (v0.20.0) |
| Self-contained composed-runtime/SDK export and reconstruction | shipped (v0.22.7) |
| Retained real-project VRM-rendered-by-hdMerlin run | downstream evidence not recorded here |

## Related documentation

- Formation model and CLI:
  [design/proposed/formations.md](../design/proposed/formations.md).
- Procedure:
  [Compose a formation](../guides/compose-a-formation.md).
- The projects: [USD 3DGS Plugins](usd-3dgs-plugins.md),
  [USD Point Cloud Plugins](usd-pointcloud-plugins.md),
  [USD VRM Plugins](usd-vrm-plugins.md), and [hdMerlin](hydra-merlin.md).
- Reference Projects overview: [README.md](README.md).
