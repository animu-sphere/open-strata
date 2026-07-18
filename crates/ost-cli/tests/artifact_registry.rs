// SPDX-License-Identifier: Apache-2.0
//! End-to-end tests for the local artifact registry (Phase 6 MVP).
//!
//! These drive the real `ost` binary against a throwaway store (`OST_HOME`),
//! feeding it a producer-shaped dist directory built with the same packer
//! (`ost_build::pack_dir`) the product uses. Covered contract:
//!
//! - `artifact import` registers by digest and is idempotent on the same bytes;
//! - `artifact list|show` address the artifact by digest / unique prefix;
//! - `artifact verify` passes on intact bytes and fails (exit 5) on corruption;
//! - `artifact export` round-trips to a re-importable directory;
//! - a tampered dist dir is refused at import (exit 5, ARTIFACT_DIGEST_MISMATCH).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use camino::Utf8PathBuf;

fn ost_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ost")
}

struct Sandbox {
    base: PathBuf,
    home: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let base =
            std::env::temp_dir().join(format!("ost-art-{tag}-{}-{nanos}", std::process::id()));
        let home = base.join("home");
        std::fs::create_dir_all(&home).unwrap();
        Sandbox { base, home }
    }

    fn ost(&self, args: &[&str]) -> Output {
        Command::new(ost_bin())
            .args(args)
            .current_dir(&self.base)
            .env("OST_HOME", &self.home)
            .env_remove("ACTIONS_ID_TOKEN_REQUEST_URL")
            .env_remove("ACTIONS_ID_TOKEN_REQUEST_TOKEN")
            .output()
            .expect("spawn ost")
    }

    /// Produce a dist dir shaped like `ost plugin package` output: an archive
    /// packed with the product packer, plus a consistent `manifest.json`.
    fn make_plugin_dist(&self, name: &str, payload: &[u8]) -> PathBuf {
        let dist = Utf8PathBuf::from_path_buf(self.base.join(format!("dist-{name}"))).unwrap();
        let stage = Utf8PathBuf::from_path_buf(self.base.join(format!("stage-{name}"))).unwrap();
        std::fs::create_dir_all(stage.join("lib").as_std_path()).unwrap();
        std::fs::write(stage.join("lib/payload.bin").as_std_path(), payload).unwrap();
        std::fs::write(stage.join("plugInfo.json").as_std_path(), b"{}").unwrap();

        let archive_name = format!("{name}-0.1.0-cy2026-test.tar.zst");
        let archive = dist.join(&archive_name);
        let files = ost_build::stage_files(&stage).unwrap();
        let packed = ost_build::pack_dir(&stage, &archive, &files).unwrap();

        let files_json: Vec<_> = packed
            .files
            .iter()
            .map(|f| serde_json::json!({ "path": f.path, "sha256": f.sha256, "size": f.size }))
            .collect();
        let manifest = serde_json::json!({
            "schema": 1,
            "kind": "openstrata.plugin-bundle",
            "plugin": { "name": name, "version": "0.1.0", "kind": "usd-fileformat", "license": "Apache-2.0" },
            "target": "cy2026-test",
            "archive": archive_name,
            "archive_digest": packed.archive_digest,
            "archive_size": packed.archive_size,
            "total_size": packed.total_size,
            "created_unix": 1_750_000_000u64,
            "provenance": {
                "profile": "usd",
                "cxx_abi": "msvc143",
                "runtime": { "id": "openstrata-cy2026-usd", "digest": "sha256:feed" },
                "validation": { "passed": true },
            },
            "files": files_json,
        });
        std::fs::write(
            dist.join("manifest.json").as_std_path(),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        dist.into_std_path_buf()
    }

    fn add_evidence(&self, dist: &Path) {
        let dist = Utf8PathBuf::from_path_buf(dist.to_path_buf()).unwrap();
        let manifest_path = dist.join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(manifest_path.as_std_path()).unwrap()).unwrap();
        manifest["build"] = serde_json::json!({
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
        });
        ost_artifact::generate_evidence(&dist, &mut manifest).unwrap();
        std::fs::write(
            manifest_path.as_std_path(),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn stdout_json(out: &Output) -> serde_json::Value {
    assert!(
        out.status.success(),
        "expected success\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout is a JSON envelope")
}

fn path_str(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn import_show_verify_export_roundtrip() {
    let sb = Sandbox::new("roundtrip");
    let dist = sb.make_plugin_dist("toy", b"real plugin bytes");

    // Import registers the artifact by digest.
    let v = stdout_json(&sb.ost(&["--json", "artifact", "import", path_str(&dist)]));
    assert_eq!(v["data"]["already_present"], false);
    let digest = v["data"]["artifact"]["digest"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(digest.starts_with("sha256:"));

    // Re-import of the same bytes is a no-op.
    let v = stdout_json(&sb.ost(&["--json", "artifact", "import", path_str(&dist)]));
    assert_eq!(v["data"]["already_present"], true);

    // List sees exactly one artifact; show resolves a unique hex prefix.
    let v = stdout_json(&sb.ost(&["--json", "artifact", "list"]));
    assert_eq!(v["data"]["artifacts"].as_array().unwrap().len(), 1);
    let hex = digest.strip_prefix("sha256:").unwrap();
    let prefix = &hex[..10];
    let v = stdout_json(&sb.ost(&["--json", "artifact", "show", prefix]));
    assert_eq!(v["data"]["artifact"]["digest"], digest.as_str());
    assert_eq!(v["data"]["artifact"]["kind"], "plugin");
    assert_eq!(v["data"]["artifact"]["licenses"][0], "Apache-2.0");

    // Verify passes on intact bytes.
    let v = stdout_json(&sb.ost(&["--json", "artifact", "verify", &digest]));
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["passed"], true);

    // Export round-trips into a re-importable directory.
    let dest = sb.base.join("handoff");
    let v = stdout_json(&sb.ost(&["--json", "artifact", "export", &digest, path_str(&dest)]));
    assert_eq!(v["data"]["exported"], true);
    assert!(dest.join("manifest.json").is_file());

    // A second store (fresh OST_HOME) can import the exported dir.
    let sb2 = Sandbox::new("reimport");
    let v = stdout_json(&sb2.ost(&["--json", "artifact", "import", path_str(&dest)]));
    assert_eq!(v["data"]["artifact"]["digest"], digest.as_str());
}

#[test]
fn tampered_archive_is_refused_and_verify_fails_on_corruption() {
    let sb = Sandbox::new("tamper");
    let dist = sb.make_plugin_dist("toy", b"real plugin bytes");

    // Import first, so we can corrupt the *store* copy afterwards.
    let v = stdout_json(&sb.ost(&["--json", "artifact", "import", path_str(&dist)]));
    let digest = v["data"]["artifact"]["digest"]
        .as_str()
        .unwrap()
        .to_string();
    let hex = digest.strip_prefix("sha256:").unwrap();

    // Tamper the dist archive → a fresh import must refuse (exit 5).
    let archive = dist.join("toy-0.1.0-cy2026-test.tar.zst");
    let mut bytes = std::fs::read(&archive).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&archive, &bytes).unwrap();
    let sb_fresh = Sandbox::new("tamper-fresh");
    let out = sb_fresh.ost(&["--json", "artifact", "import", path_str(&dist)]);
    assert_eq!(out.status.code(), Some(5), "validation exit code");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "ARTIFACT_DIGEST_MISMATCH");

    // Corrupt the stored archive → verify fails with exit 5.
    let stored = sb
        .home
        .join("artifacts")
        .join("objects")
        .join("sha256")
        .join(hex)
        .join("toy-0.1.0-cy2026-test.tar.zst");
    let mut bytes = std::fs::read(&stored).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&stored, &bytes).unwrap();

    let out = sb.ost(&["--json", "artifact", "verify", &digest]);
    assert_eq!(out.status.code(), Some(5));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], false, "report envelope carries the failure");

    // Unknown digests are a precondition failure (exit 4).
    let out = sb.ost(&["--json", "artifact", "show", "deadbeef00"]);
    assert_eq!(out.status.code(), Some(4));
}

