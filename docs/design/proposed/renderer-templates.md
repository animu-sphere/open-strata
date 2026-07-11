# Renderer template direction

> Status: proposed skeleton direction. Based on the hdMerlin dogfooding report
> dated 2026-07-11. This is the renderer specialization of the common
> [OpenUSD plugin template policy](openusd-plugin-templates.md). The first
> objective is to preserve proven boundaries and executable evidence, not to
> standardize a renderer implementation from a single example.

## Decision summary

OpenStrata should provide one user-facing `renderer` project scaffold assembled
from small source packs. Its initial maturity is **skeleton**, even though
`--template renderer` remains natural CLI wording. It should not provide a
monolithic renderer framework or five unrelated top-level templates.

The initial composition is:

```text
renderer project
├─ core                     required, host/API/backend neutral
├─ extraction               required extension point, project-owned policy
├─ backend-vulkan           default backend pack
├─ validation               required bring-up and evidence pack
└─ adapter-hydra2           optional OpenUSD plugin pack
   └─ DCC integration       later, outside the renderer template MVP
```

`core`, `backend-vulkan`, `validation`, and `adapter-hydra2` are internal
composition units. The initial CLI remains a project scaffold:

```text
ost init --template renderer --name sample-renderer
```

Backend and adapter selection may become explicit flags after the committed
renderer manifest is defined. A separate `ost renderer` command is not justified
until renderer projects need a lifecycle distinct from `ost init`, `ost build`,
`ost package`, and `ost validate`.

## What OST owns

OST owns repeatable project structure, dependency direction, compatibility
checks, install layout helpers, and machine-readable validation evidence. The
generated project owns its rendering decisions and source code.

The first template should establish these invariants:

- Core public headers contain no OpenUSD, Hydra, Vulkan, Qt, window-system, or
  DCC SDK types.
- Scene changes cross the core boundary as typed handles, revisions, classified
  change aspects, and a committed change set.
- Renderer-specific draw extraction is an explicit project-owned target between
  scene records and a backend. OST supplies the seam, not the extraction policy.
- A backend is offscreen-first and reports frame completion explicitly. A window
  and swapchain are adapter concerns, not core requirements.
- Render products carry explicit AOV name, extent, pixel format, origin, color
  space, row pitch, payload size, and completion metadata.
- CPU-readable color and depth products are the portable baseline. Zero-copy
  host interop is an optional capability, never a prerequisite for correctness.
- Hydra translates paths, dirty bits, prim state, active camera, and AOV bindings
  at the adapter boundary. Core state never stores Hydra identifiers or objects.
- Build-tree discovery and install-tree execution are tested separately.
- Unsupported GPU, validation-layer, host, or feature conditions produce an
  explicit `SKIP` reason rather than a false `PASS`.

OST should provide source, CMake, schema, and test helpers for those invariants.
It should not introduce a versioned C++ renderer SDK or promise a stable binary
ABI between template releases in the first iteration. Scaffolded source becomes
project-owned as soon as it is created.

## What stays renderer-owned

The following choices are extension files or placeholders in the scaffold and
must not be encoded as OST policy:

- draw packet construction, sorting, batching, culling, and render graph design;
- material model, MaterialX feature classes, shader generation, and pipeline
  variant selection;
- triangulation, subdivision, fallback material, normals, UVs, and primvar
  interpretation;
- residency, allocation, upload-ring, dirty-range, and asynchronous ownership
  strategies;
- queue topology beyond the minimum graphics queue;
- host-specific presentation and DCC UI behavior.

The template may include a deterministic bootstrap triangle or mesh, but it must
use the same extraction, upload, draw, completion, and render-product path as
normal content. Bootstrap convenience APIs must not become production APIs.

## Promotion rule

This scaffold follows the common `reference -> skeleton -> template` maturity
policy. One renderer implementation is enough evidence to propose a skeleton and
validate a boundary, but not enough to declare a formal template or platform
semantic. Promotion requires a second independent renderer or deliberately
different backend/host implementation, stable evidence ids, the claimed OpenUSD
and platform matrix, and an appropriate security review.

The hdMerlin results justify scaffolding and validation of the target boundaries,
handle/change-set model, render-product metadata, persistent frame completion,
Hydra discovery and basic Sync path, CPU RenderBuffer baseline, and install-tree
host smoke. They do not yet justify common policy for instancing, materials,
format negotiation, upload rings, or zero-copy interop.

## Source and manifest model

The renderer project should commit an `openstrata.renderer.yaml`. This document
is both the composition record and the input to inspect/validate/package; it is
not generated state under `.strata/`. Common deterministic scaffold provenance
is recorded according to the plugin template policy; renderer composition does
not replace it.

An illustrative shape is:

```yaml
schema: openstrata.renderer/v1alpha1
renderer:
  name: sample-renderer
composition:
  backend: vulkan
  scene_inputs: [headless, hydra2]
  extraction_target: sample-render-extraction
render_products:
  required: [color, depth]
frame:
  contexts: 3
  completion: timeline
validation:
  gpu_smoke: true
  validation_messages_are_errors: true
  host_smoke: usdview
openusd:
  api: hydra2
  version: "26.05"
```

The manifest records selected composition and template provenance; it must not
attempt to serialize renderer algorithms. `openstrata.toml` remains the runtime
contract, while `openstrata.renderer.yaml` describes renderer source composition
and validation intent.

