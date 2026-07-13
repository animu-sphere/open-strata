# {{name}} — OpenExec computation skeleton

Scaffolded by:

```sh
ost plugin new usd-exec {{name}} \
  --schema-bundle {{schema_bundle}} \
  --schema-type {{SchemaType}}
```

The generated library publishes the `compute{{Name}}` prim computation for the
OpenUSD schema type `{{SchemaType}}`. OpenExec discovers the registration from
`Info.Exec.Schemas` in `plugInfo.json` and loads this library when a computation
for that schema is requested.

The bundle depends on the public contract of the `{{schema_bundle}}` schema
bundle. Edit the starter version range and contract in
`openstrata.plugin.yaml` to match that schema before composing the workspace.
Do not depend on importer or file-format implementation state.

`src/{{Name}}Computation.cpp` is the project-owned evaluation seam. Add typed
inputs with OpenExec's computation-definition DSL in `src/{{Name}}Plugin.cpp`,
then read only those inputs from `VdfContext` in the evaluation function. The
skeleton deliberately does not define graph construction, scheduling, cache
ownership, invalidation policy, solvers, stage mutation, or CPU/GPU execution.

The smoke USD file proves the ordinary bundle lifecycle only; it cannot apply
an arbitrary external schema identifier automatically. Add a workspace fixture
that applies `{{SchemaType}}`'s authored schema identifier and a small
`ExecUsdSystem` client test as part of the schema/Exec product integration.

`cmake/OpenStrataPlugin.cmake` is a pinned, self-contained copy of the shared
build/install mechanics. The generated bundle does not need an OpenStrata
checkout or `ost` when built directly with CMake or as a workspace
subdirectory.

```sh
ost plugin inspect .
ost plugin build .
ost plugin test . --up-to 2
```
