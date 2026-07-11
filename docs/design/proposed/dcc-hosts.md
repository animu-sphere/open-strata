# OpenStrata × DCC hosts — third-party host support (direction)

> Status: directional plan. OpenStrata's first-class surface is **applications
> grown on the OpenStrata Runtime** (`ost app` over the certified runtime,
> capabilities, extensions, and sessions). Existing DCCs — Maya, Houdini, Nuke —
> are supported as **third-party external hosts**: discovered, fingerprinted,
> driven headlessly, packaged for, and checked for cross-DCC USD compatibility —
> never abstracted, bundled, installed, or made central.

This keeps the repo's runtime-centric stance (§1.3, §2.2): "OpenStrata は DCC 中心
ではなく runtime 中心である." DCC support is additive and lives behind a host
adapter boundary, exactly as Kubernetes lives behind an execution backend
([kubernetes.md](kubernetes.md)).

## First-class vs third-party

| | First-class (the product) | Third-party (host support) |
| --- | --- | --- |
| What | Apps on the OpenStrata Runtime (`ost app`) | Maya / Houdini / Nuke installs already on the machine |
| Runtime | OpenStrata certified runtime (pulled/built/adopted) | the DCC's own bundled Python / USD / libs |
| Identity | runtime id + variant + digest | discovered host instance id + fingerprint |
| OpenStrata owns | the whole stack: resolve → build → validate → package → publish | only the *environment, artifacts, tests, and compatibility* around the host |
| API surface | capability/extension graph | **no DCC API abstraction** — host adapters expose `detect/inspect/run/test`, not `create-node` |

The DCC's own bundled OpenUSD is, in repo terms, an **adopted runtime** — the same
shape as `ost runtime pull … --from-usd <path>` (the `local`/adopt source). A DCC
host is "a real-but-not-reproducible runtime we found, plus a launch contract."

## Responsibility split

| OpenStrata (what & under which contract) | The DCC (where it runs) |
| --- | --- |
| discover / validate / fingerprint installs | ships its own Python / USD / libs |
| resolve host runtime + ABI + plugin deps | loads plugins / modules / packages |
| generate host-standard package layout (`.mod`, Houdini package JSON) | its own discovery of those packages |
| compose a clean launch environment (no inherited mutation) | executes the headless command |
| run headless smoke tests + collect logs | the actual host process |
| verify cross-DCC USD data contracts | reads/writes USD |
| record artifacts, locks, results, support policy | — |

Principle: **the DCC decides how it runs a host process; OpenStrata decides which
environment, which artifacts, which tests, and under which compatibility
contract.** DCC differences are pushed into host adapters, never leaked into the
core.

## Non-goals

Consistent with §2.2, and to avoid an unmaintainable surface:

- **No DCC API abstraction.** No `ost create-node --type publish`; that papers
  over Maya/Houdini/Blender differences into a lossy bespoke API. Host adapters
  expose explicit `build/test/package/run`, and shared logic is contract-shaped
  (e.g. a publish *contract*), not a unified node API.
- No DCC install, update, or license authentication.
- No farm scheduler / CI product / Kubernetes substitute (those are separate
  backends — [kubernetes.md](kubernetes.md), Phase 5 CI).
- No GUI-required tests in the core path; first value is headless.
- No unconditional inheritance of the user's `PYTHONPATH` /
  `PXR_PLUGINPATH_NAME` / DCC env (hidden mutation is forbidden, quality bar).