The first release is a one-shot scaffold. Re-running it must refuse to overwrite
project files. Later upgrade support should be an explicit diff/migration command
based on the committed manifest, never silent regeneration. Files intended for
custom policy must be visibly separate from mechanical adapter/CMake helpers so
future migrations can avoid rewriting project code.

## OpenUSD and plugin integration

The Hydra adapter is both a renderer adapter and an OpenUSD plugin. It should use
the existing runtime, session, plugin bundle, artifact, and report machinery
instead of creating a parallel renderer package system.

The likely model change is a future `hydra-renderer` plugin kind with renderer
specific test declarations and a `hydra-renderer:<name>` capability. It should
retain the existing plugin pyramid:

- Level 0: bundle, resources, shaders, and relative paths;
- Level 1: OpenUSD version, build configuration, C++ runtime ABI, and target;
- Level 2: plugin discovery and actual delegate creation as separate assertions;
- Level 7: GPU render, RenderBuffer/presentation, first frame, and stable updates.

Level 7 is a group of independently reported checks, not one usdview process exit
code. Initial stable ids should include:

```text
renderer.core.boundary
renderer.backend.capability
renderer.gpu.frame
renderer.render_product.color
renderer.render_product.depth
renderer.plugin.discovery
renderer.delegate.creation
renderer.host.first_frame
renderer.host.stable_update
```

Every check reports `PASS`, `FAIL`, or `SKIP`, the observed fact, relevant device
and runtime identity, and artifact paths. Renderer reports should use the existing
OpenStrata report envelope and deterministic exit categories.

The Hydra pack must also provide helpers for:

- imported-target-derived OpenUSD runtime paths instead of assuming DLLs are in
  `bin` rather than `lib`;
- multi-config plugin/resource/shader placement in both build and install trees;
- discovery without GPU initialization;
- Gf-to-renderer matrix convention conversion with a translation/coverage test;
- CPU color/depth RenderBuffers and explicit separation from optional Hgi/Vulkan
  interop;
- first-frame completion and live-edit evidence beyond process startup.

Hydra 1 and Hydra 2 adapters must be different binary targets. The first OST pack
targets Hydra 2 only; sharing is limited to the host-neutral core contract.

## Validation baseline

The generated project is not considered healthy merely because it compiles. Its
minimum deterministic path is:

```text
RenderWorld commit
  -> project extraction
  -> persistent Vulkan upload/draw
  -> color + depth completion
  -> common render-product validation
```

The initial validation pack should contain:

- core-only build plus forbidden-dependency scan;
- deterministic 64x64 offscreen scene;
- Vulkan capability reporting and validation callback capture;
- repeated unchanged frames proving no redundant scene upload;
- color metadata/payload validation and numeric depth checks;
- a 1,000-frame validation-clean loop;
- build-tree and install-tree layout checks;
- structured vendor, device, driver, API, completion, and skip evidence.

Golden image and diff artifacts belong in the validation pack, but land only
after an image sink with PNG/EXR replacement points exists. Validation messages
must become structured report artifacts rather than remaining stderr-only.

Do not call the CPU RenderBuffer baseline "Tier 0": OpenStrata already uses
levels and tiers for verification and CI/DCC policy. Use explicit capability
names such as `presentation: cpu-readback` and `interop: vulkan-hgi`.

## Delivery slices

### Slice A — renderer scaffold and headless evidence

- Add the renderer manifest schema and parser.
- Add the composed renderer project scaffold with core, extraction seam, Vulkan
  offscreen backend, headless adapter, and validation pack.
- Add core-only, GPU capability, color/depth, persistent-frame, and install-tree
  checks.
- Record template composition and validation results in machine-readable form.

Done when a fresh scaffold can build without OpenUSD, render the deterministic
scene when Vulkan is available, and truthfully skip GPU checks when it is not.

### Slice B — Hydra 2 adapter and Level 7

- Add `hydra-renderer` bundle modeling or an equivalent renderer capability without
  bypassing the existing plugin bundle lifecycle.
- Add ABI/configuration and runtime path helpers.
- Add discovery, delegate creation, basic mesh/camera Sync, color/depth CPU
  RenderBuffer, install-tree usdview first-frame, and stable-update assertions.

Done when discovery, delegate creation, rendering, and host presentation can fail
independently and produce separate evidence.

### Slice C — expand only after dogfooding

- Dogfood instancing, material binding, normal/UV primvars, `primId`/`instanceId`,
  selection, and format negotiation.
- Dogfood device-local upload rings, dirty ranges, async ownership, pipeline
  caches, and device-lost classification.
- Promote only the parts that satisfy the independent-implementation rule.

Zero-copy Vulkan/Hgi interop is an optional later capability. DCC integration and
MaterialX compiler templates remain separate efforts built on the validated
renderer and artifact contracts.

## Explicit non-goals for the first template

- A universal renderer abstraction or render graph.
- A shared OST-owned C++ renderer runtime library.
- A source generator that continuously owns project implementation files.
- Mandatory windowing, Qt, usdview, DCC, or GPU interop in the core path.
- Claiming support from plugin discovery or process startup alone.
- Standardizing unverified material, instancing, allocation, or interop policy.
