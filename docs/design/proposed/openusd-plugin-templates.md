# OpenUSD plugin template policy

> Status: proposed. This reconciles the 2026-07-12 OpenUSD plugin template
> report with the OpenStrata implementation that already ships embedded
> scaffolds, `ost plugin new`, plugin bundle manifests, verification levels,
> reports, packaging, and artifact transport.

## Outcome

OpenStrata should grow its existing template catalog rather than create a second
template product. A scaffold is valuable only when the generated result can use
the normal OST lifecycle:

```text
ost plugin new
  -> inspect / build / doctor / test
  -> package / publish
  -> artifact-backed CI verification
```

Template work standardizes bundle boundaries, registration, installation,
diagnostics, and evidence. It does not standardize domain algorithms.

## Decisions from the report

| Proposal | Decision | OST interpretation |
| --- | --- | --- |
| `reference` / `skeleton` / `template` maturity | adopt | Every catalog entry declares maturity; promotion requires evidence. |
| Common metadata and promotion policy | adopt | Add a versioned descriptor and deterministic generated provenance. |
| Clean generation/build/install/discovery tests | adopt | Map them onto the existing plugin verification and report system. |
| Shared CMake helpers | adopt with constraints | Maintain once in OST, but copy/version them into a scaffold so builds never require an OST source checkout. |
| Separate `ost-templates` repository | defer | Embedded templates currently make the `ost` binary offline and self-contained. Split only when independent distribution is required. |
| New `ost-new` and rendering engine | reject | Extend `ost plugin new` and `ost init`; do not create a competing CLI or require Python/Jinja. |
| One mandatory directory layout for every template | modify | Require outcomes and contracts; codeless schemas, compiled plugins, and tools legitimately have different trees. |
| Package resolver is immediately a formal template | reject for now | Its security boundary is promising, but a second independent package format is still required. Start as a skeleton. |
| Exec and Hydra remain skeletons | adopt | Their architecture choices are not stable enough for formal templates. |
| `generated_at` in committed metadata | reject | Wall-clock data breaks deterministic generation. Put timestamps in generation reports only. |
| Tool package represented as a plugin bundle | reject | Tools use project/package scaffolding unless they also contain an OpenUSD plugin. |

## Vocabulary and identity

The word "template" is overloaded. In this policy a **catalog entry** is any
scaffold OST knows about. Its `maturity` is one of:

- **reference** — dogfood implementation and gap report; not exposed as a stable
  scaffold command;
- **skeleton** — registration, build, install, manifest, and test seams are
  reusable, while important architecture remains project-owned;
- **template** — a supported scaffold whose extension points and compatibility
  policy have survived independent use.

Maturity is metadata, not part of the stable id. For example,
`hydra-render-delegate-cpp` can move from `skeleton` to `template` without being
renamed. CLI and documentation must visibly label skeletons as experimental.

Plugin **kind** and scaffold **template id** are different concepts. A kind
defines bundle and verification semantics; multiple implementations can exist:

```text
kind:        usd-schema
template id: usd-schema-codeless | usd-schema-cpp

kind:        usd-fileformat
template id: usd-fileformat-cpp
```

The current `ost plugin new <kind> <name>` continues to choose the default
template. Add `--template <id>` only when a kind has more than one scaffold.

## Repository and command ownership

Template sources remain under this repository's `templates/` directory and are
compiled into the `ost` binary. The existing Rust scaffold implementation stays
the authority for input validation, safe paths, overwrite protection, token
replacement, and file creation.

The command split is:

- `ost plugin new` for self-describing OpenUSD plugin bundles;
- `ost init --template ...` for projects, tools, workspaces, and renderer
  compositions that are not themselves one plugin bundle;
- existing `ost plugin inspect|build|doctor|test|package|publish` for the
  generated plugin lifecycle.

A future `ost template list|show|validate|diff` may expose catalog metadata and
compare a project with a newer scaffold. It should be added only after the
descriptor and provenance contract exist. Silent regeneration is forbidden.

Reference projects do not need to be copied into the OpenStrata repository. A
catalog descriptor may record their repository, revision, report digest, and
validated OST/OpenUSD versions. This avoids vendoring product code or blurring
its license and ownership.

## Template descriptor and generated provenance

Each template source should contain a `template.yaml`, validated by a versioned
schema. It describes the scaffold, not the generated plugin:

```yaml
schema: openstrata.template/v1alpha1
template:
  id: usd-fileformat-cpp
  version: 1.0.0
  maturity: template
  artifact_kind: plugin
plugin_kind: usd-fileformat
variables:
  - name: name
    type: portable-identifier
    required: true
  - name: extension
    type: file-extension
    required: true
outputs:
  required: [manifest, cmake-project, plugin-resources, tests]
verification:
  levels: [0, 1, 2, 3, 4, 5]
compatibility:
  openusd: ">=25.05,<27.0"
```

The descriptor must declare:

