---
title: Canonical OpenUSD CY2026 runtimes
status: proposed
owners:
  - openstrata-maintainers
created: 2026-08-24
updated: 2026-08-24
applies_to: v0.22.3+
---

# Canonical OpenUSD CY2026 runtimes

## Decision being proposed

OpenStrata should describe, build, verify and publish the canonical OpenUSD
CY2026 runtime set from one normalized, data-driven model. The primary matrix is:

| OpenUSD | Linux x86_64 | Windows x86_64 | macOS arm64 |
| --- | --- | --- | --- |
| 26.08 | `core`, `gl`, `vulkan` | `core`, `gl`, `vulkan` | `core`, `gl`, `metal` |
| 26.05 | `core`, `gl`, `vulkan` | `core`, `gl`, `vulkan` | `core`, `gl`, `metal` |

macOS x86_64 may be published as an explicitly optional cell with `core`, `gl`
and `metal`; it is not part of the primary acceptance matrix.

This policy is the OpenUSD base-runtime input to the broader
[runtime-composition contract](runtime-composition.md). The implementation and
release gates are scheduled in the
[v0.22.x roadmap](../../roadmap/runtime-composition.md); this document owns the
identity, variant, producer, verification and publication decisions that those
roadmap items must preserve.

## Keep profile and graphics variant separate

`profile = usd` identifies the OpenUSD capability bundle. The graphics variant
identifies how that same profile can execute imaging work:

```text
profile = usd
variant = core | gl | vulkan | metal
```

Variants are not new top-level profiles. Human-facing shorthand may combine the
axes, but manifests, compatibility selectors, locks and artifact comparisons
must retain them separately.

`gl` is the canonical replacement for the existing OpenUSD variant value
`standard`. Readers and CLI inputs remain migration-safe:

1. accept `standard` as a legacy input alias;
2. normalize it to `gl` before identity or selector derivation;
3. warn when legacy configuration is consumed;
4. continue reading older artifacts according to their schema version; and
5. remove the alias only in a later schema-breaking release.

`standard` and `gl` must never identify different compatibility cells. The
existing `headless` value is likewise legacy once `core` lands. It may normalize
to `core` only when the recorded capabilities prove the no-imaging semantics;
otherwise a migrated reader records the value as legacy/unknown rather than
inventing compatibility facts.

## Normalized compatibility identity

An OpenUSD compatibility cell records at least:

- VFX Reference Platform year, OS and architecture;
- capability profile and graphics variant;
- exact producer OpenUSD version and the consumer version constraint;
- compiler provider/version, native runtime provider/version and C++ standard;
- Python provider/version and ABI;
- TBB family/provider/version;
- normalized runtime capabilities; and
- platform ABI floors, including an explicit macOS deployment target and SDK
  identity where applicable.

The deterministic compatibility selector is derived from these normalized
facts. It remains distinct from both the OCI digest and a convenience tag:

- the **selector** answers whether two cells are compatible;
- the **digest** is immutable artifact identity; and
- the **tag** is a human-friendly discovery handle.

Mutable tags, local paths and ambient producer state never contribute to the
selector or artifact identity. Schema evolution should be additive where
possible. A migrated reader marks absent modern fields as legacy/unknown instead
of guessing them, and selector derivation remains deterministic across schema
versions.

## Variant semantics

Variants describe runtime capabilities first. Provider-specific build flags are
an implementation of the following contract, not its definition:

| Variant | Imaging | OpenGL | Vulkan | Metal |
| --- | ---: | ---: | ---: | ---: |
| `core` | no | no | no | no |
| `gl` | yes | yes | no | no |
| `vulkan` | yes | yes | yes | no |
| `metal` | yes | platform-dependent/optional | no | yes |

The initial OpenUSD source-build provider translates one normalized cell into a
version-aware build plan containing the source version, platform cell, profile,
variant, dependency providers, generated `build_usd.py` arguments and generated
CMake cache entries. Its intended shape is:

```text
OpenUsdBuildPlan
  source_version
  platform_cell
  profile
  variant
  dependency_providers
  build_arguments
  cmake_cache_entries
```

The mapping starts with these upstream intents:

- `core`: imaging, USD imaging and `usdview` off;
- `gl`: imaging and USD imaging on, OpenGL on, Vulkan and Metal off;
- `vulkan`: imaging and USD imaging on, OpenGL on and Vulkan support on; and
- `metal`: imaging and USD imaging on, Metal support on and Vulkan off.

Exact upstream options are version-aware and validated in code because they can
change between OpenUSD releases. Callers must not assemble compatibility-critical
flags independently of the build-plan layer.

## One matrix producer

The existing Vulkan-specific publisher is an implemented bootstrap, not the
long-term public producer interface. It should be generalized rather than copied
into one script per variant. The target organization is conceptually:

```text
support/
  publish-openusd-runtimes.*
  validate-openusd-runtime.*
  build/
    linux.*
    windows.*
    macos.*
```

