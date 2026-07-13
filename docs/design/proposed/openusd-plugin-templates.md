# OpenUSD plugin template policy

> Status: proposed; delivery steps 1–3 are implemented and steps 4–5 are in
> progress. This reconciles the 2026-07-12 OpenUSD plugin template
> report and the follow-up bundle/workspace implementation proposal with the
> OpenStrata implementation that already ships embedded scaffolds, `ost plugin
> new`, a multi-bundle workspace scaffold, plugin bundle manifests,
> verification levels, reports, packaging, and artifact transport.

## Outcome

OpenStrata should grow its existing template catalog rather than create a second
template product. A scaffold is valuable only when the generated result can use
the normal OST lifecycle, both alone and in a composed workspace:

```text
ost init --template usd-plugin-workspace
  -> ost plugin new
  -> inspect / build / doctor / test / test --workspace
  -> package / publish
  -> artifact-backed CI and clean-install verification
```

Template work standardizes bundle boundaries, registration, installation,
dependency declarations, diagnostics, and evidence. It does not standardize
domain algorithms. A plugin bundle remains the independently buildable,
testable, and publishable unit; a workspace composes bundles during development,
and a product aggregate may compose their artifacts for distribution.

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

## Decisions from the bundle/workspace proposal

| Proposal | Decision | OST interpretation |
| --- | --- | --- |
| Bundle, not repository, is the plugin identity boundary | adopt | One repository may contain several independently buildable and publishable bundles. Repository splitting remains an ownership/release decision. |
| Extend `plugin-workspace` for multiple bundles | adopt | Build on the shipped `usd-plugin-workspace` scaffold and `ost plugin test --workspace`; do not introduce a second workspace product. |
| Declare bundle-to-bundle dependencies in `openstrata.plugin.yaml` | adopt in stages | The versioned extension and read-only graph checks shipped in v0.14.0; v0.15 consumes that graph for deterministic source-workspace session/build composition. Preserve standalone and packaged boundaries. |
| Keep schema, resolver, file format, and Exec in separate bundles | adopt as the composition default | Public/reusable contracts should be separate. Co-hosted schema remains supported for small or legacy bundles and is not silently split. |
| Add a compiled-schema scaffold | adopt as a skeleton | Use the catalog id `usd-schema-cpp`; `usd-schema-compiled` describes the same candidate and must not become a competing id. |
| One `usd-resolver-cpp` template selects asset or package resolver | modify | Keep `usd-asset-resolver-cpp` and `usd-package-resolver-cpp` separate because their interfaces, security tests, and promotion evidence differ. |
| Commit generated schema sources with generation modes | adopt with constraints | Support pinned `AUTO`, `GENERATE`, `PREGENERATED`, and `VERIFY` behavior; CI promotion evidence uses `VERIFY`, and committed generated files carry generator compatibility policy. |
| Shared non-plugin libraries under `libs/` | adopt | They are ordinary CMake packages/libraries, never fake plugin bundles, and cannot contain OpenUSD registration or stage-authoring policy. |
| Product manifest and `ost product ...` | adopt as a future composition layer | Specify aggregate identity, bundle pins, outputs, and clean-install tests before adding commands. Preserve each member bundle's identity and provenance. |
| Generate CMake dependency targets directly from manifests | defer mechanics | The manifest is the dependency source of truth, but plain-CMake standalone builds remain mandatory. Prove target naming/export and installed-package discovery before automating it. |
| Add `ost template`, `ost plugin require`, `ost schema`, and `ost product` commands immediately | defer | First land the descriptors and schemas. New commands must orchestrate existing lifecycle operations rather than create parallel sources of truth. |

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

## Composition model

Repository, workspace, bundle, library, and product are deliberately different
boundaries:

```text
repository
└── OST workspace                     development and shared runtime selection
    ├── plugin bundles                independent plugin identity/lifecycle
    │   ├── schema
    │   ├── asset or package resolver
    │   ├── file format
    │   └── Exec
    ├── libraries                     ordinary link-time dependencies
    ├── integration tests             cross-bundle behavior
    └── product descriptors           distribution-time aggregation
```

A repository may contain one bundle or many. A workspace is not a publishable
plugin and a product is not a larger plugin. Aggregation must not flatten member
manifests, merge several plugin kinds into one identity, or make standalone
bundle builds depend on the workspace source tree.

For a multi-bundle composition, use these dependency directions:

- a public schema bundle owns the authored data contract and has no dependency
  on a file format, resolver, or Exec implementation;
- a file-format bundle may consume a schema bundle and an ordinary parser
  library, but owns parsing and stage mapping;
