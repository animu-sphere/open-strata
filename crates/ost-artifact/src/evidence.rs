// SPDX-License-Identifier: Apache-2.0
//! Optional evidence sidecars carried with an OpenStrata artifact.

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use ost_core::{digest, Category, Error, Result};

use crate::{ArtifactPolicy, PublisherIdentity};

pub const SBOM_FILE: &str = "sbom.spdx.json";
pub const PROVENANCE_FILE: &str = "provenance.intoto.jsonl";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceDigest {
    pub path: String,
    pub digest: String,
    pub size: u64,
}

impl EvidenceDigest {
    pub fn from_file(path: &Utf8Path, name: &str) -> Result<Self> {
        let bytes = std::fs::read(path.as_std_path())
            .map_err(|source| Error::io(path.to_string(), source))?;
        Ok(Self {
            path: name.to_string(),
            digest: digest::sha256_hex(&bytes),
            size: bytes.len() as u64,
        })
    }
}

/// Build metadata available on GitHub Actions. All fields are required so a
/// partial environment never produces provenance that looks authoritative.
pub fn github_build_metadata() -> Option<serde_json::Value> {
    let repository = nonempty_env("GITHUB_REPOSITORY")?;
    let revision = nonempty_env("GITHUB_SHA")?;
    let workflow_ref = nonempty_env("GITHUB_WORKFLOW_REF")?;
    let git_ref = nonempty_env("GITHUB_REF")?;
    let actor = nonempty_env("GITHUB_ACTOR")?;
    let event = nonempty_env("GITHUB_EVENT_NAME")?;
    let workflow_path = workflow_ref
        .strip_prefix(&format!("{repository}/"))?
        .split('@')
        .next()?
        .to_string();
    if !workflow_path.starts_with(".github/workflows/") {
        return None;
    }
    let builder_id = format!("https://github.com/{workflow_ref}");
    Some(serde_json::json!({
        "source": {
            "repository": repository,
            "revision": revision,
        },
        "builder": {
            "id": builder_id,
            "identity": {
                "repository": repository,
                "workflow_path": workflow_path,
                "git_ref": git_ref,
                "actor": actor,
                "event": event,
            }
        }
    }))
}

fn nonempty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

/// Generate the deterministic SPDX SBOM and, when complete GitHub build
/// metadata is available, SLSA/in-toto provenance. The artifact archive digest
/// remains the identity; evidence is attached alongside it.
pub fn generate_evidence(
    dist: &Utf8Path,
    manifest: &mut serde_json::Value,
) -> Result<Vec<EvidenceDigest>> {
    std::fs::create_dir_all(dist.as_std_path()).map_err(|e| Error::io(dist.to_string(), e))?;
    // Explicit build metadata wins: a producer that already recorded its
    // source/builder must not have it silently replaced by whatever CI
    // environment this process happens to run inside.
    if manifest.get("build").is_none() {
        if let Some(build) = github_build_metadata() {
            manifest["build"] = build;
        }
    }

    let sbom = spdx_document(manifest)?;
    write_json(&dist.join(SBOM_FILE), &sbom)?;
    let mut evidence = vec![EvidenceDigest::from_file(&dist.join(SBOM_FILE), SBOM_FILE)?];

    let provenance_path = dist.join(PROVENANCE_FILE);
    if manifest.get("build").is_some() {
        let provenance = provenance_statement(manifest)?;
        let line = serde_json::to_string(&provenance)
            .map_err(|source| Error::parse(PROVENANCE_FILE, anyhow::Error::new(source)))?;
        std::fs::write(provenance_path.as_std_path(), format!("{line}\n"))
            .map_err(|source| Error::io(provenance_path.to_string(), source))?;
        evidence.push(EvidenceDigest::from_file(
            &provenance_path,
            PROVENANCE_FILE,
        )?);
    } else if let Err(source) = std::fs::remove_file(provenance_path.as_std_path()) {
        if source.kind() != std::io::ErrorKind::NotFound {
            return Err(Error::io(provenance_path.to_string(), source));
        }
    }
    Ok(evidence)
}

