// SPDX-License-Identifier: Apache-2.0
//! End-to-end tests for the CI support matrix (Phase 5 MVP):
//! `ost ci init | validate [--resolve] | generate github` and
//! `ost artifact extract` (the step the generated workflow runs).
//!
//! Covered contract:
//!
//! - `init` scaffolds a parseable starter matrix and refuses to clobber it;
//! - `validate` accepts the starter structurally, and `--resolve` fails
//!   (exit 5) until the pinned digests actually exist in the local registry;
//! - `generate github` emits deterministic, YAML-parseable workflow with one
//!   explicit include entry per cell, refusing to overwrite without `--force`;
//! - with real registry entries (an exported runtime + an imported plugin),
//!   `validate --resolve` passes and `artifact extract` unpacks the plugin.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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
            std::env::temp_dir().join(format!("ost-ci-{tag}-{}-{nanos}", std::process::id()));
        let home = base.join("home");
        std::fs::create_dir_all(&home).unwrap();
        Sandbox { base, home }
    }

    fn ost(&self, args: &[&str]) -> Output {
        Command::new(ost_bin())
            .args(args)
            .current_dir(&self.base)
            .env("OST_HOME", &self.home)
            .env_remove("OST_USD_ROOT")
            .env_remove("OST_USD_SRC")
            .env_remove("OST_USD_DEPS")
            .output()
            .expect("spawn ost")
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
fn init_validate_generate_lifecycle() {
    let sb = Sandbox::new("lifecycle");

    // init scaffolds; re-init refuses (the user's edits are sacred).
    let v = stdout_json(&sb.ost(&["--json", "ci", "init"]));
    assert_eq!(v["data"]["created"], true);
    assert!(sb.base.join("openstrata.ci.yaml").is_file());
    assert_eq!(sb.ost(&["--json", "ci", "init"]).status.code(), Some(2));

    // The starter matrix is structurally valid…
    let v = stdout_json(&sb.ost(&["--json", "ci", "validate"]));
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["cells"], 1);

    // …but its placeholder digests do not resolve in an empty registry.
    let out = sb.ost(&["--json", "ci", "validate", "--resolve"]);
    assert_eq!(out.status.code(), Some(5));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["data"]["unresolved"].as_array().unwrap().len(), 2);

    // generate --stdout prints the workflow itself (parseable YAML).
    let out = sb.ost(&["ci", "generate", "github", "--stdout"]);
    assert!(out.status.success());
    let doc: serde_yaml::Value = serde_yaml::from_slice(&out.stdout).unwrap();
    let include = &doc["jobs"]["cell"]["strategy"]["matrix"]["include"];
    assert_eq!(include.as_sequence().unwrap().len(), 1);
    assert_eq!(include[0]["name"], "example-linux-cy2026-usd");

    // generate writes the default path; overwrite needs --force.
    let v = stdout_json(&sb.ost(&["--json", "ci", "generate", "github"]));
    let workflow = PathBuf::from(v["data"]["workflow"].as_str().unwrap());
    assert!(sb.base.join(&workflow).is_file());
    assert_eq!(
        sb.ost(&["--json", "ci", "generate", "github"])
            .status
            .code(),
        Some(2)
    );
    stdout_json(&sb.ost(&["--json", "ci", "generate", "github", "--force"]));

    // A matrix that fails structural validation is a configuration error.
    std::fs::write(
        sb.base.join("bad.yaml"),
        "schema: 1\ncells:\n  - name: Bad_Name\n    runtime_artifact: sha256:xyz\n    plugin_artifact: sha256:xyz\n    platform: cy2026\n    profile: usd\n",
    )
    .unwrap();
    let out = sb.ost(&["--json", "ci", "validate", "--matrix", "bad.yaml"]);
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn resolve_passes_with_registry_artifacts_and_extract_unpacks_the_plugin() {
    let sb = Sandbox::new("resolve");

    // A runtime artifact: pull mock, promote to a validated build, export.
    stdout_json(&sb.ost(&["--json", "runtime", "pull", "cy2026", "--profile", "usd"]));
    let runtimes = sb.home.join("runtimes");
    let prefix = std::fs::read_dir(&runtimes)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let manifest_path = prefix.join("runtime.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["source"] = "build".into();
    manifest["validation"] = "passed".into();
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    for (rel, content) in [
        ("plugin/usd/plugInfo.json", "{}"),
        ("lib/python/pxr/__init__.py", ""),
    ] {
        let path = prefix.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
    }
    let v = stdout_json(&sb.ost(&["--json", "runtime", "export", "cy2026", "--profile", "usd"]));
    let runtime_digest = v["data"]["digest"].as_str().unwrap().to_string();

    // A plugin artifact: a minimal packaged bundle, imported by digest.
    let stage = camino::Utf8PathBuf::from_path_buf(sb.base.join("stage")).unwrap();
    std::fs::create_dir_all(stage.join("lib").as_std_path()).unwrap();
    std::fs::write(stage.join("lib/toy.dll").as_std_path(), b"lib bytes").unwrap();
    std::fs::write(stage.join("plugInfo.json").as_std_path(), b"{}").unwrap();
    let dist = camino::Utf8PathBuf::from_path_buf(sb.base.join("dist")).unwrap();
    let archive = dist.join("toy-0.1.0.tar.zst");
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
        "plugin": { "name": "toy", "version": "0.1.0", "kind": "usd-fileformat", "license": "Apache-2.0" },
        "target": "cy2026-test",
        "archive": "toy-0.1.0.tar.zst",
        "archive_digest": packed.archive_digest,
        "archive_size": packed.archive_size,
        "total_size": packed.total_size,
        "files": files_json,
    });
    std::fs::write(
        dist.join("manifest.json").as_std_path(),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let v = stdout_json(&sb.ost(&["--json", "artifact", "import", dist.as_str()]));
    let plugin_digest = v["data"]["artifact"]["digest"]
        .as_str()
        .unwrap()
        .to_string();

    // A matrix pinning both digests resolves against the local registry.
    std::fs::write(
        sb.base.join("openstrata.ci.yaml"),
        format!(
            "schema: 1\ncells:\n  - name: linux-usd-toy\n    runtime_artifact: {runtime_digest}\n    plugin_artifact: {plugin_digest}\n    platform: cy2026\n    profile: usd\n    up_to: 4\n    host:\n      os: linux\n      labels: [self-hosted, linux]\n"
        ),
    )
    .unwrap();
    let v = stdout_json(&sb.ost(&["--json", "ci", "validate", "--resolve"]));
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["unresolved"].as_array().unwrap().len(), 0);

    // The generated workflow pins the same digests into the include entry.
    let out = sb.ost(&["ci", "generate", "github", "--stdout"]);
    let doc: serde_yaml::Value = serde_yaml::from_slice(&out.stdout).unwrap();
    let entry = &doc["jobs"]["cell"]["strategy"]["matrix"]["include"][0];
    assert_eq!(entry["runtime_artifact"], runtime_digest.as_str());
    assert_eq!(entry["plugin_artifact"], plugin_digest.as_str());
    assert_eq!(entry["up_to"], 4);

    // `artifact extract` — the workflow's unpack step — restores the bundle.
    let dest = sb.base.join("plugin-under-test");
    let v = stdout_json(&sb.ost(&[
        "--json",
        "artifact",
        "extract",
        &plugin_digest,
        path_str(&dest),
    ]));
    assert_eq!(v["data"]["extracted"], true);
    assert!(dest.join("lib/toy.dll").is_file());
    assert!(dest.join("plugInfo.json").is_file());
}
