# {{name}}

An OpenUSD plugin scaffolded by OpenStrata (`ost init --template usd-plugin`).

## Build

```bash
ost runtime pull <platform> --profile <profile> --from-usd <path-to-usd>
ost build
ost package
```

`ost build` resolves the runtime, generates the CMake toolchain/presets under
`.strata/`, and drives CMake. Your `CMakePresets.json` (if any) is never
modified; run `ost presets install` to wire the generated presets into it.

## Layout

```
src/{{name}}.cpp                    plugin registration (TfType / schema / fileformat)
plugin/resources/plugInfo.json      USD plugin manifest
CMakeLists.txt                      build + install rules
```

> Note: `plugInfo.json`'s `LibraryPath` ends in `.so`; adjust to `.dylib`
> (macOS) or `.dll` (Windows) for those platforms.
