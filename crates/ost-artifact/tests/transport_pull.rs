// SPDX-License-Identifier: Apache-2.0
//! Integration tests for the artifact transport (remote-artifact-transport.md,
//! Phase 1): digest-pinned OCI pull against a mock registry, the filesystem
//! backend, and every "must fail" case the plan names — corrupt blobs,
//! manifest substitution, wrong platform / kind, mutable-only references, and
//! unsafe archives. The mock registry speaks just enough of the OCI
//! Distribution Spec pull surface (manifest GET, blob GET, bearer token
//! exchange) over plain HTTP on a loopback port.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};

use ost_artifact::transport::oci::{
    MEDIA_TYPE_ARCHIVE, MEDIA_TYPE_DEBUG_ARCHIVE, MEDIA_TYPE_PRODUCER_MANIFEST,
};
use ost_artifact::{
    pull, ArtifactKind, ArtifactSource, ArtifactStore, ArtifactTransport, FileTransport,
    OciTransferPolicy, OciTransport, PullPolicy, RemoteReference,
};
use ost_core::digest;

// ---------------------------------------------------------------------------
// Mock OCI registry
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Route {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
    extra_headers: String,
    body_start_delay: Duration,
    chunk_delay: Duration,
    chunk_size: usize,
    disconnect_after: Option<usize>,
    disconnects_remaining: u32,
    range_behavior: RangeBehavior,
}

#[derive(Clone, Copy)]
enum RangeBehavior {
    Honor,
    Ignore,
    Reject,
    ChangedTotal,
}

type RequestLog = Arc<Mutex<Vec<(String, Option<String>)>>>;

struct MockRegistry {
    addr: SocketAddr,
    routes: Arc<Mutex<HashMap<String, Route>>>,
    /// When set, every /v2/ request must carry `Authorization: Bearer <this>`;
    /// the mock answers 401 with a token-exchange challenge otherwise.
    required_token: Arc<Mutex<Option<String>>>,
    requests: RequestLog,
}

impl MockRegistry {
    fn start() -> MockRegistry {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock registry");
        let addr = listener.local_addr().unwrap();
        let routes: Arc<Mutex<HashMap<String, Route>>> = Arc::new(Mutex::new(HashMap::new()));
        let required_token: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let requests = Arc::new(Mutex::new(Vec::new()));

        let thread_routes = Arc::clone(&routes);
        let thread_token = Arc::clone(&required_token);
        let thread_requests = Arc::clone(&requests);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let _ = handle_connection(
                    stream,
                    addr,
                    &thread_routes,
                    &thread_token,
                    &thread_requests,
                );
            }
        });

        MockRegistry {
            addr,
            routes,
            required_token,
            requests,
        }
    }

    fn host(&self) -> String {
        format!("127.0.0.1:{}", self.addr.port())
    }

    fn put(&self, path: &str, content_type: &'static str, body: Vec<u8>) {
        self.routes.lock().unwrap().insert(
            path.to_string(),
            Route {
                status: 200,
                content_type,
                body,
                extra_headers: String::new(),
                body_start_delay: Duration::ZERO,
                chunk_delay: Duration::ZERO,
                chunk_size: usize::MAX,
                disconnect_after: None,
                disconnects_remaining: 0,
                range_behavior: RangeBehavior::Honor,
            },
        );
    }

    fn redirect(&self, path: &str, location: &str) {
        self.routes.lock().unwrap().insert(
            path.to_string(),
            Route {
                status: 307,
                content_type: "application/json",
                body: b"{}".to_vec(),
                extra_headers: format!("Location: {location}\r\n"),
                body_start_delay: Duration::ZERO,
                chunk_delay: Duration::ZERO,
                chunk_size: usize::MAX,
                disconnect_after: None,
                disconnects_remaining: 0,
                range_behavior: RangeBehavior::Honor,
            },
        );
    }

    fn stream_body(&self, path: &str, chunk_size: usize, chunk_delay: Duration) {
        let mut routes = self.routes.lock().unwrap();
        let route = routes.get_mut(path).expect("registered route");
        route.chunk_size = chunk_size.max(1);
        route.chunk_delay = chunk_delay;
    }

    fn stall_body(&self, path: &str, delay: Duration) {
        let mut routes = self.routes.lock().unwrap();
        routes
            .get_mut(path)
            .expect("registered route")
            .body_start_delay = delay;
    }

    fn require_token(&self, token: &str) {
        *self.required_token.lock().unwrap() = Some(token.to_string());
    }

    fn disconnect(&self, path: &str, after: usize, times: u32) {
        let mut routes = self.routes.lock().unwrap();
        let route = routes.get_mut(path).expect("registered route");
        route.disconnect_after = Some(after);
        route.disconnects_remaining = times;
    }

    fn ignore_ranges(&self, path: &str) {
        self.routes
            .lock()
            .unwrap()
            .get_mut(path)
            .expect("registered route")
            .range_behavior = RangeBehavior::Ignore;
    }

    fn reject_ranges(&self, path: &str) {
        self.routes
            .lock()
            .unwrap()
            .get_mut(path)
            .expect("registered route")
            .range_behavior = RangeBehavior::Reject;
    }

    fn change_range_total(&self, path: &str) {
        self.routes
            .lock()
            .unwrap()
            .get_mut(path)
            .expect("registered route")
            .range_behavior = RangeBehavior::ChangedTotal;
    }

    fn ranges_for(&self, path: &str) -> Vec<Option<String>> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(request_path, _)| request_path == path)
            .map(|(_, range)| range.clone())
            .collect()
    }

    /// Register a bundle's manifest (under tag and digest) and its blobs.
    fn register(&self, repo: &str, tag: &str, bundle: &Bundle) {
        self.put(
            &format!("/v2/{repo}/manifests/{tag}"),
            "application/vnd.oci.image.manifest.v1+json",
            bundle.oci_manifest.clone(),
        );
        self.put(
            &format!("/v2/{repo}/manifests/{}", bundle.oci_digest),
            "application/vnd.oci.image.manifest.v1+json",
            bundle.oci_manifest.clone(),
        );
        self.put(
            &format!("/v2/{repo}/blobs/{}", digest::sha256_hex(&bundle.archive)),
            "application/octet-stream",
            bundle.archive.clone(),
        );
        self.put(
            &format!(
                "/v2/{repo}/blobs/{}",
                digest::sha256_hex(&bundle.producer_manifest)
            ),
            "application/octet-stream",
            bundle.producer_manifest.clone(),
        );
        if let (Some(name), Some(bytes)) = (&bundle.debug_name, &bundle.debug_archive) {
            self.put(
                &format!("/v2/{repo}/blobs/{}", digest::sha256_hex(bytes)),
                "application/octet-stream",
                bytes.clone(),
            );
            assert!(name.ends_with("-debug.tar.zst"));
        }
    }
}

