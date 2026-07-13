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

pub use github::{
    generate_github, generate_release, generate_source, generate_support, GeneratedWorkflow,
    RELEASE_WORKFLOW_PATH, SOURCE_WORKFLOW_PATH, WORKFLOW_PATH,
};
pub use matrix::{
    is_placeholder_digest, starter_matrix, Acknowledgement, Billing, Bootstrap, HostOs, HostSpec,
    Lane, OstBootstrap, Publish, ReleaseLane, ReleaseMode, RunnerKind, RunnerProfile,
    RuntimeRemote, SupportCell, SupportMatrix, TrustRequirements, MATRIX_FILE, MATRIX_SCHEMA,
    MAX_LEVEL,
};