- No naïve matrix product (every DCC × USD × OS × renderer); only
  production-guaranteed runtime sets are matrix cells (see [Matrix](#matrix)).
- No full source build of every DCC/USD/renderer.

## Discovery model

Studios run many concurrent versions, patch levels, and site layouts of one DCC
(`/usr/autodesk/maya2025`, `/tools/maya/2025.3`, `current -> 2025.3`,
`/opt/hfs20.5.550`). Each **validated install instance** is a first-class entity,
not a single global `maya`.

Two phases, mirroring the plugin harness's "candidate → validated, never a false
PASS":

1. **Candidate discovery** — a path/env/registry/rule suggests a possible
   install. Composable, order-stable **providers**, deduplicated by canonical
   (realpath) root, retaining *all* provenance:

   ```text
   explicit_path → configured_roots → environment → registry/package_db
     → known_install_roots → executable_path → custom_rules
   ```

2. **Validation** — a host-specific validator confirms markers, resolves the real
   version (least-invasive first: metadata → short-timeout `--version` /
   `mayapy -c` / `hython -c`; directory name only as low-confidence fallback),
   reads the embedded Python, and produces a deterministic instance id +
   fingerprint. Validation is **read-only, bounded, timeout-protected** (default
   5s), never sources shell setup, never launches a GUI, and isolates failures so
   one bad candidate can't sink the scan. Rejections keep their reason.

Record statuses: `candidate · validated · rejected · stale · unreachable ·
invalidated`. Records persist in a reviewable project inventory
(`.strata/hosts/…`) plus a fast user cache (`~/.ost/cache/host-discovery/`),
written atomically (temp → fsync → rename, bounded lock), invalidated on
root/exe/mtime/size change, schema bump, or `--refresh`.

### Fingerprint

A fingerprint distinguishes installs that share a nominal version but differ in
patch/runtime/layout. **Standard** mode is cheap and bounded (host family,
version, OS/arch, canonical root, selected exe size/mtime, embedded Python,
marker/plugin-root metadata) — it does **not** hash a whole install. **Deep**
mode (`--fingerprint deep`) opt-in hashes an allowlisted set of binaries. Inputs
are versioned. This is the same discipline as runtime/plugin digests, and it
feeds the future `ost compat diff` / `ost reproduce`.

## Host adapter model

Each DCC adapter implements a small, explicit capability set — the host analogue
of an extension/plugin:

```text
detect · inspect · resolve · environment · package · run · test · report
```

| Capability | Responsibility |
| --- | --- |
| `detect` | executable, version, platform, Python, SDK, related runtimes |
| `inspect` | host capabilities as JSON (e.g. MayaUSD present + version; Houdini Solaris/Karma/husk) |
| `resolve` | host dependency + runtime set |
| `environment` | the launch env (reusing the runtime `EnvSet`; no inherited mutation) |
| `package` | host-standard layout — Maya `.mod`, Houdini package JSON — never editing `Maya.env`/user config |
| `run` | launch the host / headless exe with the composed env; propagate exit code |
| `test` | package load, plugin load, USD smoke test |
| `report` | capabilities / artifacts / results, machine-readable |

The shared, DCC-independent logic (publish/validate rules, USD layer authoring,
custom schema/resolver, naming/path rules) lives in a `core`; only UI, node/op
registration, scene access, and host packaging live in adapters. Cross-DCC work
is defined as a **contract** (e.g. a publish contract: collect context → build
stage → validate → publish) that adapters fulfil — not a unified node API.

The cross-DCC USD compatibility check reuses the plugin verification harness's
level model ([phase-4-plugin-harness.md](../accepted/phase-4-plugin-harness.md)): stage open,
schema load, reference/payload resolve, asset-resolver behaviour, layer-stack
integrity, metadata + material-binding preservation.

## Matrix

A naïve `Maya × Houdini × USD × OS × renderer` product explodes into thousands of
cells. Instead, a **matrix cell** is a single, production-guaranteed runtime set
(host + platform + profile + pinned runtime + tier + support line). DCC versions
are **support lines** (`current / supported / maintenance`), not bare strings.
Cross-DCC data contracts are **edges** (`from` cell → `to` cell + USD checks), so
the matrix is a graph, not a Cartesian product.

Tiers gate where each cell/edge runs (Tier 0 local/core · Tier 1 PR gate, primary
cells · Tier 2 nightly cross-DCC · Tier 3 release · Tier 4 legacy/best-effort).
Recommended primary cells: `maya-2026 + linux + MayaUSD + production`,
`houdini-21.0 + linux + Solaris + production`.

## Command surface

```text
# Hosts (third-party installs)
ost host discover [--host maya] [--roots …] [--path …] [--refresh] [--register]
ost host list
ost host inspect <instance-id>
ost host probe <instance-id> [--fingerprint deep] [--output <file>]
ost host run  <instance-id> -- <cmd …>          # composed env, exit-code passthrough
ost host test <instance-id> --tool <t> --suite <s>

# Per-host artifacts (reuse the existing verbs with a host selector)
ost build|test|package <tool> --host <instance-id>     # or --cell <cell-id>
ost package <tool> --all-hosts

# Matrix & compatibility
ost matrix list | show <cell-id> | status | resolve --tier <t> | test --tier <t>
ost compat verify --edge <edge-id> | --from <cell-id> --to <cell-id>
ost compat report <tool> [--json]
```

Host **selectors** accept a stable instance id (preferred), an exact path,
`--host <family> --path <p>`, or a logical selector resolved **only when
unambiguous** — an ambiguous selector fails with the candidate list, never
silently picks by path order.

## Repo-fit decisions

This direction adopts the external DCC briefs' good architecture (providers,
validate/fingerprint, adapter capability model, tiered matrix + cross-DCC edges)
but reconciles them to this repo:

- **Rust crate, not Python modules.** A new `ost-host` crate (host record model,
  selectors, inventory, discovery providers, `HostValidator` / `HostAdapter`
  traits, per-DCC modules) mirroring `ost-plugin` / `ost-execution`. Platform
  specifics stay at provider boundaries.
- **One output contract.** `--json` uses the shipped envelope and category exit
  codes ([json-schema.md](../../reference/json-output.md)) — *not* the briefs' ad-hoc
  `0/1/2/3/4`. `ost host discover` is diagnostic, so it exits `0` even when it
  finds nothing (like `doctor`); real failures use the category codes
  (`usage` 2, `precondition` 4, `io` 7, …). A persisted host record carries its
  own `schema_version` (as runtime.json / plugin-report do), distinct from the
  envelope `schema`.
- **Reuse, don't reinvent.** Launch envs come from the runtime `EnvSet` (no
  hidden mutation); cross-DCC USD checks reuse the plugin harness levels; digests
  reuse the existing fingerprint discipline; CI is template generation, not a CI
  product (§13, Phase 5).
