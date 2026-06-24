# {{name}}

A C++ library scaffolded by OpenStrata (`ost init --template cpp-library`).

## Build

```bash
ost runtime pull <platform> --profile <profile>   # once, to provision the toolchain
ost build                                         # configure + build via CMake/Ninja
ost package                                        # install + pack the artifact
```

`ost build` generates the CMake toolchain and presets under `.strata/` and
drives CMake. Your `CMakePresets.json` (if any) is never modified; run
`ost presets install` to wire the generated presets into it.

## Layout

```
include/{{name}}/{{name}}.hpp   public header
src/{{name}}.cpp                implementation
CMakeLists.txt                  build + install rules
```
