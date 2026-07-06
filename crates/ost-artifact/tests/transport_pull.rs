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

use camino::{Utf8Path, Utf8PathBuf};

use ost_artifact::transport::oci::{MEDIA_TYPE_ARCHIVE, MEDIA_TYPE_PRODUCER_MANIFEST};
use ost_artifact::{
    pull, ArtifactKind, ArtifactSource, ArtifactStore, ArtifactTransport, FileTransport,
    OciTransport, PullPolicy, RemoteReference,
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
}

struct MockRegistry {
    addr: SocketAddr,
    routes: Arc<Mutex<HashMap<String, Route>>>,
    /// When set, every /v2/ request must carry `Authorization: Bearer <this>`;
    /// the mock answers 401 with a token-exchange challenge otherwise.
    required_token: Arc<Mutex<Option<String>>>,
}

impl MockRegistry {
    fn start() -> MockRegistry {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock registry");
        let addr = listener.local_addr().unwrap();
        let routes: Arc<Mutex<HashMap<String, Route>>> = Arc::new(Mutex::new(HashMap::new()));
        let required_token: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        let thread_routes = Arc::clone(&routes);
        let thread_token = Arc::clone(&required_token);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let _ = handle_connection(stream, addr, &thread_routes, &thread_token);
            }
        });

        MockRegistry {
            addr,
            routes,
            required_token,
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
            },
        );
    }

    fn require_token(&self, token: &str) {
        *self.required_token.lock().unwrap() = Some(token.to_string());
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
    }
}

fn handle_connection(
    mut stream: TcpStream,
    addr: SocketAddr,
    routes: &Mutex<HashMap<String, Route>>,
    required_token: &Mutex<Option<String>>,
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
    let auth = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .map(|(_, v)| v.trim().to_string());

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

    match routes.lock().unwrap().get(path) {
        Some(route) => {
            let route = route.clone();
            respond(
                &mut stream,
                route.status,
                route.content_type,
                &route.body,
                "",
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
    stream.write_all(body)?;
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
    oci_manifest: Vec<u8>,
    oci_digest: String,
    artifact_digest: String,
}

/// A plugin-bundle artifact with one payload file, plus its OCI bundle.
fn make_bundle(name: &str, content: &[u8]) -> Bundle {
    let archive = tar_zst(&[("lib/payload.bin", content)]);
    make_bundle_from_archive(name, content, archive)
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
    let artifact_digest = digest::sha256_hex(&archive);
    let oci = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "artifactType": "application/vnd.openstrata.artifact.v1",
        "config": {
            "mediaType": "application/vnd.openstrata.artifact.descriptor.v1+json",
            "digest": digest::sha256_hex(b"{}"),
            "size": 2,
        },
        "layers": [
            {
                "mediaType": MEDIA_TYPE_ARCHIVE,
                "digest": artifact_digest,
                "size": archive.len(),
                "annotations": { "org.opencontainers.image.title": archive_name },
            },
            {
                "mediaType": MEDIA_TYPE_PRODUCER_MANIFEST,
                "digest": digest::sha256_hex(&producer_manifest),
                "size": producer_manifest.len(),
                "annotations": { "org.opencontainers.image.title": "manifest.json" },
            },
        ],
    });
    let oci_manifest = serde_json::to_vec_pretty(&oci).unwrap();
    let oci_digest = digest::sha256_hex(&oci_manifest);
    Bundle {
        archive,
        archive_name,
        producer_manifest,
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
    // Every chain step passed (none skipped: the policy pinned everything).
    for (step, status) in &evidence.verification {
        assert_eq!(*status, "passed", "step {step}");
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
