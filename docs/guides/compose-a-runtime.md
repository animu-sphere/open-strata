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

The initial layout preserves each artifact under `components/<component-id>/`.
`metadata/` holds the lock, original producer manifests, available component
SBOM/provenance, attribution and composition validation. This is not yet the
flattened SDK/activation layout planned for v0.22.8. Install and environment
contributions are locked decisions, not active environment modifications.

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
`runtime-execution` is explicitly `not-run`: this release does not claim loader,
plugin/resolver/schema discovery, CMake consumption, a GPU observation or a
rendered frame. The artifact's aggregate runtime validation remains `pending`.
Those execution/SDK checks belong to the subsequent composition slices.
