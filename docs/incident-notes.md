# Incident Notes

Short notes for debugging context that was learned the hard way. These are not
full postmortems; they capture symptoms, root cause, fix, and future guardrails.

## 2026-06-30: codeless schema UTF-8 docs vs `usdGenSchema` locale

### Summary

After adding the codeless USD schema scaffold, coding agents started spending a
long time reasoning after CLI runs that touched schema generation. The CLI was
not hanging. The confusing part was the `usdGenSchema` failure mode: on a
Japanese-locale Windows host, Python text I/O used `cp932`, then failed when a
schema `doc=` string or scaffold prose contained UTF-8-only characters such as
an em dash. The traceback pointed at the codec, not at the offending schema
documentation.

### Trigger

- `ost plugin build` for a `usd-schema` bundle runs CMake.
- The codeless schema template runs `usdGenSchema schema.usda <resources>`.
- `usdGenSchema` is a USD Python tool and writes generated files through Python
  text encoders.
- The initial scaffold included non-ASCII prose in `schema.usda`; real user docs
  may also contain UTF-8 text.

### Symptoms

- The build exits through CMake/Python failure rather than a structured
  OpenStrata diagnostic.
- Logs mention encoding failures such as `cp932` / codec encode errors.
- Agents over-analyze the traceback because it does not clearly identify the USD
  schema doc string as the root cause.

### Fix

- `ost plugin build` now forces UTF-8 for schema-generation environments with
  `PYTHONUTF8=1` and `PYTHONIOENCODING=utf-8`.
- The codeless schema template's own CMake target also wraps `usdGenSchema` in
  `cmake -E env` with the same variables, so direct CMake builds are protected.
- The starter `schema.usda` prose is ASCII to avoid failing before users edit
  the scaffold, while edited UTF-8 doc text remains supported.
- Regression coverage checks the composed build env and the scaffolded CMake /
  starter schema properties.

### Adjacent Finding

The same investigation surfaced a separate `usdGenSchema` naming footgun:
`libraryPrefix` is composed with the schema class name for generated C++/TfType
names. A prefix such as `Foo` with a class such as `FooBarAPI` can produce a
double-prefix shape. `ost plugin doctor` now emits a non-failing
`schema.library_prefix` hint for that pattern.

### Guardrails

- Keep `ost`-owned schema-generation paths UTF-8 forced.
- Keep direct-template CMake builds protected, not only CLI builds.
- Prefer ASCII for scaffold starter prose, but do not reject user-authored UTF-8
  schema documentation.
- If schema generation fails with a locale codec error, inspect `schema.usda`
  `doc=` text first.
