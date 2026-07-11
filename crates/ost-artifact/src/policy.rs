// SPDX-License-Identifier: Apache-2.0
//! Artifact trust policy parsing and publisher-identity matching.
//!
//! The policy is deliberately strict: unknown fields are rejected at every
//! level, references are checked when the file is loaded, and every failure has
//! a stable `ARTIFACT_POLICY_*` code. This keeps a misspelled trust control from
//! silently weakening the publish boundary.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::str::FromStr;

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use ost_core::{Category, Error, Result};

pub const ARTIFACT_POLICY_FILE: &str = "openstrata-artifact-policy.toml";
pub const ARTIFACT_POLICY_SCHEMA: u32 = 1;

/// Ordered assurance level carried by an artifact or publisher identity.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustLevel {
    #[default]
    Local,
    Unsigned,
    Attested,
    Verified,
    Trusted,
}

impl TrustLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            TrustLevel::Local => "local",
            TrustLevel::Unsigned => "unsigned",
            TrustLevel::Attested => "attested",
            TrustLevel::Verified => "verified",
            TrustLevel::Trusted => "trusted",
        }
    }
}

impl fmt::Display for TrustLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TrustLevel {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "local" => Ok(Self::Local),
            "unsigned" => Ok(Self::Unsigned),
            "attested" => Ok(Self::Attested),
            "verified" => Ok(Self::Verified),
            "trusted" => Ok(Self::Trusted),
            _ => Err(policy_invalid(format!(
                "unknown trust level '{value}' (expected local, unsigned, attested, verified, or trusted)"
            ))),
        }
    }
}

/// One strict policy document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPolicy {
    pub schema: u32,
    /// Minimum assurance accepted by `ost artifact verify --policy`.
    #[serde(default)]
    pub minimum_trust: TrustLevel,
    #[serde(default)]
    pub protected_namespaces: Vec<ProtectedNamespace>,
    #[serde(default)]
    pub allowed_publishers: Vec<AllowedPublisher>,
}

/// A registry/repository namespace whose pushes require an approved identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedNamespace {
    /// Registry plus repository prefix, e.g. `ghcr.io/animu-sphere`.
    pub namespace: String,
    pub minimum_trust: TrustLevel,
    pub allowed_publishers: Vec<String>,
}

/// An OIDC publisher rule. All five identity dimensions must match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowedPublisher {
    pub id: String,
    pub trust: TrustLevel,
    pub repository: String,
    pub workflow_path: String,
    pub git_refs: Vec<String>,
    pub actors: Vec<String>,
    pub events: Vec<String>,
}

/// Verified claims extracted from an OIDC identity token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublisherIdentity {
    pub repository: String,
    pub workflow_path: String,
    pub git_ref: String,
    pub actor: String,
    pub event: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublisherAuthorization {
    pub namespace: String,
    pub publisher: String,
    pub trust: TrustLevel,
}

