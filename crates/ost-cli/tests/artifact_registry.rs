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
