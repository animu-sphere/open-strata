# JSON output contract

Every `ost` command accepts `--json` for machine-readable output. Under `--json`
the output is a **stable, versioned contract** (design §14.3/§14.4): agents and
CI can branch on structured fields without parsing human prose. This document is
the reference for that contract and its compatibility policy.

## Envelope

With `--json`, a command prints exactly **one** JSON document to stdout. Progress,
warnings, and diagnostics go to stderr; no colours, spinners, or banners are
mixed into the stdout document, so pipes stay clean.

Every envelope carries the same top-level shape:

| Field | Type | Meaning |
| --- | --- | --- |
| `ok` | bool | `true` on success, `false` on failure or a failed report. |
| `schema` | integer | Output contract version. Currently `1`. |
| `data` | object | Present when `ok` is `true` (and on report failures). The command's result. |
| `error` | object | Present when a command fails outright (see [Errors](#errors)). |
| `warnings` | array | Non-fatal notes. Empty when there are none. |

### Success

```json
{
  "ok": true,
  "schema": 1,
  "data": {
    "runtimes": []
  },
  "warnings": []
}
```

### Report commands

Some commands *are* a pass/fail report: `build --check`, `validate`,
`lock --check`, `doctor`, `runtime validate`, and `ost plugin inspect|build|test`.
These still return a single envelope; the top-level `ok` carries the outcome and
`data` carries the detail. The process exit code reflects the failure category
(see below), so an agent can branch on either `ok` or the exit code.

```json
{
  "ok": false,
  "schema": 1,
  "data": {
    "target": "cy2026-windows-x86_64-py313-usd",
    "checks": [
      { "name": "CMakeLists.txt", "status": "ok", "detail": "/abs/CMakeLists.txt" },
      { "name": "runtime", "status": "failed",
        "detail": "runtime '…' not pulled", "hint": "run `ost runtime pull …` first" }
    ]
  },
  "warnings": []
}
```

### Failure

When a command cannot produce a result it prints the failure envelope — still a
single JSON document on stdout — and exits non-zero:

```json
{
  "ok": false,
  "schema": 1,
  "error": {
    "code": "RUNTIME_NOT_FOUND",
    "category": "precondition",
    "message": "runtime 'openstrata-cy2026-…-usd' not pulled — run `ost runtime pull cy2026 --profile usd` first",
    "hint": "use --from-usd or --build when a real OpenUSD runtime is required"
  },
  "warnings": []
}
```

`error.hint` is present only when an actionable hint exists. Branch on
`error.code` and `error.category`, never on `error.message`.

## Errors

### Categories and exit codes

Every failure has a `category` that determines the process exit code. The raw
exit code of any child process (CMake, Ninja, …) is preserved separately from the
CLI's own normalized code.

| Category | Exit | Meaning |
| --- | ---: | --- |
| `usage` | 2 | Bad arguments or usage. |
| `configuration` | 3 | Invalid manifest, lock, or config file. |
| `precondition` | 4 | A missing prerequisite: runtime, tool, directory. |
| `validation` | 5 | A validation mismatch (`validate`, `lock --check`, plugin tests). |
| `external_tool` | 6 | An external tool failed (CMake, Ninja, compiler, OpenUSD). |
| `io` | 7 | Filesystem or permission error. |
| `internal` | 70 | An unexpected internal error. |

Exit `0` is success or a no-op. Report commands (`doctor`, `validate`,
`lock --check`, `build --check`, plugin checks) exit `0` when they pass and the
relevant failure code otherwise — `validate` / `lock --check` / plugin checks use
`validation` (5); `doctor` / `build --check` use `precondition` (4).

### Codes

`code` is a stable, screaming-snake-case identifier. The initial set:

| Code | Category |
| --- | --- |
| `INVALID_ARGUMENT` | usage |
| `PLATFORM_NOT_FOUND` | usage |
| `PROJECT_EXISTS` | usage |
| `INVALID_CONFIG` | configuration |
| `MANIFEST_INVALID` | configuration |
| `PARSE_FAILED` | configuration |
| `PROJECT_NOT_FOUND` | precondition |
| `RUNTIME_NOT_FOUND` | precondition |
| `REQUIRED_TOOL_MISSING` | precondition |
| `REAL_RUNTIME_REQUIRED` | precondition |
| `PRECONDITION_FAILED` | precondition |
| `VALIDATION_FAILED` | validation |
| `EXTERNAL_TOOL_FAILED` | external_tool |
| `IO_ERROR` | io |
| `INTERNAL_ERROR` | internal |
| `OPERATION_FAILED` | precondition |

`OPERATION_FAILED` is a transitional, generic code for failures not yet migrated
to a specific code; it defaults to the `precondition` category. New code should
use a specific code, so agents should prefer branching on `category` where a
precise code is not yet guaranteed.

## Compatibility policy

The `--json` contract is additive and versioned. Within a `schema` version:

- existing fields are not removed or repurposed;
- new fields may be added to `data`, `error`, or a command's output;
- new `code` values and new `warnings` may appear, so consumers must tolerate
  unknown codes (fall back to `category`) and ignore unknown fields;
- category-to-exit-code mappings are stable.

A breaking change to the envelope shape bumps `schema`. Because new codes and
fields can appear without a version bump, treat `code`/`category`/`status`
matching as open sets, not exhaustive enums.

## Agent flow

A safe observe → plan → act sequence, branching on exit codes and `ok`:

```bash
ost doctor --json            # environment & prerequisites (exit 4 if a tool/runtime is missing)
ost build --check --json     # preflight only, no writes (exit 4 if a precondition fails)
ost build --dry-run --json   # planned commands + files, no writes
ost build --json             # the real build
ost validate --json          # structural checks (exit 5 on mismatch)
ost lock --check --json      # lockfile drift (exit 5 if stale)
```