#[test]
fn verify_enforces_strict_artifact_policy() {
    let sb = Sandbox::new("policy");
    let dist = sb.make_plugin_dist("toy", b"policy checked bytes");
    let imported = stdout_json(&sb.ost(&["--json", "artifact", "import", path_str(&dist)]));
    let digest = imported["data"]["artifact"]["digest"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(imported["data"]["artifact"]["trust"], "local");

    let policy = sb.base.join("openstrata-artifact-policy.toml");
    std::fs::write(&policy, "schema = 1\nminimum_trust = \"local\"\n").unwrap();
    let verified = stdout_json(&sb.ost(&[
        "--json",
        "artifact",
        "verify",
        &digest,
        "--policy",
        path_str(&policy),
    ]));
    assert_eq!(verified["data"]["policy"]["passed"], true);
    assert_eq!(verified["data"]["trust"], "local");

    std::fs::write(&policy, "schema = 1\nminimum_trust = \"unsigned\"\n").unwrap();
    let out = sb.ost(&[
        "--json",
        "artifact",
        "verify",
        &digest,
        "--policy",
        path_str(&policy),
    ]);
    assert_eq!(out.status.code(), Some(5));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["ok"], false);
    assert_eq!(
        report["data"]["policy"]["error_code"],
        "ARTIFACT_POLICY_TRUST_INSUFFICIENT"
    );
    let out = sb.ost(&["artifact", "verify", &digest, "--policy", path_str(&policy)]);
    assert_eq!(out.status.code(), Some(5));
    let human = String::from_utf8_lossy(&out.stdout);
    assert!(human.contains("policy result:  FAIL"), "{human}");
    assert!(human.contains("result: FAIL"), "{human}");

    std::fs::write(&policy, "schema = 1\nminimum_trsut = \"local\"\n").unwrap();
    let out = sb.ost(&[
        "--json",
        "artifact",
        "verify",
        &digest,
        "--policy",
        path_str(&policy),
    ]);
    assert_eq!(out.status.code(), Some(3));
    let error: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(error["error"]["code"], "ARTIFACT_POLICY_PARSE_FAILED");
}