- id, semantic version, maturity, artifact kind, and plugin kind when relevant;
- typed variables, defaults, validation, and which variables may affect paths;
- generated files and conditional features;
- required verification assertions;
- supported OST/OpenUSD ranges and platform claims;
- license/notice behavior and reference evidence.

Do not add a general Jinja-compatible language in the first iteration. Extend
the typed Rust renderer with the minimum required tokens and conditions. Every
rendered path is revalidated after substitution, duplicate destinations are an
error, and unresolved known template tokens fail generation.

Every generated scaffold should commit deterministic provenance, conceptually:

```yaml
schema: openstrata.scaffold/v1alpha1
template:
  id: usd-fileformat-cpp
  version: 1.0.0
generator:
  name: ost
  version: 0.14.0
inputs:
  name: sample
  extension: sample
```

The final filename and whether this is embedded into an existing domain manifest
should be decided with the schema implementation. The invariant matters more:
template id/version, generator version, and normalized non-secret inputs are
committed; generation time, user paths, hostnames, and environment values are
not. A timestamp belongs in the uncommitted/machine-readable generation report.

`openstrata.plugin.yaml` remains the generated bundle contract. Template
metadata must not duplicate its runtime requirements, capabilities, artifact
paths, or tests as a second source of truth.

## Generated bundle requirements

All formal plugin templates and skeletons must generate:

- a parseable `openstrata.plugin.yaml` using an existing or explicitly proposed
  plugin kind;
- a buildable, out-of-source CMake project;
- correct build-tree and install-tree plugin resources;
- explicit install rules and runtime dependency declarations;
- a README describing extension points, compatibility, and verification;
- source/license identifiers and third-party notice hooks;
- fixtures and registration tests appropriate to the plugin kind;
- no unresolved template token, personal namespace, secret, absolute asset path,
  or unexplained ABI assumption.

`include/`, `src/`, generated schema resources, and shared libraries are required
only when the plugin kind needs them. A codeless schema must not grow dummy C++
just to match a universal layout.

A generated project is not accepted because it works from its build directory.
Its install-tree test must run without the source or build tree in plugin,
library, Python, shader, or asset paths.

## Shared CMake policy

Common CMake logic is appropriate for:

- OpenUSD imported-target discovery and configuration/ABI checks;
- plugin resource and configured `plugInfo.json` placement;
- build-tree/install-tree test environments;
- schema generation;
- Windows DLL and Unix loader/RPATH handling;
- ABI and artifact metadata;
- registration test declaration.

The canonical helpers may live under `templates/_shared/cmake/`, but generation
copies the pinned helper files into the new bundle. Generated CMake must be
usable with plain CMake and the selected OpenUSD SDK; it must not locate the
OpenStrata repository, invoke an unpinned remote download, or require `ost` at
build time.

Helpers should encode mechanics, not hide policy. A template must still make its
OpenUSD components, targets, resources, install destinations, and tests readable
from its top-level CMake files.

## Verification contract

Template CI validates the template itself by generating representative bundles
in temporary clean rooms. Generated bundle tests then use the normal plugin
pyramid.

### Catalog generation gates

- minimal and all-feature generation;
- invalid identifiers/options rejected before writes;
- path traversal, absolute output, and duplicate output rejected;
- non-empty destination protected;
- no unresolved known tokens;
- byte-identical output from identical inputs;
- descriptor outputs agree with actual generated files.

### Bundle lifecycle gates

- Level 0: manifest, paths, resources, fixtures, licenses, and library shape;
- Level 1: target, OpenUSD range, C++/Python ABI, configuration, and components;
- Level 2: install-tree discovery and actual module/type creation where possible;
- Levels 3+: kind-specific semantic tests;
- package/security tests for archive and resolver boundaries;
- machine-readable `PASS` / `FAIL` / `SKIP` with observed facts and artifacts.

Compatibility is a declared support matrix, not an unconditional promise that
every template runs on every host. A formal template needs two supported
OpenUSD lines and every OS/configuration it claims. Missing infrastructure is a
reported `SKIP`; it cannot be counted as promotion evidence. Linux remains the
first-class OST target, while Windows multi-config/CRT behavior must be tested
before Windows support is claimed. macOS is added when a maintained lane exists.

## Domain boundaries

### Schema

Common:

- CMake/`usdGenSchema`, generated resource placement, registration, API apply,
  fallback and token tests, manifest capabilities.

Project-owned:

- actual properties/tokens, prim contracts, schema evolution, and migration.

Keep `usd-schema-codeless` and future `usd-schema-cpp` as different template ids.
The shipped co-hosted compiled-schema workflow is evidence for the latter, but
does not automatically prove a standalone compiled-schema scaffold.

### File format

Common:

- registration, extension metadata, `CanRead`, read/write seams, diagnostics,
  malformed fixtures, deterministic normalization, and install layout.

Project-owned:

- parser, canonical model, stage mapping, materials, and resolver policy.

Read support is the minimum formal contract. Write and streaming support are
optional features whose tests become mandatory when selected.

### Asset resolver

Common skeleton candidates:

