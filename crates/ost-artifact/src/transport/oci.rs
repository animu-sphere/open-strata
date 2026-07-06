// SPDX-License-Identifier: Apache-2.0
//! Read-only OCI registry transport (transport plan, Phase 1).
//!
//! Speaks the OCI Distribution Spec pull surface against GHCR-class registries
//! (GHCR, Harbor, ECR, GAR, ACR, `registry:2`), consuming the ORAS-style
//! artifact bundle this crate defines: an OCI image manifest whose layers are
//! the canonical `tar.zst` archive and the producer `manifest.json`
//! (identified by media type, with the `org.opencontainers.image.title`
//! annotation as a fallback for bundles pushed by a stock `oras` CLI).
//!
//! Every transferred byte is digest-checked against the OCI descriptor chain
//! before the artifact core sees it; the pull verification chain
//! ([`crate::transport::verify`]) then re-proves the producer manifest's own
//! claims. Auth follows the registry token flow: anonymous → `WWW-Authenticate`
//! bearer exchange (optionally with basic credentials from the environment) →
//! retry; or a static token via `OST_REGISTRY_TOKEN`.
//!
//! Redirects are followed manually (blob GETs redirect to CDNs) so the
//! `Authorization` header is never replayed to a different host.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};

use ost_core::{digest, Category, Error, Result};

use crate::record::{is_sha256_ref, MANIFEST_FILE};
use crate::reference::{OciReference, RemoteReference};
use crate::transport::{ArtifactTransport, ResolvedRemote};

/// OCI `artifactType` for OpenStrata bundles.
pub const OCI_ARTIFACT_TYPE: &str = "application/vnd.openstrata.artifact.v1";
/// Layer media type of the canonical artifact archive.
pub const MEDIA_TYPE_ARCHIVE: &str = "application/vnd.openstrata.artifact.archive.v1+tar+zstd";
/// Layer media type of the producer manifest JSON.
pub const MEDIA_TYPE_PRODUCER_MANIFEST: &str =
    "application/vnd.openstrata.artifact.manifest.v1+json";
/// Config media type: the OpenStrata artifact descriptor (unused on pull).
pub const MEDIA_TYPE_DESCRIPTOR: &str = "application/vnd.openstrata.artifact.descriptor.v1+json";

/// OCI annotation carrying a layer's original filename.
const TITLE_ANNOTATION: &str = "org.opencontainers.image.title";

/// Accepted OCI manifest media types on resolve.
const MANIFEST_ACCEPT: &str =
    "application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json";

/// Hard caps on JSON documents fetched into memory. Archives stream to disk;
/// these only bound manifests, so a hostile registry cannot balloon memory.
const MAX_OCI_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PRODUCER_MANIFEST_BYTES: u64 = 256 * 1024 * 1024;

/// Maximum redirect hops on a blob GET (registries bounce to CDN storage).
const MAX_REDIRECTS: usize = 5;

/// Environment variables supplying registry credentials.
const ENV_TOKEN: &str = "OST_REGISTRY_TOKEN";
const ENV_USER: &str = "OST_REGISTRY_USER";
const ENV_PASSWORD: &str = "OST_REGISTRY_PASSWORD";

/// Read-only OCI registry backend.
pub struct OciTransport {
    agent: ureq::Agent,
    /// Use plain `http://` instead of `https://` — fixture registries and
    /// air-gapped mirrors only, never the public internet.
    plain_http: bool,
    /// Bearer tokens per registry host, filled by the 401 exchange.
    tokens: RefCell<HashMap<String, String>>,
    /// Verified OCI manifest bytes per resolved digest, so `fetch` right after
    /// `resolve` does not re-download.
    manifests: RefCell<HashMap<String, Vec<u8>>>,
    /// How the last request authenticated, for pull evidence.
    auth_mode: RefCell<&'static str>,
}

impl OciTransport {
    pub fn new(plain_http: bool) -> OciTransport {
        OciTransport {
            agent: ureq::AgentBuilder::new()
                .redirects(0) // followed manually; see get_blob
                .timeout_connect(Duration::from_secs(30))
                .timeout_read(Duration::from_secs(120))
                .user_agent(&format!("ost/{}", env!("CARGO_PKG_VERSION")))
                .build(),
            plain_http,
            tokens: RefCell::new(HashMap::new()),
            manifests: RefCell::new(HashMap::new()),
            auth_mode: RefCell::new("anonymous"),
        }
    }

