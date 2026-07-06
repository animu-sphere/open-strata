// SPDX-License-Identifier: Apache-2.0
//! The artifact transport contract (remote-artifact-transport.md, Phase 1).
//!
//! A transport moves artifact *bytes*; it never defines artifact *identity*.
//! Archive structure, manifest schema, digest computation, and verification
//! policy stay in this crate's core ([`crate::store`] / [`transport::verify`]);
//! backends — the local filesystem ([`file::FileTransport`]) and OCI
//! registries ([`oci::OciTransport`]) — are adapters behind
//! [`ArtifactTransport`].
//!
//! The one public workflow is [`pull`]: resolve a **digest-pinned** reference,
//! fetch the producer files, run the full verification chain (archive digest →
//! manifest schema → pre-extraction safety → per-file digests → kind / target /
//! pinned-digest match → trust policy), then import atomically into the local
//! registry. Transport success alone is never success, and a failed step never
//! leaves a usable artifact behind.

pub mod file;
pub mod oci;
mod verify;

use std::time::{SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};

use ost_core::{Category, Error, Result};

use crate::record::{ArtifactKind, ArtifactRecord, ArtifactSource};
use crate::reference::RemoteReference;
use crate::store::ArtifactStore;

/// A remote reference resolved to its immutable identity.
#[derive(Debug, Clone)]
pub struct ResolvedRemote {
    /// Canonical locator, digest-pinned where the backend has one.
    pub locator: String,
    /// Registry identity: `host[:port]` for OCI, `local-filesystem` for file.
    pub registry: String,
    /// Repository path within the registry (empty for the file backend).
    pub repository: String,
    /// The resolved OCI manifest digest (`sha256:<hex>`); `None` for backends
    /// without an OCI manifest (the file backend).
    pub oci_digest: Option<String>,
    /// How the transport authenticated: `anonymous` / `static-token` /
    /// `token-exchange` / `basic` / `none`.
    pub auth_mode: String,
}

/// The transport contract: resolve a reference, fetch producer files.
///
/// `push` completes the contract in the publish phase (plan Phase 3); until
/// then every backend reports it as unsupported rather than pretending.
pub trait ArtifactTransport {
    /// Turn a reference into an immutable identity. A tag may resolve to a
    /// digest here, but [`pull`] enforces that CI-facing references arrive
    /// already pinned.
    fn resolve(&self, reference: &RemoteReference) -> Result<ResolvedRemote>;

    /// Fetch the artifact's producer files (`<archive>` + `manifest.json`)
    /// into a dist-shaped directory, verifying transport-level digests on the
    /// way down. Returns the directory holding the files (which may be the
    /// source itself for local backends). `scratch` is a caller-owned empty
    /// directory the backend may download into.
    fn fetch(
        &self,
        reference: &RemoteReference,
        resolved: &ResolvedRemote,
        scratch: &Utf8Path,
    ) -> Result<Utf8PathBuf>;

    /// Publish a local artifact to the destination. Lands with the publish
    /// phase (plan Phase 3); read-only backends refuse.
    fn push(&self, _artifact: &ArtifactRecord, _destination: &RemoteReference) -> Result<()> {
        Err(Error::coded(
            "ARTIFACT_PUSH_UNSUPPORTED",
            Category::Usage,
            "artifact push is not supported yet (publish lands with the v0.10.0 publish phase)",
        ))
    }
}

/// What a pull must additionally prove beyond artifact integrity.
#[derive(Debug, Clone, Default)]
pub struct PullPolicy {
    /// The OpenStrata artifact digest the support line / lockfile pins
    /// (`sha256:<hex>` of the archive). A mismatch is always an error.
    pub expected_artifact_digest: Option<String>,
    /// Require the artifact to be of this kind (e.g. a runtime for a runtime
    /// support line).
    pub require_kind: Option<ArtifactKind>,
    /// Require the artifact's target id to match (platform / ABI pin).
    pub require_target: Option<String>,
}

/// Status of one verification step, stable for `--json` evidence.
pub type StepStatus = (&'static str, &'static str);

/// Evidence for one completed pull (plan § "Minimum JSON output").
#[derive(Debug, Clone)]
pub struct PullEvidence {
    /// The reference as given by the caller.
    pub reference: String,
    /// The resolved remote identity.
    pub remote: ResolvedRemote,
    /// Ordered `(step, passed|skipped)` pairs; a failed step is an error, so
    /// evidence only exists for chains whose every step passed or was skipped.
    pub verification: Vec<StepStatus>,
    /// The imported artifact's registry record.
    pub record: ArtifactRecord,
    /// `imported` or `already-present` (same digest was already stored).
    pub import_status: &'static str,
    /// Local registry object directory holding the artifact.
    pub import_path: Utf8PathBuf,
}

/// Resolve, fetch, verify, and atomically import one artifact.
///
/// Digest-pin policy: the reference must be pinned (`@sha256:<…>` for OCI);
/// mutable-only references fail with `ARTIFACT_REFERENCE_MUTABLE` before any
/// network traffic. `ost artifact resolve` exists to turn a tag into a pin.
pub fn pull(
    transport: &dyn ArtifactTransport,
    reference: &RemoteReference,
    store: &ArtifactStore,
    policy: &PullPolicy,
) -> Result<PullEvidence> {
    if !reference.is_pinned() {
        return Err(Error::coded(
            "ARTIFACT_REFERENCE_MUTABLE",
            Category::Usage,
            format!(
                "'{}' pins no digest — tags are convenience, digests are the contract",
                reference.locator()
            ),
        )
        .with_hint(
            "resolve the tag first: `ost artifact resolve <reference>` and pull the \
             @sha256:<digest> form it prints",
        ));
    }
    if let Some(expected) = &policy.expected_artifact_digest {
        if !crate::record::is_sha256_ref(expected) {
            return Err(Error::usage(format!(
                "expected artifact digest '{expected}' is not sha256:<64 hex chars>"
            )));
        }
    }

    let resolved = transport.resolve(reference)?;

    // Downloads stage under the store root so the final import's rename stays
    // on one filesystem; the scratch dir is removed on every exit path.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let scratch = store
        .root()
        .join(format!(".tmp-pull-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(scratch.as_std_path())
        .map_err(|e| Error::io(scratch.to_string(), e))?;

    let result = (|| -> Result<PullEvidence> {
        let dist = transport.fetch(reference, &resolved, &scratch)?;
        let chain = verify::verify_dist(&dist, policy)?;
        let outcome = store.import(&dist, ArtifactSource::Imported)?;

        let mut verification: Vec<StepStatus> = Vec::new();
        verification.push((
            "oci_digest",
            if resolved.oci_digest.is_some() {
                "passed"
            } else {
                "skipped"
            },
        ));
        verification.extend(chain.steps);
        verification.push(("local_import", "passed"));

        let import_path = store.object_dir(outcome.record.digest_hex());
        Ok(PullEvidence {
            reference: reference.locator(),
            remote: resolved.clone(),
            verification,
            record: outcome.record,
            import_status: if outcome.already_present {
                "already-present"
            } else {
                "imported"
            },
            import_path,
        })
    })();

    // Success or failure, the scratch tree goes away: a failed step never
    // leaves a partially fetched artifact where something could trust it.
    let _ = std::fs::remove_dir_all(scratch.as_std_path());
    result
}
