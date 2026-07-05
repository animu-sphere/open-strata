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

    // The starter matrix is structurally valid, but its untouched placeholder
    // digests are called out as envelope warnings (never a silent pass).
    let v = stdout_json(&sb.ost(&["--json", "ci", "validate"]));
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["cells"], 1);
    assert_eq!(v["data"]["placeholders"].as_array().unwrap().len(), 2);
    let warnings = v["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 2);
    assert_eq!(warnings[0]["code"], "CI_PLACEHOLDER_DIGEST");

    // …and those placeholders do not resolve in an empty registry.
    let out = sb.ost(&["--json", "ci", "validate", "--resolve"]);
    assert_eq!(out.status.code(), Some(5));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["data"]["unresolved"].as_array().unwrap().len(), 2);

    // generate refuses a placeholder matrix unless explicitly overridden.
    let out = sb.ost(&["--json", "ci", "generate", "github", "--stdout"]);
    assert_eq!(out.status.code(), Some(5));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["code"], "CI_PLACEHOLDER_DIGESTS");

    // generate --stdout prints the workflow itself (parseable YAML).
    let out = sb.ost(&[
        "ci",
        "generate",
        "github",
        "--stdout",
        "--allow-placeholders",
    ]);
    assert!(out.status.success());
    let doc: serde_yaml::Value = serde_yaml::from_slice(&out.stdout).unwrap();
    let include = &doc["jobs"]["scheduled"]["strategy"]["matrix"]["include"];
    assert_eq!(include.as_sequence().unwrap().len(), 1);
    assert_eq!(include[0]["name"], "example-linux-cy2026-usd");
    // A column-0 `steps:` would still parse (as a stray top-level key), so
    // assert the steps block actually sits under the job.
    assert!(!doc["jobs"]["scheduled"]["steps"]
        .as_sequence()
        .unwrap()
        .is_empty());
    assert!(doc.get("steps").is_none(), "no stray top-level steps key");

    // generate writes the default path; overwrite needs --force.
    let v = stdout_json(&sb.ost(&["--json", "ci", "generate", "github", "--allow-placeholders"]));
    let workflow = PathBuf::from(v["data"]["workflow"].as_str().unwrap());
    assert!(sb.base.join(&workflow).is_file());
    assert_eq!(
        sb.ost(&["--json", "ci", "generate", "github", "--allow-placeholders"])
            .status
            .code(),
        Some(2)
    );
    stdout_json(&sb.ost(&[
        "--json",
        "ci",
        "generate",
        "github",
        "--force",
        "--allow-placeholders",
    ]));

    // A matrix that fails structural validation is a configuration error.
    std::fs::write(
        sb.base.join("bad.yaml"),
        "schema: 1\ncells:\n  - name: Bad_Name\n    runtime_artifact: sha256:xyz\n    plugin_artifact: sha256:xyz\n    platform: cy2026\n    profile: usd\n",
    )
    .unwrap();
    let out = sb.ost(&["--json", "ci", "validate", "--matrix", "bad.yaml"]);
    assert_eq!(out.status.code(), Some(3));
}

