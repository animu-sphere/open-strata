---
title: Component package-contract delivery plan
status: candidate
owners:
  - openstrata-maintainers
created: 2026-08-29
updated: 2026-08-29
applies_to: post-v0.22.9
---

# Component package-contract delivery plan

This candidate plan turns the
[component package-contract proposal](../design/proposed/component-package-contracts.md)
into independently reviewable slices. It is intentionally not assigned a
release until the active consumer-packaging/runtime-UX/DCC milestone ladder is
reconciled with release capacity.

Legend: 🚧 in progress · ⬜ not started

## Phase A - installed consumer correctness (P0)

- 🚧 Prototype one descriptor-scoped `verify-consumer` lifecycle for ordinary
  libraries: build, install, clean prefix, generated CMake consumer, link, and
  optional execution.
- 🚧 Add package name, exported target, public-header, standalone, aggregate, and
  consumer-probe metadata without breaking `openstrata.library/v1alpha1`.
- 🚧 Verify only the declared package closure and exclude source-tree targets,
  ambient prefixes, and unrelated workspace installs.
- ⬜ Add a workspace-wide orchestration path and per-component structured result.
- ⬜ Integrate the result into source CI with negative tests for missing targets
  and source-tree leakage.

## Phase B - dependency correctness (P0)

- ⬜ Add public/private visibility to native dependency metadata while retaining
  the existing identity-based workspace graph.
- ⬜ Validate PUBLIC/INTERFACE edges against installed package resolution and
  required `find_dependency()` behavior.
- ⬜ Reject unresolved installed `INTERFACE_LINK_LIBRARIES` targets and private
  dependencies that leak into the public interface.
- ⬜ Add host-platform contracts for system packages, targets, frameworks, and
  raw platform libraries.
- ⬜ Emit deterministic diagnostics and prove them through dependency-removal
  injection tests.

## Phase C - architecture lint (P1)

- ⬜ Add explicit dependency allow/deny, include-prefix, and namespace rules.
- ⬜ Derive a workspace graph from the same member/dependency contract used by
  build, test, and package.
- ⬜ Emit human, JSON, DOT, and Mermaid graph forms.
- ⬜ Lint cycles, undeclared/forbidden edges, aggregate reverse edges, shared-leaf
  restrictions, and adapter/vendor boundaries.
- ⬜ Add architecture `role`; defer reusable layer policies until two independent
  workspaces prove the same rules.
- ⬜ Extend aggregate membership checks with package mode and excluded-role
  policy while preserving pinned `release_members` evidence.

## Phase D - template and migration integration (P1/P2)

- ⬜ Update the plain-library scaffold with installed package/target metadata,
  dependency visibility, and generated consumer verification.
- ⬜ Update USD plugin scaffolds with bundle identity, plugin registry/runtime
  probes, standalone closure, and aggregate membership.
- ⬜ Prove an adapter-role example based on an ordinary library; keep I/O and
  vendor dependencies inside the adapter and deny sibling-adapter edges by
  default only after independent evidence.
- ⬜ Keep OpenExec computation I/O-free and separate registration from algorithm
  implementation in its candidate scaffold.
- ⬜ Document and dogfood the contract-first component migration recipe.
- ⬜ Produce component-oriented CI evidence that reports build, tests, standalone
  package, external consumer, public closure, and boundary lint separately.

## Exit criteria

- An independently installed ordinary library configures and links a generated
  consumer using only its declared closure.
- Removing one required package resolution or exported target fails with a
  stable, actionable diagnostic.
- A workspace-wide run distinguishes standalone, aggregate-only, and excluded
  members without inferring policy from directory layout.
- A negative forbidden-edge injection fails graph lint and names the edge.
- `usd-vrm-plugins` and one independent workspace publish component-level
  evidence without repository-local copies of reusable checks.
