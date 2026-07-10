// SPDX-License-Identifier: Apache-2.0
//! OCI registry transport (transport plan, Phase 1 pull + v0.10.0 push).
//!
//! Speaks the OCI Distribution Spec pull *and* push surface against GHCR-class
//! registries (GHCR, Harbor, ECR, GAR, ACR, `registry:2`), moving the ORAS-style
//! artifact bundle this crate defines: an OCI image manifest whose layers are
//! the canonical `tar.zst` archive and the producer `manifest.json`
//! (identified by media type, with the `org.opencontainers.image.title`
//! annotation as a fallback for bundles pushed by a stock `oras` CLI). Push
//! emits exactly that shape, so a pushed bundle round-trips cleanly back through
//! the pull path.
//!
//! Every transferred byte is digest-checked against the OCI descriptor chain
//! before the artifact core sees it; the pull verification chain
//! ([`crate::transport::verify`]) then re-proves the producer manifest's own
//! claims. Auth follows the registry token flow: anonymous → `WWW-Authenticate`
//! bearer exchange (optionally with basic credentials from the environment) →
//! retry; or a static token via `OST_REGISTRY_TOKEN`. A **push** needs the
//! credential path (`OST_REGISTRY_USER` + `OST_REGISTRY_PASSWORD`): the token
//! exchange requests `pull,push` scope, whereas a bearer presented verbatim
//! (`OST_REGISTRY_TOKEN`) is accepted for reads but cannot carry push scope on
//! GHCR-class registries, so a write with it answers 403.
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

/// The OCI image-manifest media type the push writes and registries index by.
pub const OCI_IMAGE_MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

/// The OCI "empty" config blob media type. An artifact bundle carries no
/// runnable config, so push references the 2-byte `{}` empty descriptor — the
/// same shape a stock `oras push` emits, and one the pull path ignores.
const MEDIA_TYPE_EMPTY_CONFIG: &str = "application/vnd.oci.empty.v1+json";

