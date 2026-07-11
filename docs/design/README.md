# Design

Why significant technical decisions were made. Design documents follow a
lightweight lifecycle:

```text
proposed → accepted → superseded (or rejected)
```

- **proposed** — a direction under consideration; not yet built.
- **accepted** — the decision was taken; it is implemented or actively being built.
- **superseded** — replaced by a later decision (kept for history).

Accepted documents are not silently rewritten. A material change creates a new
decision or explicitly marks the old one as superseded. Allowed status values are
defined in [contributing/documentation.md](../contributing/documentation.md).

## Canonical spec

| Document | Status | Purpose |
| --- | --- | --- |
| [spec.md](spec.md) | canonical | The long-form design specification — the source of truth for the summaries under [architecture/](../architecture/), [concepts/](../concepts/), and [reference/](../reference/). |

## Accepted

| Document | Purpose |
| --- | --- |
| [accepted/phase-4-plugin-harness.md](accepted/phase-4-plugin-harness.md) | The OpenUSD plugin verification harness, mapped onto the codebase (largely implemented). |
| [accepted/plugin-harness-source.md](accepted/plugin-harness-source.md) | Long-form directional design for the plugin verification harness. |
| [accepted/remote-artifact-transport.md](accepted/remote-artifact-transport.md) | Remote artifact transport + GitHub-hosted source CI (shipped in v0.9.0/v0.10.0). |

## Proposed

| Document | Purpose |
| --- | --- |
| [proposed/dcc-hosts.md](proposed/dcc-hosts.md) | Third-party DCC host support (Maya/Houdini/Nuke) — discovery, headless run/package, cross-DCC USD compatibility. |
| [proposed/kubernetes.md](proposed/kubernetes.md) | Kubernetes as a pluggable execution backend (`ost submit` / `ost jobs`). |
