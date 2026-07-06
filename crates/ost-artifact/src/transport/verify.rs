// SPDX-License-Identifier: Apache-2.0
//! The pull-side verification chain (transport plan, "Verification order on
//! pull", steps 4–10).
//!
//! Runs against a fetched dist directory *before* anything is imported, so a
//! failed step never leaves a usable artifact. Every failure carries a stable
//! `ARTIFACT_*` code so CI can branch on cause. The final import re-hashes the
//! archive once more inside [`crate::store::ArtifactStore::import`] — that
//! invariant stays self-contained; this chain is the stronger, transport-facing
//! gate in front of it.

use std::fs::File;

use camino::Utf8Path;

use ost_core::{digest, Category, Error, Result};

use crate::record::{manifest_files, ArtifactRecord};
use crate::store::{compare_archive_files, locate_manifest, walk_archive};
use crate::transport::{PullPolicy, StepStatus};

/// Outcome of a fully passed chain: ordered per-step evidence. The verified
/// record itself is re-derived (and the archive re-hashed) by the atomic
/// import that follows, so the chain only reports.
pub(crate) struct ChainOutcome {
    pub steps: Vec<StepStatus>,
}

/// Verify a fetched dist directory against the pull policy.
pub(crate) fn verify_dist(dist: &Utf8Path, policy: &PullPolicy) -> Result<ChainOutcome> {
    let mut steps: Vec<StepStatus> = Vec::new();

    // Manifest schema: the producer manifest must parse and derive a record.
    let (dist_dir, manifest_path) = locate_manifest(dist)?;
    let manifest_bytes = std::fs::read(manifest_path.as_std_path())
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).map_err(|e| {
        Error::coded(
            "ARTIFACT_MANIFEST_INVALID",
            Category::Validation,
            format!("fetched manifest.json is not valid JSON: {e}"),
        )
    })?;
    let record = ArtifactRecord::from_producer_manifest(
        &manifest,
        crate::record::ArtifactSource::Imported,
        0,
        "verification",
    )
    .map_err(|e| {
        Error::coded(
            "ARTIFACT_MANIFEST_INVALID",
            Category::Validation,
            format!("fetched manifest.json is not a producer manifest: {e}"),
        )
    })?;
    let expected_files = manifest_files(&manifest).map_err(|e| {
        Error::coded(
            "ARTIFACT_MANIFEST_INVALID",
            Category::Validation,
            format!("fetched manifest.json has no usable file list: {e}"),
        )
    })?;
    steps.push(("manifest_schema", "passed"));

    // Archive digest: the downloaded bytes are what the manifest describes.
    let archive = dist_dir.join(&record.archive);
    let mut f = File::open(archive.as_std_path()).map_err(|e| Error::io(archive.to_string(), e))?;
    let (actual, actual_size) =
        digest::sha256_hex_reader(&mut f).map_err(|e| Error::io(archive.to_string(), e))?;
    if actual != record.digest || actual_size != record.archive_size {
        return Err(Error::coded(
            "ARTIFACT_ARCHIVE_DIGEST_MISMATCH",
            Category::Validation,
            format!(
                "fetched archive '{}' hashes to {actual} ({actual_size} bytes) but its \
                 manifest records {} ({} bytes)",
                record.archive, record.digest, record.archive_size
            ),
        )
        .with_hint("the remote bundle is inconsistent — do not trust it; re-publish upstream"));
    }
    steps.push(("archive_digest", "passed"));

    // Pinned artifact digest: the support line / lockfile pin, when given.
    match &policy.expected_artifact_digest {
        Some(expected) if *expected != record.digest => {
            return Err(Error::coded(
                "ARTIFACT_ARCHIVE_DIGEST_MISMATCH",
                Category::Validation,
                format!(
                    "pulled artifact digest {} does not match the pinned digest {expected}",
                    record.digest
                ),
            )
            .with_hint(
                "the remote reference points at a different artifact than the pin — \
                 update the pin deliberately or fix the reference",
            ));
        }
        Some(_) => steps.push(("pinned_artifact_digest", "passed")),
        None => steps.push(("pinned_artifact_digest", "skipped")),
    }

    // Pre-extraction safety + per-file digests, in one decode pass.
    let walk = walk_archive(&archive)?;
    if !walk.unsafe_entries.is_empty() {
        return Err(Error::coded(
            "ARTIFACT_ARCHIVE_UNSAFE",
            Category::Validation,
            format!(
                "fetched archive '{}' contains {} entr{} unsafe to extract: {}",
                record.archive,
                walk.unsafe_entries.len(),
                if walk.unsafe_entries.len() == 1 {
                    "y"
                } else {
                    "ies"
                },
                walk.unsafe_entries.join("; ")
            ),
        ));
    }
    steps.push(("archive_safety", "passed"));

    let cmp = compare_archive_files(&walk.files, &expected_files);
    if !cmp.passed() {
        let mut detail = Vec::new();
        if !cmp.mismatched.is_empty() {
            detail.push(format!("mismatched: {}", cmp.mismatched.join(", ")));
        }
        if !cmp.missing.is_empty() {
            detail.push(format!("missing: {}", cmp.missing.join(", ")));
        }
        if !cmp.extra.is_empty() {
            detail.push(format!("extra: {}", cmp.extra.join(", ")));
        }
        return Err(Error::coded(
            "ARTIFACT_FILE_DIGEST_MISMATCH",
            Category::Validation,
            format!(
                "fetched archive contents do not match the manifest file list ({})",
                detail.join("; ")
            ),
        ));
    }
    steps.push(("file_digests", "passed"));

    // Artifact kind against the support line's requirement.
    match policy.require_kind {
        Some(kind) if kind != record.kind => {
            return Err(Error::coded(
                "ARTIFACT_SUPPORT_LINE_MISMATCH",
                Category::Validation,
                format!(
                    "pulled artifact is a {} but the support line requires a {}",
                    record.kind.as_str(),
                    kind.as_str()
                ),
            ));
        }
        Some(_) => steps.push(("kind_match", "passed")),
        None => steps.push(("kind_match", "skipped")),
    }

    // Target / platform / ABI pin.
    match &policy.require_target {
        Some(target) if *target != record.target => {
            return Err(Error::coded(
                "ARTIFACT_PLATFORM_MISMATCH",
                Category::Validation,
                format!(
                    "pulled artifact targets '{}' but '{target}' is required",
                    record.target
                ),
            ));
        }
        Some(_) => steps.push(("target_match", "passed")),
        None => steps.push(("target_match", "skipped")),
    }

    // Trust policy (plan § "Trust policy"): a pull that passed the full
    // digest / manifest / file chain reaches the `verified` level, which is
    // what source CI admits. `trusted` (publisher identity, provenance, SBOM)
    // lands with plan Phase 4.
    steps.push(("trust_policy", "passed"));

    Ok(ChainOutcome { steps })
}