    fn oci<'a>(&self, reference: &'a RemoteReference) -> Result<&'a OciReference> {
        match reference {
            RemoteReference::Oci(r) => Ok(r),
            RemoteReference::File(f) => Err(Error::usage(format!(
                "'{}' is a file reference — the OCI transport only handles oci://",
                f.locator()
            ))),
        }
    }

    fn base_url(&self, registry: &str) -> String {
        let scheme = if self.plain_http { "http" } else { "https" };
        format!("{scheme}://{registry}")
    }

    /// One authorized GET against the registry, with a single token-exchange
    /// retry on 401. Returns any non-4xx/5xx response (including 3xx).
    fn get(&self, reference: &OciReference, url: &str, accept: &str) -> Result<ureq::Response> {
        let auth = self.auth_header(reference)?;
        match self.raw_get(url, accept, auth.as_deref()) {
            Err(RequestFailure::Status(401, resp)) => {
                let challenge = resp.header("www-authenticate").unwrap_or("").to_string();
                let token = self.exchange_token(reference, &challenge)?;
                let bearer = format!("Bearer {token}");
                match self.raw_get(url, accept, Some(&bearer)) {
                    Ok(resp) => Ok(resp),
                    Err(f) => Err(self.classify(reference, url, f)),
                }
            }
            Ok(resp) => Ok(resp),
            Err(f) => Err(self.classify(reference, url, f)),
        }
    }

    fn raw_get(
        &self,
        url: &str,
        accept: &str,
        auth: Option<&str>,
    ) -> std::result::Result<ureq::Response, RequestFailure> {
        let mut req = self.agent.get(url).set("Accept", accept);
        if let Some(auth) = auth {
            req = req.set("Authorization", auth);
        }
        match req.call() {
            Ok(resp) => Ok(resp),
            Err(ureq::Error::Status(code, resp)) => {
                Err(RequestFailure::Status(code, Box::new(resp)))
            }
            Err(ureq::Error::Transport(t)) => Err(RequestFailure::Transport(t.to_string())),
        }
    }

    /// Map a request failure onto the stable `ARTIFACT_*` codes.
    fn classify(&self, reference: &OciReference, url: &str, failure: RequestFailure) -> Error {
        match failure {
            RequestFailure::Status(404, _) => Error::coded(
                "ARTIFACT_REMOTE_NOT_FOUND",
                Category::Precondition,
                format!(
                    "'{}' does not exist on {} (404 at {url})",
                    reference.locator(),
                    reference.registry
                ),
            )
            .with_hint("check the repository path and that the artifact was published"),
            RequestFailure::Status(code @ (401 | 403), _) => Error::coded(
                "ARTIFACT_AUTH_DENIED",
                Category::Precondition,
                format!(
                    "{} denied access to '{}' ({code})",
                    reference.registry,
                    reference.locator()
                ),
            )
            .with_hint(format!(
                "for a private registry set {ENV_TOKEN}, or {ENV_USER} + {ENV_PASSWORD} \
                 for the token exchange"
            )),
            RequestFailure::Status(code, _) => Error::coded(
                "ARTIFACT_TRANSPORT_FAILED",
                Category::ExternalTool,
                format!("{url} answered HTTP {code}"),
            ),
            RequestFailure::Transport(msg) => Error::coded(
                "ARTIFACT_TRANSPORT_FAILED",
                Category::ExternalTool,
                format!("request to {url} failed: {msg}"),
            ),
        }
    }

    /// The Authorization header to try first: a cached bearer, a static token
    /// from the environment, or nothing (anonymous).
    fn auth_header(&self, reference: &OciReference) -> Result<Option<String>> {
        if let Some(token) = self.tokens.borrow().get(&reference.registry) {
            return Ok(Some(format!("Bearer {token}")));
        }
        if let Ok(token) = std::env::var(ENV_TOKEN) {
            if !token.trim().is_empty() {
                *self.auth_mode.borrow_mut() = "static-token";
                return Ok(Some(format!("Bearer {}", token.trim())));
            }
        }
        Ok(None)
    }

    /// Run the registry token exchange named by a 401's `WWW-Authenticate`.
    fn exchange_token(&self, reference: &OciReference, challenge: &str) -> Result<String> {
        let auth_denied = |detail: String| {
            Error::coded(
                "ARTIFACT_AUTH_DENIED",
                Category::Precondition,
                format!(
                    "{} requires authentication for '{}': {detail}",
                    reference.registry,
                    reference.locator()
                ),
            )
            .with_hint(format!(
                "for a private registry set {ENV_TOKEN}, or {ENV_USER} + {ENV_PASSWORD} \
                 for the token exchange"
            ))
        };

        let params = parse_challenge(challenge);
        if !challenge
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("bearer")
        {
            return Err(auth_denied(format!(
                "unsupported challenge '{challenge}' (only Bearer token exchange is supported)"
            )));
        }
        let realm = params
            .get("realm")
            .ok_or_else(|| auth_denied("challenge names no realm".to_string()))?;

        let mut url = format!("{realm}?");
        if let Some(service) = params.get("service") {
            url.push_str(&format!("service={}&", url_encode(service)));
        }
        let scope = params
            .get("scope")
            .cloned()
            .unwrap_or_else(|| format!("repository:{}:pull", reference.repository));
        url.push_str(&format!("scope={}", url_encode(&scope)));

        // Basic credentials, when the environment provides them; GHCR-class
        // public pulls succeed anonymously.
        let user = std::env::var(ENV_USER).ok().filter(|s| !s.is_empty());
        let password = std::env::var(ENV_PASSWORD).ok().filter(|s| !s.is_empty());
        let basic = match (user, password) {
            (Some(u), Some(p)) => Some(format!("Basic {}", base64(format!("{u}:{p}").as_bytes()))),
            _ => None,
        };
        *self.auth_mode.borrow_mut() = if basic.is_some() {
            "token-exchange-basic"
        } else {
            "token-exchange"
        };

        let resp = self
            .raw_get(&url, "application/json", basic.as_deref())
            .map_err(|f| match f {
                RequestFailure::Status(code, _) => {
                    auth_denied(format!("token endpoint {realm} answered HTTP {code}"))
                }
                RequestFailure::Transport(msg) => Error::coded(
                    "ARTIFACT_TRANSPORT_FAILED",
                    Category::ExternalTool,
                    format!("token request to {realm} failed: {msg}"),
                ),
            })?;
        let body = read_capped(&mut resp.into_reader(), MAX_OCI_MANIFEST_BYTES)
            .map_err(|e| Error::io(url.clone(), e))?;
        let json: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|e| auth_denied(format!("token endpoint returned invalid JSON: {e}")))?;
        let token = json
            .get("token")
            .or_else(|| json.get("access_token"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| auth_denied("token endpoint returned no token".to_string()))?
            .to_string();
        self.tokens
            .borrow_mut()
            .insert(reference.registry.clone(), token.clone());
        Ok(token)
    }

    /// Fetch the OCI manifest for a tag or digest reference, verified against
    /// the pin when the reference carries one. Returns (bytes, resolved digest).
    fn fetch_manifest(&self, reference: &OciReference) -> Result<(Vec<u8>, String)> {
        let manifest_ref = reference.manifest_reference()?;
        let url = format!(
            "{}/v2/{}/manifests/{manifest_ref}",
            self.base_url(&reference.registry),
            reference.repository
        );
        let resp = self.get(reference, &url, MANIFEST_ACCEPT)?;
        let body = read_capped(&mut resp.into_reader(), MAX_OCI_MANIFEST_BYTES)
            .map_err(|e| Error::io(url.clone(), e))?;
        let computed = digest::sha256_hex(&body);
        if let Some(pin) = &reference.digest {
            if *pin != computed {
                return Err(Error::coded(
                    "ARTIFACT_OCI_DIGEST_MISMATCH",
                    Category::Validation,
                    format!(
                        "OCI manifest at '{}' hashes to {computed} but the reference pins {pin}",
                        reference.locator()
                    ),
                )
                .with_hint("the registry served different bytes than the pin — do not trust it"));
            }
        }
        self.manifests
            .borrow_mut()
            .insert(computed.clone(), body.clone());
        Ok((body, computed))
    }

    /// Download one blob to `dest`, streaming and hashing, and require it to
    /// match the descriptor's digest and size.
    fn fetch_blob_to(
        &self,
        reference: &OciReference,
        layer: &Layer,
        dest: &Utf8Path,
    ) -> Result<()> {
        let url = format!(
            "{}/v2/{}/blobs/{}",
            self.base_url(&reference.registry),
            reference.repository,
            layer.digest
        );

        let mut resp = self.get(reference, &url, "application/octet-stream")?;
        // Follow CDN redirects by hand so Authorization never crosses hosts.
        let mut hops = 0;
        let mut current_url = url.clone();
        while (300..400).contains(&resp.status()) {
            hops += 1;
            if hops > MAX_REDIRECTS {
                return Err(Error::coded(
                    "ARTIFACT_TRANSPORT_FAILED",
                    Category::ExternalTool,
                    format!("{url} redirected more than {MAX_REDIRECTS} times"),
                ));
            }
            let location = resp.header("location").ok_or_else(|| {
                Error::coded(
                    "ARTIFACT_TRANSPORT_FAILED",
                    Category::ExternalTool,
                    format!("{current_url} redirected without a Location header"),
                )
            })?;
            let next = absolutize(&current_url, location);
            let same_host = host_of(&next) == host_of(&current_url);
            let auth = if same_host {
                self.auth_header(reference)?
            } else {
                None
            };
            resp = self
                .raw_get(&next, "application/octet-stream", auth.as_deref())
                .map_err(|f| self.classify(reference, &next, f))?;
            current_url = next;
        }

        let file = std::fs::File::create(dest.as_std_path())
            .map_err(|e| Error::io(dest.to_string(), e))?;
        let mut writer = std::io::BufWriter::new(file);
        let (actual, size) = digest::sha256_hex_copy(&mut resp.into_reader(), &mut writer)
            .map_err(|e| Error::io(dest.to_string(), e))?;
        drop(writer);
        if actual != layer.digest || size != layer.size {
            let _ = std::fs::remove_file(dest.as_std_path());
            return Err(Error::coded(
                "ARTIFACT_OCI_DIGEST_MISMATCH",
                Category::Validation,
                format!(
                    "blob {} from '{}' arrived as {actual} ({size} bytes), expected {} bytes",
                    layer.digest,
                    reference.locator(),
                    layer.size
                ),
            )
            .with_hint("the registry served corrupted or substituted bytes — do not trust it"));
        }
        Ok(())
    }
}