fn handle_connection(
    mut stream: TcpStream,
    addr: SocketAddr,
    routes: &Mutex<HashMap<String, Route>>,
    required_token: &Mutex<Option<String>>,
    requests: &Mutex<Vec<(String, Option<String>)>>,
) -> std::io::Result<()> {
    // Read the request head (GET requests only — no bodies).
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte)? == 0 {
            return Ok(());
        }
        head.push(byte[0]);
        if head.len() > 64 * 1024 {
            return Ok(());
        }
    }
    let head = String::from_utf8_lossy(&head);
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or("");
    let path_with_query = request_line.split(' ').nth(1).unwrap_or("/");
    let path = path_with_query.split('?').next().unwrap_or("/");
    let headers: Vec<_> = lines.filter_map(|line| line.split_once(':')).collect();
    let auth = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .map(|(_, v)| v.trim().to_string());
    let range = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("range"))
        .map(|(_, v)| v.trim().to_string());
    requests
        .lock()
        .unwrap()
        .push((path.to_string(), range.clone()));

    let token = required_token.lock().unwrap().clone();
    if let Some(token) = token {
        if path == "/token" {
            let body = format!("{{\"token\":\"{token}\"}}");
            return respond(&mut stream, 200, "application/json", body.as_bytes(), "");
        }
        if auth.as_deref() != Some(&format!("Bearer {token}")) {
            let challenge = format!(
                "WWW-Authenticate: Bearer realm=\"http://{addr}/token\",service=\"mock\"\r\n"
            );
            return respond(&mut stream, 401, "application/json", b"{}", &challenge);
        }
    }

    let route = {
        let mut routes = routes.lock().unwrap();
        routes.get_mut(path).map(|route| {
            let mut response = route.clone();
            if route.disconnects_remaining > 0 {
                route.disconnects_remaining -= 1;
            } else {
                response.disconnect_after = None;
            }
            response
        })
    };
    match route {
        Some(route) => {
            let mut status = route.status;
            let mut body = route.body.as_slice();
            let mut extra_headers = route.extra_headers.clone();
            if let Some(start) = range
                .as_deref()
                .and_then(|value| value.strip_prefix("bytes="))
                .and_then(|value| value.strip_suffix('-'))
                .and_then(|value| value.parse::<usize>().ok())
            {
                match route.range_behavior {
                    RangeBehavior::Honor if start < body.len() => {
                        status = 206;
                        extra_headers.push_str(&format!(
                            "Content-Range: bytes {start}-{}/{}\r\n",
                            body.len() - 1,
                            body.len()
                        ));
                        body = &body[start..];
                    }
                    RangeBehavior::Honor | RangeBehavior::Reject => {
                        status = 416;
                        extra_headers
                            .push_str(&format!("Content-Range: bytes */{}\r\n", body.len()));
                        body = &[];
                    }
                    RangeBehavior::ChangedTotal if start < body.len() => {
                        status = 206;
                        extra_headers.push_str(&format!(
                            "Content-Range: bytes {start}-{}/{}\r\n",
                            body.len() - 1,
                            body.len() + 1
                        ));
                        body = &body[start..];
                    }
                    RangeBehavior::ChangedTotal => {
                        status = 416;
                        body = &[];
                    }
                    RangeBehavior::Ignore => {}
                }
            }
            respond_slow(
                &mut stream,
                status,
                route.content_type,
                body,
                &extra_headers,
                route.body_start_delay,
                route.chunk_delay,
                route.chunk_size,
                route.disconnect_after,
            )
        }
        None => respond(&mut stream, 404, "application/json", b"{}", ""),
    }
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    extra_headers: &str,
) -> std::io::Result<()> {
    respond_slow(
        stream,
        status,
        content_type,
        body,
        extra_headers,
        Duration::ZERO,
        Duration::ZERO,
        usize::MAX,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn respond_slow(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    extra_headers: &str,
    body_start_delay: Duration,
    chunk_delay: Duration,
    chunk_size: usize,
    disconnect_after: Option<usize>,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Mock",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n",
        body.len()
    )?;
    stream.flush()?;
    std::thread::sleep(body_start_delay);
    let send_len = disconnect_after.unwrap_or(body.len()).min(body.len());
    for chunk in body[..send_len].chunks(chunk_size.max(1)) {
        stream.write_all(chunk)?;
        stream.flush()?;
        std::thread::sleep(chunk_delay);
    }
    stream.flush()
}

// ---------------------------------------------------------------------------
// Fixture bundle
// ---------------------------------------------------------------------------

const TARGET: &str = "cy2026-linux-x86_64-gcc11-py313-usd";

struct Bundle {
    archive: Vec<u8>,
    archive_name: String,
    producer_manifest: Vec<u8>,
    debug_archive: Option<Vec<u8>>,
    debug_name: Option<String>,
    oci_manifest: Vec<u8>,
    oci_digest: String,
    artifact_digest: String,
}

fn pseudo_random_bytes(len: usize) -> Vec<u8> {
    let mut state = 0x1234_5678_u32;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}

/// A plugin-bundle artifact with one payload file, plus its OCI bundle.
fn make_bundle(name: &str, content: &[u8]) -> Bundle {
    let archive = tar_zst(&[("lib/payload.bin", content)]);
    make_bundle_from_archive(name, content, archive)
}

fn make_bundle_with_debug(name: &str, content: &[u8], symbols: &[u8]) -> Bundle {
    let archive = tar_zst(&[("lib/payload.bin", content)]);
    let archive_name = format!("{name}-0.1.0-{TARGET}.tar.zst");
    let debug_archive = tar_zst(&[("lib/payload.pdb", symbols)]);
    let debug_name = format!("{name}-0.1.0-{TARGET}-debug.tar.zst");
    let producer = serde_json::json!({
        "schema": 1,
        "kind": "openstrata.plugin-bundle",
        "plugin": { "name": name, "version": "0.1.0", "kind": "usd-fileformat", "license": "Apache-2.0" },
        "target": TARGET,
        "archive": archive_name,
        "archive_digest": digest::sha256_hex(&archive),
        "archive_size": archive.len(),
        "total_size": content.len(),
        "created_unix": 1_750_000_000u64,
        "provenance": {
            "profile": "usd",
            "runtime": { "id": "rt", "digest": "sha256:beef" },
            "validation": { "passed": true },
        },
        "files": [
            { "path": "lib/payload.bin", "sha256": digest::sha256_hex(content), "size": content.len() },
        ],
        "debug": {
            "archive": debug_name,
            "archive_digest": digest::sha256_hex(&debug_archive),
            "archive_size": debug_archive.len(),
            "total_size": symbols.len(),
            "files": [
                { "path": "lib/payload.pdb", "sha256": digest::sha256_hex(symbols), "size": symbols.len() },
            ],
        },
    });
    let producer_manifest = serde_json::to_vec_pretty(&producer).unwrap();
    finish_bundle_with_debug(
        archive,
        archive_name,
        producer_manifest,
        Some((debug_name, debug_archive)),
    )
}

fn make_bundle_from_archive(name: &str, content: &[u8], archive: Vec<u8>) -> Bundle {
    let archive_name = format!("{name}-0.1.0-{TARGET}.tar.zst");
    let producer = serde_json::json!({
        "schema": 1,
        "kind": "openstrata.plugin-bundle",
        "plugin": { "name": name, "version": "0.1.0", "kind": "usd-fileformat", "license": "Apache-2.0" },
        "target": TARGET,
        "archive": archive_name,
        "archive_digest": digest::sha256_hex(&archive),
        "archive_size": archive.len(),
        "total_size": content.len(),
        "created_unix": 1_750_000_000u64,
        "provenance": {
            "profile": "usd",
            "runtime": { "id": "rt", "digest": "sha256:beef" },
            "validation": { "passed": true },
        },
        "files": [
            { "path": "lib/payload.bin", "sha256": digest::sha256_hex(content), "size": content.len() },
        ],
    });
    let producer_manifest = serde_json::to_vec_pretty(&producer).unwrap();
    finish_bundle(archive, archive_name, producer_manifest)
}

fn finish_bundle(archive: Vec<u8>, archive_name: String, producer_manifest: Vec<u8>) -> Bundle {
    finish_bundle_with_debug(archive, archive_name, producer_manifest, None)
}

fn finish_bundle_with_debug(
    archive: Vec<u8>,
    archive_name: String,
    producer_manifest: Vec<u8>,
    debug: Option<(String, Vec<u8>)>,
) -> Bundle {
    let artifact_digest = digest::sha256_hex(&archive);
    let mut layers = vec![serde_json::json!({
        "mediaType": MEDIA_TYPE_ARCHIVE,
        "digest": artifact_digest,
        "size": archive.len(),
        "annotations": { "org.opencontainers.image.title": archive_name },
    })];
    if let Some((name, bytes)) = &debug {
        layers.push(serde_json::json!({
            "mediaType": MEDIA_TYPE_DEBUG_ARCHIVE,
            "digest": digest::sha256_hex(bytes),
            "size": bytes.len(),
            "annotations": { "org.opencontainers.image.title": name },
        }));
    }
    layers.push(serde_json::json!({
        "mediaType": MEDIA_TYPE_PRODUCER_MANIFEST,
        "digest": digest::sha256_hex(&producer_manifest),
        "size": producer_manifest.len(),
        "annotations": { "org.opencontainers.image.title": "manifest.json" },
    }));
    let oci = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "artifactType": "application/vnd.openstrata.artifact.v1",
        "config": {
            "mediaType": "application/vnd.openstrata.artifact.descriptor.v1+json",
            "digest": digest::sha256_hex(b"{}"),
            "size": 2,
        },
        "layers": layers,
    });
    let oci_manifest = serde_json::to_vec_pretty(&oci).unwrap();
    let oci_digest = digest::sha256_hex(&oci_manifest);
    Bundle {
        archive,
        archive_name,
        producer_manifest,
        debug_archive: debug.as_ref().map(|(_, bytes)| bytes.clone()),
        debug_name: debug.map(|(name, _)| name),
        oci_manifest,
        oci_digest,
        artifact_digest,
    }
}

