---
title: Runtime composition foundation
status: proposed
owners:
  - openstrata-maintainers
created: 2026-08-24
updated: 2026-08-24
applies_to: v0.22.3+
---

# Runtime composition foundation

## Decision being proposed

OpenStrata should treat a runtime as a first-class, immutable, executable
composition rather than as another name for an OpenUSD install prefix:

```text
Runtime = platform + OpenUSD + components + environment + compatibility evidence
```

Every distributable input to that composition should be an OST artifact. The
initial component kinds are OpenUSD runtimes, plugin bundles and products,
ordinary libraries, tools, renderers, and data-only runtime layers. Each keeps
its own identity, dependency metadata, provenance, attribution, and validation
evidence; composition does not flatten those facts into an opaque archive.

The implementation is staged in the
[v0.22.x runtime-composition roadmap](../../roadmap/runtime-composition.md). This
document records the intended contract and the boundaries that the staged work
must preserve. It does not describe commands or schemas as shipped before they
exist.

## Why this is a separate contract

The current artifact and Formation foundations solve important but different
problems:

- an **artifact** is one immutable distribution unit;
- a **composed runtime** is a distributable, validated execution/SDK environment
  materialized from several artifacts;
- a **Formation** selects a runtime and components for one execution purpose,
  resolves their compatibility, locks the result, and launches a command; and
- an ecosystem **package** such as a wheel or npm package is a consumer-facing
  entry point derived from the canonical OST artifacts, not a second source of
  artifact identity.

Runtime composition must reuse Formation's component resolution, compatibility,
environment, lock, and diagnostic machinery. It must not introduce a second
solver or a parallel plugin/loader path model. The additional responsibility is
to materialize that resolved graph into an independently transportable runtime
artifact with a predictable SDK layout and composition-level evidence.

## Lifecycle and identity

The producer and consumer lifecycles stay separate:

```text
source -> build -> component artifact

component artifacts -> resolve -> lock -> materialize -> validate
                    -> composed runtime artifact -> distribute -> consume
```

The proposed user-facing lifecycle is:

```text
ost runtime compose
ost runtime validate
ost runtime export
ost runtime pull
ost runtime explain
ost runtime doctor
ost runtime exec
```

Command names remain provisional until their CLI contracts land. A composed
runtime identity is derived from the canonical composition manifest, the exact
component artifact digests, target/variant and compatibility facts, and the
materialized file inventory. Filesystem paths, mutable tags, registry locations,
and ambient host state do not contribute identity.

## Component contract

Every component that participates in a runtime must expose enough
machine-readable information to resolve and verify it:

- stable kind, id, version, target and content digest;
- exact artifact dependencies plus version/ABI requirements;
- capabilities provided and required;
- environment contributions (plugin, loader, executable, Python and CMake
  roots) without mutating the host;
- provenance, SBOM/attribution and validation evidence; and
- an install mapping into the runtime layout.

Dependencies are edges, not copied payloads. An ordinary library that declares
`requires.libraries` must build and package against the same closure the graph
validator accepts. Shared profiles, schemas, configuration and other non-code
payloads need a data-only component or an equivalent project-relative source to
install mapping; copying shared data under a tool merely to make it packageable
is not an acceptable ownership model.

Aggregate-product membership is also a declared contract. Discovery determines
what exists in a workspace; it must not silently decide what belongs to a release
artifact. Packaging exposes the resolved membership set and supports a pinned
expectation so a discovery-rule change fails at the release boundary.

## Capabilities and providers

Composition should allow a requirement to name a capability rather than a
repository implementation, for example:

```text
usd
usd.resolve.http
usd.fileformat.copc
usd.fileformat.geotiff
```

The resolver selects a provider only when the input policy leaves the provider
open. A lock records the chosen provider id, version, artifact digest, source and
dependency edges. An explicit provider pin remains valid for support and release
matrices. Multiple providers for a singleton capability, incompatible ABI or
OpenUSD requirements, colliding install destinations, and conflicting
environment contributions fail with deterministic diagnostics.

## Layout and activation

The materialized runtime has a predictable SDK-oriented layout:

```text
runtime/
  bin/
  lib/
  include/
  share/
  plugins/
  python/
  node/
  metadata/
```

Not every runtime contains every directory. The composition manifest records
which component owns each installed path and every activation contribution.
C++ consumers must be able to reach exported CMake packages through normal
`find_package(... CONFIG)` discovery. Python, JavaScript/Wasm and native SDK
packages consume the same canonical component artifacts; they do not redefine
the runtime graph.

## Lock and reconstruction

Composition is not complete until it is reproducible. The lock records at least:

- target platform and concrete variant;
- selected component/provider identity, version and digest;
- immutable source/transport identity;
- the complete dependency graph;
- compatibility decisions and environment contributions; and
- the expected composed-runtime identity.

A clean machine with access to the pinned artifacts must reconstruct the same
runtime identity. Cache hits and local paths may change transfer cost, never the
resolved result.

## Validation boundary

Component validation is necessary but insufficient. Composition adds checks for
the combined result:

- loader and link closure;
- OpenUSD plugin, resolver and schema discovery;
- executable and CMake package reachability;
- environment and install-path conflicts;
- capability-specific probes; and
- basic execution in an isolated consumer prefix.

The validation report names every component digest and distinguishes component
failure, composition conflict, host-capability SKIP and consumer-boundary
failure. Structured output and error codes are public contracts suitable for CI
and agents.

## Geospatial reference composition

[`animu-sphere/usd-geospatial-runtime`](https://github.com/animu-sphere/usd-geospatial-runtime)
is the first composition repository. It is intentionally allowed to begin as a
manifest-and-acceptance repository while the OST primitives land; it must not
grow a repository-specific monolithic build script that duplicates resolution,
locking, activation or verification.

The first target graph is:

```text
OpenUSD 26.08
  + usd-http-resolver
  + usd-pointcloud-plugins
  + usd-raster-plugins
  -> usd-geospatial-runtime
```

The repository is currently empty. Its first useful commit should pin scope,
capabilities and acceptance fixtures, then consume each OST slice as it becomes
available. The HTTP resolver remains a readiness gate: the 2026-08-16
[pre-implementation report](https://github.com/animu-sphere/usd-pointcloud-plugins/blob/main/docs/reports/ost/04-2026-08-16-usd-http-resolver-preimplementation.md)
found an intentionally empty build/test/CI skeleton and requested no OST change.
Tier 2 geospatial acceptance starts only after that resolver ships a bundle,
backend, stable identity metadata and registered tests.

## Non-goals for v0.22.x

- a general-purpose package manager or unrestricted dependency solver;
- rebuilding every dependency from source during composition;
- DCC host adapters or the host support matrix;
- sessions, remote execution, distributed builds or a registry service;
- a GUI or enterprise policy layer; and
- exposing OST-specific concepts as the public Python or JavaScript runtime API.

## Acceptance

The foundation is accepted when a clean consumer can reconstruct a locked
geospatial runtime from immutable artifacts, verify its composition, use its
OpenUSD tools/plugins/resolver through the generated activation contract and
consume at least one exported CMake package, with evidence binding the runtime
identity to every input digest. The detailed vertical slices and intermediate
exit criteria live in the roadmap.
