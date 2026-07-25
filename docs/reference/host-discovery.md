# DCC host discovery contract

`ost host discover | list | inspect` finds third-party DCC installs already on
the machine, confirms what they are, and records the result. OpenStrata never
installs, updates, licenses, or modifies a host, and it does not abstract the
products behind a shared API. What it owns is the *record*: where an install
was found, what confirmed it, which version and Python ABI it has, and how much
that answer is worth.

Supported families: `maya` (Autodesk Maya), `houdini` (SideFX Houdini).

## The two phases

Discovery mirrors the plugin harness's "candidate → validated, never a false
PASS":

1. **Candidate discovery** — a provider *suggests* a root. Providers are
   composable and order-stable, and are deduplicated by canonical root while
   retaining every provenance.
2. **Validation** — a family validator confirms markers and executables,
   resolves the version, reads the embedded Python, and produces a deterministic
   instance id and fingerprint. Validation is read-only, bounded, and never
   interactive: it does not source shell setup, launch a GUI, write anything
   inside a host install, or let one unreadable candidate sink a scan.

## Providers

In resolution order. What an operator names beats what a project configures,
which beats what the environment hints, which beats a documented default.

| Source | Origin | Default |
| --- | --- | --- |
| `explicit-path` | `--path <PATH>` | on when passed |
| `configured-root` | `[host.discovery].roots`, `--root <PATH>` | on when declared |
| `environment` | `MAYA_LOCATION`, `HFS` | on (`--no-environment` disables) |
| `known-install-root` | this platform's documented install locations | on (`--no-known-roots` disables) |
| `executable-path` | an anchor executable on `PATH` | **off** (`--from-path` enables) |

`executable-path` is opt-in because `PATH` reflects the shell that happened to
invoke `ost`, not the machine's installs; letting it contribute silently would
make discovery depend on the caller's environment.

Documented install locations, scanned one level deep with a name filter:

| Platform | Maya | Houdini |
| --- | --- | --- |
| Windows | `%ProgramFiles%\Autodesk\Maya*` (and `%ProgramW6432%`) | `%ProgramFiles%\Side Effects Software\Houdini*` |
| Linux | `/usr/autodesk/maya*` | `/opt/hfs*`, `/opt/sidefx/*`, `/usr/local/sidefx/*` |
| macOS | `/Applications/Autodesk/maya*` | `/Applications/Houdini/Houdini*` |

A root reached by several providers is **one** record carrying all of them: an
install found three ways is one install with three provenances, and dropping two
would hide how a site is actually configured.

## Declaring discovery roots

Roots are declared in the project manifest and are **declarative only** — no
globs are executed, no shell or environment expansion is performed, and no rule
may name a filesystem root:

```toml
[host.discovery]
roots = ["/tools/dcc", "/mnt/site/apps"]
max_depth = 2
families = ["maya", "houdini"]
```

| Key | Meaning |
| --- | --- |
| `roots` | Absolute site directories holding installs. |
| `max_depth` | How far below each root an install may sit; 1–4, default 2. |
| `families` | Restrict discovery to these families; empty means all. |

A root is rejected at manifest-parse time — before any filesystem is touched —
when it is empty, relative, a filesystem root, contains `..`, contains glob
metacharacters, or needs `$`/`~`/`%` expansion. The check runs against the
declared text, so a Windows root is rejected on Linux too.

A scan is additionally bounded by a directory budget (4096 across all
providers). Exhausting it emits `HOST_SCAN_BUDGET_EXHAUSTED` rather than
continuing.

## Status model

| Status | Meaning |
| --- | --- |
| `candidate` | A provider suggested this root; no validator has confirmed it. |
| `validated` | Confirmed markers, executables, and identity. **The only usable status.** |
| `rejected` | A validator looked and refused; `rejection.code`/`message` say why. |
| `stale` | Previously validated, but the install's bytes have changed since. |
| `unreachable` | Previously validated, and the root is no longer readable. |
| `invalidated` | Recorded under a record schema or fingerprint input set this `ost` no longer honours. |

A rejection is reported for a root the operator *named* (`--path`, a vendor
environment variable). A root that merely turned up in a scan is skipped
silently — otherwise scanning a site root would report every directory on the
machine as "not Maya".

## Version resolution and confidence

Resolved least-invasively, recording which rung answered:

| Source | How | Confidence |
| --- | --- | --- |
| `metadata` | A file the vendor ships: Maya's `include/maya/MTypes.h` (`MAYA_APP_VERSION`, else the leading year of `MAYA_API_VERSION`), Houdini's `toolkit/include/SYS/SYS_Version.h` (`SYS_VERSION_FULL`). | `high` |
| `probe` | The host's own version banner, run under a bounded timeout. Opt-in via `--probe`. Maya is asked with `maya -v`; Houdini with `husk --version`, because `hython --version` reports *Python's* version and would be a confidently wrong answer. | `high` |
| `directory-name` | The install directory's name (`maya2026`, `hfs20.5.550`, `Houdini 22.0.386.4`). | `low` |

