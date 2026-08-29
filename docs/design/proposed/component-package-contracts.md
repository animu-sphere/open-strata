---
title: Component package contracts and workspace architecture lint
status: proposed
owners:
  - openstrata-maintainers
created: 2026-08-29
updated: 2026-08-29
applies_to: post-v0.22.9
---

# Component package contracts and workspace architecture lint

## Decision being proposed

OpenStrata should verify that a component remains usable after it leaves its
source workspace. A successful workspace build is not sufficient evidence for
an installed CMake package, because source-tree targets can hide an incomplete
`*Config.cmake`, a missing `find_dependency()`, an unresolved imported target,
or an incomplete install closure.

The proposed contract is:

```text
source graph
  -> component build
  -> isolated install/package closure
  -> generated external consumer
  -> configure + link + optional runtime probe
  -> package-contract evidence
```

This work extends the existing `openstrata.library.yaml`, plugin-workspace graph,
descriptor-scoped library lifecycle, aggregate membership, and artifact evidence
contracts. It must not create a second workspace graph or replace the runtime
consumer-package contract described in
[runtime-composition.md](runtime-composition.md).

## Why this is a separate boundary

OpenStrata currently proves several adjacent facts:

- a source workspace graph is complete and acyclic;
- a selected library can build, test, and package with its declared library
  closure;
- a plugin package can activate its runtime directories from an extracted
  archive; and
- a composed runtime can expose installed CMake packages to a clean consumer.

None of those facts alone proves that every component's exported CMake package
is self-consistent. In particular, this is valid in a source build but invalid
for an external consumer when `fooConfig.cmake` does not resolve `osc`:

```cmake
target_link_libraries(foo PUBLIC osc::osc)
```

An installed-package test must therefore be component-scoped and use only the
declared package closure in a clean prefix. It is distinct from the ecosystem
wheel/npm/native-SDK adapter lifecycle: that lifecycle binds a composed runtime
to a registry-facing entry point, while this proposal validates the native
package surface of each component inside that runtime or product.

## Identity and vocabulary

The following terms have separate meanings:

- **component kind** describes build and packaging semantics, such as ordinary
  library, USD schema bundle, USD file-format bundle, OpenExec plugin, tool, or
  aggregate product;
- **role** describes an architecture position, such as adapter, shared leaf,
  motion core, or computation layer;
- **package contract** describes the installed surface promised to an external
  consumer; and
- **aggregate membership** describes which independently valid components are
  included in a distribution.

Architecture roles must not be encoded as fake build kinds. For example, an
adapter remains `kind: library` with `role: adapter`; its architecture policy is
additional metadata rather than a different compiler or package lifecycle.

## Package contract model

The first schema should extend the owning component descriptor rather than add a
parallel manifest. The exact field names remain provisional until the prototype
lands, but the normalized model keeps package mode separate from its exported
consumer surface:

```yaml
package:
  standalone: true
  aggregate_member: false

package_contract:
  package_name: osc
  exported_targets:
    - osc::osc
  public_headers:
    - include/osc/**
  consumer:
    include: osc/OscPacket.h
    symbol: osc::DecodeOscPacket
```

Plugin-specific surfaces extend the same component-owned contract:

```yaml
package_contract:
  package_name: usdVrmFileFormat
  exported_targets:
    - usdVrmFileFormat::usdVrmFileFormat
  plugin:
    registry: plugInfo.json
    file_extensions: [vrm]
```

A schema bundle may additionally declare identifiers that an install-only probe
can inspect:

```yaml
package_contract:
  schema:
    identifiers:
      - VrmHumanoidAPI
      - VrmExpressionAPI
```

The `package` block owns standalone/aggregate mode exactly once. The existing
producer identity, version, target, digest, `requires` edges, runtime
directories, SBOM, provenance, and validation evidence remain authoritative.
Package-contract fields describe a verifiable public surface; they do not rename
the component or copy its dependency graph.

Adoption should be incremental. A descriptor without a package contract keeps
its current behavior. Once a contract is declared, unknown fields and invalid
combinations fail closed, and CI can require the contract for selected template
maturities or release lanes.

## Dependency visibility and CMake package resolution

The workspace graph remains identity-based. It should gain enough visibility
metadata to distinguish public and private native dependencies without making
CMake target spelling the component identity:

```yaml
requires:
  libraries:
    - id: osc
      version: ">=0.4,<0.5"
      visibility: public
```