- a resolver bundle may consume a container/access library, but does not author
  the stage and does not depend on the file-format plugin;
- an Exec bundle consumes authored schema contracts, not importer state;
- shared parsing, buffer access, and format-independent data structures live in
  an ordinary library when two bundles need them;
- a product selects compatible bundle artifacts and tests them together without
  changing their internal dependency direction.

For example:

```text
container library
  ├──> package resolver
  └──> file format ──> schema <── Exec

product = schema + package resolver + file format [+ Exec]
```

This is a composition default, not a migration mandate. The existing co-hosted
schema workflow remains valid when the schema is private to one bundle or when
splitting it would create a release boundary with no independent consumer.
Promotion of a standalone compiled-schema template must nevertheless prove the
separate consumer workflow.

## Workspace contract

The shipped `usd-plugin-workspace` already provides a dual-mode CMake root that
discovers immediate child bundles and `plugins/*`, plus `ost plugin test
--workspace`. Extend that path instead of replacing it. The current discovery
behavior remains the backward-compatible baseline while the dependency schema
is introduced.

A mature workspace composition should provide:

- deterministic discovery of bundle manifests without hand-maintained bundle
  names in the root `CMakeLists.txt`;
- one selected OpenUSD runtime/toolchain contract for the workspace, while each
  bundle remains directly buildable with plain CMake;
- dependency graph validation, missing-dependency diagnostics, version and
  schema-contract checks, and cycle detection before any build;
- workspace integration tests in addition to each bundle's own tests;
- both whole-workspace and selected-bundle workflows;
- dependency-aware runtime sessions and build prefixes derived from the
  validated manifest graph, without repeating the closure in CLI or CI config;
- no relative sibling paths encoded into generated bundle manifests or install
  interfaces.

Auto-discovery alone does not imply dependency order. Before the versioned graph
contract, a workspace could discover and test bundles but could not infer
semantic dependencies from directory names, CMake target names, or link errors.
The shipped graph now permits ordering only from validated `requires.bundles`
edges; those other inference sources remain forbidden.

The graph contract has now landed and survived a real schema/file-format split.
v0.15 consumes that validated graph for source workspaces:

- test/run/doctor sessions include the primary bundle's transitive dependency
  closure automatically; explicit `--with` remains additive for external or
  ad-hoc composition;
- build walks dependencies in deterministic topological order, installs them to
  an OST-owned target-specific prefix, and exposes them through
  `CMAKE_PREFIX_PATH`/normal config-package discovery;
- generated source CI derives the same closure from its existing `bundle:`
  selector and must not introduce a second cell-level dependency list;
- source composition never rewrites a bundle to use `add_subdirectory`, a
  sibling path, or a workspace-only link target.

Packaged support cells are a separate composition boundary: they may compose
multiple bundles only after a product/artifact descriptor pins every member by
digest and can prove clean-install discovery without the workspace tree.

`libs/` is a recommended convention for shared non-plugin code, not a second
plugin catalog. A future `openstrata.library.yaml` may describe exportable
libraries, but ordinary CMake package config and target export remain the
portable integration contract.

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

The committed file is `openstrata.scaffold.yaml`, kept separate from runtime
domain manifests. It records template id/version, generator version, and
normalized non-secret inputs; generation time, user paths, hostnames, and
environment values are excluded. A timestamp belongs only in the
uncommitted/machine-readable generation report. Plugin packaging preserves this
file when present while remaining compatible with adopted legacy bundles.

`openstrata.plugin.yaml` remains the generated bundle contract. Template
metadata must not duplicate its runtime requirements, capabilities, artifact
paths, or tests as a second source of truth.

## Bundle dependency and schema contract

The current plugin manifest already owns plugin identity, the OpenUSD runtime
range, provided capabilities, required runtime capabilities/components,
bundle-relative runtime libraries, plugin resources, schema source, notices,
and tests. Bundle composition extends that document; it does not replace the
current shape with the proposal's illustrative `kind: usd-plugin` form.

The composition model needs two distinct dependency classes:

- **libraries** are link/build dependencies resolved as normal CMake packages or
  exported targets;
- **bundles** are independently discoverable OpenUSD plugin bundles required at
  runtime or for integration testing.

The bundle class is implemented as a versioned additive extension:

```yaml
manifest:
  schema: openstrata.plugin/v1alpha1
plugin:
  name: usdVrmFileFormat
  version: 0.2.0
  kind: usd-fileformat
runtime:
  openusd: ">=25.11,<27.0"
provides:
  - usd-fileformat:vrm
requires:
  capabilities: [usd-stage-read]
  bundles:
    - id: vrmSchema
      version: ">=0.2,<0.3"
      contract: 1
    - id: vrmResolver
      version: ">=0.2,<0.3"
```

