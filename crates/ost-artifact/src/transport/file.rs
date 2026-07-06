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

use camino::{Utf8Path, Utf8PathBuf};

use ost_core::{Category, Error, Result};

use crate::record::is_sha256_ref;
use crate::reference::RemoteReference;
use crate::store::locate_manifest;
use crate::transport::{ArtifactTransport, ResolvedRemote};

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
            auth_mode: "none".to_string(),
        })
    }

    fn fetch(
        &self,
        reference: &RemoteReference,
        _resolved: &ResolvedRemote,
        _scratch: &Utf8Path,
    ) -> Result<Utf8PathBuf> {
        // The dist dir already holds the producer files; the verification
        // chain reads them in place and import copies them into the store.
        Ok(self.dist_dir(reference)?.to_owned())
    }
}
