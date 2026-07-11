# Release-lane CI and v0.14 dogfood intake

> Status: proposed. Intake decision for the usd-vrm-plugins v0.13.0 release-lane
> report dated 2026-07-12. This document separates immediate correctness fixes
> from the larger generated trusted-CI design already scheduled after the trust
> and provenance foundations.

## Outcome

The downstream release lane proves that v0.13.0 achieved its packaging goal:
unchanged inputs produce stable digests, symbol sidecars are split when present,
and packaged artifacts can be tested from a clean extracted layout on all three
operating systems.

The report does not require changing the v0.14.0 objective. v0.14 remains the
artifact trust-policy foundation. Three small failures exposed by the release
lane are correctness/trust hardening and should accompany that milestone:

1. reject unknown support-matrix keys instead of silently dropping them;
2. write a native Windows directory to `GITHUB_PATH` in generated bootstrap;
3. resolve a bare Python command against the runtime Python ABI in
   `ost plugin run`.

Full tag-triggered release-lane generation is a larger policy surface. It belongs
with generated trusted CI after trust and provenance exist, not as an arbitrary
`extra_steps` escape hatch in v0.14.

## Current facts

- `source_checks` already preserves repository-specific post-pyramid commands in
  generated pull-request/main source lanes. It shipped in v0.11.0 and is not an
  open corpus-smoke gap.
- `bootstrap.ost.sha256` already supports exact per-target OST release-asset
  pins. The missing part is safe pin maintenance, not the data model.
- The generated `pinned=""` branch is a deliberate fallback to the release's
  downloaded checksum sidecar, not an unfinished bootstrap. It is weaker than an
  independently authored exact pin and should not be used for trusted release
  lanes.
- Generated source CI is Bash-based on every OS. Its bootstrap currently writes
  a POSIX-style MSYS path to `GITHUB_PATH`; later native process creation on
  Windows cannot reliably consume that path.
- Plugin build/test code already has runtime-aware host-Python resolution.
  `ost plugin run` bypasses it and launches the user's token verbatim.
- `ost ci` uses Serde's default unknown-field behavior, so a misspelled cell key
  can validate and then disappear from generated output.
- v0.13 symbol splitting is correct. The observed tiny PDB was stale input, not a
  packaging failure; clean Release builds produced no sidecar.

## Intake decisions

| Report ask | Decision | Target |
| --- | --- | --- |
| Repository-specific release steps/lanes | split | Existing `source_checks` closes ordinary corpus smoke. Model release lanes later as typed trusted CI, not raw cell hooks. |
| Reject unknown cell keys | accept, required | v0.14 hardening |
| Bootstrap version/checksum update helper | accept, non-blocking | v0.14 if capacity; otherwise the generated trusted-CI slice |
| Carried corpus CTest smoke | close | Already delivered by `source_checks`; correct stale backlog text. |
| Native Windows `GITHUB_PATH` | accept, required | v0.14 hardening |
| ABI-matched `plugin run -- python` | accept, required | v0.14 hardening |
| Warn on stale PDB | retain as diagnostic candidate | Packaging backlog; warning only until symbol identity can be checked. |
| Publish an OpenUSD 25.05 runtime | operational artifact work | Runtime publishing/trusted publisher lane, not a new CLI feature. |

## Strict CI manifest policy

Unknown keys in a versioned CI contract are errors. Silently ignoring a key is
unsafe because the author may believe a verification or publication control is
active when the generator never saw it.

Apply strict deserialization recursively to every authored support-matrix object,
including the matrix, bootstrap blocks, runner profiles, cells, host/runtime
records, and source checks. The diagnostic must include the nearest field path
and, where practical, a close known-key suggestion.

Schema evolution rules:

- additive fields require an OST version that knows those fields;
- incompatible shape changes require a matrix schema-version bump;
- an older OST must reject a newer schema or unknown field, never render a
  partial workflow;
- free-form maps are allowed only where the contract explicitly defines user
  keys, such as named runner profiles or target-triple checksum maps.

Acceptance evidence includes unknown keys at the top level, inside a cell, and
inside nested bootstrap/runtime/host objects. The real report case
`from_package: true` on a cell must fail with an actionable message.

## Windows bootstrap path policy

The generated workflow may continue to use `shell: bash`, but anything exported
to another process boundary must use the consumer platform's native path form.

After locating the extracted OST binary, bootstrap should:

1. resolve its directory physically;
2. convert that directory with `cygpath -w` when `RUNNER_OS == Windows`;
3. append the resulting native path to `GITHUB_PATH`;
4. record both the executable and exported path in bootstrap evidence;
5. verify the next step can launch `ost --version` through native process
   creation, not only from the same Bash process.

