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

/// A two-lane matrix with the v0.9.0 hosted source-CI contract: a hosted PR
/// cell carrying a `runtime_remote` pin plus the matrix-level `bootstrap`
/// block, and a self-hosted support cell (air-gapped local import).
fn lanes_yaml() -> String {
    format!(
        "\
schema: 1
bootstrap:
  ost:
    version: \"0.9.0\"
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
    runtime_remote:
      uri: oci://ghcr.io/owner/openstrata-runtime@sha256:{o}
      expected_oci_digest: sha256:{o}
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
        b = "cd".repeat(32),
        o = "ee".repeat(32),
    )
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

#[test]
fn public_support_claims_gate_hosted_generation() {
    let sb = Sandbox::new("public-support");
    let base = format!(
        "schema: 1
cells:
  - name: hosted-linux
    runtime_artifact: sha256:{}
    plugin_artifact: sha256:{}
    platform: cy2026
    profile: usd
",
        "ab".repeat(32),
        "cd".repeat(32)
    );
    std::fs::write(sb.base.join("openstrata.ci.yaml"), &base).unwrap();
    let supported = "[levels.stable]\n\
description='covered'\n\
[levels.unsupported]\n\
description='not covered'\n\
[[platforms]]\n\
id='linux_x86_64'\n\
label='Linux x86_64'\n\
[[features]]\n\
id='github_hosted_ci'\n\
label='GitHub-hosted CI'\n\
support={linux_x86_64='stable'}\n\
[[features]]\n\
id='plugin_test'\n\
label='Plugin test'\n\
support={linux_x86_64='stable'}\n";
    std::fs::write(sb.base.join("platforms.toml"), supported).unwrap();

    // Enabling the gate makes a missing hosted mapping an explicit validation
    // failure rather than inferring architecture from a runner image.
    let out = sb.ost(&["--json", "ci", "validate", "--support", "platforms.toml"]);
    assert_eq!(out.status.code(), Some(5));
    let envelope: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(envelope["data"]["support_issues"][0]
        .as_str()
        .unwrap()
        .contains("omits support.platform"));

    let claimed = base.replace(
        "    profile: usd\n",
        "    profile: usd\n    support:\n      platform: linux_x86_64\n      features: [plugin_test]\n",
    );
    std::fs::write(sb.base.join("openstrata.ci.yaml"), claimed).unwrap();
    stdout_json(&sb.ost(&["--json", "ci", "validate", "--support", "platforms.toml"]));
    assert!(sb
        .ost(&[
            "ci",
            "generate",
            "github",
            "--stdout",
            "--support",
            "platforms.toml",
        ])
        .status
        .success());

    let unsupported = supported.replace(
        "support={linux_x86_64='stable'}\n",
        "support={linux_x86_64='unsupported'}\n",
    );
    std::fs::write(sb.base.join("platforms.toml"), unsupported).unwrap();
    let out = sb.ost(&[
        "--json",
        "ci",
        "generate",
        "github",
        "--stdout",
        "--support",
        "platforms.toml",
    ]);
    assert_eq!(out.status.code(), Some(5));
    let envelope: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(envelope["error"]["code"], "CI_SUPPORT_CLAIM_UNSUPPORTED");
}

/// Runner profiles + lanes: a hosted PR cell renders into a source-CI
/// workflow, billing acknowledgement is warned about while missing and
/// gates publish-capable cells, and `generate` writes one file per lane
/// family.
#[test]
fn runner_profiles_and_lanes_render_source_and_support_workflows() {
    let sb = Sandbox::new("lanes");
    let lanes_yaml = lanes_yaml();
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
    let publishing = lanes_yaml
        .replace(
            "schema: 1\n",
            "schema: 1\ntrust:\n  policy: openstrata-artifact-policy.toml\n  release_min_trust: verified\nrelease:\n  version: 1.2.3\n  mode: draft\n",
        )
        .replace(
            "    version: \"0.9.0\"\n",
            &format!(
                "    version: \"0.9.0\"\n    sha256:\n      x86_64-pc-windows-msvc: {}\n",
                "ef".repeat(32)
            ),
        )
        .replace(
            "    lane: pull_request\n",
            "    lane: main\n    publish: candidate\n    trust: verified\n",
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
    assert!(entries[0]["runtime_remote"]
        .as_str()
        .unwrap()
        .starts_with("oci://ghcr.io/owner/openstrata-runtime@sha256:"));
    let steps = doc["jobs"]["pr"]["steps"].as_sequence().unwrap();
    let names: Vec<&str> = steps.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert!(names.iter().any(|n| n.starts_with("Bootstrap ost 0.9.0")));
    assert!(names.iter().any(|n| n.contains("registry cache")));
    assert!(names.iter().any(|n| n.contains("remote reference")));
    for step_name in [
        "Build the plugin from source",
        "Run the verification pyramid",
    ] {
        let run = steps
            .iter()
            .find(|step| step["name"] == step_name)
            .and_then(|step| step["run"].as_str())
            .expect("source-CI step has a command");
        assert!(run.contains("${{ matrix.bundle }}"), "{run}");
        assert!(
            !run.contains("--with"),
            "manifest closure is not duplicated: {run}"
        );
    }
    assert!(
        entries[0].get("with").is_none(),
        "source cells have no dependency list"
    );
    assert_eq!(doc["permissions"]["contents"], "read");
    let text = std::fs::read_to_string(&source).unwrap();
    assert!(!text.contains("plugin publish"));
    assert!(!text.contains("secrets."));

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
    let lanes_yaml = lanes_yaml();
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
    assert_eq!(d["trust"]["pr_min_trust"], "local");
    assert_eq!(d["trust"]["main_min_trust"], "local");
    assert_eq!(d["trust"]["release_min_trust"], "local");
    assert_eq!(d["trust"]["cells"][0]["effective_minimum"], "local");
    assert_eq!(d["workflows"].as_array().unwrap().len(), 2);
    // v0.9.0 remote-transport facts: the bootstrap pin and which cells pull
    // remotely vs stay air-gapped.
    assert_eq!(d["bootstrap"]["ost_version"], "0.9.0");
    assert_eq!(d["bootstrap"]["repository"], "animu-sphere/open-strata");
    assert_eq!(
        d["remote_runtime_cells"],
        serde_json::json!(["plugin-pr-windows"])
    );
    assert_eq!(d["air_gapped_source_cells"], serde_json::json!([]));

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
    assert_eq!(
        v["data"]["operator_managed_runner_classes"],
        serde_json::json!(["self-hosted, linux, x64"])
    );
    assert_eq!(v["data"]["workflows"].as_array().unwrap().len(), 1);
}

#[test]
fn trust_aware_matrix_plans_and_generates_evidence_gates() {
    let sb = Sandbox::new("trusted-ci");
    let matrix = lanes_yaml()
        .replace(
            "schema: 1\n",
            "schema: 1\ntrust:\n  policy: policies/artifacts.toml\n  pr_min_trust: attested\n  main_min_trust: verified\n  release_min_trust: trusted\n",
        )
        .replace(
            "    lane: pull_request\n",
            "    lane: pull_request\n    trust: unsigned\n",
        );
    std::fs::write(sb.base.join("trusted.yaml"), &matrix).unwrap();

    let plan = stdout_json(&sb.ost(&["--json", "ci", "plan", "--matrix", "trusted.yaml"]));
    assert_eq!(plan["data"]["trust"]["policy"], "policies/artifacts.toml");
    assert_eq!(plan["data"]["trust"]["cells"][0]["target"], "unsigned");
    assert_eq!(
        plan["data"]["trust"]["cells"][0]["effective_minimum"],
        "attested"
    );

    let generated = stdout_json(&sb.ost(&[
        "--json",
        "ci",
        "generate",
        "github",
        "--matrix",
        "trusted.yaml",
    ]));
    assert_eq!(generated["ok"], true);
    let source =
        std::fs::read_to_string(sb.base.join(".github/workflows/ost-source-ci.yml")).unwrap();
    for required in [
        "minimum_trust: attested",
        "--minimum-trust ${{ matrix.minimum_trust }}",
        "--require-sbom",
        "--require-provenance",
        "--policy policies/artifacts.toml",
    ] {
        assert!(source.contains(required), "missing {required}:\n{source}");
    }
    assert!(!source.contains("plugin publish"));
    assert!(!source.contains("artifact push"));
}

#[test]
fn typed_release_contract_plans_and_generates_isolated_publisher() {
    let sb = Sandbox::new("release-lane");
    let matrix = lanes_yaml()
        .replace(
            "schema: 1\n",
            "schema: 1\ntrust:\n  policy: openstrata-artifact-policy.toml\n  main_min_trust: verified\n  release_min_trust: trusted\nrelease:\n  version: 1.2.3\n  mode: publish\n  destination: oci://ghcr.io/owner/plugin\n  publisher_runner: windows-hosted\n  environment: release\n  reproducible: true\n  from_package: true\n  checks:\n    - name: Release corpus smoke\n      run: ctest --test-dir build/corpus --output-on-failure\n",
        )
        .replace(
            "    version: \"0.9.0\"\n",
            &format!(
                "    version: \"0.9.0\"\n    sha256:\n      x86_64-pc-windows-msvc: {}\n",
                "ef".repeat(32)
            ),
        )
        .replace(
            "    image: windows-2022\n",
            "    image: windows-2022\n    billing:\n      acknowledgement: required\n",
        )
        .replace(
            "    lane: pull_request\n",
            "    lane: main\n    publish: candidate\n    trust: trusted\n",
        );
    std::fs::write(sb.base.join("openstrata.ci.yaml"), matrix).unwrap();

    let plan = stdout_json(&sb.ost(&["--json", "ci", "plan"]));
    assert_eq!(plan["data"]["workflows"].as_array().unwrap().len(), 3);
    assert_eq!(plan["data"]["release"]["version"], "1.2.3");
    assert_eq!(plan["data"]["release"]["mode"], "publish");
    assert_eq!(
        plan["data"]["release"]["candidate_cells"],
        serde_json::json!(["plugin-pr-windows"])
    );

    let generated = stdout_json(&sb.ost(&["--json", "ci", "generate", "github"]));
    assert_eq!(generated["data"]["workflows"].as_array().unwrap().len(), 3);
    let release = sb.base.join(".github/workflows/ost-release.yml");
    let text = std::fs::read_to_string(&release).unwrap();
    let doc: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
    assert_eq!(doc["permissions"]["contents"], "read");
    assert_eq!(doc["jobs"]["publish"]["permissions"]["id-token"], "write");
    assert_eq!(doc["jobs"]["publish"]["permissions"]["packages"], "write");
    let candidates = serde_yaml::to_string(&doc["jobs"]["candidates"]).unwrap();
    assert!(candidates.contains("Repackage and prove reproducibility"));
    assert!(candidates.contains("--from-package"));
    assert!(candidates.contains("Release corpus smoke"));
    assert!(!candidates.contains("artifact push"));
    let publisher = serde_yaml::to_string(&doc["jobs"]["publish"]).unwrap();
    assert!(publisher.contains("artifact push"));
    assert!(publisher.contains("secrets.GITHUB_TOKEN"));
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
        ("bin/usdcat", "#!/bin/sh\n"),
    ] {
        let path = prefix.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            prefix.join("bin/usdcat"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
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
            "schema: 1\nrequire_evidence: none\ncells:\n  - name: linux-usd-toy\n    runtime_artifact: {runtime_digest}\n    plugin_artifact: {plugin_digest}\n    platform: cy2026\n    profile: usd\n    up_to: 4\n    host:\n      os: linux\n      labels: [self-hosted, linux]\n"
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
            "schema: 1\nrequire_evidence: none\ncells:\n  - name: swapped-kinds\n    runtime_artifact: {plugin_digest}\n    plugin_artifact: {runtime_digest}\n    platform: cy2026\n    profile: usd\n"
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
            "schema: 1\nrequire_evidence: none\ncells:\n  - name: linux-usd-toy\n    runtime_artifact: {runtime_digest}\n    plugin_artifact: {plugin_digest}\n    platform: cy2026\n    profile: usd\n    up_to: 4\n    host:\n      os: linux\n      labels: [self-hosted, linux]\n"
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

/// The v0.17.0 adoption failure, caught locally instead of on nine red runners:
/// a matrix whose pinned artifact predates evidence renders a gate the artifact
/// cannot pass. `generate` warns, `validate` fails, and both name a way out.
#[test]
fn evidence_gate_gap_warns_on_generate_and_fails_validate() {
    let sb = Sandbox::new("evidence-gate");

    // A plugin artifact with no evidence sidecars — the pre-0.17 shape.
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

    let write_matrix = |require: &str| {
        std::fs::write(
            sb.base.join("openstrata.ci.yaml"),
            format!(
                "schema: 1\n{require}cells:\n  - name: linux-usd-toy\n    runtime_artifact: {plugin_digest}\n    plugin_artifact: {plugin_digest}\n    platform: cy2026\n    profile: usd\n    host:\n      os: linux\n      labels: [self-hosted, linux]\n"
            ),
        )
        .unwrap();
    };

    // Default `all`: validate refuses, and says which sidecar is missing.
    write_matrix("");
    let out = sb.ost(&["--json", "ci", "validate"]);
    assert_eq!(
        out.status.code(),
        Some(5),
        "an unsatisfiable gate must fail"
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], false);
    let gaps = v["data"]["evidence_gaps"].as_array().unwrap();
    assert_eq!(gaps.len(), 2, "runtime and plugin pins both lack evidence");
    assert!(gaps
        .iter()
        .all(|g| g.as_str().unwrap().contains("require_evidence is 'all'")));

    // Generate warns rather than failing: the generating machine's registry is
    // not necessarily the one the lane will run against.
    let out = sb.ost(&["--json", "ci", "generate", "github"]);
    assert!(out.status.success(), "generate must not fail on a gap");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true);
    let warnings = v["warnings"].as_array().unwrap();
    assert!(!warnings.is_empty(), "generate must warn about the gap");
    assert!(warnings
        .iter()
        .all(|w| w["code"] == "CI_EVIDENCE_GATE_UNSATISFIABLE"));

    // With `--stdout` the workflow *is* stdout, so the warning must reach
    // stderr instead of vanishing into whatever the caller redirected to.
    let out = sb.ost(&["ci", "generate", "github", "--stdout"]);
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("the rendered gate cannot pass"),
        "a redirected stdout must not hide the warning: {stderr}"
    );

    // The documented way out: declare a level the pins can meet. The gate is
    // still rendered, it just demands what exists today.
    write_matrix("require_evidence: none\n");
    let v = stdout_json(&sb.ost(&["--json", "ci", "validate"]));
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["evidence_gaps"].as_array().unwrap().len(), 0);

    let out = sb.ost(&["ci", "generate", "github", "--stdout"]);
    let doc: serde_yaml::Value = serde_yaml::from_slice(&out.stdout).unwrap();
    let entry = &doc["jobs"]["scheduled"]["strategy"]["matrix"]["include"][0];
    assert_eq!(entry["require_evidence"], "none");
    assert_eq!(entry["evidence_flags"], "");

    // A partial level is honoured too: demanding only an SBOM still reports
    // the missing SBOM, and nothing about provenance.
    write_matrix("require_evidence: sbom\n");
    let out = sb.ost(&["--json", "ci", "validate"]);
    assert_eq!(out.status.code(), Some(5));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let gaps = v["data"]["evidence_gaps"].as_array().unwrap();
    assert!(gaps
        .iter()
        .all(|g| g.as_str().unwrap().contains("has no SBOM ")));
}
