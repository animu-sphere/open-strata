# Release process

How to cut an OpenStrata (`ost`) release. Releasing is two stages: a
**prepare-release** pull request (version + attribution + docs), then a **tag**
that triggers the cargo-dist build and publishes the GitHub Release.

Follow this checklist every time — the easy things to forget are the
`THIRD_PARTY_NOTICES.md` regeneration and the `Cargo.lock` refresh.

## 0. Preconditions

- `main` is green (fmt, clippy, test, licenses, docs).
- Decide the new version `X.Y.Z` (SemVer; pre-1.0, so a feature slice is a minor
  bump and fixes are a patch bump).
- If the release includes a **toolchain / MSRV** change, that is a deliberate,
  separate step: bump `channel` in [`rust-toolchain.toml`](../../rust-toolchain.toml)
  and keep `rust-version` in [`Cargo.toml`](../../Cargo.toml) (the MSRV floor) in
  sync. Do this in its own commit, ideally ahead of the release.

## 1. Prepare-release pull request

Create a branch (e.g. `release/vX.Y.Z`) and make these changes together. This
mirrors the existing `chore: prepare vX.Y.Z release` commits.

### 1a. Bump the workspace version

The version is defined once in the root [`Cargo.toml`](../../Cargo.toml) under
`[workspace.package]`; every crate inherits it via `version.workspace = true`.
Edit that single line:

```toml
[workspace.package]
version = "X.Y.Z"
```

### 1b. Refresh `Cargo.lock`

The workspace crates' own entries in `Cargo.lock` carry the version, so the lock
must be regenerated:

```bash
cargo build            # or: cargo check --workspace
```

Confirm the only `Cargo.lock` change is the version lines for the `ost-*` crates
(no unexpected dependency drift). If a dependency genuinely changed, that belongs
in its own commit reviewed against the licenses gate.

### 1c. Regenerate `THIRD_PARTY_NOTICES.md`

Attribution for bundled/linked/distributed third-party crates must match the
dependency graph — the `licenses` CI job fails a stale file. Regenerate it with
`cargo-about`:

```bash
cargo about generate about.hbs --output-file THIRD_PARTY_NOTICES.md
```

**CRLF gotcha:** some upstream license texts ship with CRLF line endings, but the
repo file is LF; a raw regeneration can introduce `\r` and show a spurious diff
(and the `licenses` job normalizes CRLF→LF before diffing, so a local mismatch is
easy to miss). Strip carriage returns when regenerating:

```bash
cargo about generate about.hbs | tr -d '\r' > THIRD_PARTY_NOTICES.md
```

`cargo-about` and `cargo-deny` versions are pinned in the `licenses` workflow;
their output formatting can change between releases, so if the file churns for no
dependency reason, match the pinned versions locally.

### 1d. Update documentation

- **Add a release record:** [`docs/releases/vX.Y.Z.md`](../releases/) — objective,
  shipped capabilities, compatibility notes, and known limitations. Add it to the
  [releases index](../releases/README.md). Release records are immutable history
  once the version ships.
- **Update the roadmap:** in [`docs/roadmap/`](../roadmap/), mark the milestone
  delivered and remove now-completed work from the active roadmap (its detail
  lives in the release record). The roadmap holds only incomplete work.
- **Update the README:** set the current-release line in the
  [Status](../../README.md) section.
- **Check links:**

  ```bash
  python3 scripts/check_doc_links.py .
  ```

### 1e. Land it

Commit as `chore: prepare vX.Y.Z release`, open a pull request, and merge once the
full gate is green (fmt, clippy, test, licenses, docs).

## 2. Tag and publish

The release build runs on a version tag, **not** on merge. After the prepare PR is
merged, from an updated `main`:

```bash
git checkout main && git pull
git tag vX.Y.Z            # tag pattern: **[0-9]+.[0-9]+.[0-9]+*
git push origin vX.Y.Z
```

Pushing the tag triggers [`release.yml`](../../.github/workflows/release.yml)
(cargo-dist), which:

- builds binaries for `x86_64-unknown-linux-musl` (fully static, old-glibc
  portable), `aarch64-apple-darwin`, `x86_64-apple-darwin`, and
  `x86_64-pc-windows-msvc`;
- generates `shell` + `powershell` installers and checksums;
- bundles `NOTICE` + `THIRD_PARTY_NOTICES.md` into every archive (configured in
  [`dist-workspace.toml`](../../dist-workspace.toml));
- attaches SLSA build-provenance attestations;
- creates the GitHub Release with all assets.

## 3. Verify the release

- The GitHub Release exists with binaries, installers, and checksums for all four
  targets.
- Provenance verifies:

  ```bash
  gh attestation verify <asset> --repo animu-sphere/open-strata
  ```

- Smoke-test an installer (`ost --version` reports `X.Y.Z`).

## Special cases

- **Bumping `cargo-dist-version`.** `dist-workspace.toml` sets `allow-dirty =
  ["ci"]` because the generated `release.yml` is hand-edited to pin every
  third-party action to a full commit SHA (harness §SEC-004). After a dist bump,
  regenerate the workflow with `dist` and **re-pin every action to a full SHA** by
  hand before releasing. Tracked under the roadmap
  [security baseline](../roadmap/README.md#security-baseline).
- **New third-party dependencies.** The `licenses` job (`cargo-deny`) must pass on
  permissive licenses/advisories, and `THIRD_PARTY_NOTICES.md` must be regenerated
  (step 1c).
- **No `CHANGELOG.md` yet.** Release records under [`docs/releases/`](../releases/)
  are the user-facing history for now; a root `CHANGELOG.md` is a planned addition.

## Checklist

```text
- [ ] main is green
- [ ] version bumped in Cargo.toml [workspace.package]
- [ ] Cargo.lock refreshed (only ost-* version lines changed)
- [ ] THIRD_PARTY_NOTICES.md regenerated (CRLF stripped)
- [ ] docs/releases/vX.Y.Z.md added + indexed
- [ ] roadmap updated (completed work removed)
- [ ] README current-release line updated
- [ ] doc link check passes
- [ ] prepare-release PR merged with full CI green
- [ ] tag vX.Y.Z pushed
- [ ] release.yml succeeded; assets + attestations present
- [ ] installer smoke test (ost --version)
```