The provider's own descriptor supplies its CMake package and exported targets.
For non-workspace or platform packages, an explicit conditional resolution
contract is required. OpenStrata must not infer a portable package identity from
a raw linker token.

At minimum, verification checks:

- every exported `PUBLIC` or `INTERFACE` target dependency has a declared
  public package edge and is resolvable from the clean prefix;
- the generated or hand-written package config performs the corresponding
  package resolution, including `find_dependency()` where required;
- private dependencies do not unnecessarily leak into the consumer interface;
- every target named by `INTERFACE_LINK_LIBRARIES` resolves after installation;
  and
- conditional platform dependencies agree with the exported package interface.

Generation is optional. OpenStrata may generate `find_dependency()` statements
from authoritative metadata or validate a project-owned config, but it must not
silently maintain two divergent declarations.

## Platform dependencies

Platform libraries need an explicit conditional contract because an interface
that works on one host can be invalid elsewhere:

```yaml
platform_dependencies:
  windows:
    link: [ws2_32]
  posix:
    packages: [Threads]
    targets: [Threads::Threads]
```

The first implementation validates the host platform. Later schema lint may
reject obviously impossible cross-platform interfaces, such as a Windows-only
raw library leaking into a POSIX package branch, without claiming it executed a
foreign toolchain.

## Installed-component consumer verification

The ordinary-library prototype is available as `ost library verify-consumer`;
the plugin and workspace-wide command shapes remain candidates:

```text
ost library verify-consumer <path>
ost plugin verify-consumer <path>
ost workspace verify-consumers
```

The ordinary-library command rebuilds the declared library closure, creates a
fresh generated consumer below the selected target state, disables ambient
CMake package registries and prefix variables, and configures and links only
against the isolated install prefixes named by that closure. It writes
`library-consumer.json` with separate configure and link results on both success
and failure. Descriptor adoption remains incremental: existing libraries build,
test, and package unchanged, while `verify-consumer` requires explicit
`package` and `package_contract` blocks.

Names are provisional until CLI design and implementation land. Each command
performs the same bounded lifecycle:

1. build the selected component and its declared prerequisites;
2. install or package it into an OST-owned staging area;
3. create a new temporary prefix containing only the allowed closure;
4. generate or select a minimal consumer fixture;
5. configure with normal `find_package(... CONFIG REQUIRED)` discovery;
6. link every declared exported target; and
7. optionally run a symbol, plugin-discovery, schema, or executable smoke probe.

For ordinary libraries, OpenStrata should generate the common fixture:

```cmake
cmake_minimum_required(VERSION 3.24)
project(openstrata_consumer LANGUAGES CXX)

find_package(osc CONFIG REQUIRED)
add_executable(consumer main.cpp)
target_link_libraries(consumer PRIVATE osc::osc)
```

The generated `main.cpp` supports three increasing levels: link-only, one public
header include, and an optional symbol smoke test. Projects retain hand-written
fixtures for APIs that need construction data or domain behavior; generated
fixtures are not a replacement for semantic tests.

Multi-target packages run one fixture per declared target or one fixture that
links the declared set. Reports must identify which exported target failed.

## Standalone package closure

Every component declares whether it is meaningful alone and whether it may join
an aggregate:

```yaml
package:
  standalone: true
  aggregate_member: true
```

The four meaningful states are validated rather than inferred from directory
layout:

| Standalone | Aggregate member | Meaning |
| --- | --- | --- |
| true | false | independent package excluded from the product |
| true | true | independently valid package also included in a product |
| false | true | product-only member such as a delivery tool |
| false | false | invalid for a releasable component |

`verify-consumer` receives only the selected package and its declared public
runtime/package dependencies. Source paths, sibling build targets, ambient
`CMAKE_PREFIX_PATH`, and unrelated workspace installs are excluded. A passing
result therefore proves the closure rather than the original monorepo.

## Declarative boundary policy

Repository-local boundary scripts have demonstrated useful checks, but their
reusable primitives belong in OpenStrata metadata:

```yaml
boundaries:
  allow_dependencies:
    - motionCore
    - liveTransport
    - osc
  deny_dependencies:
    - vrmAdapterMocopi
    - vrmAdapterVrchatOsc
  deny_include_prefixes:
    - adapters/liveCapture/mocopi
    - adapters/liveCapture/vrchatOsc
  deny_namespaces:
    - vrmAdapterMocopi
    - vrmAdapterVrchatOsc
```

