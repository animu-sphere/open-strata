# Contributing to documentation

Documentation changes are part of feature completion. A change is not done until
its relevant documentation reflects the implemented state.

## Pick the right category

Each document answers one primary class of question. Put a fact where it is owned:

| Content type | Destination |
| --- | --- |
| What OpenStrata is / core ideas | [concepts/](../concepts/) |
| How the current system is structured | [architecture/](../architecture/) |
| How to accomplish a task | [guides/](../guides/) |
| Factual contracts (JSON, schemas, exit codes) | [reference/](../reference/) |
| Not-yet-complete future work | [roadmap/](../roadmap/) |
| A released version's record | [releases/](../releases/) |
| Why a decision was made | [design/](../design/) |
| Evidence from a real run | [reports/](../reports/) |

Do not mix historical implementation detail into current normative documentation.
Do not describe planned work as implemented, or implemented work as planned.

## One source of truth

A fact should have exactly one owner. Prefer generating or validating against the
source over copying it:

| Fact | Source of truth |
| --- | --- |
| Workspace version / crate list | Cargo workspace metadata / root `Cargo.toml` |
| CLI commands and flags | CLI parser definitions |
| JSON output structure | Rust types and schemas (`schemas/`) |
| CI lanes / support lines | `openstrata.ci.yaml` (the `ost-ci` model) |
| Current architecture | [architecture/](../architecture/) |
| Future plans | [roadmap/](../roadmap/) |
| Historical changes | [releases/](../releases/) |
| Design rationale | [design/](../design/) accepted records |
| Validation evidence | [reports/](../reports/) |

When you must repeat a fact, link to its owner instead of restating it.

Some reference pages are **generated** from these sources by `ost internal docs
generate` and drift-checked in CI — do not edit them by hand:
[reference/cli.md](../reference/cli.md) (clap command tree),
[reference/exit-codes.md](../reference/exit-codes.md) (`ost_core::Category`),
[reference/schemas.md](../reference/schemas.md) (`schemas/*.json`), and
[reference/support-matrix.md](../reference/support-matrix.md)
(`support/platforms.toml`). Change the source, then regenerate:

```bash
cargo run -q -p ost-cli -- internal docs generate
```

## Front matter and statuses

Documents whose lifecycle matters (design records, release records, and — once
decomposed — roadmap items) carry YAML front matter. Allowed `status` values:

**Design** (`design/`):

```text
proposed   accepted   superseded   rejected
```

**Roadmap** (`roadmap/`):

```text
candidate   planned   active   blocked
```

Do not use `completed` in the active roadmap — completed work moves to a
[release record](../releases/).

**Report** (`reports/`):

```text
draft   final
```

Example (design record):

```yaml
---
title: OCI Artifact Trust Policy
status: proposed
owners:
  - openstrata-maintainers
created: 2026-07-11
updated: 2026-07-11
tracking_issue: 123
applies_to: v0.13+
---
```

## Adding or changing documents

- **New design record:** add it under `design/proposed/` with `status: proposed`
  front matter; promote to `design/accepted/` when the decision is taken. Do not
  silently rewrite an accepted record — supersede it.
- **Updating the roadmap:** keep only incomplete work. When a milestone ships,
  move its detail into a [release record](../releases/) and remove it from the
  roadmap.
- **Preparing a release note:** add `releases/vX.Y.Z.md` with objective, shipped
  capabilities, compatibility notes, and known limitations. Release records are
  immutable history once written for a released version.
- **Reports** capture evidence: state the date, tested commit/version,
  environment, procedure, observed result, and limitations.

## Previewing and checking

- Preview locally by rendering the Markdown (any Markdown viewer / your editor).
- Run the documentation checks before opening a pull request:

  ```bash
  python3 scripts/check_doc_links.py .          # relative links + anchors resolve
  python3 scripts/check_docs_consistency.py .   # crate/version/roadmap + hygiene
  cargo run -q -p ost-cli -- internal docs generate  # refresh generated reference
  ```

  The `docs` CI workflow runs the link and consistency checks on every push and
  pull request, and `ci.yml` fails on generated-reference drift — so a broken
  link, a stale crate/version/roadmap fact, trailing whitespace, or an
  out-of-date generated page all fail the build.

## Definition of done (documentation)

- [ ] User-facing behavior is documented.
- [ ] Architecture documentation reflects the implemented state.
- [ ] Roadmap items were updated or removed.
- [ ] Migration notes were added when required.
- [ ] Release note entry prepared (for a release).
- [ ] No new duplicated source of truth was introduced.
- [ ] Relative links and anchors resolve.
