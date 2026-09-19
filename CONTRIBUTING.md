# Contributing to OpenStrata

Thank you for taking an interest in OpenStrata. Contributions of every size are
welcome: reporting a problem, improving documentation, sharing an idea, or
opening a pull request.

## Getting started

- Search existing issues before opening a new one. Use the issue forms when
  they fit; a clear, short report is enough to start a conversation.
- For a larger change, open an issue first so we can agree on the direction.
- Keep pull requests focused. Explain what changed and why, and include tests
  when behavior changes.

Before submitting code, run the checks that apply to your change:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Documentation changes should follow the
[documentation guide](docs/contributing/documentation.md). Contributor-only
release steps are in the [release process](docs/contributing/release-process.md).

## Community and reporting

Please follow the [Code of Conduct](CODE_OF_CONDUCT.md) in all project spaces.
To report behavior that does not meet it, email
**piggypenguin583@gmail.com**. Repository administrators accept and review
these reports privately.

For security vulnerabilities, use the private process in
[SECURITY.md](SECURITY.md), not a public issue.

## What to expect

Maintainers review contributions on a best-effort basis. We may ask questions,
request a small adjustment, or suggest a different approach. A contribution
does not need to be perfect before you open it.
