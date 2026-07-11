# Architecture

How the current system is structured, as it exists on the default branch —
crate boundaries, the domain model, and the on-disk layout. Historical
alternatives and rationale belong in [design/](../design/), not here.

| Document | Purpose |
| --- | --- |
| [overview.md](overview.md) | High-level structure: workspace layout, domain model, on-disk layout, platform resolution, output/CI, toolchain pinning. |
| [crates.md](crates.md) | Crate-level reference: the workspace members, their responsibilities, and boundaries. Kept in sync with the root `Cargo.toml`. |
