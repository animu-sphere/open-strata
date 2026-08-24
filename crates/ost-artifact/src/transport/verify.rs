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
use ost_platform::{
    version_satisfies_constraint, ResolvedOpenUsdCompatibility, ResolvedOpenUsdMacos,
    ResolvedOpenUsdProvider,
};

use crate::evidence::{
    verify_evidence_digest, verify_provenance, verify_sbom, EvidenceDigest, PROVENANCE_FILE,
    SBOM_FILE,
};
use crate::policy::TrustLevel;
use crate::record::{manifest_debug_archive, manifest_files, ArtifactRecord};
use crate::store::{compare_archive_files, locate_manifest, walk_archive};
use crate::transport::{PullPolicy, StepStatus};

/// Outcome of a fully passed chain: ordered per-step evidence. The verified
/// record itself is re-derived (and the archive re-hashed) by the atomic
/// import that follows, so the chain only reports.
pub(crate) struct ChainOutcome {
    pub steps: Vec<StepStatus>,
    pub effective_trust: TrustLevel,
    pub required_trust: TrustLevel,
    pub matched_publisher: Option<String>,
}

/// Verify a fetched dist directory against the pull policy.
pub(crate) fn verify_dist(
    dist: &Utf8Path,
    policy: &PullPolicy,
    remote_openusd_selector: Option<&str>,
) -> Result<ChainOutcome> {
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
    let debug = manifest_debug_archive(&manifest).map_err(|e| {
        Error::coded(
            "ARTIFACT_MANIFEST_INVALID",
            Category::Validation,
            format!("fetched manifest.json has an invalid debug archive: {e}"),
        )
    })?;
    steps.push(("manifest_schema", "passed"));

    // OCI annotations are selection evidence, not authority. Recompute the
    // normalized selector from the producer manifest and require exact
    // agreement before fetched bytes can lead to a local import.
    match remote_openusd_selector {
        Some(declared) => {
            let Some(computed) = record.openusd_selector() else {
                return Err(Error::coded(
                    "ARTIFACT_OPENUSD_SELECTOR_MISMATCH",
                    Category::Validation,
                    format!(
                        "the resolved OCI manifest declares OpenUSD selector '{declared}', but \
                         the producer manifest has no verified normalized OpenUSD identity"
                    ),
                )
                .with_hint(
                    "re-publish the runtime so its OCI annotation is derived from the same \
                     producer manifest carried by the artifact",
                ));
            };
            // Source/dependency identities were added to the selector hash
            // after the annotation first shipped. An immutable artifact
            // published by that older implementation has no selector-schema
            // marker and must continue to verify against the legacy hash. New
            // manifests opt in explicitly and accept only the current hash.
            let legacy_matches = manifest
                .get(crate::record::OPENUSD_SELECTOR_SCHEMA_FIELD)
                .is_none()
                && record.legacy_openusd_selector().as_deref() == Some(declared);
            if declared != computed && !legacy_matches {
                return Err(Error::coded(
                    "ARTIFACT_OPENUSD_SELECTOR_MISMATCH",
                    Category::Validation,
                    format!(
                        "the resolved OCI manifest declares OpenUSD selector '{declared}', but \
                         the producer manifest resolves to '{computed}'"
                    ),
                )
                .with_hint(
                    "do not import this inconsistent bundle; resolve a correctly published \
                     compatibility tag or re-publish upstream",
                ));
            }
            steps.push(("openusd_selector", "passed"));
        }
        None => steps.push(("openusd_selector", "skipped")),
    }

    match &policy.require_openusd {
        Some(required) => {
            verify_openusd_requirement(
                &record,
                required,
                policy.require_openusd_version.as_deref(),
            )?;
            steps.push(("openusd_requirement", "passed"));
        }
        None => steps.push(("openusd_requirement", "skipped")),
    }

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

    // A lean plugin's symbol sidecar is part of the producer promise even
    // though the primary archive remains the artifact identity. Verify its
    // transport bytes, extraction safety, and file list before importing it.
    if let Some(debug) = debug {
        let debug_path = dist_dir.join(&debug.archive);
        let mut f = File::open(debug_path.as_std_path())
            .map_err(|e| Error::io(debug_path.to_string(), e))?;
        let (actual, actual_size) =
            digest::sha256_hex_reader(&mut f).map_err(|e| Error::io(debug_path.to_string(), e))?;
        if actual != debug.digest || actual_size != debug.archive_size {
            return Err(Error::coded(
                "ARTIFACT_ARCHIVE_DIGEST_MISMATCH",
                Category::Validation,
                format!(
                    "fetched debug archive '{}' hashes to {actual} ({actual_size} bytes) but its \
                     manifest records {} ({} bytes)",
                    debug.archive, debug.digest, debug.archive_size
                ),
            ));
        }
        let walk = walk_archive(&debug_path)?;
        if !walk.unsafe_entries.is_empty() {
            return Err(Error::coded(
                "ARTIFACT_ARCHIVE_UNSAFE",
                Category::Validation,
                format!(
                    "fetched debug archive '{}' contains entries unsafe to extract: {}",
                    debug.archive,
                    walk.unsafe_entries.join("; ")
                ),
            ));
        }
        let cmp = compare_archive_files(&walk.files, &debug.files);
        if !cmp.passed() {
            return Err(Error::coded(
                "ARTIFACT_FILE_DIGEST_MISMATCH",
                Category::Validation,
                format!(
                    "fetched debug archive '{}' does not match its manifest file list",
                    debug.archive
                ),
            ));
        }
        steps.push(("debug_archive", "passed"));
    }

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

    // Trust evidence is a consume gate, not a post-import audit. Validate every
    // fetched sidecar even when it is optional, and enforce requested presence
    // and assurance before these bytes enter the local registry.
    let sbom = evidence_in_dist(&dist_dir, SBOM_FILE)?;
    match &sbom {
        Some(evidence) => {
            verify_evidence_digest(&dist_dir, evidence)?;
            verify_sbom(
                &dist_dir.join(SBOM_FILE),
                &record.digest,
                &record.dependency_identities,
            )?;
            steps.push(("sbom", "passed"));
        }
        None if policy.require_sbom => {
            return Err(Error::coded(
                "ARTIFACT_SBOM_REQUIRED",
                Category::Validation,
                "pulled artifact has no SPDX SBOM",
            )
            .with_hint("select or publish an artifact carrying subject-bound SBOM evidence"));
        }
        None => steps.push(("sbom", "skipped")),
    }

    let required_trust = std::cmp::max(
        policy.minimum_trust.unwrap_or_default(),
        policy
            .artifact_policy
            .as_ref()
            .map(|policy| policy.minimum_trust)
            .unwrap_or_default(),
    );
    let provenance = evidence_in_dist(&dist_dir, PROVENANCE_FILE)?;
    let publisher_policy = policy
        .artifact_policy
        .as_ref()
        .filter(|_| policy.require_provenance || required_trust > TrustLevel::Attested);
    let matched_publisher = match &provenance {
        Some(evidence) => {
            verify_evidence_digest(&dist_dir, evidence)?;
            let publisher = verify_provenance(
                &dist_dir.join(PROVENANCE_FILE),
                &manifest,
                &record.digest,
                publisher_policy,
            )?;
            steps.push(("provenance", "passed"));
            publisher
        }
        None if policy.require_provenance => {
            return Err(Error::coded(
                "ARTIFACT_PROVENANCE_REQUIRED",
                Category::Validation,
                "pulled artifact has no SLSA/in-toto provenance",
            )
            .with_hint(
                "select or publish an artifact carrying subject-bound provenance evidence",
            ));
        }
        None => {
            steps.push(("provenance", "skipped"));
            None
        }
    };

    let effective_trust = if let Some(publisher) = matched_publisher.as_deref() {
        if sbom.is_some() {
            publisher_policy
                .and_then(|policy| policy.publisher(publisher))
                .map(|publisher| std::cmp::max(TrustLevel::Attested, publisher.trust))
                .unwrap_or(TrustLevel::Attested)
        } else {
            TrustLevel::Attested
        }
    } else if provenance.is_some() {
        TrustLevel::Attested
    } else {
        record.trust
    };
    if effective_trust < required_trust {
        return Err(Error::coded(
            "ARTIFACT_POLICY_TRUST_INSUFFICIENT",
            Category::Validation,
            format!(
                "pulled artifact trust '{effective_trust}' is below required minimum \
                 '{required_trust}'"
            ),
        )
        .with_hint(
            "select an artifact with the required SBOM/provenance and allowed publisher evidence",
        )
        .with_data(serde_json::json!({
            "effective_trust": effective_trust,
            "required_trust": required_trust,
            "matched_publisher": matched_publisher,
            "sbom_present": sbom.is_some(),
            "provenance_present": provenance.is_some(),
        })));
    }
    steps.push((
        "trust_policy",
        if required_trust > TrustLevel::Local
            || policy.artifact_policy.is_some()
            || policy.minimum_trust.is_some()
        {
            "passed"
        } else {
            "skipped"
        },
    ));

    Ok(ChainOutcome {
        steps,
        effective_trust,
        required_trust,
        matched_publisher,
    })
}