fn spdx_document(manifest: &serde_json::Value) -> Result<serde_json::Value> {
    let digest = manifest_str(manifest, "archive_digest")?;
    let digest_hex = digest.strip_prefix("sha256:").unwrap_or(&digest);
    let name = manifest
        .pointer("/plugin/name")
        .or_else(|| manifest.get("name"))
        .and_then(|value| value.as_str())
        .unwrap_or("openstrata-artifact");
    let version = manifest
        .pointer("/plugin/version")
        .or_else(|| manifest.get("version"))
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let licenses = manifest
        .pointer("/plugin/license")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .or_else(|| {
            manifest
                .get("licenses")
                .and_then(|value| value.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str())
                        .collect::<Vec<_>>()
                        .join(" AND ")
                })
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "NOASSERTION".to_string());

    let files = manifest
        .get("files")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let spdx_files = files
        .iter()
        .enumerate()
        .filter_map(|(index, file)| {
            let path = file.get("path")?.as_str()?;
            let sha = file.get("sha256")?.as_str()?;
            Some(serde_json::json!({
                "SPDXID": format!("SPDXRef-File-{index:06}"),
                "fileName": path,
                "checksums": [{
                    "algorithm": "SHA256",
                    "checksumValue": sha.strip_prefix("sha256:").unwrap_or(sha),
                }],
                "licenseConcluded": "NOASSERTION",
                "copyrightText": "NOASSERTION",
            }))
        })
        .collect::<Vec<_>>();
    let relationships = spdx_files
        .iter()
        .filter_map(|file| file.get("SPDXID").and_then(|value| value.as_str()))
        .map(|id| {
            serde_json::json!({
                "spdxElementId": "SPDXRef-Package",
                "relationshipType": "CONTAINS",
                "relatedSpdxElement": id,
            })
        })
        .collect::<Vec<_>>();

    Ok(serde_json::json!({
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": format!("{name}-{version}"),
        "documentNamespace": format!("https://openstrata.dev/spdx/{digest_hex}"),
        "creationInfo": {
            "created": "1970-01-01T00:00:00Z",
            "creators": [format!("Tool: ost-{}", env!("CARGO_PKG_VERSION"))],
            "comment": "Timestamp is fixed for deterministic OpenStrata evidence; producer time remains in manifest.json.",
        },
        "packages": [{
            "name": name,
            "SPDXID": "SPDXRef-Package",
            "versionInfo": version,
            "downloadLocation": "NOASSERTION",
            "filesAnalyzed": true,
            "licenseConcluded": "NOASSERTION",
            "licenseDeclared": licenses,
            "copyrightText": "NOASSERTION",
            "checksums": [{
                "algorithm": "SHA256",
                "checksumValue": digest_hex,
            }],
        }],
        "files": spdx_files,
        "relationships": relationships,
    }))
}

fn provenance_statement(manifest: &serde_json::Value) -> Result<serde_json::Value> {
    let archive = manifest_str(manifest, "archive")?;
    let digest = manifest_str(manifest, "archive_digest")?;
    let digest_hex = digest.strip_prefix("sha256:").unwrap_or(&digest);
    let build = manifest
        .get("build")
        .cloned()
        .ok_or_else(|| Error::validation("cannot generate provenance without build metadata"))?;
    Ok(serde_json::json!({
        "_type": "https://in-toto.io/Statement/v1",
        "subject": [{
            "name": archive,
            "digest": { "sha256": digest_hex },
        }],
        "predicateType": "https://slsa.dev/provenance/v1",
        "predicate": {
            "buildDefinition": {
                "buildType": "https://openstrata.dev/build/v1",
                "externalParameters": {
                    "source": build["source"],
                },
            },
            "runDetails": {
                "builder": build["builder"],
            },
        },
    }))
}

fn manifest_str(manifest: &serde_json::Value, field: &str) -> Result<String> {
    manifest
        .get(field)
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| Error::validation(format!("producer manifest is missing '{field}'")))
}

fn write_json(path: &Utf8Path, value: &serde_json::Value) -> Result<()> {
    let json = serde_json::to_string_pretty(value)
        .map_err(|source| Error::parse(path.to_string(), anyhow::Error::new(source)))?;
    std::fs::write(path.as_std_path(), format!("{json}\n"))
        .map_err(|source| Error::io(path.to_string(), source))
}

/// Reconstruct an evidence descriptor from an artifact record's optional
/// path/digest/size triple. All three fields must be present together and the
/// path must be the fixed sidecar name; anything else fails closed.
pub(crate) fn record_evidence(
    path: Option<&str>,
    digest: Option<&str>,
    size: Option<u64>,
    label: &str,
    expected_path: &str,
) -> Result<Option<EvidenceDigest>> {
    match (path, digest, size) {
        (None, None, None) => Ok(None),
        (Some(path), Some(digest), Some(size)) if path == expected_path => {
            Ok(Some(EvidenceDigest {
                path: path.to_string(),
                digest: digest.to_string(),
                size,
            }))
        }
        (Some(path), Some(_), Some(_)) => Err(Error::coded(
            "ARTIFACT_EVIDENCE_INVALID",
            Category::Validation,
            format!("artifact record {label} path must be '{expected_path}', got '{path}'"),
        )),
        _ => Err(Error::coded(
            "ARTIFACT_EVIDENCE_INVALID",
            Category::Validation,
            format!("artifact record has incomplete {label} path/digest/size metadata"),
        )),
    }
}

