// SPDX-License-Identifier: Apache-2.0
//! `ost-artifact` — the local, digest-first artifact registry (Phase 6 MVP).
//!
//! Runtimes, plugin bundles, and project packages become **artifacts**: a
//! `tar.zst` + producer `manifest.json` + checksums, addressed by the archive's
//! SHA-256 digest. This crate owns the identity record ([`ArtifactRecord`]),
//! the content-addressed store + JSON index ([`ArtifactStore`]), and integrity
//! verification ([`VerifyReport`]).
//!
//! It is local-first — the store is the root of trust CI pins — with remote
//! movement behind the [`transport`] contract: the filesystem backend and a
//! read-only OCI backend (remote-artifact-transport.md, Phase 1). Transports
//! move bytes; identity (digests, manifest schema, verification policy) never
//! leaves this crate's core.

pub mod record;
pub mod reference;
pub mod store;
pub mod transport;

pub use record::{
    is_sha256_ref, manifest_files, ArtifactKind, ArtifactRecord, ArtifactSource, ManifestFile,
    MANIFEST_FILE, PLUGIN_BUNDLE_KIND, RECORD_FILE, RECORD_SCHEMA, RUNTIME_KIND,
};
pub use reference::{FileReference, OciReference, RemoteReference};
pub use store::{ArtifactStore, ImportOutcome, Index, VerifyReport, INDEX_FILE};
pub use transport::{
    file::FileTransport, oci::OciTransport, pull, ArtifactTransport, PullEvidence, PullPolicy,
    ResolvedRemote,
};
