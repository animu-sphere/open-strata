# OpenStrata Documentation

Documentation is organized by responsibility: each category answers one primary
class of question. When a summary disagrees with the canonical design spec, the
spec wins and the summary is a bug.

| Category | Answers | Start here |
| --- | --- | --- |
| [concepts/](concepts/) | What OpenStrata is and the ideas it is built on. | [overview.md](concepts/overview.md) |
| [projects/](projects/) | The reference projects (real downstream repositories) and the cross-repository ecosystem. | [README.md](projects/README.md) |
| [architecture/](architecture/) | How the current system is structured (crates, domain model, on-disk layout). | [overview.md](architecture/overview.md) |
| [guides/](guides/) | How to accomplish a task (command tour, migrations). | [examples.md](guides/examples.md) |
| [reference/](reference/) | Factual contracts: `--json` output, schemas, exit codes. | [json-output.md](reference/json-output.md) |
| [roadmap/](roadmap/) | What is planned next (only incomplete work). | [README.md](roadmap/README.md) |
| [releases/](releases/) | Immutable per-version release records. | [README.md](releases/README.md) |
| [design/](design/) | Why significant decisions were made (proposed / accepted / superseded). | [README.md](design/README.md) |
| [reports/](reports/) | Evidence from real runs (incidents, dogfooding). | [reports/](reports/) |
| [contributing/](contributing/) | Contributor procedures: documentation and releases. | [README.md](contributing/README.md) |

## Canonical design spec

[design/spec.md](design/spec.md) is the long-form canonical specification. The
category documents above are navigable, current-state summaries that track what is
actually implemented; the spec is the source of truth when they disagree.

## Reorganization status

The documentation is being reorganized in phases (see the reorg plan). Category
boundaries are established; the [roadmap](roadmap/) is decomposed into
`current` / `backlog` with per-version [release records](releases/) and a
[delivery history](reports/delivery-history.md) holding the granular detail.
[Architecture](architecture/) is split into `overview` + `crates`; the
[reference](reference/) CLI / exit-code / schema / support-matrix pages are
generated from source (`ost internal docs generate`) with a CI drift check; and
link, consistency (crate inventory / version / roadmap state), and hygiene checks
run in CI. Remaining: a documentation
[website](roadmap/backlog.md#documentation--tooling).