pub fn verify_evidence_digest(root: &Utf8Path, evidence: &EvidenceDigest) -> Result<()> {
    let path = root.join(&evidence.path);
    let actual = EvidenceDigest::from_file(&path, &evidence.path)?;
    if actual.digest != evidence.digest || actual.size != evidence.size {
        return Err(Error::coded(
            "ARTIFACT_EVIDENCE_DIGEST_MISMATCH",
            Category::Validation,
            format!(
                "evidence '{}' hashes to {} ({} bytes), expected {} ({} bytes)",
                evidence.path, actual.digest, actual.size, evidence.digest, evidence.size
            ),
        ));
    }
    Ok(())
}

pub fn verify_sbom(path: &Utf8Path, artifact_digest: &str) -> Result<()> {
    let document: serde_json::Value = serde_json::from_slice(
        &std::fs::read(path.as_std_path()).map_err(|source| Error::io(path.to_string(), source))?,
    )
    .map_err(|source| {
        Error::coded(
            "ARTIFACT_SBOM_INVALID",
            Category::Validation,
            format!("{SBOM_FILE} is not valid JSON: {source}"),
        )
    })?;
    if document.get("spdxVersion").and_then(|v| v.as_str()) != Some("SPDX-2.3") {
        return Err(Error::coded(
            "ARTIFACT_SBOM_INVALID",
            Category::Validation,
            format!("{SBOM_FILE} must be an SPDX-2.3 JSON document"),
        ));
    }
    let expected = artifact_digest
        .strip_prefix("sha256:")
        .unwrap_or(artifact_digest);
    let subject_matches = document
        .get("packages")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .flat_map(|package| {
            package
                .get("checksums")
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
        })
        .any(|checksum| {
            checksum.get("algorithm").and_then(|v| v.as_str()) == Some("SHA256")
                && checksum.get("checksumValue").and_then(|v| v.as_str()) == Some(expected)
        });
    if !subject_matches {
        return Err(Error::coded(
            "ARTIFACT_SBOM_SUBJECT_MISMATCH",
            Category::Validation,
            format!("{SBOM_FILE} does not identify artifact digest {artifact_digest}"),
        ));
    }
    Ok(())
}