`requires.bundles` and `schema.contract` require the explicit manifest schema;
legacy manifests without composition fields remain accepted during migration.
Dependency entries reject unknown keys. `requires.libraries` remains reserved
until a portable library identity/discovery contract exists; inferring it from
CMake target names would make missing-package validation unreliable. A
versioned manifest must reject that reserved key (and every other unknown
`requires:` key) rather than accept a declaration that has no effect.

Semantic package version and authored-data contract version solve different
problems. `version` selects a compatible bundle implementation. A schema
`contract` identifies the stable type/property/token surface used by authored
stages and downstream code. Changing implementation version does not
necessarily change the contract; breaking a contract requires a new baseline,
migration notes, and consumer evidence. Contract identity must be provided by
the schema bundle and checked against every consumer declaration, not merely
copied into consumers.

Graph validation happens before CMake configuration or packaging and reports at
least duplicate ids, missing required bundles/libraries, incompatible versions,
contract mismatch, forbidden dependency direction, and cycles. Optional
dependencies must have explicit behavior and tests; absence cannot silently
change the meaning of an authored schema.

Manifests remain the source of dependency truth. Generated CMake may consume a
validated graph and expose aliases such as `ost::<bundle-id>`, but a bundle must
also support installed-package discovery and standalone plain-CMake use. It may
not rely on `add_subdirectory(../sibling)` or a workspace-only target appearing
by accident.

## Generated bundle requirements

All formal plugin templates and skeletons must generate:

- a parseable `openstrata.plugin.yaml` using an existing or explicitly proposed
  plugin kind;
- one primary plugin-kind boundary; cross-kind composition belongs in a
  workspace/product, while documented legacy co-hosting remains supported;
- a buildable, out-of-source CMake project;
- standalone and workspace build paths with the same public install interface;
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

Installed-consumer fixtures that include OpenUSD headers must carry the same
platform hygiene as plugin targets (`NOMINMAX` on Windows). Generated package
configs must also avoid re-entering `pxrConfig` after the caller has already
resolved `pxr`; guard `find_dependency(pxr CONFIG)` on `pxr_FOUND` so
non-idempotent transitive imported targets such as `TBB::tbb` are not declared
twice.

The canonical helpers may live under `templates/_shared/cmake/`, but generation
copies the pinned helper files into the new bundle. Generated CMake must be
usable with plain CMake and the selected OpenUSD SDK; it must not locate the
OpenStrata repository, invoke an unpinned remote download, or require `ost` at
build time.

Helpers should encode mechanics, not hide policy. A template must still make its
OpenUSD components, targets, resources, install destinations, and tests readable
from its top-level CMake files.

## Product aggregation

A product aggregate is a distribution recipe over already valid bundle
artifacts. It exists so users can install one tested product without forcing
schema, resolver, file format, and Exec into one plugin bundle.

A versioned product descriptor should declare:

- product id and version;
- member bundle ids plus exact or policy-resolved versions/artifact digests;
- supported targets and required OpenUSD runtime range;
- archive/install-tree outputs and release naming;
- cross-bundle smoke tests and fixtures;
- license/notice aggregation rules;
- whether optional members such as Exec are included.

Conceptually:

```yaml
schema: openstrata.product/v1alpha1
product:
  id: usd-vrm
  version: 0.2.0
bundles:
  - id: vrmSchema
  - id: vrmResolver
  - id: usdVrmFileFormat
outputs: [archive, install-tree]
validation:
  clean_install: true
  smoke: [open-vrm-stage]
```

The descriptor references bundle contracts; it does not restate their plugin
resources, runtime libraries, capabilities, or tests. Packaging preserves every
member's manifest, provenance, license, and notices, and emits aggregate
provenance that records the resolved artifact digests.

Product validation must prove that:

- every member is present, compatible, and discoverable;
- the dependency graph and schema contracts are satisfied;
- loader paths work without the workspace, source tree, or build tree;
- install-tree-only integration tests can open the representative assets;
- the packed archive passes the same checks after extraction elsewhere;
- no file collision or ambiguous plugin identity is hidden by aggregation.

Commands such as `ost product build|test|package|install` are reasonable after
this descriptor and report schema exist. They should orchestrate the existing
bundle build/test/package and artifact transport primitives. Until then,
examples using `ost product` are design sketches, not supported CLI.

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

