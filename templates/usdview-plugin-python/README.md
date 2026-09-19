# {{name}} — usdview host add-on

This bundle extends `usdview` through its Python `PluginContainer` API. It is a
first-class OpenStrata `host-addon` component, not a codeless schema or a file
format/resolver plugin.

```sh
ost plugin inspect .
ost plugin build . --target cy2026 --profile lookdev
ost plugin package . --target cy2026 --profile lookdev
ost plugin view <primary-bundle> <scene.usda> --with . --target cy2026
```

The scaffold is Python-only. A project may add a native extension below
`python/{{identCamel}}`; OpenStrata records its Python/C++ ABI and build-output digest
through the same build and package lifecycle.