/// Build a `tar.zst` holding the given (path, content) regular files.
fn tar_zst(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let enc = zstd::stream::write::Encoder::new(&mut out, 3)
            .unwrap()
            .auto_finish();
        let mut tar = tar::Builder::new(enc);
        for (path, content) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, path, *content).unwrap();
        }
        tar.finish().unwrap();
    }
    out
}

fn tmp_root(tag: &str) -> Utf8PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mut d = Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
    d.push(format!(
        "ost-transport-{tag}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(d.as_std_path()).unwrap();
    d
}

fn oci_ref(registry: &MockRegistry, repo: &str, suffix: &str) -> RemoteReference {
    RemoteReference::parse(&format!("oci://{}/{repo}{suffix}", registry.host())).unwrap()
}

fn assert_store_empty(store: &ArtifactStore) {
    assert!(
        store.list().unwrap().is_empty(),
        "a failed pull must never leave a usable artifact"
    );
}

fn retry_policy(max_attempts: u32) -> OciTransferPolicy {
    OciTransferPolicy {
        connect_timeout: Some(Duration::from_secs(1)),
        response_timeout: Some(Duration::from_secs(1)),
        body_idle_timeout: Some(Duration::from_secs(1)),
        overall_timeout: None,
        max_attempts,
        initial_retry_backoff: Duration::ZERO,
        max_retry_backoff: Duration::ZERO,
        ..OciTransferPolicy::default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn resolve_turns_a_tag_into_the_oci_digest() {
    let registry = MockRegistry::start();
    let bundle = make_bundle("toy", b"plugin bytes");
    registry.register("fixtures/rt", "v1", &bundle);

    let transport = OciTransport::new(true);
    let reference = oci_ref(&registry, "fixtures/rt", ":v1");
    let resolved = transport.resolve(&reference).unwrap();

    assert_eq!(
        resolved.oci_digest.as_deref(),
        Some(bundle.oci_digest.as_str())
    );
    assert_eq!(
        resolved.locator,
        format!(
            "oci://{}/fixtures/rt@{}",
            registry.host(),
            bundle.oci_digest
        )
    );
    assert_eq!(resolved.registry, registry.host());
    assert_eq!(resolved.auth_mode, "anonymous");
}

#[test]
fn resolve_follows_manifest_redirects() {
    let registry = MockRegistry::start();
    let bundle = make_bundle("toy", b"plugin bytes");
    registry.register("fixtures/rt", "v1", &bundle);
    registry.redirect(
        "/v2/fixtures/rt/manifests/v1",
        &format!("/v2/fixtures/rt/manifests/{}", bundle.oci_digest),
    );

    let transport = OciTransport::new(true);
    let reference = oci_ref(&registry, "fixtures/rt", ":v1");
    let resolved = transport.resolve(&reference).unwrap();

    assert_eq!(
        resolved.oci_digest.as_deref(),
        Some(bundle.oci_digest.as_str())
    );
}

#[test]
fn digest_pinned_pull_imports_and_verifies() {
    let registry = MockRegistry::start();
    let bundle = make_bundle("toy", b"plugin bytes");
    registry.register("fixtures/rt", "v1", &bundle);

    let root = tmp_root("pull-ok");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::new(true);
    let reference = oci_ref(&registry, "fixtures/rt", &format!("@{}", bundle.oci_digest));

    let policy = PullPolicy {
        expected_artifact_digest: Some(bundle.artifact_digest.clone()),
        require_kind: Some(ArtifactKind::Plugin),
        require_target: Some(TARGET.to_string()),
    };
    let evidence = pull(&transport, &reference, &store, &policy).unwrap();

    assert_eq!(evidence.record.digest, bundle.artifact_digest);
    assert_eq!(evidence.record.name, "toy");
    assert_eq!(evidence.import_status, "imported");
    assert_eq!(
        evidence.remote.oci_digest.as_deref(),
        Some(bundle.oci_digest.as_str())
    );
    // Every required chain step passed. Evidence sidecars are optional for this
    // legacy fixture and therefore reported as skipped.
    for (step, status) in &evidence.verification {
        let expected = if matches!(*step, "sbom" | "provenance") {
            "skipped"
        } else {
            "passed"
        };
        assert_eq!(*status, expected, "step {step}");
    }

    // The imported artifact is fully usable: registry lists it and verify passes.
    let listed = store.list().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].source, ArtifactSource::Imported);
    assert!(store.verify(&bundle.artifact_digest).unwrap().passed());
    assert!(store
        .object_dir(listed[0].digest_hex())
        .join(&bundle.archive_name)
        .as_std_path()
        .is_file());

    // Pulling the same digest again is idempotent.
    let again = pull(&transport, &reference, &store, &policy).unwrap();
    assert_eq!(again.import_status, "already-present");

    // No scratch directory survives under the store root.
    let leftovers: Vec<_> = std::fs::read_dir(store.root().as_std_path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp-pull-"))
        .collect();
    assert!(leftovers.is_empty(), "scratch dirs must be cleaned up");

    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn advancing_blob_can_outlast_the_body_idle_budget() {
    let registry = MockRegistry::start();
    let content = pseudo_random_bytes(64 * 1024);
    let bundle = make_bundle("slow-progress", &content);
    registry.register("fixtures/slow-progress", "v1", &bundle);
    let archive_path = format!(
        "/v2/fixtures/slow-progress/blobs/{}",
        digest::sha256_hex(&bundle.archive)
    );
    registry.stream_body(&archive_path, 8 * 1024, Duration::from_millis(30));

    let root = tmp_root("pull-slow-progress");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::with_transfer_policy(
        true,
        OciTransferPolicy {
            connect_timeout: Some(Duration::from_secs(1)),
            response_timeout: Some(Duration::from_secs(1)),
            body_idle_timeout: Some(Duration::from_millis(100)),
            overall_timeout: None,
            ..OciTransferPolicy::default()
        },
    );
    let reference = oci_ref(
        &registry,
        "fixtures/slow-progress",
        &format!("@{}", bundle.oci_digest),
    );

    let evidence = pull(&transport, &reference, &store, &PullPolicy::default()).unwrap();
    assert_eq!(evidence.record.digest, bundle.artifact_digest);
    let manifest = evidence
        .transfer
        .manifest
        .as_ref()
        .expect("OCI pulls record manifest evidence");
    assert_eq!(manifest.digest, bundle.oci_digest);
    assert_eq!(manifest.received_bytes, bundle.oci_manifest.len() as u64);
    let archive = evidence
        .transfer
        .layers
        .iter()
        .find(|layer| layer.digest == bundle.artifact_digest)
        .expect("archive layer evidence");
    assert_eq!(archive.title, bundle.archive_name);
    assert_eq!(archive.expected_bytes, bundle.archive.len() as u64);
    assert_eq!(archive.received_bytes, archive.expected_bytes);
    assert_eq!(archive.attempts.last().unwrap().decision, "complete");
    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn disconnected_blob_resumes_from_a_validated_range() {
    let registry = MockRegistry::start();
    let content = pseudo_random_bytes(128 * 1024);
    let bundle = make_bundle("resume", &content);
    registry.register("fixtures/resume", "v1", &bundle);
    let archive_path = format!(
        "/v2/fixtures/resume/blobs/{}",
        digest::sha256_hex(&bundle.archive)
    );
    registry.disconnect(&archive_path, bundle.archive.len() / 3, 1);

    let root = tmp_root("pull-resume");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::with_transfer_policy(true, retry_policy(3));
    let reference = oci_ref(
        &registry,
        "fixtures/resume",
        &format!("@{}", bundle.oci_digest),
    );

    let evidence = pull(&transport, &reference, &store, &PullPolicy::default()).unwrap();
    assert_eq!(evidence.record.digest, bundle.artifact_digest);
    let archive = evidence
        .transfer
        .layers
        .iter()
        .find(|layer| layer.digest == bundle.artifact_digest)
        .expect("archive layer evidence");
    assert!(archive.attempts.len() >= 2, "{:#?}", archive.attempts);
    assert_eq!(archive.attempts.first().unwrap().decision, "retry");
    assert!(archive.attempts.first().unwrap().received_bytes > 0);
    assert!(archive.attempts.last().unwrap().resume_offset > 0);
    assert_eq!(archive.attempts.last().unwrap().decision, "complete");
    let json = serde_json::to_value(&evidence.transfer).unwrap();
    assert_eq!(json["layers"][1]["attempts"][0]["decision"], "retry");
    assert!(json["layers"][1]["attempts"][0]["elapsed_ms"].is_u64());
    assert!(json["layers"][1]["attempts"][0]["idle_age_ms"].is_u64());
    let ranges = registry.ranges_for(&archive_path);
    assert_eq!(ranges.first(), Some(&None));
    assert!(
        ranges.iter().skip(1).any(|range| range
            .as_deref()
            .is_some_and(|value| value.starts_with("bytes=") && value != "bytes=0-")),
        "resume requests: {ranges:?}"
    );
    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn ignored_range_restarts_safely_from_the_full_response() {
    let registry = MockRegistry::start();
    let content = pseudo_random_bytes(96 * 1024);
    let bundle = make_bundle("ignore-range", &content);
    registry.register("fixtures/ignore-range", "v1", &bundle);
    let archive_path = format!(
        "/v2/fixtures/ignore-range/blobs/{}",
        digest::sha256_hex(&bundle.archive)
    );
    registry.disconnect(&archive_path, bundle.archive.len() / 4, 1);
    registry.ignore_ranges(&archive_path);

    let root = tmp_root("pull-ignore-range");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::with_transfer_policy(true, retry_policy(3));
    let reference = oci_ref(
        &registry,
        "fixtures/ignore-range",
        &format!("@{}", bundle.oci_digest),
    );

    let evidence = pull(&transport, &reference, &store, &PullPolicy::default()).unwrap();
    assert_eq!(evidence.record.digest, bundle.artifact_digest);
    assert!(
        registry
            .ranges_for(&archive_path)
            .iter()
            .any(Option::is_some),
        "the second request must attempt a range before accepting the full 200 response"
    );
    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn rejected_range_discards_the_partial_and_restarts() {
    let registry = MockRegistry::start();
    let content = pseudo_random_bytes(96 * 1024);
    let bundle = make_bundle("reject-range", &content);
    registry.register("fixtures/reject-range", "v1", &bundle);
    let archive_path = format!(
        "/v2/fixtures/reject-range/blobs/{}",
        digest::sha256_hex(&bundle.archive)
    );
    registry.disconnect(&archive_path, bundle.archive.len() / 4, 1);
    registry.reject_ranges(&archive_path);

    let root = tmp_root("pull-reject-range");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::with_transfer_policy(true, retry_policy(4));
    let reference = oci_ref(
        &registry,
        "fixtures/reject-range",
        &format!("@{}", bundle.oci_digest),
    );

    let evidence = pull(&transport, &reference, &store, &PullPolicy::default()).unwrap();
    assert_eq!(evidence.record.digest, bundle.artifact_digest);
    let ranges = registry.ranges_for(&archive_path);
    assert!(ranges.iter().any(Option::is_some), "requests: {ranges:?}");
    assert_eq!(ranges.last(), Some(&None), "restart requests: {ranges:?}");
    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn changed_content_range_is_rejected_and_the_partial_is_discarded() {
    let registry = MockRegistry::start();
    let content = pseudo_random_bytes(96 * 1024);
    let bundle = make_bundle("changed-range", &content);
    registry.register("fixtures/changed-range", "v1", &bundle);
    let archive_path = format!(
        "/v2/fixtures/changed-range/blobs/{}",
        digest::sha256_hex(&bundle.archive)
    );
    registry.disconnect(&archive_path, bundle.archive.len() / 4, 1);
    registry.change_range_total(&archive_path);

    let root = tmp_root("pull-changed-range");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::with_transfer_policy(true, retry_policy(3));
    let reference = oci_ref(
        &registry,
        "fixtures/changed-range",
        &format!("@{}", bundle.oci_digest),
    );
    let error = pull(&transport, &reference, &store, &PullPolicy::default()).unwrap_err();

    assert_eq!(error.code(), "ARTIFACT_RANGE_INVALID");
    assert!(error.to_string().contains("Content-Range"));
    assert_store_empty(&store);
    let partial = store.root().join(".partial-blobs").join(format!(
        "{}.part",
        bundle.artifact_digest.trim_start_matches("sha256:")
    ));
    assert!(!partial.as_std_path().exists());
    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn corrupt_partial_is_discarded_before_a_clean_retry() {
    let registry = MockRegistry::start();
    let content = pseudo_random_bytes(96 * 1024);
    let bundle = make_bundle("corrupt-partial", &content);
    registry.register("fixtures/corrupt-partial", "v1", &bundle);
    let archive_path = format!(
        "/v2/fixtures/corrupt-partial/blobs/{}",
        digest::sha256_hex(&bundle.archive)
    );

    let root = tmp_root("pull-corrupt-partial");
    let store = ArtifactStore::at(root.join("store"));
    let partial_dir = store.root().join(".partial-blobs");
    std::fs::create_dir_all(partial_dir.as_std_path()).unwrap();
    let partial = partial_dir.join(format!(
        "{}.part",
        bundle.artifact_digest.trim_start_matches("sha256:")
    ));
    std::fs::write(partial.as_std_path(), vec![0xff; 4096]).unwrap();

    let transport = OciTransport::with_transfer_policy(true, retry_policy(3));
    let reference = oci_ref(
        &registry,
        "fixtures/corrupt-partial",
        &format!("@{}", bundle.oci_digest),
    );
    let evidence = pull(&transport, &reference, &store, &PullPolicy::default()).unwrap();

    assert_eq!(evidence.record.digest, bundle.artifact_digest);
    let ranges = registry.ranges_for(&archive_path);
    assert!(
        ranges.first().is_some_and(Option::is_some),
        "requests: {ranges:?}"
    );
    assert_eq!(ranges.last(), Some(&None), "clean retry: {ranges:?}");
    assert!(!partial.as_std_path().exists());
    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn stale_partial_is_scavenged_before_transfer() {
    let registry = MockRegistry::start();
    let content = pseudo_random_bytes(64 * 1024);
    let bundle = make_bundle("stale-partial", &content);
    registry.register("fixtures/stale-partial", "v1", &bundle);
    let archive_path = format!(
        "/v2/fixtures/stale-partial/blobs/{}",
        digest::sha256_hex(&bundle.archive)
    );

    let root = tmp_root("pull-stale-partial");
    let store = ArtifactStore::at(root.join("store"));
    let partial_dir = store.root().join(".partial-blobs");
    std::fs::create_dir_all(partial_dir.as_std_path()).unwrap();
    let partial = partial_dir.join(format!(
        "{}.part",
        bundle.artifact_digest.trim_start_matches("sha256:")
    ));
    std::fs::write(partial.as_std_path(), &bundle.archive[..4096]).unwrap();
    std::thread::sleep(Duration::from_millis(2));

    let mut policy = retry_policy(2);
    policy.partial_max_age = Duration::ZERO;
    let transport = OciTransport::with_transfer_policy(true, policy);
    let reference = oci_ref(
        &registry,
        "fixtures/stale-partial",
        &format!("@{}", bundle.oci_digest),
    );
    let evidence = pull(&transport, &reference, &store, &PullPolicy::default()).unwrap();

    assert_eq!(evidence.record.digest, bundle.artifact_digest);
    assert_eq!(registry.ranges_for(&archive_path).first(), Some(&None));
    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn retry_exhaustion_retains_an_invisible_partial_for_the_next_pull() {
    let registry = MockRegistry::start();
    let content = pseudo_random_bytes(128 * 1024);
    let bundle = make_bundle("exhausted", &content);
    registry.register("fixtures/exhausted", "v1", &bundle);
    let archive_path = format!(
        "/v2/fixtures/exhausted/blobs/{}",
        digest::sha256_hex(&bundle.archive)
    );
    registry.disconnect(&archive_path, 4096, 10);

    let root = tmp_root("pull-exhausted");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::with_transfer_policy(true, retry_policy(2));
    let reference = oci_ref(
        &registry,
        "fixtures/exhausted",
        &format!("@{}", bundle.oci_digest),
    );
    let error = pull(&transport, &reference, &store, &PullPolicy::default()).unwrap_err();

    assert_eq!(error.code(), "ARTIFACT_TRANSPORT_FAILED");
    assert!(error.to_string().contains("exhausted 2 attempt"));
    let data = error.data().expect("structured terminal transfer evidence");
    assert_eq!(data["transfer"]["manifest_digest"], bundle.oci_digest);
    assert_eq!(data["transfer"]["layer"]["digest"], bundle.artifact_digest);
    assert_eq!(
        data["transfer"]["layer"]["expected_bytes"],
        bundle.archive.len() as u64
    );
    assert_eq!(
        data["transfer"]["layer"]["attempts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(data["transfer"]["layer"]["attempts"][1]["decision"], "stop");
    assert!(data["transfer"]["next_action"]
        .as_str()
        .is_some_and(|action| action.contains("resume")));
    assert_store_empty(&store);
    let partial = store.root().join(".partial-blobs").join(format!(
        "{}.part",
        bundle.artifact_digest.trim_start_matches("sha256:")
    ));
    assert!(
        partial.as_std_path().is_file(),
        "retained partial: {partial}"
    );
    assert!(std::fs::metadata(partial.as_std_path()).unwrap().len() > 0);

    // A distinct invocation reuses the digest-keyed prefix after the transient
    // server failure is gone; nothing from the failed invocation was imported.
    registry.disconnect(&archive_path, 0, 0);
    let resumed_transport = OciTransport::with_transfer_policy(true, retry_policy(2));
    let evidence = pull(
        &resumed_transport,
        &reference,
        &store,
        &PullPolicy::default(),
    )
    .unwrap();
    assert_eq!(evidence.record.digest, bundle.artifact_digest);
    assert!(!partial.as_std_path().exists());
    assert!(
        registry
            .ranges_for(&archive_path)
            .last()
            .is_some_and(Option::is_some),
        "the next pull must resume the retained prefix"
    );
    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn optional_overall_budget_stops_an_active_blob() {
    let registry = MockRegistry::start();
    let content = pseudo_random_bytes(256 * 1024);
    let bundle = make_bundle("overall-budget", &content);
    registry.register("fixtures/overall-budget", "v1", &bundle);
    let archive_path = format!(
        "/v2/fixtures/overall-budget/blobs/{}",
        digest::sha256_hex(&bundle.archive)
    );
    registry.stream_body(&archive_path, 16 * 1024, Duration::from_millis(30));

    let root = tmp_root("pull-overall-budget");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::with_transfer_policy(
        true,
        OciTransferPolicy {
            connect_timeout: Some(Duration::from_secs(1)),
            response_timeout: Some(Duration::from_secs(1)),
            body_idle_timeout: Some(Duration::from_millis(100)),
            overall_timeout: Some(Duration::from_millis(150)),
            max_attempts: 1,
            ..OciTransferPolicy::default()
        },
    );
    let reference = oci_ref(
        &registry,
        "fixtures/overall-budget",
        &format!("@{}", bundle.oci_digest),
    );

    let error = pull(&transport, &reference, &store, &PullPolicy::default()).unwrap_err();
    assert_eq!(error.code(), "ARTIFACT_TRANSFER_TIMEOUT");
    assert!(
        error
            .hint()
            .is_some_and(|hint| hint.contains("--overall-timeout")),
        "overall timeout decision: {:?}",
        error.hint()
    );
    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn stalled_blob_reports_transfer_timeout_and_received_bytes() {
    let registry = MockRegistry::start();
    let bundle = make_bundle("stalled", b"plugin bytes");
    registry.register("fixtures/stalled", "v1", &bundle);
    let archive_path = format!(
        "/v2/fixtures/stalled/blobs/{}",
        digest::sha256_hex(&bundle.archive)
    );
    registry.stall_body(&archive_path, Duration::from_millis(300));

    let root = tmp_root("pull-stalled");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::with_transfer_policy(
        true,
        OciTransferPolicy {
            connect_timeout: Some(Duration::from_secs(1)),
            response_timeout: Some(Duration::from_secs(1)),
            body_idle_timeout: Some(Duration::from_millis(100)),
            overall_timeout: None,
            max_attempts: 1,
            ..OciTransferPolicy::default()
        },
    );
    let reference = oci_ref(
        &registry,
        "fixtures/stalled",
        &format!("@{}", bundle.oci_digest),
    );

    let error = pull(&transport, &reference, &store, &PullPolicy::default()).unwrap_err();
    assert_eq!(error.code(), "ARTIFACT_TRANSFER_TIMEOUT");
    let detail = error.to_string();
    assert!(detail.contains("0/"), "received-byte evidence: {detail}");
    assert!(
        detail.contains(&bundle.artifact_digest),
        "layer digest: {detail}"
    );
    assert!(
        error
            .hint()
            .is_some_and(|hint| hint.contains("--body-idle-timeout")),
        "direct next action: {:?}",
        error.hint()
    );
    let attempt = &error.data().unwrap()["transfer"]["layer"]["attempts"][0];
    assert_eq!(attempt["attempt"], 1);
    assert_eq!(attempt["resume_offset"], 0);
    assert_eq!(attempt["decision"], "stop");
    assert!(attempt["elapsed_ms"].is_u64());
    assert!(attempt["idle_age_ms"].is_u64());
    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn digest_pinned_pull_preserves_and_verifies_the_debug_sidecar() {
    let registry = MockRegistry::start();
    let bundle = make_bundle_with_debug("toy", b"plugin bytes", b"debug symbols");
    registry.register("fixtures/debug", "v1", &bundle);

    let root = tmp_root("pull-debug");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::new(true);
    let reference = oci_ref(
        &registry,
        "fixtures/debug",
        &format!("@{}", bundle.oci_digest),
    );
    let evidence = pull(&transport, &reference, &store, &PullPolicy::default()).unwrap();

    assert!(evidence.verification.contains(&("debug_archive", "passed")));
    let debug_name = bundle.debug_name.as_deref().unwrap();
    assert!(store
        .object_dir(evidence.record.digest_hex())
        .join(debug_name)
        .as_std_path()
        .is_file());

    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn mutable_reference_is_refused_before_any_transport() {
    // No routes registered: a network hit would fail loudly if it happened.
    let registry = MockRegistry::start();
    let root = tmp_root("mutable");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::new(true);

    let reference = oci_ref(&registry, "fixtures/rt", ":latest");
    let err = pull(&transport, &reference, &store, &PullPolicy::default())
        .expect_err("tag-only pull must be refused");
    assert_eq!(err.code(), "ARTIFACT_REFERENCE_MUTABLE");
    assert_store_empty(&store);

    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn corrupt_archive_blob_fails_the_oci_digest_check() {
    let registry = MockRegistry::start();
    let bundle = make_bundle("toy", b"plugin bytes");
    registry.register("fixtures/rt", "v1", &bundle);
    // Serve tampered bytes at the archive blob's address.
    let mut tampered = bundle.archive.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xff;
    registry.put(
        &format!("/v2/fixtures/rt/blobs/{}", bundle.artifact_digest),
        "application/octet-stream",
        tampered,
    );

    let root = tmp_root("corrupt-blob");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::new(true);
    let reference = oci_ref(&registry, "fixtures/rt", &format!("@{}", bundle.oci_digest));

    let err = pull(&transport, &reference, &store, &PullPolicy::default())
        .expect_err("corrupt blob must be refused");
    assert_eq!(err.code(), "ARTIFACT_OCI_DIGEST_MISMATCH");
    assert_store_empty(&store);

    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn substituted_producer_manifest_fails_the_archive_digest_check() {
    // The attacker controls the OCI manifest, so the substituted producer
    // manifest's blob digest is consistent — but its archive_digest no longer
    // matches the served archive bytes.
    let registry = MockRegistry::start();
    let honest = make_bundle("toy", b"plugin bytes");
    let decoy = make_bundle("toy", b"different bytes entirely");
    let substituted = finish_bundle(
        honest.archive.clone(),
        honest.archive_name.clone(),
        decoy.producer_manifest.clone(),
    );
    registry.register("fixtures/rt", "v1", &substituted);

    let root = tmp_root("substitution");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::new(true);
    let reference = oci_ref(
        &registry,
        "fixtures/rt",
        &format!("@{}", substituted.oci_digest),
    );

    let err = pull(&transport, &reference, &store, &PullPolicy::default())
        .expect_err("manifest substitution must be refused");
    assert_eq!(err.code(), "ARTIFACT_ARCHIVE_DIGEST_MISMATCH");
    assert_store_empty(&store);

    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn wrong_platform_and_wrong_kind_fail_policy_checks() {
    let registry = MockRegistry::start();
    let bundle = make_bundle("toy", b"plugin bytes");
    registry.register("fixtures/rt", "v1", &bundle);

    let root = tmp_root("policy");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::new(true);
    let reference = oci_ref(&registry, "fixtures/rt", &format!("@{}", bundle.oci_digest));

    let err = pull(
        &transport,
        &reference,
        &store,
        &PullPolicy {
            require_target: Some("cy2026-windows-x86_64-msvc143-py313-usd".to_string()),
            ..PullPolicy::default()
        },
    )
    .expect_err("wrong platform must be refused");
    assert_eq!(err.code(), "ARTIFACT_PLATFORM_MISMATCH");

    let err = pull(
        &transport,
        &reference,
        &store,
        &PullPolicy {
            require_kind: Some(ArtifactKind::Runtime),
            ..PullPolicy::default()
        },
    )
    .expect_err("wrong kind must be refused");
    assert_eq!(err.code(), "ARTIFACT_SUPPORT_LINE_MISMATCH");

    let err = pull(
        &transport,
        &reference,
        &store,
        &PullPolicy {
            expected_artifact_digest: Some(format!("sha256:{}", "11".repeat(32))),
            ..PullPolicy::default()
        },
    )
    .expect_err("pin mismatch must be refused");
    assert_eq!(err.code(), "ARTIFACT_ARCHIVE_DIGEST_MISMATCH");

    assert_store_empty(&store);
    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn oci_digest_pin_mismatch_is_refused_at_resolve() {
    let registry = MockRegistry::start();
    let bundle = make_bundle("toy", b"plugin bytes");
    registry.register("fixtures/rt", "v1", &bundle);
    // Register the honest manifest under a *different* digest address, as a
    // registry serving substituted bytes for a pinned reference would.
    let wrong_pin = format!("sha256:{}", "22".repeat(32));
    registry.put(
        &format!("/v2/fixtures/rt/manifests/{wrong_pin}"),
        "application/vnd.oci.image.manifest.v1+json",
        bundle.oci_manifest.clone(),
    );

    let root = tmp_root("oci-pin");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::new(true);
    let reference = oci_ref(&registry, "fixtures/rt", &format!("@{wrong_pin}"));

    let err = pull(&transport, &reference, &store, &PullPolicy::default())
        .expect_err("manifest bytes not matching the pin must be refused");
    assert_eq!(err.code(), "ARTIFACT_OCI_DIGEST_MISMATCH");
    assert_store_empty(&store);

    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn unsafe_archive_entries_are_refused_before_import() {
    // An archive smuggling a symlink: transport digests all match (the OCI
    // manifest is built over the hostile bytes), so only the pre-extraction
    // safety gate stands between the download and the store.
    let registry = MockRegistry::start();
    let mut archive_bytes = Vec::new();
    {
        let enc = zstd::stream::write::Encoder::new(&mut archive_bytes, 3)
            .unwrap()
            .auto_finish();
        let mut tar = tar::Builder::new(enc);
        let mut header = tar::Header::new_gnu();
        header.set_size(12);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "lib/payload.bin", &b"plugin bytes"[..])
            .unwrap();
        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Symlink);
        link.set_size(0);
        tar.append_link(&mut link, "lib/escape", "../../outside")
            .unwrap();
        tar.finish().unwrap();
    }
    let bundle = make_bundle_from_archive("toy", b"plugin bytes", archive_bytes);
    registry.register("fixtures/rt", "v1", &bundle);

    let root = tmp_root("unsafe");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::new(true);
    let reference = oci_ref(&registry, "fixtures/rt", &format!("@{}", bundle.oci_digest));

    let err = pull(&transport, &reference, &store, &PullPolicy::default())
        .expect_err("symlink smuggling must be refused");
    assert_eq!(err.code(), "ARTIFACT_ARCHIVE_UNSAFE");
    assert_store_empty(&store);

    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn duplicate_archive_paths_are_refused_before_import() {
    let registry = MockRegistry::start();
    let mut archive_bytes = Vec::new();
    {
        let enc = zstd::stream::write::Encoder::new(&mut archive_bytes, 3)
            .unwrap()
            .auto_finish();
        let mut tar = tar::Builder::new(enc);
        for content in [&b"plugin bytes"[..], &b"substituted bytes"[..]] {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, "lib/payload.bin", content)
                .unwrap();
        }
        tar.finish().unwrap();
    }
    let bundle = make_bundle_from_archive("toy", b"plugin bytes", archive_bytes);
    registry.register("fixtures/rt", "v1", &bundle);

    let root = tmp_root("duplicate-path");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::new(true);
    let reference = oci_ref(&registry, "fixtures/rt", &format!("@{}", bundle.oci_digest));

    let err = pull(&transport, &reference, &store, &PullPolicy::default())
        .expect_err("duplicate paths must be refused");
    assert_eq!(err.code(), "ARTIFACT_ARCHIVE_UNSAFE");
    assert_store_empty(&store);

    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn missing_artifact_reports_remote_not_found() {
    let registry = MockRegistry::start();
    let transport = OciTransport::new(true);
    let reference = oci_ref(
        &registry,
        "fixtures/rt",
        &format!("@sha256:{}", "33".repeat(32)),
    );
    let err = transport
        .resolve(&reference)
        .expect_err("nothing is registered");
    assert_eq!(err.code(), "ARTIFACT_REMOTE_NOT_FOUND");
}

#[test]
fn bearer_token_exchange_authenticates_the_pull() {
    let registry = MockRegistry::start();
    registry.require_token("fixture-token");
    let bundle = make_bundle("toy", b"plugin bytes");
    registry.register("fixtures/rt", "v1", &bundle);

    let root = tmp_root("auth");
    let store = ArtifactStore::at(root.join("store"));
    let transport = OciTransport::new(true);
    let reference = oci_ref(&registry, "fixtures/rt", &format!("@{}", bundle.oci_digest));

    let evidence = pull(&transport, &reference, &store, &PullPolicy::default()).unwrap();
    assert_eq!(evidence.remote.auth_mode, "token-exchange");
    assert_eq!(evidence.import_status, "imported");

    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn file_transport_pulls_a_dist_dir_with_the_same_chain() {
    let root = tmp_root("file-pull");
    // Lay out a producer dist dir: archive + manifest.json.
    let bundle = make_bundle("toy", b"plugin bytes");
    let dist = root.join("dist");
    std::fs::create_dir_all(dist.as_std_path()).unwrap();
    std::fs::write(
        dist.join(&bundle.archive_name).as_std_path(),
        &bundle.archive,
    )
    .unwrap();
    std::fs::write(
        dist.join("manifest.json").as_std_path(),
        &bundle.producer_manifest,
    )
    .unwrap();

    let store = ArtifactStore::at(root.join("store"));
    let transport = FileTransport::new();
    let reference = RemoteReference::parse(&format!("file://{dist}")).unwrap();

    let evidence = pull(
        &transport,
        &reference,
        &store,
        &PullPolicy {
            expected_artifact_digest: Some(bundle.artifact_digest.clone()),
            require_kind: Some(ArtifactKind::Plugin),
            require_target: Some(TARGET.to_string()),
        },
    )
    .unwrap();

    assert_eq!(evidence.remote.registry, "local-filesystem");
    assert_eq!(evidence.remote.auth_mode, "none");
    assert_eq!(evidence.record.digest, bundle.artifact_digest);
    // The oci_digest step is skipped for a backend without an OCI manifest.
    assert!(evidence
        .verification
        .iter()
        .any(|(step, status)| *step == "oci_digest" && *status == "skipped"));
    assert!(store.verify(&bundle.artifact_digest).unwrap().passed());

    // The source dist dir is untouched (fetch reads in place, import copies).
    assert!(dist.join(&bundle.archive_name).as_std_path().is_file());

    std::fs::remove_dir_all(root.as_std_path()).ok();
}

#[test]
fn file_transport_missing_dir_reports_remote_not_found() {
    let transport = FileTransport::new();
    let reference = RemoteReference::parse("file:///nonexistent/dist").unwrap();
    let err = transport.resolve(&reference).expect_err("missing dir");
    assert_eq!(err.code(), "ARTIFACT_REMOTE_NOT_FOUND");
}

/// Keep the helper honest: the fixture archive round-trips through the store.
#[test]
fn fixture_bundle_is_a_valid_dist_dir() {
    let root = tmp_root("fixture-sanity");
    let bundle = make_bundle("toy", b"plugin bytes");
    let dist = root.join("dist");
    std::fs::create_dir_all(dist.as_std_path()).unwrap();
    std::fs::write(
        dist.join(&bundle.archive_name).as_std_path(),
        &bundle.archive,
    )
    .unwrap();
    std::fs::write(
        dist.join("manifest.json").as_std_path(),
        &bundle.producer_manifest,
    )
    .unwrap();

    let store = ArtifactStore::at(root.join("store"));
    let out = store
        .import(Utf8Path::new(dist.as_str()), ArtifactSource::Imported)
        .unwrap();
    assert_eq!(out.record.digest, bundle.artifact_digest);

    std::fs::remove_dir_all(root.as_std_path()).ok();
}