impl ArtifactTransport for OciTransport {
    fn resolve(&self, reference: &RemoteReference) -> Result<ResolvedRemote> {
        let r = self.oci(reference)?;
        let (_, resolved_digest) = self.fetch_manifest(r)?;
        Ok(ResolvedRemote {
            locator: format!("oci://{}/{}@{resolved_digest}", r.registry, r.repository),
            registry: r.registry.clone(),
            repository: r.repository.clone(),
            oci_digest: Some(resolved_digest),
            auth_mode: self.auth_mode.borrow().to_string(),
        })
    }

    fn fetch(
        &self,
        reference: &RemoteReference,
        resolved: &ResolvedRemote,
        scratch: &Utf8Path,
    ) -> Result<Utf8PathBuf> {
        let r = self.oci(reference)?;
        let oci_digest = resolved.oci_digest.as_deref().ok_or_else(|| {
            Error::usage("fetch needs a resolved OCI digest — call resolve first")
        })?;

        let manifest_bytes = match self.manifests.borrow().get(oci_digest) {
            Some(bytes) => bytes.clone(),
            None => {
                let pinned = OciReference {
                    digest: Some(oci_digest.to_string()),
                    tag: None,
                    ..r.clone()
                };
                self.fetch_manifest(&pinned)?.0
            }
        };
        let manifest = parse_oci_manifest(&manifest_bytes, &resolved.locator)?;

        // Producer manifest first: it is small and names the archive.
        let producer_layer = manifest
            .find_layer(MEDIA_TYPE_PRODUCER_MANIFEST, |title| title == MANIFEST_FILE)
            .ok_or_else(|| {
                oci_manifest_invalid(
                    &resolved.locator,
                    "no producer-manifest layer (media type \
                     application/vnd.openstrata.artifact.manifest.v1+json or a \
                     manifest.json title annotation)",
                )
            })?;
        if producer_layer.size > MAX_PRODUCER_MANIFEST_BYTES {
            return Err(oci_manifest_invalid(
                &resolved.locator,
                &format!(
                    "producer-manifest layer claims {} bytes (limit {MAX_PRODUCER_MANIFEST_BYTES})",
                    producer_layer.size
                ),
            ));
        }
        let producer_path = scratch.join(MANIFEST_FILE);
        self.fetch_blob_to(r, producer_layer, &producer_path)?;

        let producer: serde_json::Value = serde_json::from_slice(
            &std::fs::read(producer_path.as_std_path())
                .map_err(|e| Error::io(producer_path.to_string(), e))?,
        )
        .map_err(|e| {
            Error::coded(
                "ARTIFACT_MANIFEST_INVALID",
                Category::Validation,
                format!(
                    "producer manifest from '{}' is not JSON: {e}",
                    resolved.locator
                ),
            )
        })?;
        let archive_name = producer
            .get("archive")
            .and_then(|v| v.as_str())
            .filter(|name| is_plain_filename(name))
            .ok_or_else(|| {
                Error::coded(
                    "ARTIFACT_MANIFEST_INVALID",
                    Category::Validation,
                    format!(
                        "producer manifest from '{}' names no plain archive filename",
                        resolved.locator
                    ),
                )
            })?
            .to_string();

        let archive_layer = manifest
            .find_layer(MEDIA_TYPE_ARCHIVE, |title| title == archive_name)
            .ok_or_else(|| {
                oci_manifest_invalid(
                    &resolved.locator,
                    &format!(
                        "no archive layer (media type {MEDIA_TYPE_ARCHIVE} or a \
                         '{archive_name}' title annotation)"
                    ),
                )
            })?;
        self.fetch_blob_to(r, archive_layer, &scratch.join(&archive_name))?;

        Ok(scratch.to_owned())
    }
}