- registration, resolver context, asset/cache interfaces, path-normalization
  seam, diagnostics, test doubles, install layout, and thread-safety harness.

Project-owned:

- protocol, authentication, retry, path grammar, cache invalidation, and remote
  trust policy.

The manifest already models `usd-asset-resolver`; the next step is a skeleton
and kind-specific discovery checks, not a new CLI or artifact type.

### Package resolver

A package resolver skeleton may additionally provide bounded random access, an
entry lookup interface, and security fixtures. It must not standardize a GLB,
ZIP, encrypted-container, or MIME model.

Before promotion, tests must cover traversal and absolute inner paths, NUL and
invalid encoding where the API accepts raw input, integer/offset/size overflow,
declared/actual size mismatch, duplicate entries, oversized reads, corrupt
containers, and decompression limits when compression exists.

This remains a skeleton until at least two independent package backends prove
the boundary. Architectural similarity alone is not promotion evidence.

### Exec

Only registration, discovery, tokens, diagnostics, deterministic test seams,
and a minimal evaluation-context boundary are skeleton candidates. Scheduling,
graph construction, state ownership, invalidation semantics, solvers, mutation,
and CPU/GPU execution remain reference policy.

### Hydra and renderer

Scene index, imaging adapter, render delegate, and shader integration are
different plugin kinds/capabilities even if one reference repository contains
several of them. Do not publish one ambiguous `hydra-plugin` formal template.

The renderer-project and Hydra render-delegate skeleton are specified separately
in [renderer-templates.md](renderer-templates.md). They reuse this catalog,
provenance, CMake, install, promotion, and verification policy.

### Tools

An OpenUSD CLI, validator, converter, fixture generator, or package inspector is
a project/package template. It uses `ost init`, `ost build`, `ost package`, and
artifact verification. It uses `openstrata.plugin.yaml` and `ost plugin ...`
only if it actually ships a discoverable OpenUSD plugin.

## Current and target catalog

| Catalog entry | Current evidence | Policy state |
| --- | --- | --- |
| `usd-fileformat-cpp` | shipped scaffold and L0-L5 lifecycle | template |
| `usd-schema-codeless` | shipped scaffold, generation, registration lifecycle | template |
| `usd-schema-cpp` | co-hosted compiled-schema path exists | skeleton candidate |
| `usd-asset-resolver-cpp` | manifest kind exists; no scaffold | skeleton candidate |
| `usd-package-resolver-cpp` | one format/reference report | reference, then skeleton |
| `usd-exec-cpp` | architecture/reference reports | reference, then skeleton |
| `hydra-render-delegate-cpp` | hdMerlin dogfood evidence | skeleton candidate |
| `renderer-project` | hdMerlin end-to-end structure | skeleton candidate |
| `usd-tool-cpp` | normal project/package substrate exists | project-template candidate |

This table describes policy maturity, not release status. A candidate is not
available until its descriptor, source scaffold, tests, and CLI wiring land.

## Promotion and deprecation

### Reference to skeleton

Required:

- at least one real project and a reproducible dogfood report;
- framework/domain separation visible in targets and source ownership;
- stable registration and install layout;
- common clean-room test harness;
- architecture choices and non-goals documented;
- no project-specific names, paths, assets, or licenses in scaffold output.

### Skeleton to template

Required:

- two independent projects or deliberately different implementations;
- the same extension seams work without project-specific branches in shared
  helpers;
- stable manifest capabilities, install layout, and error/report ids;
- security review appropriate to the input and package boundary;
- two supported OpenUSD lines and the complete claimed platform matrix;
- versioning, migration, and breaking-change policy;
- evidence that project code is added at documented extension points rather than
  by replacing most scaffold architecture.

A "70% reused" measurement may be supporting evidence, but is not a hard gate:
line counts are easy to game and do not prove that the reused portion is the
correct contract.

### Deprecation

A catalog entry may be deprecated when its OpenUSD extension point disappears,
it encodes unsafe or misleading architecture, it cannot meet current security
requirements, or a more specific compatible entry replaces it. Lack of use is a
maintenance signal, not by itself proof that generated projects are invalid.
Deprecation must state the last compatible OST/OpenUSD versions and migration
path; existing generated source remains project-owned.

## Delivery order

1. Define and validate template descriptor plus deterministic scaffold
   provenance.
2. Describe the two shipped templates and make generation tests descriptor
   driven without changing their output unexpectedly.
3. Extract self-contained shared CMake mechanics and test copied helpers.
4. Add the asset-resolver skeleton and kind-specific registration evidence.
5. Dogfood a package-resolver skeleton against a second package backend before
   promotion.
6. Add standalone compiled-schema and tool-project candidates as their distinct
   lifecycle gaps are proven.
7. Add Exec and Hydra/renderer skeletons; keep their algorithmic/reference code
   outside the formal scaffold.
8. Add an explicit template diff/gap report before any migration feature.

The first implementation slice should improve metadata and verification around
the templates already shipped. It should not bootstrap a new repository, CLI,
or general-purpose text templating engine.