fn evidence_in_dist(dist: &Utf8Path, name: &str) -> Result<Option<EvidenceDigest>> {
    let path = dist.join(name);
    if !path.as_std_path().exists() {
        return Ok(None);
    }
    if !path.as_std_path().is_file() {
        return Err(Error::coded(
            "ARTIFACT_EVIDENCE_INVALID",
            Category::Validation,
            format!("fetched evidence '{name}' is not a regular file"),
        ));
    }
    EvidenceDigest::from_file(&path, name).map(Some)
}

fn verify_openusd_requirement(
    record: &ArtifactRecord,
    required: &ResolvedOpenUsdCompatibility,
    required_version: Option<&str>,
) -> Result<()> {
    let Some(selected) = &record.openusd_compatibility else {
        return Err(openusd_mismatch(
            "ARTIFACT_OPENUSD_IDENTITY_MISSING",
            "identity",
            record,
            required,
            required_version,
            "has no verified normalized OpenUSD compatibility identity".to_string(),
            "select a normalized OpenUSD runtime artifact published for the required cell",
        ));
    };

    if required_version.is_some_and(|version| record.version != version) {
        return Err(openusd_mismatch(
            "ARTIFACT_OPENUSD_VERSION_MISMATCH",
            "openusd-version",
            record,
            required,
            required_version,
            format!(
                "provides OpenUSD {}, but the consumer requires {}",
                record.version,
                required_version.expect("the mismatch branch has a required version")
            ),
            "resolve an artifact published for the required OpenUSD release",
        ));
    }

    if selected.schema != required.schema
        || selected.platform != required.platform
        || selected.os != required.os
        || selected.arch != required.arch
    {
        return Err(openusd_mismatch(
            "ARTIFACT_OPENUSD_PLATFORM_MISMATCH",
            "platform",
            record,
            required,
            required_version,
            format!(
                "declares {} {}/{} (schema {}), not required {} {}/{} (schema {})",
                selected.platform,
                selected.os.as_str(),
                selected.arch.as_str(),
                selected.schema,
                required.platform,
                required.os.as_str(),
                required.arch.as_str(),
                required.schema,
            ),
            "resolve an artifact tag for the required platform and architecture",
        ));
    }

    let compiler_ok = selected.toolchain.family == required.toolchain.family
        && selected.toolchain.provider == required.toolchain.provider
        && selected.toolchain.cxx_standard == required.toolchain.cxx_standard
        && version_matches(
            selected.toolchain.version.as_deref(),
            required.toolchain.version.as_deref(),
            &required.toolchain.version_constraint,
        );
    let runtime_ok = provider_matches(&selected.toolchain.runtime, &required.toolchain.runtime);
    if !compiler_ok || !runtime_ok {
        return Err(openusd_mismatch(
            "ARTIFACT_OPENUSD_TOOLCHAIN_MISMATCH",
            "toolchain",
            record,
            required,
            required_version,
            format!(
                "uses compiler {}@{} {} (C++ {}) and native runtime {}@{} {}, but the consumer requires compiler {}@{} {} (C++ {}) and native runtime {}@{} {}",
                selected.toolchain.family,
                selected.toolchain.provider,
                selected.toolchain.version.as_deref().unwrap_or("unresolved"),
                selected.toolchain.cxx_standard,
                selected.toolchain.runtime.family,
                selected.toolchain.runtime.provider,
                selected.toolchain.runtime.version.as_deref().unwrap_or("unresolved"),
                required.toolchain.family,
                required.toolchain.provider,
                required.toolchain.version.as_deref().unwrap_or(&required.toolchain.version_constraint),
                required.toolchain.cxx_standard,
                required.toolchain.runtime.family,
                required.toolchain.runtime.provider,
                required.toolchain.runtime.version.as_deref().unwrap_or(&required.toolchain.runtime.version_constraint),
            ),
            "resolve an artifact built with the required compiler and native runtime providers",
        ));
    }

    verify_provider_dimension(
        "python",
        "ARTIFACT_OPENUSD_PYTHON_MISMATCH",
        record,
        required,
        required_version,
        &selected.python,
        &required.python,
    )?;
    verify_provider_dimension(
        "tbb",
        "ARTIFACT_OPENUSD_TBB_MISMATCH",
        record,
        required,
        required_version,
        &selected.tbb,
        &required.tbb,
    )?;

    let macos_ok = macos_matches(selected.macos.as_ref(), required.macos.as_ref());
    if !macos_ok {
        let describe = |value: Option<&ResolvedOpenUsdMacos>| {
            value.map_or_else(
                || "none".to_string(),
                |macos| {
                    format!(
                        "{}@{} {} with deployment target {}",
                        macos.sdk.family,
                        macos.sdk.provider,
                        macos
                            .sdk
                            .version
                            .as_deref()
                            .unwrap_or(&macos.sdk.version_constraint),
                        macos.deployment_target
                    )
                },
            )
        };
        return Err(openusd_mismatch(
            "ARTIFACT_OPENUSD_MACOS_MISMATCH",
            "macos",
            record,
            required,
            required_version,
            format!(
                "uses macOS {}, but the consumer requires {}",
                describe(selected.macos.as_ref()),
                describe(required.macos.as_ref()),
            ),
            "resolve an artifact built with the required macOS SDK and deployment target",
        ));
    }

    let missing = required
        .capabilities
        .iter()
        .filter(|capability| !selected.capabilities.iter().any(|have| have == *capability))
        .cloned()
        .collect::<Vec<_>>();
    if selected.variant != required.variant || !missing.is_empty() {
        let missing = if missing.is_empty() {
            "none".to_string()
        } else {
            missing.join(", ")
        };
        return Err(openusd_mismatch(
            "ARTIFACT_OPENUSD_GRAPHICS_MISMATCH",
            "graphics",
            record,
            required,
            required_version,
            format!(
                "provides variant '{}' with capabilities [{}], but variant '{}' with capabilities [{}] is required (missing: {missing})",
                selected.variant.as_str(),
                selected.capabilities.join(", "),
                required.variant.as_str(),
                required.capabilities.join(", "),
            ),
            "resolve the compatibility tag for the required OpenUSD variant and graphics capabilities",
        ));
    }

    Ok(())
}