impl ArtifactPolicy {
    pub fn load(path: &Utf8Path) -> Result<Self> {
        let text = fs::read_to_string(path.as_std_path()).map_err(|source| {
            Error::coded(
                "ARTIFACT_POLICY_READ_FAILED",
                Category::Io,
                format!("could not read artifact policy at {path}: {source}"),
            )
        })?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self> {
        let policy: ArtifactPolicy = toml::from_str(text).map_err(|source| {
            Error::coded(
                "ARTIFACT_POLICY_PARSE_FAILED",
                Category::Configuration,
                format!("artifact policy TOML is invalid: {source}"),
            )
        })?;
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != ARTIFACT_POLICY_SCHEMA {
            return Err(Error::coded(
                "ARTIFACT_POLICY_SCHEMA_UNSUPPORTED",
                Category::Configuration,
                format!(
                    "artifact policy schema {} is not supported (expected {})",
                    self.schema, ARTIFACT_POLICY_SCHEMA
                ),
            ));
        }

        let mut publisher_ids = HashSet::new();
        for publisher in &self.allowed_publishers {
            validate_publisher(publisher)?;
            if !publisher_ids.insert(publisher.id.as_str()) {
                return Err(policy_invalid(format!(
                    "allowed publisher id '{}' is duplicated",
                    publisher.id
                )));
            }
        }

        let mut namespaces = HashSet::new();
        for protected in &self.protected_namespaces {
            validate_namespace(protected)?;
            if !namespaces.insert(protected.namespace.as_str()) {
                return Err(policy_invalid(format!(
                    "protected namespace '{}' is duplicated",
                    protected.namespace
                )));
            }
            for id in &protected.allowed_publishers {
                let publisher = self.publisher(id).ok_or_else(|| {
                    policy_invalid(format!(
                        "protected namespace '{}' references unknown publisher '{id}'",
                        protected.namespace
                    ))
                })?;
                if publisher.trust < protected.minimum_trust {
                    return Err(policy_invalid(format!(
                        "publisher '{id}' has trust '{}' below namespace '{}' minimum '{}'",
                        publisher.trust, protected.namespace, protected.minimum_trust
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn publisher(&self, id: &str) -> Option<&AllowedPublisher> {
        self.allowed_publishers.iter().find(|p| p.id == id)
    }

    /// The most-specific protected namespace covering a registry/repository.
    pub fn protected_namespace(&self, destination: &str) -> Option<&ProtectedNamespace> {
        self.protected_namespaces
            .iter()
            .filter(|p| namespace_contains(&p.namespace, destination))
            .max_by_key(|p| p.namespace.len())
    }

    /// Match verified OIDC claims against the allow-list for a protected push.
    /// Unprotected destinations return `Ok(None)`.
    pub fn authorize_publisher(
        &self,
        destination: &str,
        identity: &PublisherIdentity,
    ) -> Result<Option<PublisherAuthorization>> {
        let Some(protected) = self.protected_namespace(destination) else {
            return Ok(None);
        };
        for id in &protected.allowed_publishers {
            let publisher = self
                .publisher(id)
                .expect("policy validation checks references");
            if publisher.matches(identity) {
                return Ok(Some(PublisherAuthorization {
                    namespace: protected.namespace.clone(),
                    publisher: publisher.id.clone(),
                    trust: publisher.trust,
                }));
            }
        }
        Err(Error::coded(
            "ARTIFACT_POLICY_PUBLISHER_UNTRUSTED",
            Category::Validation,
            format!(
                "publisher identity is not allowed to publish to protected namespace '{}'",
                protected.namespace
            ),
        )
        .with_hint("use an allowed repository/workflow/ref/actor/event identity, or pass --allow-untrusted-publisher explicitly"))
    }

    pub fn verify_trust(&self, actual: TrustLevel) -> Result<()> {
        if actual < self.minimum_trust {
            return Err(Error::coded(
                "ARTIFACT_POLICY_TRUST_INSUFFICIENT",
                Category::Validation,
                format!(
                    "artifact trust '{actual}' is below policy minimum '{}'",
                    self.minimum_trust
                ),
            ));
        }
        Ok(())
    }
}

impl AllowedPublisher {
    pub fn matches(&self, identity: &PublisherIdentity) -> bool {
        self.repository == identity.repository
            && self.workflow_path == identity.workflow_path
            && self
                .git_refs
                .iter()
                .any(|p| ref_matches(p, &identity.git_ref))
            && self.actors.iter().any(|v| v == &identity.actor)
            && self.events.iter().any(|v| v == &identity.event)
    }
}

fn validate_namespace(value: &ProtectedNamespace) -> Result<()> {
    if !is_namespace(&value.namespace) {
        return Err(policy_invalid(format!(
            "protected namespace '{}' must be a lowercase registry/repository prefix without a scheme or tag",
            value.namespace
        )));
    }
    if value.allowed_publishers.is_empty() {
        return Err(policy_invalid(format!(
            "protected namespace '{}' must allow at least one publisher",
            value.namespace
        )));
    }
    Ok(())
}

fn validate_publisher(value: &AllowedPublisher) -> Result<()> {
    if value.id.trim().is_empty()
        || value.repository.trim().is_empty()
        || value.workflow_path.trim().is_empty()
        || value.git_refs.is_empty()
        || value.actors.is_empty()
        || value.events.is_empty()
    {
        return Err(policy_invalid(
            "allowed publisher id, repository, workflow_path, git_refs, actors, and events must all be non-empty",
        ));
    }
    if !value.workflow_path.starts_with(".github/workflows/")
        || !value.workflow_path.ends_with(".yml") && !value.workflow_path.ends_with(".yaml")
    {
        return Err(policy_invalid(format!(
            "publisher '{}' workflow_path must name a .github/workflows/*.yml or *.yaml file",
            value.id
        )));
    }
    for pattern in &value.git_refs {
        let stars = pattern.bytes().filter(|b| *b == b'*').count();
        if !pattern.starts_with("refs/") || stars > 1 || stars == 1 && !pattern.ends_with('*') {
            return Err(policy_invalid(format!(
                "publisher '{}' git ref pattern '{}' must start with refs/ and may only use a trailing *",
                value.id, pattern
            )));
        }
    }
    Ok(())
}

fn policy_invalid(message: impl Into<String>) -> Error {
    Error::coded("ARTIFACT_POLICY_INVALID", Category::Configuration, message)
}

fn is_namespace(value: &str) -> bool {
    !value.is_empty()
        && value == value.to_ascii_lowercase()
        && !value.contains("://")
        && !value.contains('@')
        && !value.contains(':')
        && !value.starts_with('/')
        && !value.ends_with('/')
        && value.split('/').count() >= 2
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"./_-".contains(&b))
}

fn namespace_contains(namespace: &str, destination: &str) -> bool {
    destination == namespace
        || destination
            .strip_prefix(namespace)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn ref_matches(pattern: &str, value: &str) -> bool {
    pattern
        .strip_suffix('*')
        .map_or(value == pattern, |prefix| value.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
schema = 1
minimum_trust = "unsigned"

[[protected_namespaces]]
namespace = "ghcr.io/animu-sphere"
minimum_trust = "verified"
allowed_publishers = ["release"]

[[allowed_publishers]]
id = "release"
trust = "verified"
repository = "animu-sphere/open-strata"
workflow_path = ".github/workflows/release.yml"
git_refs = ["refs/tags/v*"]
actors = ["release-bot"]
events = ["push"]
"#;

    fn identity() -> PublisherIdentity {
        PublisherIdentity {
            repository: "animu-sphere/open-strata".into(),
            workflow_path: ".github/workflows/release.yml".into(),
            git_ref: "refs/tags/v0.14.0".into(),
            actor: "release-bot".into(),
            event: "push".into(),
        }
    }

    #[test]
    fn parses_strict_policy_and_orders_trust() {
        let policy = ArtifactPolicy::parse(VALID).unwrap();
        assert_eq!(policy.minimum_trust, TrustLevel::Unsigned);
        assert!(TrustLevel::Trusted > TrustLevel::Verified);
        assert!(TrustLevel::Attested > TrustLevel::Unsigned);
    }

    #[test]
    fn rejects_unknown_fields_with_stable_code() {
        let err = ArtifactPolicy::parse(&VALID.replace(
            "events = [\"push\"]",
            "events = [\"push\"]\nevnets = [\"push\"]",
        ))
        .unwrap_err();
        assert_eq!(err.code(), "ARTIFACT_POLICY_PARSE_FAILED");
        assert_eq!(err.category(), Category::Configuration);
    }

    #[test]
    fn rejects_unknown_publisher_reference() {
        let err = ArtifactPolicy::parse(&VALID.replace(
            "allowed_publishers = [\"release\"]",
            "allowed_publishers = [\"missing\"]",
        ))
        .unwrap_err();
        assert_eq!(err.code(), "ARTIFACT_POLICY_INVALID");
    }

    #[test]
    fn matches_all_oidc_dimensions_for_protected_namespace() {
        let policy = ArtifactPolicy::parse(VALID).unwrap();
        let auth = policy
            .authorize_publisher("ghcr.io/animu-sphere/runtime/cy2026", &identity())
            .unwrap()
            .unwrap();
        assert_eq!(auth.publisher, "release");
        assert_eq!(auth.trust, TrustLevel::Verified);

        let mut wrong = identity();
        wrong.event = "pull_request".into();
        let err = policy
            .authorize_publisher("ghcr.io/animu-sphere/runtime", &wrong)
            .unwrap_err();
        assert_eq!(err.code(), "ARTIFACT_POLICY_PUBLISHER_UNTRUSTED");
        assert_eq!(err.category(), Category::Validation);
    }

    #[test]
    fn unprotected_namespace_needs_no_authorization() {
        let policy = ArtifactPolicy::parse(VALID).unwrap();
        assert_eq!(
            policy
                .authorize_publisher("example.com/scratch/runtime", &identity())
                .unwrap(),
            None
        );
    }

    #[test]
    fn enforces_minimum_artifact_trust() {
        let policy = ArtifactPolicy::parse(VALID).unwrap();
        let err = policy.verify_trust(TrustLevel::Local).unwrap_err();
        assert_eq!(err.code(), "ARTIFACT_POLICY_TRUST_INSUFFICIENT");
        policy.verify_trust(TrustLevel::Unsigned).unwrap();
    }
}
