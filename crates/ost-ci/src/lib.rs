// SPDX-License-Identifier: Apache-2.0
//! `ost-ci` — the CI support matrix and workflow generation (Phase 5).
//!
//! A project declares the runtime×plugin combinations it stands behind as
//! **explicit support cells** in `openstrata.ci.yaml`, each pinning a runtime
//! artifact and a plugin artifact by full registry digest. Generators render
//! that one source of truth into CI configuration — GitHub Actions first
//! ([`generate_github`]), Jenkins later.

pub mod github;
pub mod matrix;

pub use github::{generate_github, WORKFLOW_PATH};
pub use matrix::{
    is_placeholder_digest, starter_matrix, HostOs, HostSpec, SupportCell, SupportMatrix,
    MATRIX_FILE, MATRIX_SCHEMA, MAX_LEVEL,
};
