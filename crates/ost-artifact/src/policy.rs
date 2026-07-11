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
use std::io::Read;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use ost_core::{Category, Error, Result};

pub const ARTIFACT_POLICY_FILE: &str = "openstrata-artifact-policy.toml";
pub const ARTIFACT_POLICY_SCHEMA: u32 = 1;
pub const ARTIFACT_POLICY_OIDC_AUDIENCE: &str = "openstrata-artifact-publish";

const GITHUB_OIDC_ISSUER: &str = "https://token.actions.githubusercontent.com";
const GITHUB_OIDC_REQUEST_URL: &str = "ACTIONS_ID_TOKEN_REQUEST_URL";
const GITHUB_OIDC_REQUEST_TOKEN: &str = "ACTIONS_ID_TOKEN_REQUEST_TOKEN";
const MAX_OIDC_RESPONSE_BYTES: u64 = 1024 * 1024;
const OIDC_CLOCK_SKEW_SECS: u64 = 60;

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

#[derive(Debug, Deserialize)]
struct GithubOidcResponse {
    value: String,
}

#[derive(Debug, Deserialize)]
struct GithubOidcClaims {
    iss: String,
    aud: Audience,
    exp: u64,
    #[serde(default)]
    nbf: Option<u64>,
    repository: String,
    workflow_ref: String,
    #[serde(rename = "ref")]
    git_ref: String,
    actor: String,
    event_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

impl Audience {
    fn contains(&self, expected: &str) -> bool {
        match self {
            Audience::One(value) => value == expected,
            Audience::Many(values) => values.iter().any(|value| value == expected),
        }
    }
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

    /// Find the nearest policy at `start` or one of its parent directories.
    pub fn discover(start: &Utf8Path) -> Result<Option<(Utf8PathBuf, Self)>> {
        for directory in start.ancestors() {
            let path = directory.join(ARTIFACT_POLICY_FILE);
            if path.as_std_path().is_file() {
                let policy = Self::load(&path)?;
                return Ok(Some((path, policy)));
            }
        }
        Ok(None)
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

impl PublisherIdentity {
    /// Request a short-lived GitHub Actions OIDC token and extract the claims
    /// used by publisher policy. The request URL is pinned to GitHub's issuer;
    /// claims are accepted only for OpenStrata's dedicated audience and time
    /// window. Since the JWT is obtained directly from that authenticated TLS
    /// endpoint, no caller-supplied token or unverified environment claims are
    /// trusted here.
    pub fn from_github_actions_oidc() -> Result<Self> {
        let request_url = required_oidc_env(GITHUB_OIDC_REQUEST_URL)?;
        let request_token = required_oidc_env(GITHUB_OIDC_REQUEST_TOKEN)?;
        if !is_github_oidc_request_url(&request_url) {
            return Err(identity_invalid(format!(
                "{GITHUB_OIDC_REQUEST_URL} must use {GITHUB_OIDC_ISSUER}, got an untrusted URL"
            )));
        }

        let separator = if request_url.contains('?') { '&' } else { '?' };
        let url = format!("{request_url}{separator}audience={ARTIFACT_POLICY_OIDC_AUDIENCE}");
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .user_agent(format!("ost/{}", env!("CARGO_PKG_VERSION")))
            .build();
        let agent = ureq::Agent::new_with_config(config);
        let response = agent
            .get(&url)
            .header("Accept", "application/json")
            .header("Authorization", format!("bearer {request_token}"))
            .call()
            .map_err(|source| {
                identity_unavailable(format!("could not request GitHub OIDC identity: {source}"))
            })?;
        if !(200..300).contains(&response.status().as_u16()) {
            return Err(identity_unavailable(format!(
                "GitHub OIDC identity request returned HTTP {}",
                response.status().as_u16()
            )));
        }

        let mut reader = response
            .into_body()
            .into_reader()
            .take(MAX_OIDC_RESPONSE_BYTES + 1);
        let mut body = Vec::new();
        reader.read_to_end(&mut body).map_err(|source| {
            identity_unavailable(format!("could not read GitHub OIDC response: {source}"))
        })?;
        if body.len() as u64 > MAX_OIDC_RESPONSE_BYTES {
            return Err(identity_invalid(
                "GitHub OIDC response is unexpectedly large",
            ));
        }
        let response: GithubOidcResponse = serde_json::from_slice(&body).map_err(|source| {
            identity_invalid(format!("GitHub OIDC response is invalid: {source}"))
        })?;
        Self::from_github_oidc_jwt(&response.value, unix_now()?)
    }

    fn from_github_oidc_jwt(token: &str, now: u64) -> Result<Self> {
        let mut segments = token.split('.');
        let header = segments.next().unwrap_or_default();
        let payload = segments.next().unwrap_or_default();
        let signature = segments.next().unwrap_or_default();
        if header.is_empty()
            || payload.is_empty()
            || signature.is_empty()
            || segments.next().is_some()
        {
            return Err(identity_invalid(
                "GitHub OIDC response did not contain a JWT",
            ));
        }
        let payload = decode_base64url(payload)
            .ok_or_else(|| identity_invalid("GitHub OIDC JWT payload is not valid base64url"))?;
        let claims: GithubOidcClaims = serde_json::from_slice(&payload).map_err(|source| {
            identity_invalid(format!("GitHub OIDC claims are invalid: {source}"))
        })?;

        if claims.iss != GITHUB_OIDC_ISSUER {
            return Err(identity_invalid(format!(
                "GitHub OIDC issuer '{}' is not trusted",
                claims.iss
            )));
        }
        if !claims.aud.contains(ARTIFACT_POLICY_OIDC_AUDIENCE) {
            return Err(identity_invalid(format!(
                "GitHub OIDC audience does not include '{ARTIFACT_POLICY_OIDC_AUDIENCE}'"
            )));
        }
        if claims.exp.saturating_add(OIDC_CLOCK_SKEW_SECS) < now {
            return Err(identity_invalid("GitHub OIDC token has expired"));
        }
        if claims
            .nbf
            .is_some_and(|not_before| not_before > now.saturating_add(OIDC_CLOCK_SKEW_SECS))
        {
            return Err(identity_invalid("GitHub OIDC token is not valid yet"));
        }

        let workflow_prefix = format!("{}/", claims.repository);
        let workflow = claims
            .workflow_ref
            .strip_prefix(&workflow_prefix)
            .and_then(|value| value.rsplit_once('@').map(|(path, _)| path))
            .filter(|path| path.starts_with(".github/workflows/"))
            .ok_or_else(|| {
                identity_invalid(format!(
                    "GitHub OIDC workflow_ref '{}' does not belong to repository '{}'",
                    claims.workflow_ref, claims.repository
                ))
            })?;

        Ok(PublisherIdentity {
            repository: claims.repository,
            workflow_path: workflow.to_string(),
            git_ref: claims.git_ref,
            actor: claims.actor,
            event: claims.event_name,
        })
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

fn identity_unavailable(message: impl Into<String>) -> Error {
    Error::coded(
        "ARTIFACT_POLICY_IDENTITY_UNAVAILABLE",
        Category::Precondition,
        message,
    )
    .with_hint("grant the job `id-token: write`, or pass --allow-untrusted-publisher explicitly")
}

fn identity_invalid(message: impl Into<String>) -> Error {
    Error::coded(
        "ARTIFACT_POLICY_IDENTITY_INVALID",
        Category::Validation,
        message,
    )
}

fn required_oidc_env(name: &str) -> Result<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            identity_unavailable(format!(
                "{name} is not available; protected publishes require a GitHub Actions OIDC identity"
            ))
        })
}

fn unix_now() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|source| {
            identity_invalid(format!("system clock is before the Unix epoch: {source}"))
        })
}