/// The canonical empty-config blob bytes (`{}`), uploaded as the manifest config.
const EMPTY_CONFIG_BYTES: &[u8] = b"{}";

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
                let scope = format!("repository:{}:pull", reference.repository);
                let token = self.exchange_token(reference, &challenge, &scope)?;
                let bearer = format!("Bearer {token}");
                match self.raw_get(url, accept, Some(&bearer)) {
                    Ok(resp) => Ok(resp),
                    Err(f) => Err(self.classify(reference, url, false, f)),
                }
            }
            Ok(resp) => Ok(resp),
            Err(f) => Err(self.classify(reference, url, false, f)),
        }
    }

    /// Follow registry redirects manually. Authorization is only preserved
    /// while the entire chain stays on the original registry origin.
    fn get_following_redirects(
        &self,
        reference: &OciReference,
        url: &str,
        accept: &str,
    ) -> Result<(ureq::Response, String)> {
        let registry_origin = origin_of(url).to_string();
        let mut auth_still_allowed = true;
        let mut resp = self.get(reference, url, accept)?;
        let mut hops = 0;
        let mut current_url = url.to_string();

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
            let auth = if redirect_auth_allowed(&registry_origin, &mut auth_still_allowed, &next) {
                self.auth_header(reference)?
            } else {
                None
            };
            resp = self
                .raw_get(&next, accept, auth.as_deref())
                .map_err(|f| self.classify(reference, &next, false, f))?;
            current_url = next;
        }

        if !(200..300).contains(&resp.status()) {
            return Err(self.classify(
                reference,
                &current_url,
                false,
                RequestFailure::Status(resp.status(), Box::new(resp)),
            ));
        }
        Ok((resp, current_url))
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

    /// Map a request failure onto the stable `ARTIFACT_*` codes. `write` selects
    /// the push-aware hint for a 401/403 (a read gets the pull-side hint).
    fn classify(
        &self,
        reference: &OciReference,
        url: &str,
        write: bool,
        failure: RequestFailure,
    ) -> Error {
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
            RequestFailure::Status(code @ (401 | 403), _) => {
                let hint = if write {
                    self.write_auth_hint(reference)
                } else {
                    format!(
                        "for a private registry set {ENV_TOKEN}, or {ENV_USER} + {ENV_PASSWORD} \
                         for the token exchange"
                    )
                };
                Error::coded(
                    "ARTIFACT_AUTH_DENIED",
                    Category::Precondition,
                    format!(
                        "{} denied access to '{}' ({code})",
                        reference.registry,
                        reference.locator()
                    ),
                )
                .with_hint(hint)
            }
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

    /// The hint for a write (`push`) that the registry answered 401/403, keyed on
    /// how this request authenticated. The common GHCR footgun: a raw
    /// `OST_REGISTRY_TOKEN` is accepted for reads but can never carry `push` scope
    /// — a bearer presented verbatim is not exchanged, so the registry answers 403
    /// (not 401, so there is no exchange retry). The fix is the credential path,
    /// which runs the token exchange requesting `pull,push`.
    fn write_auth_hint(&self, reference: &OciReference) -> String {
        let repo = &reference.repository;
        match *self.auth_mode.borrow() {
            "static-token" => format!(
                "{ENV_TOKEN} authenticated but the registry refused the write: a bearer token \
                 presented verbatim cannot obtain 'push' scope. Set {ENV_USER} + {ENV_PASSWORD} \
                 (a token with write:packages) so ost runs the credential token exchange for \
                 pull,push, and confirm the credential can publish to '{repo}'"
            ),
            "token-exchange-basic" => format!(
                "credentials were accepted but the registry still refused the write to '{repo}': \
                 confirm the token has write:packages scope and your account may publish there \
                 (on GHCR the package must grant you write, or the org must allow package creation)"
            ),
            // No credentials reached the exchange (anonymous, or an empty-scope
            // exchange): a push cannot be anonymous.
            _ => format!(
                "a push cannot be anonymous — set {ENV_USER} + {ENV_PASSWORD} (a token with \
                 write:packages that may publish to '{repo}') so ost runs the token exchange"
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
    /// `default_scope` is requested only when the challenge names none (push
    /// endpoints always name a `pull,push` scope, pull endpoints a `pull` one).
    fn exchange_token(
        &self,
        reference: &OciReference,
        challenge: &str,
        default_scope: &str,
    ) -> Result<String> {
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
            .unwrap_or_else(|| default_scope.to_string());
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
        let (resp, final_url) = self.get_following_redirects(reference, &url, MANIFEST_ACCEPT)?;
        let body = read_capped(&mut resp.into_reader(), MAX_OCI_MANIFEST_BYTES)
            .map_err(|e| Error::io(final_url, e))?;
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

        let (resp, _) =
            self.get_following_redirects(reference, &url, "application/octet-stream")?;

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

    /// Build a request with headers and an optional Authorization.
    fn build_write(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        auth: Option<&str>,
    ) -> ureq::Request {
        let mut req = self.agent.request(method, url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        if let Some(a) = auth {
            req = req.set("Authorization", a);
        }
        req
    }

    /// Send a request body, mapping ureq outcomes onto [`RequestFailure`]. The
    /// body is re-materialized per call (a file is re-opened) so a 401 retry can
    /// resend it.
    fn dispatch(
        &self,
        req: ureq::Request,
        body: &WriteBody,
    ) -> std::result::Result<ureq::Response, RequestFailure> {
        let sent = match body {
            WriteBody::Empty => req.call(),
            WriteBody::Bytes(b) => req.send_bytes(b),
            WriteBody::File(path) => match std::fs::File::open(path.as_std_path()) {
                Ok(f) => req.send(f),
                Err(e) => return Err(RequestFailure::Transport(format!("open {path}: {e}"))),
            },
        };
        match sent {
            Ok(resp) => Ok(resp),
            Err(ureq::Error::Status(code, resp)) => {
                Err(RequestFailure::Status(code, Box::new(resp)))
            }
            Err(ureq::Error::Transport(t)) => Err(RequestFailure::Transport(t.to_string())),
        }
    }

    /// A write request (`POST`/`PUT`/`HEAD`) with one token-exchange retry on
    /// 401, requesting `pull,push` scope. Any returned response (including a 3xx,
    /// which the caller inspects) comes back `Ok`; 4xx/5xx map to `ARTIFACT_*`.
    fn send_write(
        &self,
        method: &str,
        url: &str,
        reference: &OciReference,
        headers: Vec<(String, String)>,
        body: WriteBody,
    ) -> Result<ureq::Response> {
        let auth = self.auth_header(reference)?;
        let first = self.dispatch(
            self.build_write(method, url, &headers, auth.as_deref()),
            &body,
        );
        match first {
            Err(RequestFailure::Status(401, resp)) => {
                let challenge = resp.header("www-authenticate").unwrap_or("").to_string();
                let scope = format!("repository:{}:pull,push", reference.repository);
                let token = self.exchange_token(reference, &challenge, &scope)?;
                let bearer = format!("Bearer {token}");
                self.dispatch(
                    self.build_write(method, url, &headers, Some(&bearer)),
                    &body,
                )
                .map_err(|f| self.classify(reference, url, true, f))
            }
            Ok(resp) => Ok(resp),
            Err(f) => Err(self.classify(reference, url, true, f)),
        }
    }

    /// Whether the registry already stores a blob (`HEAD /blobs/<digest>`). A
    /// 404 classifies as `ARTIFACT_REMOTE_NOT_FOUND`, which here simply means
    /// "not present" — the content-addressed idempotency check.
    fn blob_exists(&self, reference: &OciReference, digest: &str) -> Result<bool> {
        let url = format!(
            "{}/v2/{}/blobs/{digest}",
            self.base_url(&reference.registry),
            reference.repository
        );
        match self.send_write("HEAD", &url, reference, Vec::new(), WriteBody::Empty) {
            Ok(resp) => Ok((200..300).contains(&resp.status())),
            Err(e) if e.code() == "ARTIFACT_REMOTE_NOT_FOUND" => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Whether the registry already stores this OCI manifest by digest.
    fn manifest_exists(&self, reference: &OciReference, oci_digest: &str) -> Result<bool> {
        let url = format!(
            "{}/v2/{}/manifests/{oci_digest}",
            self.base_url(&reference.registry),
            reference.repository
        );
        let headers = vec![("Accept".to_string(), MANIFEST_ACCEPT.to_string())];
        match self.send_write("HEAD", &url, reference, headers, WriteBody::Empty) {
            Ok(resp) => Ok((200..300).contains(&resp.status())),
            Err(e) if e.code() == "ARTIFACT_REMOTE_NOT_FOUND" => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Upload one blob if the registry does not already have it: start an upload
    /// session (`POST /blobs/uploads/`), then a monolithic `PUT` with the digest
    /// query. Idempotent — an existing blob is left untouched.
    fn upload_blob(
        &self,
        reference: &OciReference,
        digest: &str,
        size: u64,
        body: WriteBody,
    ) -> Result<()> {
        if self.blob_exists(reference, digest)? {
            return Ok(());
        }
        let start = format!(
            "{}/v2/{}/blobs/uploads/",
            self.base_url(&reference.registry),
            reference.repository
        );
        let resp = self.send_write(
            "POST",
            &start,
            reference,
            vec![("Content-Length".to_string(), "0".to_string())],
            WriteBody::Empty,
        )?;
        if !(200..300).contains(&resp.status()) {
            return Err(write_unexpected(
                &start,
                resp.status(),
                "start a blob upload",
            ));
        }
        let location = resp.header("location").ok_or_else(|| {
            Error::coded(
                "ARTIFACT_TRANSPORT_FAILED",
                Category::ExternalTool,
                format!("{start} accepted an upload but returned no Location header"),
            )
        })?;
        let put_url = append_digest_query(&absolutize(&start, location), digest);

        let headers = vec![
            (
                "Content-Type".to_string(),
                "application/octet-stream".to_string(),
            ),
            ("Content-Length".to_string(), size.to_string()),
        ];
        let resp = self.send_write("PUT", &put_url, reference, headers, body)?;
        if !(200..300).contains(&resp.status()) {
            return Err(write_unexpected(
                &put_url,
                resp.status(),
                "complete a blob upload",
            ));
        }
        Ok(())
    }

    /// `PUT` an OCI manifest under a tag or digest reference.
    fn put_manifest(
        &self,
        reference: &OciReference,
        manifest_ref: &str,
        bytes: &[u8],
    ) -> Result<()> {
        let url = format!(
            "{}/v2/{}/manifests/{manifest_ref}",
            self.base_url(&reference.registry),
            reference.repository
        );
        let headers = vec![(
            "Content-Type".to_string(),
            OCI_IMAGE_MANIFEST_MEDIA_TYPE.to_string(),
        )];
        let resp = self.send_write(
            "PUT",
            &url,
            reference,
            headers,
            WriteBody::Bytes(bytes.to_vec()),
        )?;
        if !(200..300).contains(&resp.status()) {
            return Err(write_unexpected(&url, resp.status(), "put the manifest"));
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

    fn push(
        &self,
        source: &crate::transport::PushSource,
        destination: &RemoteReference,
    ) -> Result<crate::transport::PushOutcome> {
        let r = self.oci(destination)?;

        // The producer manifest.json travels as a layer, byte-for-byte.
        // `digest::sha256_hex` already renders the `sha256:<hex>` OCI descriptor
        // form (as does `record.digest`), so it is used verbatim — never
        // re-prefixed, or the registry sees a malformed `sha256:sha256:<hex>`.
        let manifest_bytes = std::fs::read(source.manifest_path.as_std_path())
            .map_err(|e| Error::io(source.manifest_path.to_string(), e))?;
        let manifest_digest = digest::sha256_hex(&manifest_bytes);
        let manifest_size = manifest_bytes.len() as u64;

        let archive_digest = source.record.digest.clone();
        let archive_size = source.record.archive_size;
        let archive_title = source.record.archive.clone();
        let config_digest = digest::sha256_hex(EMPTY_CONFIG_BYTES);

        // Build the exact bytes the registry will store and hash them: the OCI
        // digest is content-addressed over precisely these bytes, so what we PUT
        // and what we advertise cannot diverge.
        let oci_manifest = build_oci_manifest(
            &LayerDescriptor {
                media_type: MEDIA_TYPE_ARCHIVE,
                digest: &archive_digest,
                size: archive_size,
                title: Some(&archive_title),
            },
            &LayerDescriptor {
                media_type: MEDIA_TYPE_PRODUCER_MANIFEST,
                digest: &manifest_digest,
                size: manifest_size,
                title: Some(MANIFEST_FILE),
            },
            &config_digest,
            EMPTY_CONFIG_BYTES.len() as u64,
        );
        let oci_digest = digest::sha256_hex(&oci_manifest);

        // A digest-pinned destination asserts the resulting OCI digest; refuse
        // to publish bytes that would not satisfy the pin.
        if let Some(pin) = &r.digest {
            if pin != &oci_digest {
                return Err(Error::coded(
                    "ARTIFACT_OCI_DIGEST_MISMATCH",
                    Category::Validation,
                    format!(
                        "destination pins {pin} but the artifact's OCI manifest hashes to \
                         {oci_digest}"
                    ),
                )
                .with_hint("push to the tag (or an unpinned repo) and pin the digest it prints"));
            }
        }

        // Idempotent: an identical manifest already at the destination means the
        // whole bundle (config + layers) is already there — transfer nothing.
        let already_present = self.manifest_exists(r, &oci_digest)?;
        if !already_present {
            self.upload_blob(
                r,
                &config_digest,
                EMPTY_CONFIG_BYTES.len() as u64,
                WriteBody::Bytes(EMPTY_CONFIG_BYTES.to_vec()),
            )?;
            self.upload_blob(
                r,
                &archive_digest,
                archive_size,
                WriteBody::File(source.archive_path.clone()),
            )?;
            self.upload_blob(
                r,
                &manifest_digest,
                manifest_size,
                WriteBody::Bytes(manifest_bytes.clone()),
            )?;
            // The immutable digest manifest — the contract a support line pins.
            self.put_manifest(r, &oci_digest, &oci_manifest)?;
        }
        // (Re)point the tag even on an idempotent re-push: the blobs and digest
        // manifest already exist, but the human-facing tag may need to move onto
        // this digest. Cheap (a tiny manifest PUT referencing present blobs).
        if let Some(tag) = &r.tag {
            self.put_manifest(r, tag, &oci_manifest)?;
        }

        let mut locator = format!("oci://{}/{}", r.registry, r.repository);
        if let Some(tag) = &r.tag {
            locator.push(':');
            locator.push_str(tag);
        }
        locator.push('@');
        locator.push_str(&oci_digest);

        Ok(crate::transport::PushOutcome {
            oci_digest,
            artifact_digest: archive_digest,
            locator,
            registry: r.registry.clone(),
            repository: r.repository.clone(),
            already_present,
            auth_mode: self.auth_mode.borrow().to_string(),
        })
    }
}

/// A request that did not return 2xx/3xx. The response is boxed so the happy
/// path never carries the failure variant's weight.
enum RequestFailure {
    Status(u16, Box<ureq::Response>),
    Transport(String),
}

/// A push request body, held owned so a 401 retry can resend it.
enum WriteBody {
    /// No body (a `HEAD`, or a `POST` that only opens an upload session).
    Empty,
    /// An in-memory body (the manifest / producer manifest / config blob).
    Bytes(Vec<u8>),
    /// A file streamed with an explicit `Content-Length` (the archive).
    File(Utf8PathBuf),
}

/// One OCI layer descriptor for [`build_oci_manifest`].
struct LayerDescriptor<'a> {
    media_type: &'a str,
    digest: &'a str,
    size: u64,
    title: Option<&'a str>,
}

/// Serialize the OCI image manifest for an OpenStrata bundle: an empty config,
/// the archive layer, and the producer-manifest layer, each tagged with its
/// media type and original filename. The bytes are deterministic (fixed field
/// order), so hashing them yields the digest the registry stores and the pull
/// path re-derives.
fn build_oci_manifest(
    archive: &LayerDescriptor,
    producer_manifest: &LayerDescriptor,
    config_digest: &str,
    config_size: u64,
) -> Vec<u8> {
    let layer_json = |l: &LayerDescriptor| {
        let mut obj = serde_json::Map::new();
        obj.insert("mediaType".into(), l.media_type.into());
        obj.insert("digest".into(), l.digest.into());
        obj.insert("size".into(), l.size.into());
        if let Some(title) = l.title {
            obj.insert(
                "annotations".into(),
                serde_json::json!({ TITLE_ANNOTATION: title }),
            );
        }
        serde_json::Value::Object(obj)
    };
    let manifest = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": OCI_IMAGE_MANIFEST_MEDIA_TYPE,
        "artifactType": OCI_ARTIFACT_TYPE,
        "config": {
            "mediaType": MEDIA_TYPE_EMPTY_CONFIG,
            "digest": config_digest,
            "size": config_size,
        },
        "layers": [layer_json(archive), layer_json(producer_manifest)],
    });
    // Compact, deterministic bytes (serde_json orders object keys stably), so
    // the digest we compute equals the one the registry stores and pull re-derives.
    serde_json::to_vec(&manifest).expect("OCI manifest value serializes")
}

/// Append a `digest=<digest>` query parameter to a blob-upload PUT URL.
fn append_digest_query(url: &str, digest: &str) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}digest={}", url_encode(digest))
}

/// A write step that returned an unexpected (non-2xx, typically 3xx) status.
fn write_unexpected(url: &str, status: u16, what: &str) -> Error {
    Error::coded(
        "ARTIFACT_TRANSPORT_FAILED",
        Category::ExternalTool,
        format!("failed to {what}: {url} answered HTTP {status}"),
    )
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

fn origin_of(url: &str) -> &str {
    if let Some((scheme, rest)) = url.split_once("://") {
        let host = rest.split('/').next().unwrap_or(rest);
        return &url[..scheme.len() + 3 + host.len()];
    }
    url.split('/').next().unwrap_or(url)
}

fn redirect_auth_allowed(
    registry_origin: &str,
    auth_still_allowed: &mut bool,
    next_url: &str,
) -> bool {
    if origin_of(next_url) != registry_origin {
        *auth_still_allowed = false;
    }
    *auth_still_allowed
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
        assert_eq!(origin_of("https://ghcr.io/v2/x"), "https://ghcr.io");
        assert_eq!(
            origin_of("http://localhost:5000/v2/x"),
            "http://localhost:5000"
        );
    }

    #[test]
    fn redirect_auth_sticks_to_the_registry_origin() {
        let registry = origin_of("https://registry.example/v2/o/r/blobs/sha256:ab").to_string();

        let mut allowed = true;
        assert!(redirect_auth_allowed(
            &registry,
            &mut allowed,
            "https://registry.example/v2/o/r/blobs/sha256:cd"
        ));
        assert!(allowed);

        let mut allowed = true;
        assert!(!redirect_auth_allowed(
            &registry,
            &mut allowed,
            "https://cdn.example/blob"
        ));
        assert!(!redirect_auth_allowed(
            &registry,
            &mut allowed,
            "https://cdn.example/blob2"
        ));

        let mut allowed = true;
        assert!(!redirect_auth_allowed(
            &registry,
            &mut allowed,
            "http://registry.example/v2/o/r/blobs/sha256:cd"
        ));
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
    fn built_manifest_round_trips_through_the_pull_parser() {
        // The push builder and the pull parser must agree: a manifest push emits
        // has to expose exactly the archive + producer-manifest layers pull looks
        // for, by media type and by title annotation.
        let archive_digest = format!("sha256:{}", "aa".repeat(32));
        let manifest_digest = format!("sha256:{}", "bb".repeat(32));
        let config_digest = digest::sha256_hex(EMPTY_CONFIG_BYTES);
        let bytes = build_oci_manifest(
            &LayerDescriptor {
                media_type: MEDIA_TYPE_ARCHIVE,
                digest: &archive_digest,
                size: 4096,
                title: Some("rt-26.05-linux-x86_64.tar.zst"),
            },
            &LayerDescriptor {
                media_type: MEDIA_TYPE_PRODUCER_MANIFEST,
                digest: &manifest_digest,
                size: 512,
                title: Some(MANIFEST_FILE),
            },
            &config_digest,
            EMPTY_CONFIG_BYTES.len() as u64,
        );

        // Deterministic: same inputs, same bytes (so the OCI digest is stable).
        let again = build_oci_manifest(
            &LayerDescriptor {
                media_type: MEDIA_TYPE_ARCHIVE,
                digest: &archive_digest,
                size: 4096,
                title: Some("rt-26.05-linux-x86_64.tar.zst"),
            },
            &LayerDescriptor {
                media_type: MEDIA_TYPE_PRODUCER_MANIFEST,
                digest: &manifest_digest,
                size: 512,
                title: Some(MANIFEST_FILE),
            },
            &config_digest,
            EMPTY_CONFIG_BYTES.len() as u64,
        );
        assert_eq!(bytes, again, "manifest serialization must be deterministic");

        let parsed = parse_oci_manifest(&bytes, "oci://x/y").unwrap();
        let producer = parsed
            .find_layer(MEDIA_TYPE_PRODUCER_MANIFEST, |t| t == MANIFEST_FILE)
            .expect("producer layer");
        assert_eq!(producer.digest, manifest_digest);
        assert_eq!(producer.size, 512);
        assert_eq!(producer.title.as_deref(), Some(MANIFEST_FILE));

        let archive = parsed
            .find_layer(MEDIA_TYPE_ARCHIVE, |t| t == "rt-26.05-linux-x86_64.tar.zst")
            .expect("archive layer");
        assert_eq!(archive.digest, archive_digest);
        assert_eq!(archive.size, 4096);

        // The top-level artifactType marks it as an OpenStrata bundle.
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["artifactType"], OCI_ARTIFACT_TYPE);
        assert_eq!(json["config"]["mediaType"], MEDIA_TYPE_EMPTY_CONFIG);
    }

    #[test]
    fn digest_query_appends_with_the_right_separator() {
        let dg = format!("sha256:{}", "cd".repeat(32));
        assert_eq!(
            append_digest_query("https://reg/v2/o/r/blobs/uploads/abc", &dg),
            format!("https://reg/v2/o/r/blobs/uploads/abc?digest={dg}")
        );
        assert_eq!(
            append_digest_query("https://reg/v2/o/r/blobs/uploads/abc?_state=xyz", &dg),
            format!("https://reg/v2/o/r/blobs/uploads/abc?_state=xyz&digest={dg}")
        );
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

    // --- Push integration against an in-process mock OCI registry ---

    use std::collections::HashSet;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct MockState {
        blobs: HashSet<String>,
        manifests: HashSet<String>,
        put_manifests: usize,
    }

    /// A minimal anonymous OCI registry: enough of the push surface (HEAD blob /
    /// manifest, POST upload session, monolithic PUT, PUT manifest) to drive the
    /// real network path end to end. One request per connection (`Connection:
    /// close`), so ureq opens a fresh socket each time.
    fn spawn_mock_registry() -> (String, Arc<Mutex<MockState>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(MockState::default()));
        let st = state.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());

                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
                    continue;
                }
                let mut parts = request_line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("").to_string();

                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    if line == "\r\n" || line == "\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        content_length = v.trim().parse().unwrap_or(0);
                    }
                }
                if content_length > 0 {
                    let mut body = vec![0u8; content_length];
                    let _ = reader.read_exact(&mut body);
                }

                let addr_str = format!("{addr}");
                let response = handle_mock(&method, &path, &addr_str, &st);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        (format!("{addr}"), state)
    }

    fn handle_mock(method: &str, path: &str, addr: &str, state: &Arc<Mutex<MockState>>) -> String {
        let ok = |code: &str, extra: &str| {
            format!("HTTP/1.1 {code}\r\nContent-Length: 0\r\nConnection: close\r\n{extra}\r\n")
        };
        let mut st = state.lock().unwrap();
        // /v2/<repo>/{blobs|manifests}/<ref>  (ref is the last path segment,
        // minus any query string).
        let reference = path
            .rsplit('/')
            .next()
            .unwrap_or("")
            .split('?')
            .next()
            .unwrap_or("");
        if method == "HEAD" && path.contains("/blobs/") {
            return if st.blobs.contains(reference) {
                ok("200 OK", "")
            } else {
                ok("404 Not Found", "")
            };
        }
        if method == "HEAD" && path.contains("/manifests/") {
            return if st.manifests.contains(reference) {
                ok("200 OK", "")
            } else {
                ok("404 Not Found", "")
            };
        }
        if method == "POST" && path.contains("/blobs/uploads/") {
            let loc = format!("Location: http://{addr}/v2/owner/rt/blobs/uploads/session\r\n");
            return ok("202 Accepted", &loc);
        }
        if method == "PUT" && path.contains("/blobs/uploads/") {
            // The digest travels in the query: ...?digest=sha256:<hex>
            if let Some(dg) = path.split("digest=").nth(1) {
                st.blobs.insert(dg.to_string());
            }
            return ok("201 Created", "");
        }
        if method == "PUT" && path.contains("/manifests/") {
            st.manifests.insert(reference.to_string());
            st.put_manifests += 1;
            return ok("201 Created", "");
        }
        ok("400 Bad Request", "")
    }

    fn push_fixture(dir: &Utf8Path) -> (Utf8PathBuf, Utf8PathBuf, crate::record::ArtifactRecord) {
        std::fs::create_dir_all(dir.as_std_path()).unwrap();
        let archive = dir.join("rt-26.05-linux-x86_64.tar.zst");
        let archive_bytes = b"a small fake runtime archive";
        std::fs::write(archive.as_std_path(), archive_bytes).unwrap();
        let manifest = dir.join(MANIFEST_FILE);
        std::fs::write(manifest.as_std_path(), b"{\"kind\":\"openstrata.runtime\"}").unwrap();

        let record = crate::record::ArtifactRecord {
            schema: 1,
            kind: crate::record::ArtifactKind::Runtime,
            name: "rt".into(),
            version: "26.05".into(),
            target: "cy2026-linux-x86_64-gcc11-py313-usd".into(),
            profile: Some("usd".into()),
            // `sha256_hex` already renders `sha256:<hex>`, matching how the store
            // records a real artifact digest — do not re-prefix.
            digest: digest::sha256_hex(archive_bytes),
            archive: "rt-26.05-linux-x86_64.tar.zst".into(),
            archive_size: archive_bytes.len() as u64,
            total_size: 100,
            file_count: 1,
            created_unix: 1_750_000_000,
            producer: "ost test".into(),
            source: crate::record::ArtifactSource::Published,
            validation: "passed".into(),
            licenses: vec!["Apache-2.0".into()],
            sbom: None,
            runtime_id: Some("rt".into()),
            runtime_digest: Some("sha256:beef".into()),
        };
        (archive, manifest, record)
    }

    #[test]
    fn push_uploads_bundle_and_is_idempotent() {
        let (addr, state) = spawn_mock_registry();
        let dir = std::env::temp_dir().join(format!("ost-push-{}", std::process::id()));
        let dir = Utf8PathBuf::from_path_buf(dir).unwrap();
        let (archive_path, manifest_path, record) = push_fixture(&dir);
        let source = crate::transport::PushSource {
            archive_path,
            manifest_path,
            record: &record,
        };
        let dest = RemoteReference::parse(&format!("oci://{addr}/owner/rt")).unwrap();
        let transport = OciTransport::new(true);

        let outcome = transport.push(&source, &dest).unwrap();
        assert!(!outcome.already_present, "first push transfers the bundle");
        assert!(
            is_sha256_ref(&outcome.oci_digest),
            "OCI digest must be a well-formed sha256:<hex>, not double-prefixed: {}",
            outcome.oci_digest
        );
        assert_eq!(outcome.artifact_digest, record.digest);
        assert!(outcome
            .locator
            .ends_with(&format!("@{}", outcome.oci_digest)));
        {
            let st = state.lock().unwrap();
            // config + archive + producer-manifest blobs, and the manifest PUT.
            assert_eq!(st.blobs.len(), 3, "three blobs uploaded");
            // Every blob digest sent in the `?digest=` query must be a single,
            // well-formed `sha256:<hex>` — a `sha256:sha256:<hex>` double prefix
            // (re-wrapping `digest::sha256_hex` output) is exactly what GHCR
            // rejected with HTTP 400.
            for dg in st.blobs.iter() {
                assert!(
                    is_sha256_ref(dg),
                    "uploaded blob digest is malformed (double sha256 prefix?): {dg}"
                );
            }
            assert!(st.manifests.contains(outcome.oci_digest.as_str()));
            assert_eq!(st.put_manifests, 1);
        }

        // Re-push: the manifest already exists, so nothing is uploaded again.
        let transport2 = OciTransport::new(true);
        let again = transport2.push(&source, &dest).unwrap();
        assert!(again.already_present, "re-push is idempotent");
        assert_eq!(again.oci_digest, outcome.oci_digest);
        {
            let st = state.lock().unwrap();
            assert_eq!(st.put_manifests, 1, "no second manifest PUT");
        }

        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    #[test]
    fn push_refuses_a_mismatched_pinned_destination() {
        let dir = std::env::temp_dir().join(format!("ost-push-pin-{}", std::process::id()));
        let dir = Utf8PathBuf::from_path_buf(dir).unwrap();
        let (archive_path, manifest_path, record) = push_fixture(&dir);
        let source = crate::transport::PushSource {
            archive_path,
            manifest_path,
            record: &record,
        };
        // A destination pinning the wrong OCI digest is refused (no network).
        let wrong = format!("sha256:{}", "00".repeat(32));
        let dest = RemoteReference::parse(&format!("oci://127.0.0.1:1/owner/rt@{wrong}")).unwrap();
        let err = OciTransport::new(true)
            .push(&source, &dest)
            .expect_err("mismatched pin must be refused");
        assert_eq!(err.code(), "ARTIFACT_OCI_DIGEST_MISMATCH");

        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    #[test]
    fn write_auth_hint_is_keyed_on_how_the_request_authenticated() {
        let dest = RemoteReference::parse("oci://ghcr.io/owner/rt").unwrap();
        let transport = OciTransport::new(false);
        let r = transport.oci(&dest).unwrap();

        // The report's failure: OST_REGISTRY_TOKEN authenticated (so no 401
        // retry) but the write was refused — steer to the credential path.
        *transport.auth_mode.borrow_mut() = "static-token";
        let hint = transport.write_auth_hint(r);
        assert!(hint.contains(ENV_TOKEN), "names the token that was tried");
        assert!(
            hint.contains(ENV_USER) && hint.contains(ENV_PASSWORD),
            "points at the credential exchange: {hint}"
        );
        assert!(hint.contains("push"), "explains the missing scope: {hint}");

        // Credentials were exchanged but the write was still refused — a scope /
        // permission problem, not a credential-plumbing one.
        *transport.auth_mode.borrow_mut() = "token-exchange-basic";
        let hint = transport.write_auth_hint(r);
        assert!(
            hint.contains("write:packages"),
            "names the scope to check: {hint}"
        );
        assert!(hint.contains("owner/rt"), "names the repository: {hint}");

        // No credentials reached the exchange — a push cannot be anonymous.
        for mode in ["anonymous", "token-exchange"] {
            *transport.auth_mode.borrow_mut() = mode;
            let hint = transport.write_auth_hint(r);
            assert!(
                hint.contains("anonymous")
                    && hint.contains(ENV_USER)
                    && hint.contains(ENV_PASSWORD),
                "{mode} hint must ask for credentials: {hint}"
            );
        }
    }
}