/// Runner profiles + lanes: a hosted PR cell renders into a source-CI
/// workflow, billing acknowledgement is warned about while missing and
/// gates publish-capable cells, and `generate` writes one file per lane
/// family.
#[test]
fn runner_profiles_and_lanes_render_source_and_support_workflows() {
    let sb = Sandbox::new("lanes");
    let lanes_yaml = format!(
        "\
schema: 1
runners:
  windows-hosted:
    kind: github-hosted
    image: windows-2022
  usd-linux-real:
    kind: self-hosted
    labels: [self-hosted, linux, x64, usd-26]
cells:
  - name: plugin-pr-windows
    lane: pull_request
    runner: windows-hosted
    runtime_artifact: sha256:{a}
    bundle: plugins/toy
    platform: cy2026
    profile: usd
    up_to: 4
  - name: linux-usd-support
    runner: usd-linux-real
    runtime_artifact: sha256:{a}
    plugin_artifact: sha256:{b}
    platform: cy2026
    profile: usd
",
        a = "ab".repeat(32),
        b = "cd".repeat(32)
    );
    std::fs::write(sb.base.join("openstrata.ci.yaml"), &lanes_yaml).unwrap();

    // validate passes, but warns about the unacknowledged hosted runner.
    let v = stdout_json(&sb.ost(&["--json", "ci", "validate"]));
    assert_eq!(v["ok"], true);
    assert_eq!(
        v["data"]["hosted_unacknowledged"],
        serde_json::json!(["windows-hosted"])
    );
    let warnings = v["warnings"].as_array().unwrap();
    assert!(warnings
        .iter()
        .any(|w| w["code"] == "CI_HOSTED_BILLING_UNACKNOWLEDGED"));

    // A publish-capable cell on that runner turns the warning into an error.
    let publishing = lanes_yaml.replace(
        "    lane: pull_request\n",
        "    lane: main\n    publish: candidate\n",
    );
    std::fs::write(sb.base.join("publishing.yaml"), &publishing).unwrap();
    let out = sb.ost(&["--json", "ci", "validate", "--matrix", "publishing.yaml"]);
    assert_eq!(out.status.code(), Some(5));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["data"]["billing_errors"].as_array().unwrap().len(), 1);

    // Acknowledging billing clears both.
    let acked = publishing.replace(
        "    image: windows-2022\n",
        "    image: windows-2022\n    billing:\n      acknowledgement: required\n",
    );
    std::fs::write(sb.base.join("acked.yaml"), &acked).unwrap();
    let v = stdout_json(&sb.ost(&["--json", "ci", "validate", "--matrix", "acked.yaml"]));
    assert_eq!(v["ok"], true);
    assert!(v["warnings"].as_array().unwrap().is_empty());

    // generate writes one workflow per lane family.
    let v = stdout_json(&sb.ost(&["--json", "ci", "generate", "github"]));
    let workflows = v["data"]["workflows"].as_array().unwrap();
    assert_eq!(workflows.len(), 2);
    let support = sb.base.join(".github/workflows/ost-support-matrix.yml");
    let source = sb.base.join(".github/workflows/ost-source-ci.yml");
    assert!(support.is_file());
    assert!(source.is_file());

    // The source workflow builds from source on the hosted image, gated to
    // pull_request, with no publish step and a read-only token.
    let doc: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&source).unwrap()).unwrap();
    assert!(doc["on"]
        .as_mapping()
        .unwrap()
        .contains_key(serde_yaml::Value::from("pull_request")));
    let entries = doc["jobs"]["pr"]["strategy"]["matrix"]["include"]
        .as_sequence()
        .unwrap();
    assert_eq!(entries[0]["runs_on"][0], "windows-2022");
    assert_eq!(entries[0]["hosted"], true);
    assert!(!doc["jobs"]["pr"]["steps"].as_sequence().unwrap().is_empty());
    assert_eq!(doc["permissions"]["contents"], "read");
    let text = std::fs::read_to_string(&source).unwrap();
    assert!(!text.contains("plugin publish"));

    // --out cannot target a two-workflow matrix.
    assert_eq!(
        sb.ost(&["--json", "ci", "generate", "github", "--out", "one.yml", "--force"])
            .status
            .code(),
        Some(2)
    );

    // --stdout emits a two-document YAML stream.
    let out = sb.ost(&["ci", "generate", "github", "--stdout"]);
    assert!(out.status.success());
    let stream = String::from_utf8(out.stdout).unwrap();
    let docs: Vec<&str> = stream.split("\n---\n").collect();
    assert_eq!(docs.len(), 2);
    for doc in docs {
        let _: serde_yaml::Value = serde_yaml::from_str(doc).expect("each document parses");
    }
}

