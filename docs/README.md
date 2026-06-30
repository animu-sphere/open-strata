# OpenStrata Documentation

| Document | Purpose |
| --- | --- |
| [overview.md](overview.md) | What OpenStrata is, who it is for, and the core principles (方針). |
| [architecture.md](architecture.md) | Workspace layout, crate boundaries, and the domain model. |
| [examples.md](examples.md) | Copy-pasteable examples for every `ost` command. |
| [json-schema.md](json-schema.md) | The `--json` output contract: envelope, error codes, exit codes, and compatibility policy. |
| [incident-notes.md](incident-notes.md) | Short debugging notes for incidents, root causes, fixes, and future guardrails. |
| [roadmap.md](roadmap.md) | Phased delivery plan and current status. |
| [phase-4-plugin-harness.md](phase-4-plugin-harness.md) | Phase 4 direction: the OpenUSD plugin verification harness, mapped onto the codebase. |
| [dcc-hosts.md](dcc-hosts.md) | Direction: third-party DCC host support (Maya/Houdini/Nuke) — discovery, headless run/package, and cross-DCC USD compatibility. |
| [kubernetes.md](kubernetes.md) | Phase 9 direction: Kubernetes as a pluggable execution backend (`ost submit` / `ost jobs`). |
| [design.md](design.md) | The full canonical design specification (source of truth). |

`design.md` is the long-form spec. The other documents are navigable summaries
that stay in sync with what is actually implemented; when they disagree with the
spec, the spec wins and the summary is a bug.