#[test]
fn verify_enforces_inline_minimum_trust_and_uses_the_stricter_floor() {
    let sb = Sandbox::new("minimum-trust");
    let dist = sb.make_plugin_dist("toy", b"inline trust checked bytes");
    let imported = stdout_json(&sb.ost(&["--json", "artifact", "import", path_str(&dist)]));
    let digest = imported["data"]["artifact"]["digest"]
        .as_str()
        .unwrap()
        .to_string();

    let local = stdout_json(&sb.ost(&[
        "--json",
        "artifact",
        "verify",
        &digest,
        "--minimum-trust",
        "local",
    ]));
    assert_eq!(local["data"]["policy"]["minimum_trust"], "local");
    assert_eq!(local["data"]["policy"]["passed"], true);

    let out = sb.ost(&[
        "--json",
        "artifact",
        "verify",
        &digest,
        "--minimum-trust",
        "verified",
    ]);
    assert_eq!(out.status.code(), Some(5));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["data"]["policy"]["minimum_trust"], "verified");
    assert_eq!(
        report["data"]["policy"]["error_code"],
        "ARTIFACT_POLICY_TRUST_INSUFFICIENT"
    );

    let policy = sb.base.join("stricter-policy.toml");
    std::fs::write(&policy, "schema = 1\nminimum_trust = \"attested\"\n").unwrap();
    let out = sb.ost(&[
        "--json",
        "artifact",
        "verify",
        &digest,
        "--minimum-trust",
        "unsigned",
        "--policy",
        path_str(&policy),
    ]);
    assert_eq!(out.status.code(), Some(5));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["data"]["policy"]["minimum_trust"], "attested");
}