A directory name is a site convention, not a guarantee: it can be renamed, and
an install can be patched without it changing. Consumers that need certainty
must require `high`. When no rung answers, the record carries no version rather
than a guess.

`--probe` executes the host's own binary. It is off by default so that discovery
executes nothing, and it is bounded (5s) because a host that has not answered by
then is waiting on a license server or a display that discovery must not wait
for.

## Identity and fingerprint

The **instance id** (`maya-2026-1f4c9a2b`) is derived from family, version, and
canonical root — deliberately not from the fingerprint. Patching an install in
place keeps its identity, so a pin still names the same thing, while changing
its fingerprint, so the change is visible. Two differing ids are two installs;
one id whose fingerprint moved is one install that changed.

The **fingerprint** is a SHA-256 over a versioned, explicitly listed input set,
so what it covers is readable in one place and changing it is a deliberate
`inputs_version` bump rather than silent drift.

| Mode | Inputs |
| --- | --- |
| `standard` (default) | Family, version, canonical root, OS/arch, embedded Python, corroborating markers, and each selected executable's root-relative path, size, and mtime. Never hashes an install. |
| `deep` (`--fingerprint deep`) | The above plus the SHA-256 of each selected executable's contents. |

## Inventories

Two inventories share one document shape
([`host-inventory.schema.json`](../../schemas/host-inventory.schema.json)):

- the machine-wide cache at `$OST_HOME/cache/host-discovery/inventory.json`,
  written by every `discover`;
- the project's reviewable `.strata/hosts/inventory.json`, written by
  `discover --register`.

Both are written atomically. An inventory records a *past observation*, so every
read re-checks the installs it names — root readable, executables unchanged in
size and mtime, record schema and fingerprint input set still current — before
serving any of them as usable. A record is downgraded to `stale`, `unreachable`,
or `invalidated` rather than served stale.

`discover` keeps cached records the current scan did not re-find, because a
narrow scan must not silently delete hosts a wider one recorded earlier.
`--refresh` is how an operator says "forget what you knew".

`cached_at` is cache metadata only; it takes no part in any record's identity or
fingerprint, so re-running discovery on an unchanged machine produces identical
records.

## Selectors

`ost host inspect <SELECTOR>` accepts, in order: an exact instance id, an
install path, an instance-id prefix, or a family name. The last three resolve
**only when they name exactly one install**; an ambiguous selector fails with
the candidate list and is never resolved by scan order.

## Exit behaviour and stable codes

`discover` and `list` are diagnostic, like `doctor`: finding nothing is an
answer and exits `0`. Only a real failure takes a
[category exit code](exit-codes.md).

| Code | Category | Exit | Meaning |
| --- | --- | --- | --- |
| `HOST_FAMILY_UNKNOWN` | usage | 2 | A family selector this `ost` does not support; the hint names the supported ones. |
| `HOST_STATUS_UNKNOWN` | usage | 2 | An unsupported `--status` filter. |
| `HOST_FINGERPRINT_MODE_UNKNOWN` | usage | 2 | An unsupported `--fingerprint` mode. |
| `HOST_SELECTOR_AMBIGUOUS` | usage | 2 | A selector matched several installs; the message lists their ids. |
| `HOST_INVENTORY_SCHEMA_UNSUPPORTED` | configuration | 3 | A persisted inventory uses an unknown schema. |
| `HOST_NOT_FOUND` | precondition | 4 | No record matches the selector. |
| `HOST_PROJECT_REQUIRED` | precondition | 4 | `--register` was used outside a project. |

Warnings carried in the envelope's `warnings` array:

| Code | Meaning |
| --- | --- |
| `HOST_NONE_VALIDATED` | Nothing validated; the message names which providers were consulted. |
| `HOST_INVENTORY_EMPTY` | No matching records; run `ost host discover` first. |
| `HOST_SCAN_BUDGET_EXHAUSTED` | The scan hit its directory budget and stopped. |
| `HOST_FINGERPRINT_UNAVAILABLE` | An install validated but could not be fingerprinted, so it stays a `candidate`. |

Rejection codes recorded on a record:

| Code | Meaning |
| --- | --- |
| `HOST_ROOT_UNREADABLE` | The named path is not a readable directory. |
| `HOST_MARKERS_MISSING` | No anchor executable for that family; the message names what was looked for. |

## Scope

This contract covers discovery, records, and inventories only. Launching a host,
composing its environment, host-standard packaging (Maya `.mod`, Houdini package
JSON), cross-DCC USD compatibility edges, and matrix cells are separate work —
see [dcc-hosts.md](../design/proposed/dcc-hosts.md). A host launch will go
through the same resolved [Formation](../guides/compose-a-formation.md)
environment as a runtime-native app rather than a parallel DCC-specific
mechanism.