pub fn verify_provenance(
    path: &Utf8Path,
    manifest: &serde_json::Value,
    artifact_digest: &str,
    policy: Option<&ArtifactPolicy>,
) -> Result<Option<String>> {
    let source = std::fs::read_to_string(path.as_std_path())
        .map_err(|error| Error::io(path.to_string(), error))?;
    let statements = source
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).map_err(|error| {
                Error::coded(
                    "ARTIFACT_PROVENANCE_INVALID",
                    Category::Validation,
                    format!("{PROVENANCE_FILE} contains invalid JSONL: {error}"),
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if statements.is_empty() {
        return Err(Error::coded(
            "ARTIFACT_PROVENANCE_INVALID",
            Category::Validation,
            format!("{PROVENANCE_FILE} contains no statements"),
        ));
    }
    let expected = artifact_digest
        .strip_prefix("sha256:")
        .unwrap_or(artifact_digest);
    let statement = statements
        .iter()
        .find(|statement| {
            statement
                .get("subject")
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
                .any(|subject| {
                    subject.pointer("/digest/sha256").and_then(|v| v.as_str()) == Some(expected)
                })
        })
        .ok_or_else(|| {
            Error::coded(
                "ARTIFACT_PROVENANCE_SUBJECT_MISMATCH",
                Category::Validation,
                format!(
                    "{PROVENANCE_FILE} has no subject matching artifact digest {artifact_digest}"
                ),
            )
        })?;

    let required_source =
        |document: &serde_json::Value, pointer: &str, description: &str| -> Result<String> {
            document
                .pointer(pointer)
                .and_then(|value| value.as_str())
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    Error::coded(
                        "ARTIFACT_PROVENANCE_SOURCE_MISMATCH",
                        Category::Validation,
                        format!("missing {description}"),
                    )
                })
        };
    let build_repository = required_source(
        manifest,
        "/build/source/repository",
        "manifest build.source.repository",
    )?;
    let build_revision = required_source(
        manifest,
        "/build/source/revision",
        "manifest build.source.revision",
    )?;
    let attested_repository = required_source(
        statement,
        "/predicate/buildDefinition/externalParameters/source/repository",
        "provenance source repository",
    )?;
    let attested_revision = required_source(
        statement,
        "/predicate/buildDefinition/externalParameters/source/revision",
        "provenance source revision",
    )?;
    if build_repository != attested_repository || build_revision != attested_revision {
        return Err(Error::coded(
            "ARTIFACT_PROVENANCE_SOURCE_MISMATCH",
            Category::Validation,
            "provenance source repository/revision does not match manifest build metadata",
        ));
    }

    let Some(policy) = policy else {
        return Ok(None);
    };
    let identity: PublisherIdentity = serde_json::from_value(
        statement
            .pointer("/predicate/runDetails/builder/identity")
            .cloned()
            .ok_or_else(|| {
                Error::coded(
                    "ARTIFACT_PROVENANCE_BUILDER_UNTRUSTED",
                    Category::Validation,
                    "provenance builder carries no publisher identity",
                )
            })?,
    )
    .map_err(|error| {
        Error::coded(
            "ARTIFACT_PROVENANCE_BUILDER_UNTRUSTED",
            Category::Validation,
            format!("provenance publisher identity is invalid: {error}"),
        )
    })?;
    let matched = policy
        .allowed_publishers
        .iter()
        .find(|publisher| publisher.matches(&identity))
        .ok_or_else(|| {
            Error::coded(
                "ARTIFACT_PROVENANCE_BUILDER_UNTRUSTED",
                Category::Validation,
                "provenance builder identity matches no allowed publisher policy",
            )
        })?;
    Ok(Some(matched.id.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AllowedPublisher, TrustLevel};

    fn manifest() -> serde_json::Value {
        serde_json::json!({
            "schema": 1,
            "name": "toy",
            "version": "1.0.0",
            "archive": "toy.tar.zst",
            "archive_digest": format!("sha256:{}", "ab".repeat(32)),
            "files": [{ "path": "lib/toy.so", "sha256": format!("sha256:{}", "cd".repeat(32)), "size": 3 }],
            "build": {
                "source": { "repository": "owner/repo", "revision": "deadbeef" },
                "builder": {
                    "id": "https://github.com/owner/repo/.github/workflows/release.yml@refs/tags/v1",
                    "identity": {
                        "repository": "owner/repo",
                        "workflow_path": ".github/workflows/release.yml",
                        "git_ref": "refs/tags/v1",
                        "actor": "release-bot",
                        "event": "push"
                    }
                }
            }
        })
    }

    #[test]
    fn generated_evidence_binds_sbom_and_provenance_to_the_archive() {
        let root = camino::Utf8PathBuf::from_path_buf(std::env::temp_dir())
            .unwrap()
            .join(format!("ost-evidence-{}", std::process::id()));
        std::fs::create_dir_all(root.as_std_path()).unwrap();
        let mut manifest = manifest();
        let evidence = generate_evidence(&root, &mut manifest).unwrap();
        assert_eq!(evidence.len(), 2);
        verify_sbom(
            &root.join(SBOM_FILE),
            manifest["archive_digest"].as_str().unwrap(),
        )
        .unwrap();
        verify_provenance(
            &root.join(PROVENANCE_FILE),
            &manifest,
            manifest["archive_digest"].as_str().unwrap(),
            None,
        )
        .unwrap();
        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn provenance_requires_the_manifest_source_and_an_allowed_builder() {
        let root = camino::Utf8PathBuf::from_path_buf(std::env::temp_dir())
            .unwrap()
            .join(format!("ost-provenance-policy-{}", std::process::id()));
        std::fs::create_dir_all(root.as_std_path()).unwrap();
        let mut manifest = manifest();
        generate_evidence(&root, &mut manifest).unwrap();
        let policy = ArtifactPolicy {
            schema: 1,
            minimum_trust: TrustLevel::Local,
            protected_namespaces: Vec::new(),
            allowed_publishers: vec![AllowedPublisher {
                id: "release".into(),
                trust: TrustLevel::Trusted,
                repository: "owner/repo".into(),
                workflow_path: ".github/workflows/release.yml".into(),
                git_refs: vec!["refs/tags/*".into()],
                actors: vec!["release-bot".into()],
                events: vec!["push".into()],
            }],
        };
        let digest = manifest["archive_digest"].as_str().unwrap();
        assert_eq!(
            verify_provenance(
                &root.join(PROVENANCE_FILE),
                &manifest,
                digest,
                Some(&policy),
            )
            .unwrap()
            .as_deref(),
            Some("release")
        );

        let mut wrong_builder = policy.clone();
        wrong_builder.allowed_publishers[0].repository = "someone/else".into();
        let error = verify_provenance(
            &root.join(PROVENANCE_FILE),
            &manifest,
            digest,
            Some(&wrong_builder),
        )
        .expect_err("an unapproved builder must be refused");
        assert_eq!(error.code(), "ARTIFACT_PROVENANCE_BUILDER_UNTRUSTED");

        let mut wrong_source = manifest.clone();
        wrong_source["build"]["source"]["revision"] = "different".into();
        let error = verify_provenance(&root.join(PROVENANCE_FILE), &wrong_source, digest, None)
            .expect_err("provenance for a different revision must be refused");
        assert_eq!(error.code(), "ARTIFACT_PROVENANCE_SOURCE_MISMATCH");

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }
}