#[test]
fn evidence_attaches_to_an_already_imported_digest_and_satisfies_the_gate() {
    // The usd-vrm-plugins regression, end to end: a machine that imported the
    // artifact before evidence existed adopts a build that now ships sidecars.
    // Before the fix the second import silently dropped them and every
    // evidence-gated lane went red with nothing wrong in the repo.
    let sb = Sandbox::new("late-evidence");
    let dist = sb.make_plugin_dist("toy", b"pre-evidence bytes");

    let first = stdout_json(&sb.ost(&["--json", "artifact", "import", path_str(&dist)]));
    let digest = first["data"]["artifact"]["digest"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(first["data"]["already_present"], false);
    assert!(first["data"]["artifact"]["sbom"].is_null());
    assert_eq!(
        first["data"]["evidence_attached"].as_array().unwrap().len(),
        0
    );

    // The gate cannot be satisfied yet — this is the red lane.
    let gated = sb.ost(&[
        "--json",
        "artifact",
        "verify",
        &digest,
        "--require-sbom",
        "--require-provenance",
    ]);
    assert_eq!(gated.status.code(), Some(5));

    // Same bytes, now with sidecars. Generating evidence stamps `build` into
    // the producer manifest, so this import carries a *different* manifest for
    // an identical digest — precisely the case the old guard rejected.
    sb.add_evidence(&dist);
    let second = stdout_json(&sb.ost(&["--json", "artifact", "import", path_str(&dist)]));
    assert_eq!(second["data"]["already_present"], true);
    assert_eq!(second["data"]["artifact"]["digest"], digest.as_str());
    assert_eq!(
        second["data"]["evidence_attached"],
        serde_json::json!([ost_artifact::SBOM_FILE, ost_artifact::PROVENANCE_FILE]),
        "sidecars must attach to a digest already in the registry"
    );
    assert_eq!(second["data"]["artifact"]["sbom"], ost_artifact::SBOM_FILE);

    // The gate now passes without any repo-side change.
    let verified = stdout_json(&sb.ost(&[
        "--json",
        "artifact",
        "verify",
        &digest,
        "--require-sbom",
        "--require-provenance",
    ]));
    assert_eq!(verified["data"]["evidence"]["sbom"]["passed"], true);
    assert_eq!(verified["data"]["evidence"]["provenance"]["passed"], true);

    // A further import reports the no-op rather than staying silent.
    let third = stdout_json(&sb.ost(&["--json", "artifact", "import", path_str(&dist)]));
    assert_eq!(
        third["data"]["evidence_attached"].as_array().unwrap().len(),
        0
    );
    assert_eq!(
        third["data"]["evidence_skipped"],
        serde_json::json!([ost_artifact::SBOM_FILE, ost_artifact::PROVENANCE_FILE])
    );
}

#[test]
fn artifact_rm_resets_the_registry_entry() {
    let sb = Sandbox::new("rm");
    let dist = sb.make_plugin_dist("toy", b"removable bytes");
    let imported = stdout_json(&sb.ost(&["--json", "artifact", "import", path_str(&dist)]));
    let digest = imported["data"]["artifact"]["digest"]
        .as_str()
        .unwrap()
        .to_string();

    let removed = stdout_json(&sb.ost(&["--json", "artifact", "rm", &digest]));
    assert_eq!(removed["data"]["removed"], true);
    assert_eq!(removed["data"]["artifact"]["digest"], digest.as_str());

    let listed = stdout_json(&sb.ost(&["--json", "artifact", "list"]));
    assert_eq!(listed["data"]["artifacts"].as_array().unwrap().len(), 0);

    // Removing it again is a clean coded failure, not a silent success.
    let missing = sb.ost(&["--json", "artifact", "rm", &digest]);
    assert!(!missing.status.success());
    let report: serde_json::Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(report["error"]["code"], "ARTIFACT_NOT_FOUND");

    // And the digest can be imported fresh afterwards.
    let again = stdout_json(&sb.ost(&["--json", "artifact", "import", path_str(&dist)]));
    assert_eq!(again["data"]["already_present"], false);
    assert_eq!(again["data"]["artifact"]["digest"], digest.as_str());
}

#[test]
fn verify_can_require_and_validate_attached_evidence() {
    let sb = Sandbox::new("evidence");
    let legacy = sb.make_plugin_dist("legacy", b"no evidence");
    let imported = stdout_json(&sb.ost(&["--json", "artifact", "import", path_str(&legacy)]));
    let legacy_digest = imported["data"]["artifact"]["digest"].as_str().unwrap();
    let out = sb.ost(&[
        "--json",
        "artifact",
        "verify",
        legacy_digest,
        "--require-sbom",
        "--require-provenance",
    ]);
    assert_eq!(out.status.code(), Some(5));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        report["data"]["evidence"]["sbom"]["error_code"],
        "ARTIFACT_SBOM_REQUIRED"
    );
    assert_eq!(
        report["data"]["evidence"]["provenance"]["error_code"],
        "ARTIFACT_PROVENANCE_REQUIRED"
    );

    let dist = sb.make_plugin_dist("attested", b"evidence bytes");
    sb.add_evidence(&dist);
    let imported = stdout_json(&sb.ost(&["--json", "artifact", "import", path_str(&dist)]));
    let digest = imported["data"]["artifact"]["digest"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        imported["data"]["artifact"]["sbom"],
        ost_artifact::SBOM_FILE
    );
    assert_eq!(
        imported["data"]["artifact"]["provenance"],
        ost_artifact::PROVENANCE_FILE
    );
    let verified = stdout_json(&sb.ost(&[
        "--json",
        "artifact",
        "verify",
        &digest,
        "--require-sbom",
        "--require-provenance",
    ]));
    assert_eq!(verified["data"]["evidence"]["sbom"]["passed"], true);
    assert_eq!(verified["data"]["evidence"]["provenance"]["passed"], true);
    assert_eq!(verified["data"]["record_trust"], "local");
    assert_eq!(verified["data"]["evidence_trust"], "attested");
    assert_eq!(verified["data"]["trust"], "attested");

    // A separate publisher imports a candidate with conservative local record
    // trust. Subject-bound provenance that matches an allowed publisher plus
    // the valid SBOM raises only this verification's effective trust, allowing
    // the generated release gate without making imported trust sticky.
    let policy = sb.base.join("candidate-policy.toml");
    std::fs::write(
        &policy,
        r#"schema = 1

[[allowed_publishers]]
id = "release"
trust = "trusted"
repository = "owner/repo"
workflow_path = ".github/workflows/release.yml"
git_refs = ["refs/tags/v*"]
actors = ["release-bot"]
events = ["push"]
"#,
    )
    .unwrap();
    let trusted = stdout_json(&sb.ost(&[
        "--json",
        "artifact",
        "verify",
        &digest,
        "--minimum-trust",
        "trusted",
        "--require-sbom",
        "--require-provenance",
        "--policy",
        path_str(&policy),
    ]));
    assert_eq!(trusted["data"]["record_trust"], "local");
    assert_eq!(trusted["data"]["evidence_trust"], "trusted");
    assert_eq!(trusted["data"]["trust"], "trusted");
    assert_eq!(
        trusted["data"]["evidence"]["provenance"]["matched_publisher"],
        "release"
    );

    let stored_provenance = sb
        .home
        .join("artifacts")
        .join("objects")
        .join("sha256")
        .join(digest.strip_prefix("sha256:").unwrap())
        .join(ost_artifact::PROVENANCE_FILE);
    std::fs::write(stored_provenance, b"tampered\n").unwrap();
    let out = sb.ost(&[
        "--json",
        "artifact",
        "verify",
        &digest,
        "--require-provenance",
    ]);
    assert_eq!(out.status.code(), Some(5));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        report["data"]["evidence"]["provenance"]["error_code"],
        "ARTIFACT_EVIDENCE_DIGEST_MISMATCH"
    );
}

