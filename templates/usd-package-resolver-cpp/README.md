# {{name}} — OpenUSD package resolver skeleton

Scaffolded by:

```sh
ost plugin new usd-package-resolver {{name}} --extension {{extension}}
```

The generated plugin registers `{{Name}}PackageResolver` for `.{{extension}}`
package files. USD dispatches package-relative paths like
`shot.{{extension}}[content/inner.usda]` to it by the package extension, the
same way `.usdz` works.

The starter does not parse a container at all: entries of `foo.{{extension}}`
are plain files in a sidecar directory `foo.{{extension}}.contents/`. That
placeholder makes registration, dispatch, entry lookup, and asset opening
testable end-to-end while leaving the actual container format — entry table,
bounded random access, compression, integrity — entirely to the implementer.
Replace the sidecar mapping at the `EntryLocation` seam in
`src/{{Name}}PackageResolver.cpp`.

**Security:** the entry-path guard (`IsSafeEntryPath`) rejects absolute,
rooted, and `..` entry paths so a hostile `pkg[../../secret]` cannot escape
the package boundary. That guard is the only part of the security contract
the skeleton can own. Everything a real container adds — offset/size overflow,
declared/actual size mismatch, duplicate entries, oversized reads, corrupt
containers, decompression limits — must get explicit tests before this
resolver handles untrusted packages.

The skeleton is deliberately read-only: `ArPackageResolver` has no write
interface, and the starter neither creates nor mutates packages.

`cmake/OpenStrataPlugin.cmake` is a pinned, self-contained copy of the shared
build/install mechanics. The generated bundle does not need an OpenStrata
checkout or `ost` when built directly with CMake or as a workspace subdirectory.

```sh
ost plugin inspect .
ost plugin build .
ost plugin test . --up-to 2
```