/// A request that did not return 2xx/3xx. The response is boxed so the happy
/// path never carries the failure variant's weight.
enum RequestFailure {
    Status(u16, Box<ureq::Response>),
    Transport(String),
}

/// The slice of an OCI image manifest the pull path needs.
#[derive(Debug)]
struct OciManifest {
    layers: Vec<Layer>,
}

#[derive(Debug)]
struct Layer {
    digest: String,
    size: u64,
    media_type: String,
    title: Option<String>,
}

impl OciManifest {
    /// Find a layer by media type, falling back to its title annotation.
    fn find_layer(&self, media_type: &str, title_matches: impl Fn(&str) -> bool) -> Option<&Layer> {
        self.layers
            .iter()
            .find(|l| l.media_type == media_type)
            .or_else(|| {
                self.layers
                    .iter()
                    .find(|l| l.title.as_deref().is_some_and(&title_matches))
            })
    }
}

fn oci_manifest_invalid(locator: &str, why: &str) -> Error {
    Error::coded(
        "ARTIFACT_MANIFEST_INVALID",
        Category::Validation,
        format!("OCI manifest at '{locator}' is not an OpenStrata artifact bundle: {why}"),
    )
}

fn parse_oci_manifest(bytes: &[u8], locator: &str) -> Result<OciManifest> {
    let json: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| oci_manifest_invalid(locator, &format!("not valid JSON: {e}")))?;
    let layers = json
        .get("layers")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            oci_manifest_invalid(
                locator,
                "no layers array (a multi-arch index is not an artifact bundle — \
                 point at a platform manifest)",
            )
        })?;
    let layers = layers
        .iter()
        .map(|l| {
            let digest = l
                .get("digest")
                .and_then(|v| v.as_str())
                .filter(|d| is_sha256_ref(d))
                .ok_or_else(|| oci_manifest_invalid(locator, "a layer carries no sha256 digest"))?;
            let size = l
                .get("size")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| oci_manifest_invalid(locator, "a layer carries no size"))?;
            Ok(Layer {
                digest: digest.to_string(),
                size,
                media_type: l
                    .get("mediaType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                title: l
                    .get("annotations")
                    .and_then(|a| a.get(TITLE_ANNOTATION))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(OciManifest { layers })
}

/// Parse a `WWW-Authenticate` challenge's `key="value"` parameters.
fn parse_challenge(header: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    let body = header
        .trim_start()
        .split_once(' ')
        .map(|(_, rest)| rest)
        .unwrap_or("");
    for part in body.split(',') {
        if let Some((key, value)) = part.split_once('=') {
            params.insert(
                key.trim().to_ascii_lowercase(),
                value.trim().trim_matches('"').to_string(),
            );
        }
    }
    params
}

/// Resolve a possibly relative redirect target against the current URL.
fn absolutize(current: &str, location: &str) -> String {
    if location.starts_with("http://") || location.starts_with("https://") {
        return location.to_string();
    }
    let (scheme, rest) = current.split_once("://").unwrap_or(("https", current));
    let host = rest.split('/').next().unwrap_or(rest);
    if location.starts_with('/') {
        format!("{scheme}://{host}{location}")
    } else {
        format!("{scheme}://{host}/{location}")
    }
}

fn host_of(url: &str) -> &str {
    url.split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(rest))
        .unwrap_or(url)
}

/// Minimal query-string escaping for token-exchange parameters.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b':' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Standard base64 (RFC 4648) for Basic credentials — small enough inline that
/// a dependency is not worth its supply-chain surface.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// A filename safe to create inside the scratch directory.
fn is_plain_filename(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains(':')
        && !name.chars().any(char::is_control)
}

fn read_capped(reader: &mut impl Read, cap: u64) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let read = reader.take(cap + 1).read_to_end(&mut out)?;
    if read as u64 > cap {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("response exceeded the {cap}-byte limit"),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
    }

    #[test]
    fn challenge_parsing() {
        let params = parse_challenge(
            r#"Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:o/r:pull""#,
        );
        assert_eq!(params.get("realm").unwrap(), "https://ghcr.io/token");
        assert_eq!(params.get("service").unwrap(), "ghcr.io");
        assert_eq!(params.get("scope").unwrap(), "repository:o/r:pull");
    }

    #[test]
    fn redirect_absolutize() {
        assert_eq!(
            absolutize(
                "https://ghcr.io/v2/o/r/blobs/sha256:ab",
                "https://cdn.example.com/x"
            ),
            "https://cdn.example.com/x"
        );
        assert_eq!(
            absolutize("https://ghcr.io/v2/o/r/blobs/sha256:ab", "/v2/other"),
            "https://ghcr.io/v2/other"
        );
        assert_eq!(host_of("https://ghcr.io/v2/x"), "ghcr.io");
        assert_eq!(host_of("http://localhost:5000/v2/x"), "localhost:5000");
    }

    #[test]
    fn manifest_layer_lookup_prefers_media_type_then_title() {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "layers": [
                {
                    "digest": format!("sha256:{}", "aa".repeat(32)),
                    "size": 10,
                    "mediaType": "application/octet-stream",
                    "annotations": { TITLE_ANNOTATION: "manifest.json" },
                },
                {
                    "digest": format!("sha256:{}", "bb".repeat(32)),
                    "size": 20,
                    "mediaType": MEDIA_TYPE_ARCHIVE,
                },
            ],
        }))
        .unwrap();
        let manifest = parse_oci_manifest(&bytes, "oci://x/y").unwrap();
        let producer = manifest
            .find_layer(MEDIA_TYPE_PRODUCER_MANIFEST, |t| t == MANIFEST_FILE)
            .unwrap();
        assert!(producer.digest.starts_with("sha256:aa"));
        let archive = manifest
            .find_layer(MEDIA_TYPE_ARCHIVE, |t| t == "whatever.tar.zst")
            .unwrap();
        assert!(archive.digest.starts_with("sha256:bb"));
    }

    #[test]
    fn index_without_layers_is_rejected() {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "manifests": [],
        }))
        .unwrap();
        let err = parse_oci_manifest(&bytes, "oci://x/y").unwrap_err();
        assert_eq!(err.code(), "ARTIFACT_MANIFEST_INVALID");
    }
}