/// `ost ci plan` reports execution facts (lanes, runner classes, billing)
/// without rendering workflows or estimating money.
#[test]
fn plan_reports_execution_facts() {
    let sb = Sandbox::new("plan");
    let lanes_yaml = format!(
        "\
schema: 1
runners:
  windows-hosted:
    kind: github-hosted
    image: windows-2022
  usd-linux-real:
    kind: self-hosted
    labels: [self-hosted, linux, x64, usd-26]
cells:
  - name: plugin-pr-windows
    lane: pull_request
    runner: windows-hosted
    runtime_artifact: sha256:{a}
    bundle: plugins/toy
    platform: cy2026
    profile: usd
    up_to: 4
  - name: linux-usd-support
    runner: usd-linux-real
    runtime_artifact: sha256:{a}
    plugin_artifact: sha256:{b}
    platform: cy2026
    profile: usd
",
        a = "ab".repeat(32),
        b = "cd".repeat(32)
    );
    std::fs::write(sb.base.join("lanes.yaml"), &lanes_yaml).unwrap();

    let v = stdout_json(&sb.ost(&["--json", "ci", "plan", "--matrix", "lanes.yaml"]));
    let d = &v["data"];
    assert_eq!(d["cells"], 2);
    assert_eq!(d["lanes"]["pull_request"], 1);
    assert_eq!(d["lanes"]["scheduled"], 1);
    assert_eq!(d["hosted_jobs"], 1);
    assert_eq!(
        d["metered_runner_classes"],
        serde_json::json!(["windows-hosted"])
    );
    assert_eq!(
        d["operator_managed_runner_classes"],
        serde_json::json!(["usd-linux-real"])
    );
    assert_eq!(d["requires_billing_acknowledgement"], true);
    assert_eq!(d["publish_capable_jobs"], 0);
    assert_eq!(d["workflows"].as_array().unwrap().len(), 2);

    // Acknowledged billing flips the requirement off.
    let acked = lanes_yaml.replace(
        "    image: windows-2022\n",
        "    image: windows-2022\n    billing:\n      acknowledgement: required\n",
    );
    std::fs::write(sb.base.join("acked.yaml"), &acked).unwrap();
    let v = stdout_json(&sb.ost(&["--json", "ci", "plan", "--matrix", "acked.yaml"]));
    assert_eq!(v["data"]["requires_billing_acknowledgement"], false);

    // A label-only support matrix plans one workflow and no hosted jobs.
    let v = stdout_json(&sb.ost(&["--json", "ci", "init"]));
    assert_eq!(v["data"]["created"], true);
    let v = stdout_json(&sb.ost(&["--json", "ci", "plan", "--matrix", "openstrata.ci.yaml"]));
    assert_eq!(v["data"]["hosted_jobs"], 0);
    assert_eq!(v["data"]["workflows"].as_array().unwrap().len(), 1);
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

    // The resolver also enforces the typed fields, not just digest existence.
    std::fs::write(
        sb.base.join("openstrata.ci.yaml"),
        format!(
            "schema: 1\ncells:\n  - name: swapped-kinds\n    runtime_artifact: {plugin_digest}\n    plugin_artifact: {runtime_digest}\n    platform: cy2026\n    profile: usd\n"
        ),
    )
    .unwrap();
    let out = sb.ost(&["--json", "ci", "validate", "--resolve"]);
    assert_eq!(out.status.code(), Some(5));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let unresolved = v["data"]["unresolved"].as_array().unwrap();
    assert_eq!(unresolved.len(), 2);
    assert!(unresolved
        .iter()
        .any(|v| v.as_str().unwrap().contains("expected runtime")));
    assert!(unresolved
        .iter()
        .any(|v| v.as_str().unwrap().contains("expected plugin")));

    std::fs::write(
        sb.base.join("openstrata.ci.yaml"),
        format!(
            "schema: 1\ncells:\n  - name: linux-usd-toy\n    runtime_artifact: {runtime_digest}\n    plugin_artifact: {plugin_digest}\n    platform: cy2026\n    profile: usd\n    up_to: 4\n    host:\n      os: linux\n      labels: [self-hosted, linux]\n"
        ),
    )
    .unwrap();

    // The generated workflow pins the same digests into the include entry.
    let out = sb.ost(&["ci", "generate", "github", "--stdout"]);
    let doc: serde_yaml::Value = serde_yaml::from_slice(&out.stdout).unwrap();
    let entry = &doc["jobs"]["scheduled"]["strategy"]["matrix"]["include"][0];
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

    // A reused work dir must not silently merge stale files with pinned bytes.
    let out = sb.ost(&[
        "--json",
        "artifact",
        "extract",
        &plugin_digest,
        path_str(&dest),
    ]);
    assert_eq!(out.status.code(), Some(2));
}
