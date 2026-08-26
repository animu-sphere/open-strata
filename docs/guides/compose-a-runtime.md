# Lock and distribute a composed runtime

`ost runtime compose` resolves independently published artifacts into one
deterministic runtime model. It verifies every pinned artifact and diagnoses the
combined graph before extracting or writing any payload files.

Start from the geospatial example in
[`fixtures/runtime-composition/geospatial.toml`](../../fixtures/runtime-composition/geospatial.toml).
Replace its fixture digests with full `sha256:<64-hex>` identities already in
the local artifact store, and set `composition.target` to their concrete target.

```bash
ost runtime compose runtime-composition.toml
ost --json runtime compose runtime-composition.toml
```

The manifest declares required capabilities separately from artifact
candidates. `[providers]` pins release-critical singleton capabilities to a
component id. Every candidate artifact carries its own versioned
`openstrata.component/v1alpha1` contract: provisions, requirements, target/ABI/
OpenUSD compatibility, environment contributions, and install ownership.

Resolution does not materialize payloads. It fails with a stable `COMPOSITION_*` error before
materialization for a missing or incompatible provider, an unpinned singleton,
an ABI/OpenUSD mismatch, an install destination collision, or incompatible
environment operations. Successful `--json` output uses the versioned
`openstrata.runtime-composition-resolved/v1alpha1` shape and includes a canonical
composition digest, ordered components, provider decisions, environment, and
install ownership.

## Lock and materialize (v0.22.7)

The v0.22.7 implementation adds a portable lock and component-preserving layout:

```bash
ost runtime compose runtime-composition.toml --lock runtime.lock.json --output composed
ost runtime compose runtime-composition.toml --lock runtime.lock.json --locked
ost runtime validate --composition composed
```

`--lock` writes `openstrata.runtime-composition-lock/v1alpha1` JSON. `--locked`
requires an existing matching lock and never rewrites it. `--output` is optional;
without it, inventory is derived from the verified archive entries. When present,
files are extracted into a fresh staging directory and checked against that
inventory before the directory is published. Existing output paths are refused.
Keep the external lock file outside the output prefix; overlapping paths are
rejected before fetching inputs or creating output.

The lock retains every candidate's exact archive digest, canonical producer
manifest digest, provider id/version, immutable upstream source and dependency
identities when declared, target/OpenUSD variant/ABI facts, licenses and evidence
digests. It records the selected dependency graph, environment/install decisions,
and each payload file's owner, digest, size, link target and executable bit.
Absent upstream source or validation evidence remains absent; it is not inferred.

An artifact candidate may supply an acquisition source:

```toml
[[artifacts]]
artifact = "sha256:<64-lowercase-hex-archive-digest>"
source = "oci://ghcr.io/owner/component@sha256:<64-lowercase-hex-oci-manifest-digest>"
```

Replace the placeholders with actual digests. A `file:///absolute/path/to/dist`
source is also supported for offline distribution. OCI tags without a digest are
rejected. The archive digest is verified independently from the OCI manifest pin.
Missing inputs are fetched through the existing artifact transport; a corrupted
cached artifact fails verification instead of silently falling back. Without a
source, import/pull the exact component before composing or reconstructing.
For a new composition, a declared source's producer manifest and evidence must
also match any cached copy. Only metadata is fetched for this check, not the
cached archives. A mismatch reports `COMPOSITION_SOURCE_MISMATCH` without
replacing the cache; choose a matching source or use a separate `OST_HOME`.
`--locked` and reconstruction can reuse verified, lock-matching cached inputs
without contacting their sources.

## Reconstruct on a clean consumer

```bash
# A different machine / empty OST_HOME needs only the lock and its input artifacts.
ost runtime reconstruct runtime.lock.json --output reconstructed
ost runtime validate --composition reconstructed
```

The original TOML manifest and producer's caches are not required. Acquisition
locations may be changed to mirrors, but the exact archive, producer metadata
and sidecar pins must still match. Set-like manifest inputs are sorted; local
paths, registry locations, import timestamps and ambient host observations do
not contribute to `runtime_digest`. Compatibility decisions and inventory do.

Every layout preserves each artifact under `components/<component-id>/`.
`metadata/` holds the lock, original producer manifests, available component
SBOM/provenance, attribution and composition validation. New v0.22.8 locks also
materialize the SDK described below. Existing v0.22.7 locks reconstruct with
their original component-only layout and identity; `--locked` never migrates them.

## SDK layout and activation (v0.22.8 implementation)

New locks include an additive `sdk` object. `metadata/sdk.json` records the same
`openstrata.runtime-sdk/v1alpha1` layout: each projected file's component owner,
artifact digest, original source path, content/mode/link identity, and portable
Formation environment contributions. All of it contributes to `runtime_digest`.

The public prefix always has `bin`, `lib`, `include`, `share`, `plugins`,
`python`, `node` and `metadata` directories, including empty roots after export
and reconstruction. Component `install` mappings copy files or directory trees
into that prefix. The original component prefixes remain available for relative
loader paths and producer evidence. Copies are independent, not hardlinks.
Other producer-native destinations, such as OpenUSD's `plugin/usd`, retain their
declared names. `metadata` and `components` are reserved for the composer.