One support declaration supplies OpenUSD version, platform, architecture,
variant, publication state and verification policy. Building the upstream
OpenUSD examples is required for every imaging cell (`gl`, `vulkan` and `metal`;
the source-build provider passes the version-appropriate equivalent of
`build_usd.py --examples`). `core` is explicitly exempt because it disables the
imaging surface those examples exercise. An imaging runtime built with examples
disabled is not publishable as canonical. The build-plan layer owns
variant-specific options. CI generation, local iteration and protected release
publication consume the same declaration.

macOS arm64 is a first-class producer, not a Linux-shaped exception. Its lane
captures Apple Clang, Xcode and SDK identity, the deployment target, oneTBB and
Python providers; validates dylib relocation; and declares and verifies Metal
capability. Linux assumptions such as GLX or `DISPLAY` do not leak into macOS
verification. macOS x86_64 remains an optional cell until separately promoted.

## Backend-aware verification

The existing independent verification dimensions remain public evidence:

```text
compile
link
loader
physical_device
render
```

No dimension is inferred from another. A backend-selected verifier implements
loader, device and render probes:

```text
GraphicsVerifier
  NoGraphicsVerifier
  OpenGlVerifier
  VulkanVerifier
  MetalVerifier
```

The normalized behavior is:

- `core`: graphics loader, physical-device and render checks are `not-run`;
- `gl`: probe the platform OpenGL loader/framework, observe a GL context/device
  and render an OpenUSD frame through OpenGL;
- `vulkan`: probe both required OpenGL and Vulkan loaders, enumerate a Vulkan
  physical device and render with `HGI_ENABLE_VULKAN=1`; and
- `metal`: probe the Metal framework, enumerate an `MTLDevice` and render through
  HgiMetal/OpenUSD.

Platform-specific details live below those verifiers: GLX/EGL on Linux according
to declared policy, WGL on Windows, the OpenGL framework/context on macOS, the
native Vulkan loader for Vulkan, and Metal framework APIs for Metal. Missing
host graphics prerequisites remain explicit evidence and follow the release
lane's declared pass/SKIP policy.

## OCI identity and aliases

The canonical repository is:

```text
ghcr.io/animu-sphere/openstrata-runtime-cy2026-usd
```

One formatter produces normalized leaf tags:

```text
<openusd-version>-<variant>-<os>-<arch>
```

For example:

```text
26.08-core-linux-x86_64
26.08-vulkan-windows-x86_64
26.08-metal-macos-arm64
```

The formatter rejects unknown and unnormalized legacy variant names. Production
consumers continue to pin the returned OCI digest rather than the mutable tag.

Convenience aliases such as `26.08-core`, `26.08-gl`, `26.08-vulkan` and
`26.08-metal` require OCI index/manifest-list support. Leaf publication lands
first if index transport is not robust. An alias contains only like-for-like
variant leaves: Vulkan never selects a macOS Metal runtime, Metal contains only
macOS Metal leaves, and platform selection is deterministic.

## Generated CI and release gates

The normalized support declaration generates platform jobs for build,
validation, export, SBOM, provenance, OCI push and clean pull-by-digest consumer
verification. Runner/security differences may split workflows, but not the
matrix source of truth.

A canonical runtime release passes all of these gates:

1. **Build:** clean source, exact upstream revision, expected providers,
   successful compile and link, and — for `gl`, `vulkan` and `metal` — a
   successful build of the upstream OpenUSD examples for that version and
   variant. `core` is exempt from the examples gate.
2. **Runtime:** `ost runtime validate`, exact OpenUSD version, variant capability
   verification, Python import and stage smoke test.
3. **Graphics:** backend-specific loader, device and render evidence according
   to the cell's policy.
4. **Artifact:** normalized manifest, deterministic selector, SBOM, provenance
   and archive integrity.
5. **Distribution:** protected OCI push, clean pull by digest, selector
   re-derivation, required-cell match and trust-policy verification.

Canonical GHCR publication is release-workflow-owned. The workflow has only the
permissions it needs (`contents: read`, `id-token: write`, `packages: write`),
and protected namespace policy authorizes the intended OpenStrata publication
identity. A local convenience push is never authoritative release evidence.

## Migration and cleanup boundary

Once the model and release lanes pass:

1. publish the 26.08 and 26.05 primary cells;
2. move reference projects to digest pins selected from the canonical cells;
3. generate their CI matrices from the shared support declaration;
4. exercise USD plugins against `core`/`gl`, hdMerlin against `vulkan`, and a
   macOS renderer lane against `metal`;
5. remove ad-hoc aliases and superseded runtime packages; and
6. retain only explicitly documented legacy compatibility artifacts.

The cleanup boundary is one runtime model, several producer platforms and no
variant-specific package semantics.

## Non-goals

- Treating `core`, `gl`, `vulkan` or `metal` as unrelated capability profiles.
- Collapsing selector, digest and tag into one identity mechanism.
- Modeling Metal as Vulkan or macOS as a Linux/GLX target.
- Publishing mutable convenience tags as the production consumption contract.
- Adding OCI index aliases before leaf transport and selection are proven.
- Replacing runtime composition or Formation with an OpenUSD-only solver.
