# {{name}} — OpenUSD asset resolver skeleton

Scaffolded by:

```sh
ost plugin new usd-asset-resolver {{name}} --scheme {{scheme}}
```

The generated plugin registers `{{Name}}Resolver` for the `{{scheme}}` URI
scheme. Its starter implementation maps `{{scheme}}:<filesystem-path>` to a
local file so registration, identifier normalization, resolution, and asset
opening can be tested before domain code is added.

Replace the filesystem mapping at the documented seams in
`src/{{Name}}Resolver.cpp`. Protocol access, authentication, retry, cache
invalidation, remote trust, and project-specific path grammar do not belong in
the shared template.

The skeleton is deliberately read-only. Keep it that way until write semantics,
atomicity, and authorization have explicit tests.

```sh
ost plugin inspect .
ost plugin build .
ost plugin test . --up-to 2
```