fn verify_provider_dimension(
    dimension: &'static str,
    code: &'static str,
    record: &ArtifactRecord,
    required_compatibility: &ResolvedOpenUsdCompatibility,
    required_version: Option<&str>,
    selected: &ResolvedOpenUsdProvider,
    required: &ResolvedOpenUsdProvider,
) -> Result<()> {
    if provider_matches(selected, required) {
        return Ok(());
    }
    Err(openusd_mismatch(
        code,
        dimension,
        record,
        required_compatibility,
        required_version,
        format!(
            "uses {dimension} {}@{} {}, but the consumer requires {}@{} {}",
            selected.family,
            selected.provider,
            selected.version.as_deref().unwrap_or("unresolved"),
            required.family,
            required.provider,
            required.version.as_deref().unwrap_or(&required.version_constraint),
        ),
        &format!(
            "resolve an artifact whose {dimension} provider and exact version satisfy the required cell"
        ),
    ))
}

fn provider_matches(
    selected: &ResolvedOpenUsdProvider,
    required: &ResolvedOpenUsdProvider,
) -> bool {
    selected.family == required.family
        && selected.provider == required.provider
        && version_matches(
            selected.version.as_deref(),
            required.version.as_deref(),
            &required.version_constraint,
        )
}

fn macos_matches(
    selected: Option<&ResolvedOpenUsdMacos>,
    required: Option<&ResolvedOpenUsdMacos>,
) -> bool {
    match (selected, required) {
        (None, None) => true,
        (Some(selected), Some(required)) => {
            provider_matches(&selected.sdk, &required.sdk)
                && selected.deployment_target == required.deployment_target
        }
        _ => false,
    }
}

