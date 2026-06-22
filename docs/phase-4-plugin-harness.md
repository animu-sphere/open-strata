# Phase 4 — OpenUSD Plugin Verification Harness (direction)

> Status: directional plan. Consolidates the harness design
> ([plugin-harness-source.md](plugin-harness-source.md)) onto the current
> OpenStrata codebase and sequences the work around our one hard dependency: a
> **real** OpenUSD runtime (today's `runtime pull` backend is a mock).

## North star

`ost plugin test` becomes the standard "run this first" command for OpenUSD
plugin development: **build → discover → open → verify**, with actionable,
machine-readable diagnostics, deterministic exit codes, and human + JSON output.
Everything else (publish, matrix, DCC adapters) follows from getting this right.

The MVP scope is Linux x86_64 and three plugin kinds: `usd-fileformat`,
`usd-asset-resolver`, `usd-schema`.

## How it maps onto today's OpenStrata

The harness is not a greenfield product — most of its substrate already exists.

| Harness concept | Existing piece | Gap to close |
| --- | --- | --- |
| Fixed, named runtime (`openusd-24.11-…`) | `Runtime` (platform+profile, `openusd` extension) + `runtime pull/validate/explain` | **Real OpenUSD artifacts** (current backend is mock) |
| Session env (`PXR_PLUGINPATH_NAME`, `PYTHONPATH`, `LD_LIBRARY_PATH`, …) | `EnvSet` (`ost-runtime::env`) already emits these incl. `PXR_PLUGINPATH_NAME` | add the plugin bundle's discovery/lib/python roots to the set |
| ABI / compatibility | `Variant` + extension `allowed_range`/`certified` | per-plugin ABI assertions (cxx, python ABI) |
| Build / package | `ost-build` configure/build/package | produce a `.so` + `plugInfo.json` into the bundle layout |
| Capability vocabulary | capability resolver (`ost-extension`) | plugin `provides: usd-fileformat:<ext>` etc. |
| Diagnostics with stable IDs | `runtime validate` Check/Report pattern | extend to staged `plugin doctor` checks |
| Reports | — | new (`.strata/reports/…`) |
| Plugin Bundle | — | new (`ost-plugin` crate + `openstrata.plugin.yaml`) |

Principle alignment is already good: no global env mutation (§5.2 ↔ our
`env`/`devshell`-only rule), human + JSON everywhere, deterministic exit codes,
capability-driven resolution.

## Plugin Bundle Contract

A plugin is a **self-describing bundle**, not a bare `.so` (harness §8). Adopt
the doc's layout and `openstrata.plugin.yaml`, with two adjustments for
consistency with the rest of OpenStrata:

- Reference a **profile/platform-derived runtime**, not just an `openusd`
  version string, so the existing resolver applies. The plugin declares the
  OpenUSD version *range* it tolerates; the runtime supplies the certified
  point.
- Express `provides` as **capabilities** (`usd-fileformat:lumagraph`,
  `usd-schema:LumaThing`, `usd-asset-resolver`) so `plugin doctor` and
  `runtime explain` speak the same language.

```yaml
plugin:
  name: usdluma
  version: 0.1.0
  kind: usd-fileformat        # | usd-asset-resolver | usd-schema
runtime:
  openusd: ">=24.11,<25.0"
  cxx_abi: libcxx
provides:
  - usd-fileformat:lumagraph
requires:
  capabilities: [usd-stage-read]
  components: { materialx: ">=1.39,<1.40" }
usd:
  plug_info: plugin/resources/usdluma/plugInfo.json
tests:
  smoke:    [tests/fixtures/basic.lumagraph]
  roundtrip:[tests/fixtures/basic.lumagraph]
  negative: [tests/fixtures/invalid.lumagraph]
```

## Runtime Session & Launcher

`ost plugin run <plugin> -- <cmd>` builds an ephemeral session and execs the
command (harness §9). It reuses `EnvSet` and adds the bundle's roots:

1. resolve the runtime (platform+profile) and read its manifest;
2. inspect + validate the bundle (Level 0);
3. check runtime/bundle compatibility (Level 1);
4. compose the session env = runtime `EnvSet` **+** the bundle's `plugInfo`
   root → `PXR_PLUGINPATH_NAME`, `lib/` → dynamic-lib path, `python/` →
   `PYTHONPATH`;
5. spawn the child with that env (no global mutation), capture
   stdout/stderr/exit;
6. write a report.

`ost plugin shell` is an auxiliary for interactive poking; `ost run` is the
canonical, CI-safe path. Launch scripts are *generated*, never the source of
truth (harness §9.4).

## Verification pyramid — and what's reachable today

This is the crux of sequencing. Levels 0–1 are **static** (manifest + filesystem
+ runtime manifest) and work against the current mock backend. Levels 2+ need a
**real OpenUSD runtime** (registered `plugInfo`, `usdcat`, Python bindings).

| Level | Check | Needs real runtime? |
| --- | --- | --- |
| 0 | Bundle structure (manifest valid, `plugInfo.json`/`.so`/fixtures present, portable paths) | no — **now** |
| 1 | Runtime / ABI compatibility (OpenUSD range, OS/arch, cxx/python ABI, components) | no — **now** |
| 2 | Plugin discovery (`PXR_PLUGINPATH_NAME` correct, `plugInfo` parses, lib loads, format/schema/resolver registered) | **yes** |
| 3 | `usdcat` minimal read (extension recognized, stage opens, expands to USDA) | **yes** |
| 4 | Python `Usd.Stage.Open()` (expected prims/attrs/metadata) | **yes** |
| 5 | Golden comparison / round-trip (normalized USDA vs expected) | **yes** |
| 6 | `usdview` launch (process starts, opens stage, no fatal stderr) | **yes** (+ display) |

## `ost plugin doctor`

The differentiator (harness §12). Staged checks, each with a **stable id**, a
PASS/FAIL/SKIP status, the observed fact, and machine-readable
`suggested_actions`; text + JSON; deterministic exit. Reuse the
`Check`/`ValidationReport` pattern from `runtime validate`.

Stable ids (initial): `runtime.openusd.version`, `runtime.cxx_abi`,
`runtime.python_abi`, `bundle.manifest`, `bundle.plug_info`,
`plugin.shared_library`, `session.plugin_path`, `plugin.discovery`,
`dependency.<component>`. Levels not reachable on the current backend report
`SKIP` with a reason, not a false `PASS`.

## Reports

Per run, written under **`.strata/reports/<plugin>/<UTC-timestamp>/`** (using our
`.strata/` convention, not `.openstrata/`):
`summary.txt`, `report.json`, `environment.json`, `diagnostics.log`,
`stdout.log`, `stderr.log`, `normalized-output.usda`, `diff.txt`. Local and CI
failures then share one structure.

## Command surface (Phase 4)

```text
ost plugin new <kind> <name> [--extension <ext>]   scaffold a bundle from a template
ost plugin inspect <name>                          Level 0 bundle report
ost plugin build <name> [--target <cy>]            build the .so + plugInfo (reuses ost-build)
ost plugin doctor <name> [--json]                  staged diagnostics (L0–L1 now; L2+ when real)
ost plugin run <name> -- <cmd...>                  session-launched command (needs real runtime)
ost plugin test <name> [--json]                    orchestrate L0..Ln, write a report
ost plugin verify <name> <fixture>                 golden comparison (L5)
ost plugin view / test-view <name> <fixture>       usdview (L6)
ost plugin package <name>                          bundle artifact (overlaps Phase 6)
```

## Recommended build order

**Phase 4a — framework + static verification (no real runtime needed):**

- `ost-plugin` crate: Plugin Bundle model + loader + validation
- `ost plugin new usd-fileformat <name> --extension <ext>` →
  `templates/usd-fileformat-cpp/` (C++ source, `plugInfo.json`, `CMakeLists`,
  fixtures, `openstrata.plugin.yaml`)
- `ost plugin inspect` (Level 0) and `ost plugin build` (reuse `ost-build`)
- `ost plugin doctor` skeleton with L0/L1 checks + a **session env preview**
  (shows the `PXR_PLUGINPATH_NAME`/`PYTHONPATH`/lib path it *would* set)
- reports scaffolding + JSON schema + stable error ids

**Phase 4b — light up execution levels (gated on a real OpenUSD runtime):**

- a real runtime artifact backend behind `runtime pull` — built locally or
  fetched from an artifact source (Vitrakiln is the candidate per harness §18)
- session launcher `ost plugin run`
- Levels 2–5 (discovery, `usdcat`, Python Stage Open, golden) and
  `ost plugin test` orchestration; `verify`/`snapshot`

**Phase 4c — later:** `view`/`test-view` (L6), package/publish (with Phase 6),
Jenkins runtime×plugin matrix, then DCC host adapters (separate repos).

## Decisions

1. **First slice — 4a now (decided).** Build the framework against the current
   mock backend: `ost-plugin` crate + Plugin Bundle contract, `ost plugin
   new/inspect/build`, `doctor` skeleton (Levels 0–1), and reports + JSON schema.
   Independently useful and de-risks the contract before the heavy runtime work.
2. **Real OpenUSD runtime source — deferred to 4b (decided).** 4a completes on
   the mock backend; the source for a real runtime (build ourselves, consume
   Vitrakiln artifacts, or a vendor build) is chosen when 4b starts.
3. **Bundle vs extension — separate (decided).** User plugins are a *new*
   artifact kind (`ost-plugin`), distinct from runtime-provided `ost-extension`,
   sharing the capability vocabulary.

## 4a acceptance (definition of done)

- `ost plugin new usd-fileformat toy --extension toy` scaffolds a buildable
  bundle (C++ + `plugInfo.json` + `CMakeLists` + fixtures + `openstrata.plugin.yaml`).
- `ost plugin inspect toy` reports Level 0 (structure) as a human + JSON report.
- `ost plugin build toy` produces the `.so`/`.dll` + staged `plugInfo` via
  `ost-build`.
- `ost plugin doctor toy` runs Levels 0–1, previews the session env it *would*
  set, and marks Levels 2+ as `SKIP (needs real runtime)` — never a false PASS.
- Stable error ids + a published JSON schema; deterministic exit codes.
