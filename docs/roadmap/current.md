# Current

The next milestone and active carry-over work. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## Next milestone: v0.13.0 — release-quality packaging

**Status:** planned · **Depends on:** v0.10.0 producer verb + v0.12.0 consumer
path (`ost plugin package` → `artifact extract` → relocated run), both shipped.

Dogfooding the clean-install / packaging path proved the consumer flow is solid,
but surfaced the gaps that block a reproducible, distributable **release** (the
downstream usd-vrm-plugins PR3 release lane): packaging is **not byte-reproducible**
and the shipped artifact still carries **debug symbols**. Close the
release-quality boundary so the produced artifact is reproducible, lean, and
testable from the package — the prerequisite for the trust arc (v0.14.0+).

**Scope:**

- **Deterministic packaging.** `ost plugin package` honors `SOURCE_DATE_EPOCH`
  and normalizes entry mtimes, ordering, and permissions, so the archive is a
  pure function of the staged files — a byte-identical digest across repeat runs
  of an unchanged build.
- **Symbol-split artifacts.** The default package ships lean (no `.pdb` / debug
  symbols); debug symbols are opt-in via `--with-debug` or split into a sibling
  `*-debug` package.
- **External-plugin-path run.** A documented primitive to point a session at an
  arbitrary installed/extracted plugin tree without injecting a bundle's own
  path: `ost plugin run --plugin-path <dir>` / `--no-inject` (and/or an
  `ost runtime run` that sets the USD env only), for clean-install / discovery
  testing.
- **Packaged-artifact smoke.** `ost plugin test --from-package` extracts the
  just-built package and runs discovery / open / validate against it, catching a
  build-tree path baked into `plugInfo` / `LibraryPath` that source-bundle L2
  discovery cannot see.
- **`ost artifact extract --into <DEST>` alias**, aligning the verb with
  `ost artifact export`.

**Tracks:** SEC-005 (reproducible-release + stable-checksum groundwork) and the
downstream usd-vrm-plugins PR3 release lane. Trust-level enforcement on top of
these artifacts follows in v0.14.0 (trust policy foundation).

## Carry-over follow-ups

Small open items not tied to the milestone ladder:

- **Republish the public macOS runtime (from v0.12.0).** Republish the public
  cy2026 macOS arm64 OpenUSD 26.05 SDK with preserved executable bits and confirm
  a clean GitHub-hosted `macos-15-arm64` source-CI lane reaches
  `ost plugin test --up-to 5` with no `chmod` repair; then remove the temporary
  repo-local `actions/setup-python` + `chmod` repairs in the downstream fixture
  repo. Needs a Mac + live GHCR. See [releases/v0.12.0.md](../releases/v0.12.0.md).
- **GHCR push round-trip (from v0.11.0).** Confirm the `ost artifact push`
  user/password path against GHCR end to end. Needs live credentials.
- **SEC-002 follow-up — symlink escape inside a bundle.** Reject a *real* symlink
  within a bundle that resolves outside the root at read time
  (canonicalize-and-contain), complementing the lexical manifest check.
