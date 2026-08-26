# Native SDK test fixture

The runtime-composition integration suite builds and exports this small C++
shared library and executable, composes it, moves the prefix, then builds a
separate consumer with `find_package(Tiny CONFIG REQUIRED)` and executes it
with the SDK environment. No OpenUSD installation is needed.

`plugInfo.json` and `generatedSchema.usda` exercise structural resource-path
checks only; they do not establish OpenUSD type registration or resolver behavior.