The test must exercise a path containing spaces and must not rely on the checkout
directory being representable as the same POSIX and Windows string.

## Python ABI policy for `ost plugin run`

A runtime is bound to one Python ABI. Session launch must not accidentally mix
that runtime's `pxr` extension modules with an unrelated host interpreter.

Command resolution policy:

- an explicit executable path remains explicit and is not silently replaced;
- the well-known bare commands `python`, `python3`, and the Windows Python
  launcher request runtime-aware resolution;
- prefer a runnable interpreter bundled by the runtime;
- otherwise resolve an absolute host interpreter matching the runtime's declared
  major/minor ABI using the same resolver as build/test;
- if no compatible interpreter exists, fail before child launch with the
  expected ABI, searched candidates, and a setup/remediation hint;
- if the user supplies an explicit mismatched interpreter, diagnose the mismatch
  before launch when its version can be probed. An eventual explicit override
  must be named and auditable, not an implicit fallback.

Do not rely only on prepending a directory to session `PATH`. Resolve the selected
Python executable to an absolute path so Windows `CreateProcess` cannot choose an
earlier host installation. Preserve the original arguments and apply the same
composed runtime/plugin environment to the resolved process.

Add a doctor assertion such as `session.python_abi` so the mismatch can be found
without first running a Python command. Evidence should report expected ABI,
resolved executable, observed version, and source (`runtime-bundled` or
`host-matched`) without exposing unrelated environment values.

## Bootstrap pin maintenance

The CI manifest, not generated workflow YAML, remains the source of truth for the
OST version and exact asset hashes. Generated files must never be hand-edited to
bump pins.

A future helper may use a shape such as:

```text
ost ci pin bootstrap --version 0.14.0
```

It should download the named immutable release checksums, verify that every
target used by hosted cells has a matching asset, update
`bootstrap.ost.version` and `bootstrap.ost.sha256` atomically, validate the full
matrix, and print the manifest diff. Generation itself stays offline and
deterministic; it never fetches or mutates pins as a side effect.

For trusted/release lanes, every used hosted target must carry an exact checksum
in the authored CI contract. Falling back only to a checksum downloaded from the
same release origin is acceptable for lower-trust source CI but is not a fully
pinned release policy.

## Generated release-lane direction

Do not add an unstructured per-cell `extra_steps` field. A release workflow has
different triggers, permissions, trust, provenance, and publication behavior
from pull-request source CI. Treating it as a few injected shell lines would hide
the important policy.

The future model should make the following typed concepts first-class:

- tag/ref trigger and version-source agreement;
- selected source cells/platforms and exact runtime/bootstrap pins;
- package-from-source and package-twice reproducibility gate;
- `ost plugin test --from-package` against the produced archive;
- repository checks, reusing the constrained command model where appropriate;
- lean/debug/source/checksum/SBOM/provenance artifact staging;
- draft versus publish behavior;
- trusted publisher identity and minimum artifact trust;
- verification jobs with read-only permissions, separated from the publish job
  that receives release permissions.

The sequence is:

```text
tag/ref validation
  -> pinned bootstrap + runtime materialization
  -> build/test/package on each selected cell
  -> reproducibility + from-package evidence
  -> collect immutable artifacts/checksums/provenance
  -> policy verification
  -> draft/publish in a separate trusted job
```

Repository scripts are allowed as declared checks, but they do not receive
secrets by default and cannot mutate the generated workflow structure. Built-in
gates such as reproducible packaging and `--from-package` are typed booleans or
policies, not repeated shell recipes.

This work aligns with v0.16 generated trusted CI. v0.14 provides the trust
decision; v0.15 provides provenance/SBOM evidence; the release generator then
has concrete policy and evidence to enforce.

## Debug sidecar diagnostics

File modification time can identify a suspicious sidecar, but it does not prove
that a PDB belongs to a DLL. Reproducible builds, copied files, and restored
caches can make timestamp ordering misleading.

If added, the initial check is a non-fatal structured warning when a same-basename
PDB is older than its DLL, for example `DEBUG_SYMBOL_STALE_CANDIDATE`, with both
paths and observed mtimes. It must not change archive identity or reject a
package. A future Windows-aware validator may compare the PE CodeView reference
with the PDB identity; only that stronger evidence can support a hard failure.

## v0.14 acceptance impact

The v0.14 milestone remains complete when its trust-policy scope is complete.
The three accepted hardening items are additional release-readiness gates:

- unknown CI keys fail recursively;
- generated Windows bootstrap is consumable by a native child process;
- bare Python under `ost plugin run` cannot select a mismatched ABI.

The pin helper is desirable but not a v0.14 release blocker. Generated release
lanes, provenance attachment, runtime publication, and PDB identity validation
are explicitly outside the v0.14 completion boundary.