Missing install sources, expanded file collisions (including a file used as a
parent), unsafe destinations, and symlinks with uninstalled or ambiguous targets
fail before output publication. Installed relative symlink targets are remapped
and recorded. Components must ship relocatable CMake configs and plugin resource
references; composition does not rewrite CMake code, plugInfo, RPATH or DLLs.

```bash
ost --json runtime env --composition composed
ost runtime env --composition composed --shell bash
ost runtime env --composition composed --shell pwsh
ost runtime exec --composition composed -- my-installed-tool --help
```

`runtime env` verifies the prefix before printing shell statements or JSON. It
resolves ordered `prepend`, `append` and `set` contributions against an empty
base; no inherited search paths enter the result. The common SDK directories
precede component paths. Component declarations remain relative to their retained
prefixes, so their plugin and loader relationships remain intact. Evaluation
replaces the affected shell search variables; use a disposable shell if you need
to keep your development PATH. Neither compose nor env changes the parent shell.
Prefix paths containing path-list separators are rejected.

`runtime exec` verifies the prefix and host OS/architecture before launching.
Bare commands resolve only on SDK PATH; pass an absolute executable path for an
external tool such as CMake, a compiler, or a separately built consumer. Runtime
search variables are reset, but this is not an OS sandbox: ordinary process
variables, system libraries and filesystem access remain available. Child exit
codes are propagated. JSON output records runtime identity, command, exit code,
stdout and stderr without modifying the immutable prefix.

## CMake and reachability checks

```bash
# Structural paths only; no component code is executed.
ost runtime validate --composition composed --sdk

# Explicitly execute an installed package's CMake config in a fresh build tree.
ost runtime validate --composition composed --sdk --cmake-package MyPackage

# A normal native consumer; use your host compiler/toolchain environment.
cmake -S consumer -B consumer-build -DCMAKE_PREFIX_PATH=/absolute/path/to/composed
cmake --build consumer-build
ost runtime exec --composition composed -- /absolute/path/to/consumer-build/my-app
```

The SDK check inspects activation paths, JSONC `plugInfo.json` library/resource
references, and declared schema resources. It reports missing or escaping paths
as failures. It does not emulate USD's `Includes` expansion or prove type/resolver
registration. Components must declare their plugin discovery paths. Run a
component-owned OpenUSD probe through `runtime exec` for native discovery and
resolver behavior.

`--cmake-package` is repeatable and opts in to executing package code. Use trusted
components. It calls `find_package(... CONFIG REQUIRED)` in a fresh project,
restricts search to the SDK prefix, disables ambient package registries/search
paths, and preserves configure output. Missing CMake is an actionable error; an
unresolved package fails the check. This probe configures a language-free project;
packages requiring an enabled C/C++ compiler should also be tested with a real
consumer. The integration suite builds a shared library, exports/reconstructs
and relocates its SDK, then compiles and runs a separate C++ consumer.

## Export and transport

```bash
ost runtime export --composition composed --dist composed-dist
# Move composed-dist, or use the existing ost artifact push/pull OCI commands.
ost artifact pull file:///absolute/path/to/composed-dist \
  --expect-artifact sha256:<archive-digest> --require-kind composed-runtime
ost runtime reconstruct --from-artifact sha256:<archive-digest> --output consumer
```

Exports use the additive `openstrata.composed-runtime` producer kind
(`composed-runtime` in registry records and filters). They contain the selected
payloads and retained metadata, so reconstruction from the exported artifact
does not need the input artifact store or access to input acquisition sources.
Legacy `openstrata.runtime` OpenUSD pulls are unchanged; composed artifacts use
`runtime reconstruct`, not the CY/profile `runtime pull` path.

The export attaches an SPDX SBOM with exact component dependencies and SLSA
provenance binding the artifact, runtime identity and component archive digests.
This is local, unsigned composition provenance, not a trusted CI attestation.
Per-component attribution preserves the producer's declared licenses and source
metadata. Missing license declarations are not silently filled in.
Artifact reconstruction checks the outer dependency identities against the
embedded lock, rejecting missing, additional or altered component dependencies
even when the outer SBOM and provenance agree with each other.

The compressed archive has its own digest, separate from `runtime_digest`.
Compression settings or transport metadata can change the archive digest without
changing the runtime identity. Exports use deterministic timestamps and preserve
locked executable bits even on a filesystem without Unix modes. An explicit
`--dist` must be new and outside the composed prefix; omitting it imports into
the artifact store and removes temporary producer output.

## What validation proves

`runtime validate --composition` checks the provider/compatibility lock, complete
inventory, retained producer metadata and available component evidence. Missing,
extra or modified files, changed executable modes on Unix, altered symlinks,
forged reports and metadata symlinks fail. Output appears only after successful
verification; existing prefixes are never overwritten.

The report separates component producer observations from composition checks.
`runtime-execution` remains explicitly `not-run` in retained composition evidence.
SDK structure, an explicit CMake probe, and a command run are distinct scopes;
neither is silently promoted to successful OpenUSD discovery, a GPU observation
or a rendered frame. Exported aggregate runtime validation remains `pending`.
Probe output belongs to the caller's CI/evidence capture and does not rewrite
the locked prefix. Real geospatial OpenUSD execution remains the v0.22.9 dogfood.
