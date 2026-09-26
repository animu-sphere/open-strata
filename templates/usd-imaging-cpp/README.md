# {{name}} UsdImaging adapter

Generated with:

```sh
ost plugin new usd-imaging {{name}} --schema-bundle {{schema_bundle}} --schema-type {{SchemaType}}
```

This is a **registration skeleton**, not a finished material adapter. Implement
`GetImagingSubprimData` and `InvalidateImagingSubprim`, then add scene-index tests
for your schema's data and invalidation locators. API-schema adapters require
`UsdImagingStageSceneIndex`; legacy `UsdImagingDelegate` does not consult them.

Update the version/contract of `requires.bundles` to your schema provider.
`--schema-type` is the **registered API schema name**, which may differ from its
C++ class name. The provider must be discoverable in the session; it is not linked
into this plugin. Use a runtime with the usdImaging development headers and
library, including the canonical imaging variants of the `usd` profile. The
resolved SDK is checked at build preflight and L1; no viewer is required.
When migrating a 0.1.0 scaffold, remove `hydra-preview` from
`requires.capabilities` unless your plugin independently needs that capability.
Explicitly declared capabilities still have to be promised by the profile.

`ost plugin test --up-to 2` builds a small native registry checker with the
selected SDK, CMake, Ninja and a C++ compiler. It checks the schema definition,
registry membership, and adapter construction. `USDIMAGING_ENABLE_PLUGINS=0`
fails the check. Never set `isInternal` on external adapters to bypass that flag.
Higher levels read the fixture; they do not certify renderer output.

The skeleton avoids `hd/retainedDataSource.h`: OpenUSD 26.08's specialized
constructor spelling fails with GCC 13 in C++20. Keep custom data-source
implementations until the upstream header supports the selected toolchain.

For a prim adapter, use `UsdImagingPrimAdapter` with `primTypeName` metadata and
`provides: [usd-imaging-prim:<primTypeName>]`. Each API schema or prim type key
must have one adapter owner across the composed bundles.