- **Declarative config only.** Discovery roots/rules are declarative TOML
  (`[host.discovery]`), bounded depth, never `/`-wide, never executing patterns
  or sourcing shell.

## Phases

Each phase is a usable increment; later phases never block the first.

1. **Foundation** — `ost-host` crate: record model, inventory, explicit-path /
   configured-roots / known-roots providers, **Maya** validator (fixtures, no
   GUI), `ost host discover --host maya`, JSON envelope, candidate/validated/
   rejected statuses, minimal cache + `--refresh`, tests on fixture install
   trees (no real DCC needed).
2. **Multi-host discovery** — Houdini + Nuke validators, environment +
   executable-PATH providers, custom declarative rules, cache invalidation,
   `ost host list|inspect`, JSONL.
3. **Headless run & package** — `ost host run` (composed env), host packaging
   (Maya `.mod`, Houdini package JSON), `ost host test` smoke suites, per-host
   `ost build|test|package --host`.
4. **Matrix & compatibility** — matrix cells / support lines / tiers, cross-DCC
   USD compatibility edges (reusing harness levels), `ost matrix …` /
   `ost compat …`, compatibility report; CI templates + tier wiring.
5. **Fleet & productization** — `ost host probe` deep fingerprints, fleet
   inventory export/import, `ost compat diff` / `ost reproduce`, support-line
   deprecation, optional Blender adapter.

First-PR shape (narrow but production-shaped): `ost host discover --host maya`
over explicit/configured/known roots, validating `bin/maya`/`bin/mayapy`
fixtures without a GUI, deterministic human + JSON output, the status model, a
minimal cache + `--refresh`, with unit + integration tests and documented config.
No generic plugin matrix, fleet service, remote execution, or DCC UI in that PR.

## Positioning

> OpenStrata is a runtime-centric platform. Its first-class apps run on the
> OpenStrata certified runtime; existing DCCs are supported as third-party hosts
> whose environment, artifacts, headless tests, and cross-DCC USD compatibility
> OpenStrata manages reproducibly and machine-readably — pushing each DCC's
> differences into a host adapter rather than erasing them behind a fake API.