#[test]
fn push_enforces_protected_publisher_policy_before_transport() {
    use std::io::Write;

    let sb = Sandbox::new("push-policy");
    let dist = sb.make_plugin_dist("toy", b"publisher checked bytes");
    let imported = stdout_json(&sb.ost(&["--json", "artifact", "import", path_str(&dist)]));
    let digest = imported["data"]["artifact"]["digest"].as_str().unwrap();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let policy = sb.base.join("openstrata-artifact-policy.toml");
    std::fs::write(
        &policy,
        format!(
            r#"schema = 1

[[protected_namespaces]]
namespace = "{address}/owner"
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
"#
        ),
    )
    .unwrap();
    let destination = format!("oci://{address}/owner/toy:v0.14.0");

    // The policy is auto-discovered and rejects before any registry request.
    listener.set_nonblocking(true).unwrap();
    let out = sb.ost(&["--json", "artifact", "push", digest, &destination]);
    assert_eq!(out.status.code(), Some(4));
    let error: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        error["error"]["code"],
        "ARTIFACT_POLICY_IDENTITY_UNAVAILABLE"
    );
    assert!(
        listener.accept().is_err(),
        "policy failure reached the registry"
    );

    // The explicit override crosses the policy gate and reaches transport.
    listener.set_nonblocking(false).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });
    let out = sb.ost(&[
        "--json",
        "artifact",
        "push",
        digest,
        &destination,
        "--allow-untrusted-publisher",
    ]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(6));
    let error: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(error["error"]["code"], "ARTIFACT_TRANSPORT_FAILED");
}

