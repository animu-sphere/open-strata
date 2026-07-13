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
//! Two public workflows. [`pull`]: resolve a **digest-pinned** reference, fetch
//! the producer files, run the full verification chain (archive digest →
//! manifest schema → pre-extraction safety → per-file digests → kind / target /
//! pinned-digest match → trust policy), then import atomically into the local
//! registry. Transport success alone is never success, and a failed step never
//! leaves a usable artifact behind. [`push`]: re-hash a stored artifact, then
//! publish it to a write-capable backend (OCI) in the exact layout the pull path
//! consumes — content-addressed and idempotent.

pub mod file;
pub mod oci;
mod verify;

use std::time::{SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};

use ost_core::{Category, Error, Result};

use crate::record::{manifest_debug_archive, ArtifactKind, ArtifactRecord, ArtifactSource};
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

/// The store-resolved local files a push publishes. Built by [`push`] from an
/// artifact's object directory; a backend never touches the store itself.
#[derive(Debug, Clone)]
pub struct PushSource<'a> {
    /// Absolute path of the canonical `tar.zst` archive.
    pub archive_path: Utf8PathBuf,
    /// Absolute path of the producer `manifest.json`.
    pub manifest_path: Utf8PathBuf,
    /// The artifact's registry record (identity + archive digest/size).
    pub record: &'a ArtifactRecord,
}

/// The immutable result of a completed push.
#[derive(Debug, Clone)]
pub struct PushOutcome {
    /// `sha256:<hex>` of the pushed OCI image manifest — the value that pins
    /// `runtime_remote.expected_oci_digest` in a support line.
    pub oci_digest: String,
    /// `sha256:<hex>` of the OpenStrata artifact archive (its identity).
    pub artifact_digest: String,
    /// Canonical pushed locator (`oci://reg/repo[:tag]@sha256:<oci-digest>`).
    pub locator: String,
    /// Registry identity: `host[:port]` for OCI, `local-filesystem` for file.
    pub registry: String,
    /// Repository path within the registry (empty for the file backend).
    pub repository: String,
    /// The identical OCI manifest already existed at the destination — an
    /// idempotent re-push transferred nothing new.
    pub already_present: bool,
    /// How the transport authenticated.
    pub auth_mode: String,
}

/// The transport contract: resolve a reference, fetch producer files, publish.
pub trait ArtifactTransport {
    /// Turn a reference into an immutable identity. A tag may resolve to a
    /// digest here, but [`pull`] enforces that CI-facing references arrive
    /// already pinned.
    fn resolve(&self, reference: &RemoteReference) -> Result<ResolvedRemote>;

    /// Fetch the artifact's producer files (main archive, optional debug and
    /// evidence sidecars, and `manifest.json`)
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

    /// Publish a store-resolved artifact to `destination`, emitting the exact
    /// OCI layout / media types [`crate::transport::oci::OciTransport::fetch`]
    /// consumes. Content-addressed and idempotent: pushing bytes already at the
    /// destination transfers nothing. Read-only backends refuse.
    fn push(&self, _source: &PushSource, _destination: &RemoteReference) -> Result<PushOutcome> {
        Err(Error::coded(
            "ARTIFACT_PUSH_UNSUPPORTED",
            Category::Usage,
            "this transport is read-only — only the OCI backend (oci://) can push",
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
        verification.push((
            "sbom",
            if outcome.record.sbom.is_some() {
                "passed"
            } else {
                "skipped"
            },
        ));
        verification.push((
            "provenance",
            if outcome.record.provenance.is_some() {
                "passed"
            } else {
                "skipped"
            },
        ));
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

/// Publish a stored artifact (by digest reference) to a remote destination.
///
/// Resolves the artifact in the local registry, **re-hashes its archive** and
/// refuses to publish if the store has drifted (never propagate corruption at
/// rest), then hands the store-resolved files to the transport's `push`. The
/// returned [`PushOutcome`] carries the immutable OCI manifest digest to pin.
pub fn push(
    transport: &dyn ArtifactTransport,
    store: &ArtifactStore,
    digest_ref: &str,
    destination: &RemoteReference,
) -> Result<PushOutcome> {
    let record = store.resolve(digest_ref)?;
    let object_dir = store.object_dir(record.digest_hex());
    let archive_path = object_dir.join(&record.archive);
    let manifest_path = object_dir.join(crate::record::MANIFEST_FILE);
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(manifest_path.as_std_path())
            .map_err(|e| Error::io(manifest_path.to_string(), e))?,
    )
    .map_err(|e| Error::parse(manifest_path.to_string(), anyhow::Error::new(e)))?;
    let debug = manifest_debug_archive(&manifest)?;

    // Never publish store corruption: the recorded digest is a claim, the bytes
    // are the truth. Prove they still agree before a single byte leaves the host.
    let mut f = std::fs::File::open(archive_path.as_std_path())
        .map_err(|e| Error::io(archive_path.to_string(), e))?;
    let (actual, actual_size) = ost_core::digest::sha256_hex_reader(&mut f)
        .map_err(|e| Error::io(archive_path.to_string(), e))?;
    if actual != record.digest || actual_size != record.archive_size {
        return Err(Error::coded(
            "ARTIFACT_DIGEST_MISMATCH",
            Category::Validation,
            format!(
                "stored archive for {} hashes to {actual} ({actual_size} bytes) — \
                 the local store is corrupted",
                record.short_digest()
            ),
        )
        .with_hint("re-import the artifact from its original producer output"));
    }

    if let Some(debug) = debug {
        crate::store::verify_archive_claim(&object_dir, &debug)?;
    }

    let source = PushSource {
        archive_path,
        manifest_path,
        record: &record,
    };
    transport.push(&source, destination)
}
