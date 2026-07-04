// SPDX-License-Identifier: Apache-2.0
//! `ost-artifact` — the local, digest-first artifact registry (Phase 6 MVP).
//!
//! Runtimes, plugin bundles, and project packages become **artifacts**: a
//! `tar.zst` + producer `manifest.json` + checksums, addressed by the archive's
//! SHA-256 digest. This crate owns the identity record ([`ArtifactRecord`]),
//! the content-addressed store + JSON index ([`ArtifactStore`]), and integrity
//! verification ([`VerifyReport`]). It is local-first: a content *source*, not
//! a remote service (OCI/ORAS transport is a later phase).

pub mod record;
pub mod store;

pub use record::{
    is_sha256_ref, manifest_files, ArtifactKind, ArtifactRecord, ArtifactSource, ManifestFile,
    MANIFEST_FILE, PLUGIN_BUNDLE_KIND, RECORD_FILE, RECORD_SCHEMA, RUNTIME_KIND,
};
pub use store::{ArtifactStore, ImportOutcome, Index, VerifyReport, INDEX_FILE};
