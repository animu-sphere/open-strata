# OpenStrata Documentation

Documentation is organized by responsibility: each category answers one primary
class of question. When a summary disagrees with the canonical design spec, the
spec wins and the summary is a bug.

| Category | Answers | Start here |
| --- | --- | --- |
| [concepts/](concepts/) | What OpenStrata is and the ideas it is built on. | [overview.md](concepts/overview.md) |
| [architecture/](architecture/) | How the current system is structured (crates, domain model, on-disk layout). | [overview.md](architecture/overview.md) |
| [guides/](guides/) | How to accomplish a task (command tour, migrations). | [examples.md](guides/examples.md) |
| [reference/](reference/) | Factual contracts: `--json` output, schemas, exit codes. | [json-output.md](reference/json-output.md) |
| [roadmap/](roadmap/) | What is planned next (only incomplete work). | [README.md](roadmap/README.md) |
| [releases/](releases/) | Immutable per-version release records. | [README.md](releases/README.md) |
| [design/](design/) | Why significant decisions were made (proposed / accepted / superseded). | [README.md](design/README.md) |
| [reports/](reports/) | Evidence from real runs (incidents, dogfooding). | [reports/](reports/) |
| [contributing/](contributing/) | How to write and maintain documentation. | [documentation.md](contributing/documentation.md) |

## Canonical design spec

[design/spec.md](design/spec.md) is the long-form canonical specification. The
category documents above are navigable, current-state summaries that track what is
actually implemented; the spec is the source of truth when they disagree.

## Reorganization status

The documentation is being reorganized in phases (see the reorg plan). Phase 2
established these category boundaries and moved existing documents into them
without major content rewriting. Decomposing the large hubs — the
[roadmap](roadmap/README.md) into `current` / `backlog`, and
[architecture](architecture/overview.md) into `overview` / `crates` — is the next
phase; the monolithic files remain in place until then.