### Composition gates

- deterministic workspace discovery and graph order;
- missing, incompatible, duplicate, and cyclic dependencies rejected before
  configure/build;
- schema contract provider/consumer agreement;
- selected-bundle and whole-workspace tests against one resolved runtime;
- product member manifests and digests preserved;
- archive extraction and clean-install smoke tests with no source/build paths;
- ordinary shared libraries remain usable through exported/install targets and
  do not acquire plugin discovery metadata.

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

Keep `usd-schema-codeless` and `usd-schema-cpp` as different template ids. The
standalone compiled skeleton is selected explicitly with
`ost plugin new usd-schema <name> --template usd-schema-cpp`; codeless remains
the default. Co-hosted compiled-schema evidence does not by itself promote the
standalone skeleton.

The compiled skeleton may commit generated C++ and `generatedSchema.usda` so a
source archive remains buildable without an OST checkout. Its generation policy
must be explicit:

- `AUTO` regenerates with a compatible pinned generator and otherwise uses the
  committed outputs;
- `GENERATE` requires regeneration;
- `PREGENERATED` uses only committed outputs;
- `VERIFY` regenerates in a staging directory and fails on any difference.

Formal promotion requires `VERIFY` in CI, a recorded compatible generator range,
registration and contract baselines, clean-install C++ and Python consumer
tests when bindings are claimed, and a downstream bundle that links only the
installed schema package/target. Generated files are outputs, never a second
hand-edited schema source.

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

The manifest and embedded catalog now model `usd-asset-resolver`; the shipped
skeleton establishes registration, a URI-scheme seam, deterministic provenance,
and starter fixtures. The next step is identifier normalization, cache,
concurrency, another supported platform, and broader clean-install evidence, not
a new CLI or artifact type.

Use the catalog id `usd-asset-resolver-cpp`. A generic `usd-resolver-cpp` id with
a generation switch would make materially different security and verification
contracts look interchangeable.

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

The manifest and embedded catalog now model `usd-package-resolver`; the shipped
skeleton establishes registration, extension dispatch, an entry-lookup seam, a
traversal-rejecting entry-path guard, deterministic provenance, and a smoke
fixture that sublayers a packaged path. Its starter backs entries with a
sidecar directory precisely so no container model is standardized; the real
container, its bounded random access, and the security tests above are the
promotion evidence, not part of the shared template.

The catalog id is the distinct `usd-package-resolver-cpp`. It shares pinned
CMake and test-harness mechanics with the asset-resolver scaffold, but it is
not a mode of the same formal template.

### Exec

Only registration, discovery, tokens, diagnostics, deterministic test seams,
and a minimal evaluation-context boundary are skeleton candidates. Scheduling,
graph construction, state ownership, invalidation semantics, solvers, mutation,
and CPU/GPU execution remain reference policy.

OpenUSD 26.05 stabilized the initial schema-computation extension point around
`Info.Exec.Schemas` and `EXEC_REGISTER_COMPUTATIONS_FOR_SCHEMA`. The embedded
`usd-exec-cpp` skeleton therefore uses the distinct `usd-exec` plugin kind;
`openexec-plugin-cpp` is not a second entry. In a composed product, the Exec
bundle depends on the public schema contract and not on importer or file-format
implementation state.

The shipped skeleton establishes discovery metadata, registration, private
tokens, a deterministic no-input callback seam, copied CMake/install mechanics,
and explicit `requires.bundles` schema composition. A schema-specific applied
fixture and `ExecUsdSystem` client test remain product integration evidence:
the generic scaffold cannot infer the authored schema identifier from its C++
type name.

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
| `usd-plugin-workspace` | shipped dual-mode scaffold, graph preflight, and `plugin test --workspace` | template; build-order generation pending |
| `usd-fileformat-cpp` | shipped scaffold, copied CMake helper, install rules, and L0-L5 lifecycle | template |
| `usd-schema-codeless` | shipped scaffold, generation, registration lifecycle | template |
| `usd-schema-cpp` | embedded standalone scaffold, four generation modes, committed 26.05 outputs, contract baseline, CMake export, and downstream fixture | skeleton; clean-install automation and broader matrix evidence pending |
| `usd-asset-resolver-cpp` | embedded URI resolver scaffold, copied CMake helper, descriptor, provenance, and registration fixture | skeleton; promotion evidence pending |
| `usd-package-resolver-cpp` | embedded sidecar-backed scaffold, copied CMake helper, descriptor, provenance, and registration fixture | skeleton; second package backend pending |
| `usd-exec-cpp` | embedded OpenUSD 26.05 registration scaffold, schema-contract dependency, copied CMake helper, descriptor, provenance, and discovery metadata | skeleton; schema-specific evaluation dogfood pending |
| `hydra-render-delegate-cpp` | hdMerlin dogfood evidence | skeleton candidate |
| `renderer-project` | hdMerlin end-to-end structure | skeleton candidate |
| `usd-tool-cpp` | normal project/package substrate exists | project-template candidate |
| product aggregate | existing package/artifact substrate; no composition descriptor | schema and lifecycle candidate, not a plugin template |