fn is_github_oidc_request_url(value: &str) -> bool {
    value
        .strip_prefix(GITHUB_OIDC_ISSUER)
        .is_some_and(|suffix| suffix.starts_with('/') && !suffix.starts_with("//"))
}

fn decode_base64url(value: &str) -> Option<Vec<u8>> {
    if value.len() % 4 == 1 {
        return None;
    }
    let mut output = Vec::with_capacity(value.len() * 3 / 4);
    let mut accumulator = 0u32;
    let mut bits = 0u8;
    for byte in value.bytes() {
        let decoded = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        } as u32;
        accumulator = (accumulator << 6) | decoded;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((accumulator >> bits) as u8);
            accumulator &= (1u32 << bits).saturating_sub(1);
        }
    }
    if accumulator != 0 {
        return None;
    }
    Some(output)
}

fn is_namespace(value: &str) -> bool {
    let Some((registry, repository)) = value.split_once('/') else {
        return false;
    };
    value == value.to_ascii_lowercase()
        && !value.contains("://")
        && !value.contains('@')
        && valid_registry_namespace(registry)
        && !repository.is_empty()
        && !repository.ends_with('/')
        && repository.split('/').all(|component| {
            !component.is_empty()
                && component
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        })
}

fn valid_registry_namespace(value: &str) -> bool {
    let (host, port) = match value.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (value, None),
    };
    !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b".-".contains(&b))
        && port.is_none_or(|port| {
            !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit())
                && port.parse::<u16>().is_ok_and(|value| value != 0)
        })
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

    fn base64url(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut output = String::new();
        for chunk in bytes.chunks(3) {
            let packed = ((chunk[0] as u32) << 16)
                | ((*chunk.get(1).unwrap_or(&0) as u32) << 8)
                | *chunk.get(2).unwrap_or(&0) as u32;
            output.push(TABLE[((packed >> 18) & 63) as usize] as char);
            output.push(TABLE[((packed >> 12) & 63) as usize] as char);
            if chunk.len() > 1 {
                output.push(TABLE[((packed >> 6) & 63) as usize] as char);
            }
            if chunk.len() > 2 {
                output.push(TABLE[(packed & 63) as usize] as char);
            }
        }
        output
    }

    fn oidc_jwt(overrides: serde_json::Value) -> String {
        let mut claims = serde_json::json!({
            "iss": GITHUB_OIDC_ISSUER,
            "aud": ARTIFACT_POLICY_OIDC_AUDIENCE,
            "exp": 2_000_000_000u64,
            "nbf": 1_700_000_000u64,
            "repository": "animu-sphere/open-strata",
            "workflow_ref": "animu-sphere/open-strata/.github/workflows/release.yml@refs/tags/v0.14.0",
            "ref": "refs/tags/v0.14.0",
            "actor": "release-bot",
            "event_name": "push",
        });
        for (key, value) in overrides.as_object().unwrap() {
            claims[key] = value.clone();
        }
        format!(
            "{}.{}.signature",
            base64url(br#"{"alg":"RS256","typ":"JWT"}"#),
            base64url(&serde_json::to_vec(&claims).unwrap())
        )
    }

    #[test]
    fn parses_strict_policy_and_orders_trust() {
        let policy = ArtifactPolicy::parse(VALID).unwrap();
        assert_eq!(policy.minimum_trust, TrustLevel::Unsigned);
        assert!(TrustLevel::Trusted > TrustLevel::Verified);
        assert!(TrustLevel::Attested > TrustLevel::Unsigned);
    }

    #[test]
    fn discovers_nearest_policy_from_a_child_directory() {
        let root = std::env::temp_dir().join(format!(
            "ost-policy-discovery-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let child = root.join("nested/child");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(root.join(ARTIFACT_POLICY_FILE), "schema = 1\n").unwrap();
        let child = Utf8PathBuf::from_path_buf(child).unwrap();
        let (path, policy) = ArtifactPolicy::discover(&child).unwrap().unwrap();
        assert_eq!(
            path,
            child
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(ARTIFACT_POLICY_FILE)
        );
        assert_eq!(policy.schema, ARTIFACT_POLICY_SCHEMA);
        std::fs::remove_dir_all(root).unwrap();
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
    fn protected_namespace_accepts_registry_ports() {
        let policy = ArtifactPolicy::parse(&VALID.replace(
            "ghcr.io/animu-sphere",
            "registry.internal:5000/animu-sphere",
        ))
        .unwrap();
        let auth = policy
            .authorize_publisher("registry.internal:5000/animu-sphere/runtime", &identity())
            .unwrap()
            .unwrap();
        assert_eq!(auth.namespace, "registry.internal:5000/animu-sphere");

        for invalid in [
            "registry.internal:/animu-sphere",
            "registry.internal:0/animu-sphere",
            "registry.internal:65536/animu-sphere",
            "registry.internal:tag/animu-sphere",
        ] {
            let err = ArtifactPolicy::parse(&VALID.replace("ghcr.io/animu-sphere", invalid))
                .expect_err(invalid);
            assert_eq!(err.code(), "ARTIFACT_POLICY_INVALID");
        }
    }

    #[test]
    fn github_oidc_claims_become_publisher_identity() {
        let actual = PublisherIdentity::from_github_oidc_jwt(
            &oidc_jwt(serde_json::json!({})),
            1_800_000_000,
        )
        .unwrap();
        assert_eq!(actual, identity());
    }

    #[test]
    fn github_oidc_claims_reject_wrong_audience_expiry_and_workflow() {
        let cases = [
            serde_json::json!({ "iss": "https://issuer.example" }),
            serde_json::json!({ "aud": "another-tool" }),
            serde_json::json!({ "exp": 1_700_000_000u64 }),
            serde_json::json!({ "nbf": 1_900_000_000u64 }),
            serde_json::json!({
                "workflow_ref": "other/repo/.github/workflows/release.yml@refs/tags/v0.14.0"
            }),
        ];
        for overrides in cases {
            let err = PublisherIdentity::from_github_oidc_jwt(&oidc_jwt(overrides), 1_800_000_000)
                .unwrap_err();
            assert_eq!(err.code(), "ARTIFACT_POLICY_IDENTITY_INVALID");
        }
    }

    #[test]
    fn github_oidc_request_url_is_pinned_to_the_issuer() {
        assert!(is_github_oidc_request_url(
            "https://token.actions.githubusercontent.com/example?x=1"
        ));
        assert!(!is_github_oidc_request_url(
            "https://token.actions.githubusercontent.com.evil.example/token"
        ));
        assert!(!is_github_oidc_request_url(
            "https://token.actions.githubusercontent.com//evil.example/token"
        ));
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
