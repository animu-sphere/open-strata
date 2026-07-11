# Reference

Factual, low-narrative contracts: output formats, schemas, exit codes. Several
pages are **generated** from their source (the clap command tree, the exit-code
`Category`, and the JSON schemas) by `ost internal docs generate` and checked for
drift in CI — do not edit them by hand.

| Document | Source | Purpose |
| --- | --- | --- |
| [cli.md](cli.md) | generated (clap command tree) | Every `ost` command, its arguments and options. |
| [exit-codes.md](exit-codes.md) | generated (`ost_core::Category`) | The category → exit-code contract. |
| [schemas.md](schemas.md) | generated (`schemas/*.json`) | The JSON Schemas `ost` validates documents against. |
| [support-matrix.md](support-matrix.md) | generated (`support/platforms.toml`) | Per-feature, per-platform support levels. |
| [json-output.md](json-output.md) | hand-written | The `--json` output contract: envelope, error codes, and compatibility policy. |
| [artifact-policy.md](artifact-policy.md) | hand-written | Artifact trust levels, policy TOML schema, matching rules, and stable errors. |

To regenerate the generated pages, from the repository root:

```bash
cargo run -q -p ost-cli -- internal docs generate
```

Planned (later phases): a generated `environment-variables.md`.
