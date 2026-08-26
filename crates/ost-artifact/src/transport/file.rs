// SPDX-License-Identifier: Apache-2.0
//! The filesystem transport: the existing local dist-directory flow behind the
//! [`ArtifactTransport`] contract (transport plan, "The local registry is not
//! retired").
//!
//! A `file://` reference names a producer output directory (`manifest.json` +
//! archive) — the same shape `ost artifact import` accepts and `ost artifact
//! export` writes. Pulling it runs the identical verification chain as a
//! remote pull, so air-gapped lanes get the same evidence trail; the bytes
//! just never cross a network.

use camino::Utf8Path;

use ost_core::{Category, Error, Result};

use crate::evidence::{PROVENANCE_FILE, SBOM_FILE};
use crate::record::{
    is_sha256_ref, manifest_debug_archive, ArtifactRecord, ArtifactSource, MANIFEST_FILE,
};
use crate::reference::RemoteReference;
use crate::store::locate_manifest;
use crate::transport::{ArtifactTransport, FetchOutcome, ResolvedRemote, TransferEvidence};

/// Registry identity recorded in evidence for filesystem pulls.
pub const FILE_REGISTRY_ID: &str = "local-filesystem";

/// The local filesystem backend (read side).
#[derive(Debug, Default)]
pub struct FileTransport;

impl FileTransport {
    pub fn new() -> FileTransport {
        FileTransport
    }

    fn dist_dir<'a>(&self, reference: &'a RemoteReference) -> Result<&'a Utf8Path> {
        match reference {
            RemoteReference::File(f) => Ok(f.path.as_path()),
            RemoteReference::Oci(r) => Err(Error::usage(format!(
                "'{}' is an OCI reference — the filesystem transport only handles file://",
                r.locator()
            ))),
        }
    }

    fn fetch_files(
        &self,
        reference: &RemoteReference,
        _resolved: &ResolvedRemote,
        scratch: &Utf8Path,
        payloads: bool,
    ) -> Result<FetchOutcome> {
        // Freeze caller-owned files before verification. The pull chain verifies
        // and imports this same snapshot, so a producer cannot replace a
        // manifest or evidence sidecar between those two operations.
        let (_, manifest_path) = locate_manifest(self.dist_dir(reference)?)?;
        let dist = manifest_path
            .parent()
            .ok_or_else(|| Error::usage("producer manifest has no parent directory"))?;
        let manifest_bytes = std::fs::read(manifest_path.as_std_path())
            .map_err(|error| Error::io(manifest_path.to_string(), error))?;
        let manifest: serde_json::Value =
            serde_json::from_slice(&manifest_bytes).map_err(|error| {
                Error::coded(
                    "ARTIFACT_MANIFEST_INVALID",
                    Category::Validation,
                    format!("'{manifest_path}' is not valid JSON: {error}"),
                )
            })?;
        let record = ArtifactRecord::from_producer_manifest(
            &manifest,
            ArtifactSource::Imported,
            0,
            "filesystem transport",
        )
        .map_err(|error| {
            Error::coded(
                "ARTIFACT_MANIFEST_INVALID",
                Category::Validation,
                format!("'{manifest_path}' is not a producer manifest: {error}"),
            )
        })?;
        let debug = manifest_debug_archive(&manifest).map_err(|error| {
            Error::coded(
                "ARTIFACT_MANIFEST_INVALID",
                Category::Validation,
                format!("'{manifest_path}' has an invalid debug archive: {error}"),
            )
        })?;

        std::fs::write(scratch.join(MANIFEST_FILE).as_std_path(), &manifest_bytes)
            .map_err(|error| Error::io(scratch.join(MANIFEST_FILE).to_string(), error))?;
        if payloads {
            snapshot_file(dist, scratch, &record.archive, true)?;
            if let Some(debug) = debug {
                snapshot_file(dist, scratch, &debug.archive, true)?;
            }
        }

        for name in [SBOM_FILE, PROVENANCE_FILE] {
            snapshot_file(dist, scratch, name, false)?;
        }

        Ok(FetchOutcome {
            dist: scratch.to_owned(),
            transfer: TransferEvidence::default(),
        })
    }
}

impl ArtifactTransport for FileTransport {
    fn resolve(&self, reference: &RemoteReference) -> Result<ResolvedRemote> {
        let dist = self.dist_dir(reference)?;
        if !dist.as_std_path().exists() {
            return Err(Error::coded(
                "ARTIFACT_REMOTE_NOT_FOUND",
                Category::Precondition,
                format!("'{dist}' does not exist"),
            ));
        }
        // The archive digest claimed by the producer manifest is the closest
        // thing a dist dir has to a resolved identity; the pull chain then
        // proves the claim against the bytes.
        let (_, manifest_path) = locate_manifest(dist)?;
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(manifest_path.as_std_path())
                .map_err(|e| Error::io(manifest_path.to_string(), e))?,
        )
        .map_err(|e| {
            Error::coded(
                "ARTIFACT_MANIFEST_INVALID",
                Category::Validation,
                format!("'{manifest_path}' is not valid JSON: {e}"),
            )
        })?;
        let claimed = manifest
            .get("archive_digest")
            .and_then(|v| v.as_str())
            .filter(|d| is_sha256_ref(d))
            .ok_or_else(|| {
                Error::coded(
                    "ARTIFACT_MANIFEST_INVALID",
                    Category::Validation,
                    format!("'{manifest_path}' carries no well-formed archive_digest"),
                )
            })?;

        Ok(ResolvedRemote {
            locator: format!("{}@{claimed}", reference.locator()),
            registry: FILE_REGISTRY_ID.to_string(),
            repository: String::new(),
            oci_digest: None,
            openusd_selector: None,
            auth_mode: "none".to_string(),
        })
    }

    fn fetch(
        &self,
        reference: &RemoteReference,
        resolved: &ResolvedRemote,
        scratch: &Utf8Path,
    ) -> Result<FetchOutcome> {
        self.fetch_files(reference, resolved, scratch, true)
    }

    fn fetch_metadata(
        &self,
        reference: &RemoteReference,
        resolved: &ResolvedRemote,
        scratch: &Utf8Path,
    ) -> Result<FetchOutcome> {
        self.fetch_files(reference, resolved, scratch, false)
    }
}

fn snapshot_file(dist: &Utf8Path, scratch: &Utf8Path, name: &str, required: bool) -> Result<()> {
    let source = dist.join(name);
    if !source.as_std_path().exists() {
        if required {
            return Err(Error::coded(
                "ARTIFACT_REMOTE_NOT_FOUND",
                Category::Precondition,
                format!("producer output is missing '{source}'"),
            ));
        }
        return Ok(());
    }
    if !source.as_std_path().is_file() {
        return Err(Error::coded(
            "ARTIFACT_EVIDENCE_INVALID",
            Category::Validation,
            format!("producer output path '{source}' is not a regular file"),
        ));
    }
    let destination = scratch.join(name);
    std::fs::copy(source.as_std_path(), destination.as_std_path())
        .map_err(|error| Error::io(source.to_string(), error))?;
    Ok(())
}