This table describes policy maturity, not release status. A candidate is not
available until its descriptor, source scaffold, tests, and CLI wiring land.
`usd-schema-compiled`, `usd-resolver-cpp`, and `openexec-plugin-cpp` are proposal
terms mapped above, not aliases that the CLI must accept.

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
- standalone, workspace-consumer, and clean-install product evidence when the
  entry is intended for composition;
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

1. ✅ Define and validate the template descriptor, deterministic scaffold
   provenance, and a versioning strategy for plugin dependency extensions.
2. ✅ Describe the shipped plugin templates and `usd-plugin-workspace`; make
   generation tests descriptor driven without changing their output or current
   manifest acceptance unexpectedly.
3. ✅ Add read-only dependency graph validation: deterministic discovery, duplicate
   and missing ids, version/contract checks, forbidden directions, and cycles.
4. 🚧 Compose validated source-workspace dependencies for build, doctor, test,
   run, and generated source CI. The implementation resolves transitive
   closures, installs dependency builds into a target-specific private prefix,
   and keeps `--with` additive. The remaining acceptance item is the first real
   split's full hosted verification pyramid without sibling bootstrap glue.
5. ⏳ Extract self-contained shared CMake mechanics, test copied helpers, and prove
   standalone plus workspace-consumer builds before generating graph targets.
   The versioned copied helper and standalone/workspace configure evidence have
   landed for the compiled file-format and asset-resolver entries; full build
   and clean-install evidence remains before this step is complete.
6. ⏳ Add the standalone `usd-schema-cpp` skeleton with pinned generation modes,
   contract baseline, downstream consumer fixture, and clean-install tests. The
   embedded skeleton, `VERIFY` baseline, CMake export, and fixture have landed;
   automated clean-install and second-platform/OpenUSD-line evidence remain;
   v0.15 fixes the MSVC `NOMINMAX` consumer and repeated-`pxrConfig` defects.
7. Harden the `usd-asset-resolver-cpp` skeleton with identifier, cache,
   concurrency, cross-platform, and broader clean-install evidence.
8. Define the product descriptor and aggregate report, then compose existing
   bundle packages by digest and run extraction/clean-install smoke tests.
9. Dogfood `usd-package-resolver-cpp` against a second package backend before
   promotion; do not fold it into the asset-resolver scaffold.
10. Add tool-project, Exec, and Hydra/renderer candidates as their distinct
   lifecycle gaps are proven; keep algorithmic/reference code outside formal
   scaffolds.
11. Add an explicit template diff/gap report before any migration or automatic
    manifest-editing command.

The next implementation slice makes already validated source dependency fields
control composition. It must not bootstrap a new repository, parallel CLI,
product command family, general-purpose text templating engine, or CI-only
dependency declaration.

## Completion criteria for bundle composition

The bundle/workspace proposal is complete when all of the following are proven
without weakening the single-bundle lifecycle:

- `usd-schema-cpp`, `usd-asset-resolver-cpp`, and `usd-fileformat-cpp` can be
  generated as independent bundles and built both alone and in one workspace;
- a file-format consumer can depend on installed schema and resolver bundles
  without embedding their implementation or using sibling source paths;
- `ost plugin test <consumer>` and `ost plugin test --workspace` run that
  consumer's full pyramid without a required hand-authored `--with`, and
  generated source CI uses the same closure without a cell-level dependency
  list;
- bundle dependencies and schema contracts are resolved from versioned manifests
  with actionable mismatch and cycle diagnostics;
- an ordinary shared container/parser library can be consumed by two bundles
  without becoming an OpenUSD plugin;
- a product descriptor composes pinned member artifacts, preserves their
  manifests/provenance/notices, and emits aggregate evidence;
- plugin discovery and representative asset-open tests pass from only the
  extracted install tree;
- an Exec skeleton, when introduced, builds against the schema contract without
  a dependency on the file-format/importer bundle;
- an external dogfood repository such as `usd-vrm-plugins` can adopt the model
  without OST-specific domain code entering the shared templates.