Explicit allow/deny rules should land before a generalized layer language. Once
two or more workspaces prove stable roles, the same rules may be factored into:

```yaml
architecture:
  layer: adapter
  may_depend_on: [shared-leaf, motion-core]
  may_not_depend_on: [sibling-adapter]
```

OpenStrata standardizes graph and source-policy primitives, not repository
semantics. VRM mappings, OSC addresses, tracker solve policy, motion retargeting,
and vendor protocols remain project-owned.

## Workspace graph and lint

Candidate commands are:

```text
ost workspace graph
ost workspace lint-graph
```

The graph is derived from the same authoritative member and dependency model
used by workspace build/test/package. Supported output should include a compact
human tree plus deterministic JSON, DOT, and Mermaid forms.

Lint initially detects:

- undeclared and explicitly forbidden component edges;
- cycles and self-dependencies;
- role or layer violations, including sibling-adapter dependencies;
- aggregate-to-adapter or aggregate-to-tool reverse dependencies;
- forbidden shared-leaf edges; and
- plugin-runtime code reaching a vendor SDK outside its owning adapter.

Source scans for include prefixes or namespaces are evidence supporting the
manifest graph. They must report file and line locations and avoid pretending
that text matching alone is a compiler-resolved dependency graph.

## Aggregate product closure

The existing `workspace.release_members` / `release_exclude` contract remains
the authoritative pinned membership check. Package roles can add reusable
policy on top:

```yaml
aggregate:
  name: usdVrm
  members:
    - vrmSchema
    - usdVrmFileFormat
    - usdVrmPackageResolver
    - vrmContainer
  exclude_roles: [adapter, tool]
```

An aggregate may include only components whose package mode permits membership.
It preserves each member's identity, dependency edges, package-contract result,
provenance, notices, and evidence. It does not turn an invalid standalone member
into a valid package by hiding it inside the archive.

## Evidence-oriented CI report

CI should explain what it proved for each artifact, not expose only one green
job result. A machine-readable report and its human rendering should include:

```text
Package: vrmAdapterVmc
- build: pass
- tests: 14/14
- standalone package: pass
- external consumer: pass
- public dependency: osc
- public dependency: liveTransport
- package config resolution: pass
- forbidden-edge lint: pass
```

Every check has `pass`, `fail`, `skip`, or `not-run` status with a reason. A
host-limited platform check is an explained SKIP, while a missing required tool
in a strict release lane is a failure. Reports retain the component descriptor,
package, target, install inventory, dependency closure, and evidence digests.

Negative injection is part of the acceptance strategy. Tests should deliberately
remove one `find_dependency()`, add one forbidden edge, or expose one undeclared
target and prove that the intended diagnostic fails before promotion.

## Safe migration recipe

Large component splits should follow this order:

```text
1. add the destination identity and package/architecture contract
2. add dependency and boundary rules
3. create an empty or scaffold destination component
4. move implementation code
5. migrate callers and installed-package consumers
6. remove the old identity only after negative and clean-consumer evidence passes
```

Contract-first migration makes the intended destination and forbidden edges
reviewable before a large source move. OpenStrata may eventually automate checks
around this recipe, but it must not automatically move project-owned domain code.

## Delivery and acceptance

Delivery is staged in the
[component package-contract roadmap](../../roadmap/component-package-contracts.md):

1. installed-component consumer correctness;
2. public/package dependency correctness;
3. declarative architecture lint; and
4. template and migration integration.

The proposal is ready for acceptance only when at least one ordinary library
and one plugin workspace prove all of the following on clean prefixes:

- source and installed consumers use the same public package surface;
- a missing package dependency produces a deterministic diagnostic;
- standalone closure contains no undeclared source-tree dependency;
- one injected forbidden edge is rejected;
- aggregate membership and role exclusions agree; and
- the evidence report identifies exactly which package contract was proved.

`usd-vrm-plugins` is the first dogfood source for these requirements. A second,
independent workspace is required before boundary roles or adapter defaults are
promoted into reusable templates.

## Non-goals

This proposal does not:

- infer repository architecture entirely from C++ source;
- replace CMake's target and package model;
- require OpenStrata to generate every `*Config.cmake`;
- flatten private dependencies into standalone archives;
- make every workspace member independently distributable;
- define domain-specific plugin or adapter semantics; or
- introduce a dependency resolver separate from the existing workspace,
  Formation, artifact, and runtime composition contracts.
