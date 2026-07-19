# External build provenance

`ost external import --build-dir <tree>` records what a CMake tree configured
outside OpenStrata says about itself. The record is written atomically as
`<tree>/.ost-external-build.json`. It may satisfy only the
`runtime-compatible` check; it never claims that `ost build` configured, built,
or tested the tree.

## v2 identity

New imports use `openstrata.external-build/v2`. The record binds the source and
build directories, selected runtime id/digest/root, import scope, applicable
requirements, generator identity, compiler identity, Python details, and a
digest over the inspected identity sources. `ost validate --build-dir` reloads
those sources and fails the compatibility check after a relevant reconfigure.
Legacy v1 records remain readable and verify against the v1 cache-key set.

Compiler and configuration identity follows the generator's real CMake output:

| Generator | Compiler source | Configuration model |
| --- | --- | --- |
| Ninja | `CMakeCache.txt:CMAKE_CXX_COMPILER` | single `CMAKE_BUILD_TYPE` |
| Ninja Multi-Config | cache, with compiler metadata fallback | `CMAKE_CONFIGURATION_TYPES` |
| Visual Studio | `CMakeFiles/<version>/CMakeCXXCompiler.cmake` when absent from the cache | `CMAKE_CONFIGURATION_TYPES`, plus instance/platform/toolset |
| Xcode | cache, with compiler metadata fallback | `CMAKE_CONFIGURATION_TYPES` |

The recorded `generator_flavor` is `ninja`, `ninja-multi-config`,
`visual-studio`, `xcode`, or an explicit other single/multi-config flavor. A
missing compiler diagnostic names both the detected flavor and the identity
sources that were inspected.

## Capability-scoped requirements

Import combines the resolved profile capabilities with every repeated
`--capability <name>`. Requirements are recorded with either `applied` or
`not-applicable` status:

- `cmake.cxx-compiler` is applied to every imported CMake build tree.
- `openusd.runtime` is applied when any selected capability depends on OpenUSD;
  `pxr_DIR` must then resolve within the selected runtime.
- For a core-only scope, `openusd.runtime` is `not-applicable`; absence of
  `pxr_DIR` is not treated as missing evidence.

This distinction is preserved in `external show --json` and in the
`runtime-compatible` validation detail. It means not required, not merely not
checked.

## Remediation

Hints follow the failed precondition. Incomplete C++ detection points to the
compiler metadata CMake must finish writing. A missing or foreign OpenUSD
binding recommends selecting or configuring the applicable runtime. `validate`
recommends `external import` only after confirming the path already contains a
readable `CMakeCache.txt`; an evidence-only or unconfigured directory leaves
runtime compatibility skipped and explains that CMake configuration is the
precondition.
