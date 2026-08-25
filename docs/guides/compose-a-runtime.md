# Compose a runtime model

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

Resolution is read-only. It fails with a stable `COMPOSITION_*` error before
materialization for a missing or incompatible provider, an unpinned singleton,
an ABI/OpenUSD mismatch, an install destination collision, or incompatible
environment operations. Successful `--json` output uses the versioned
`openstrata.runtime-composition-resolved/v1alpha1` shape and includes a canonical
composition digest, ordered components, provider decisions, environment, and
install ownership.

This command produces the v0.22.4 resolved model. Locking, materializing, and
exporting that model are later v0.22.x slices.