/// `ost plugin publish` consumes `ost plugin package` output, enforces its
/// gates, and registers the artifact as `published`.
#[test]
fn plugin_publish_gates_and_registers_by_digest() {
    let sb = Sandbox::new("publish");

    // A real scaffolded bundle gives publish its name/version/root.
    let out = sb.ost(&[
        "plugin",
        "new",
        "usd-fileformat",
        "toy",
        "--extension",
        "toy",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Without a package, publish is a precondition failure whose message names
    // the expected dist manifest — derive the target-specific path from it
    // rather than reimplementing the target-id computation here.
    let out = sb.ost(&[
        "--json",
        "plugin",
        "publish",
        "toy",
        "--target",
        "cy2026",
        "--profile",
        "usd",
    ]);
    assert_eq!(out.status.code(), Some(4));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["code"], "PRECONDITION_FAILED");
    let message = v["error"]["message"].as_str().unwrap();
    let manifest_path = PathBuf::from(
        message
            .split("expected ")
            .nth(1)
            .expect("message names the expected manifest path"),
    );
    let dist = manifest_path.parent().unwrap().to_path_buf();

    // Stage a packaged artifact at that path (packed by the product packer).
    std::fs::create_dir_all(&dist).unwrap();
    let stage = Utf8PathBuf::from_path_buf(sb.base.join("pub-stage")).unwrap();
    std::fs::create_dir_all(stage.join("lib").as_std_path()).unwrap();
    std::fs::write(stage.join("lib/toy.dll").as_std_path(), b"lib bytes").unwrap();
    let target_id = dist.file_name().unwrap().to_str().unwrap().to_string();
    let archive_name = format!("toy-0.1.0-{target_id}.tar.zst");
    let archive = Utf8PathBuf::from_path_buf(dist.join(&archive_name)).unwrap();
    let files = ost_build::stage_files(&stage).unwrap();
    let packed = ost_build::pack_dir(&stage, &archive, &files).unwrap();
    let files_json: Vec<_> = packed
        .files
        .iter()
        .map(|f| serde_json::json!({ "path": f.path, "sha256": f.sha256, "size": f.size }))
        .collect();
    let mut manifest = serde_json::json!({
        "schema": 1,
        "kind": "openstrata.plugin-bundle",
        "plugin": { "name": "toy", "version": "0.1.0", "kind": "usd-fileformat", "license": "Apache-2.0" },
        "target": target_id,
        "archive": archive_name,
        "archive_digest": packed.archive_digest,
        "archive_size": packed.archive_size,
        "total_size": packed.total_size,
        "provenance": {
            "profile": "usd",
            "cxx_abi": "msvc143",
            "runtime": { "id": "openstrata-cy2026-usd", "digest": "sha256:feed" },
            "validation": { "passed": true },
        },
        "files": files_json,
    });
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    // A gated field turns publish away with its own stable code (exit 5).
    manifest["provenance"]["validation"]["passed"] = false.into();
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let out = sb.ost(&[
        "--json",
        "plugin",
        "publish",
        "toy",
        "--target",
        "cy2026",
        "--profile",
        "usd",
    ]);
    assert_eq!(out.status.code(), Some(5));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["code"], "PUBLISH_VALIDATION_REQUIRED");

    // With the gates satisfied, publish registers the artifact by digest.
    manifest["provenance"]["validation"]["passed"] = true.into();
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let v = stdout_json(&sb.ost(&[
        "--json",
        "plugin",
        "publish",
        "toy",
        "--target",
        "cy2026",
        "--profile",
        "usd",
    ]));
    assert_eq!(v["data"]["published"], true);
    assert_eq!(v["data"]["artifact"]["source"], "published");
    let digest = v["data"]["digest"].as_str().unwrap().to_string();
    assert_eq!(digest, packed.archive_digest);

    // The registry now resolves it.
    let v = stdout_json(&sb.ost(&["--json", "artifact", "show", &digest]));
    assert_eq!(v["data"]["artifact"]["name"], "toy");
    assert_eq!(v["data"]["artifact"]["kind"], "plugin");
}
