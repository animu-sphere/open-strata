# Derive an ecosystem consumer package

Python wheels, npm packages, and native SDK packages are entry points to an
already exported composed runtime. They are not independent runtime artifacts
and must not carry a second dependency graph.

## 1. Export the canonical runtime

Compose, SDK-validate, and export the runtime first:

```text
ost runtime compose runtime-composition.toml --lock runtime.lock.json --output runtime
ost runtime validate --composition runtime --sdk --cmake-package Tiny
ost runtime export --composition runtime --dist dist/runtime
```

The export prints two different identities. `digest` identifies the exact
`openstrata.composed-runtime` archive bytes. `runtime_digest` identifies the
locked composition inside those bytes. A consumer package retains both.

## 2. Derive the consumer manifest

Use the full exported artifact digest, never a mutable registry tag or a digest
prefix:

```text
ost runtime consumer-manifest \
  --from-artifact sha256:<64 lowercase hex characters> \
  --kind native-sdk \
  --name tiny-sdk \
  --version 1.0.0 \
  --entrypoint Tiny \
  --output consumer-package.json
```

The supported package kinds and entrypoint meanings are:

| Kind | Public entrypoint |
| --- | --- |
| `native-sdk` | Installed CMake config package name, for example `Tiny`. |
| `python-wheel` | Import module, for example `pxr.Usd`. |
| `npm-javascript` | `package.json` export key, such as `.` or `./resolver`. |
| `npm-wasm` | `package.json` export key for the Wasm-facing adapter, such as `./wasm`. |

The command verifies the locally stored artifact, extracts it to a temporary
prefix, and verifies its embedded lock, inventory, SDK, attribution, target, and
exact dependencies. It also requires verified SPDX SBOM and provenance
sidecars, then writes the manifest atomically. The composed-runtime SBOM and
provenance digests, plus any component evidence digests, remain pinned in the
result. For `native-sdk`, every entrypoint must match an installed
`<Name>Config.cmake` or lowercase `<name>-config.cmake` in the verified SDK
inventory. This structural check is deterministic and does not execute target
CMake package code; use `runtime validate --sdk --cmake-package <Name>` before
export for the opt-in configure probe. A legacy composition without an SDK
cannot become a consumer package.

## Binder and loader boundary

The ecosystem package owns its public API. Python callers import the declared
module, JavaScript callers use the declared export, and C++ callers use the
declared CMake package. None of those callers parse an OST lock, component
contract, or activation document.

Package implementation code is private. It must:

1. obtain the exact `runtime.artifact_digest` bytes embedded in or referenced by
   the package;
2. verify the artifact before extraction;
3. refuse a runtime whose embedded identity or component digests differ from
   the consumer manifest; and
4. apply the runtime SDK activation contract before loading native code.

The generated `private_loader` object records that protocol as
`verify-extract-activate`. It is intentionally not a public Python or JavaScript
API. Registry package version and name are routing metadata; OST artifact,
runtime, target, provenance, attribution, and dependency identity remain
canonical.

This foundation emits the registry-neutral identity contract. Wheel and npm
archive assembly, platform-specific native loading, and registry publication
remain adapter-owned work and must preserve the manifest unchanged.