fn version_matches(selected: Option<&str>, exact: Option<&str>, constraint: &str) -> bool {
    let Some(selected) = selected else {
        return false;
    };
    match exact {
        Some(exact) => selected == exact,
        None => version_satisfies_constraint(selected, constraint),
    }
}

fn openusd_mismatch(
    code: &'static str,
    dimension: &'static str,
    record: &ArtifactRecord,
    required: &ResolvedOpenUsdCompatibility,
    required_version: Option<&str>,
    detail: String,
    hint: &str,
) -> Error {
    Error::coded(
        code,
        Category::Validation,
        format!(
            "selected artifact {} ({} {}) {detail}",
            record.short_digest(),
            record.name,
            record.version
        ),
    )
    .with_hint(hint)
    .with_data(serde_json::json!({
        "dimension": dimension,
        "selected_artifact": {
            "digest": record.digest,
            "name": record.name,
            "version": record.version,
            "selector": record.openusd_selector(),
            "openusd": record.openusd_compatibility,
        },
        "requirement": {
            "platform": required.platform,
            "openusd_version": required_version,
            "os": required.os,
            "arch": required.arch,
            "variant": required.variant,
            "toolchain": required.toolchain,
            "python": required.python,
            "tbb": required.tbb,
            "capabilities": required.capabilities,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn macos(
        version: Option<&str>,
        constraint: &str,
        deployment_target: &str,
    ) -> ResolvedOpenUsdMacos {
        ResolvedOpenUsdMacos {
            sdk: ResolvedOpenUsdProvider {
                family: "macos-sdk".into(),
                provider: "xcode".into(),
                version: version.map(str::to_string),
                version_constraint: constraint.into(),
            },
            deployment_target: deployment_target.into(),
        }
    }

    #[test]
    fn macos_requirement_matches_sdk_and_deployment_target() {
        let selected = macos(Some("15.5"), "15.5", "13.0");
        let required = macos(None, "15.5", "13.0");
        assert!(macos_matches(Some(&selected), Some(&required)));

        let wrong_sdk = macos(None, "16.x", "13.0");
        assert!(!macos_matches(Some(&selected), Some(&wrong_sdk)));
        let wrong_floor = macos(None, "15.5", "14.0");
        assert!(!macos_matches(Some(&selected), Some(&wrong_floor)));
        assert!(!macos_matches(Some(&selected), None));
    }
}
