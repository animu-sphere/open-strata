// SPDX-License-Identifier: Apache-2.0
//! `runtime compose` resolves immutable artifacts without materializing them.

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
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let base = std::env::temp_dir().join(format!(
            "ost-runtime-composition-{}-{nanos}",
            std::process::id()
        ));
        let home = base.join("home");
        std::fs::create_dir_all(&home).unwrap();
        Self { base, home }
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

    fn runtime_prefix(&self) -> PathBuf {
        std::fs::read_dir(self.home.join("runtimes"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
    }

    fn promote_runtime(&self) {
        let prefix = self.runtime_prefix();
        let manifest_path = prefix.join("runtime.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest["source"] = "build".into();
        manifest["validation"] = "passed".into();
        std::fs::write(
            manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        for (relative, content) in [
            ("plugin/usd/plugInfo.json", "{}"),
            ("lib/python/pxr/__init__.py", ""),
            ("bin/usdcat", "#!/bin/sh\n"),
            (
                "include/pxr/pxr.h",
                "#define PXR_MAJOR_VERSION 0\n#define PXR_MINOR_VERSION 25\n#define PXR_PATCH_VERSION 5\n",
            ),
        ] {
            let path = prefix.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        // `runtime export` re-validates, and bin-tools-executable is a real
        // check on Unix: a fixture tool written without the bit fails it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                prefix.join("bin/usdcat"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn json(output: Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn exported_runtime_resolves_through_the_component_contract() {
    let sandbox = Sandbox::new();
    json(sandbox.ost(&["--json", "runtime", "pull", "cy2026", "--profile", "usd"]));
    sandbox.promote_runtime();
    let exported =
        json(sandbox.ost(&["--json", "runtime", "export", "cy2026", "--profile", "usd"]));
    let digest = exported["data"]["digest"].as_str().unwrap();
    let target = exported["data"]["target"].as_str().unwrap();
    let manifest = format!(
        r#"schema = "openstrata.runtime-composition/v1alpha1"

[composition]
name = "runtime-only"
target = "{target}"

[[requirements]]
capability = "usd"

[[artifacts]]
artifact = "{digest}"
"#
    );
    let pathbuf = sandbox.base.join("runtime-composition.toml");
    std::fs::write(&pathbuf, manifest).unwrap();

    let resolved = json(sandbox.ost(&["--json", "runtime", "compose", path(&pathbuf)]));
    assert_eq!(
        resolved["data"]["schema"],
        "openstrata.runtime-composition-resolved/v1alpha1"
    );
    assert_eq!(resolved["data"]["components"][0]["kind"], "runtime");
    assert_eq!(resolved["data"]["components"][0]["digest"], digest);
    assert_eq!(resolved["data"]["providers"][0]["capability"], "usd");
    assert!(resolved["data"]["composition_digest"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    assert_eq!(resolved["data"]["conflicts"], serde_json::json!([]));
}

/// Real archive/transport operations with deliberately tiny, non-OpenUSD test
/// payloads. These prove the lifecycle, not native loader or render support.
fn component_artifact(
    sandbox: &Sandbox,
    id: &str,
    requires: Vec<serde_json::Value>,
) -> (String, PathBuf) {
    let stage = sandbox.base.join(format!("stage-{id}"));
    let dist = sandbox.base.join(format!("dist-{id}"));
    std::fs::create_dir_all(stage.join("share")).unwrap();
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(
        stage.join("share/payload.txt"),
        format!("immutable payload of {id}\n"),
    )
    .unwrap();
    let stage = camino::Utf8Path::from_path(&stage).unwrap();
    let dist_utf8 = camino::Utf8Path::from_path(&dist).unwrap();
    #[cfg(unix)]
    if id == "app" {
        std::os::unix::fs::symlink("payload.txt", stage.join("share/alias.txt")).unwrap();
    }
    let packed = ost_build::pack_dir_with(
        stage,
        &dist_utf8.join("payload.tar.zst"),
        &ost_build::stage_files(stage).unwrap(),
        ost_build::PackOptions {
            executable_paths: if id == "app" {
                std::collections::BTreeSet::from(["share/payload.txt".into()])
            } else {
                Default::default()
            },
            ..Default::default()
        },
        &mut |_| {},
    )
    .unwrap();
    let mut manifest = serde_json::json!({
        "schema": 1, "name": id, "version": "1.0.0", "target": "fixture-target",
        "archive": "payload.tar.zst", "archive_digest": packed.archive_digest,
        "archive_size": packed.archive_size, "total_size": packed.total_size,
        "files": packed.files.iter().map(|f| f.manifest_json()).collect::<Vec<_>>(),
        "licenses": ["Apache-2.0"], "producer": "composition-test-fixture",
        "provenance": {"validation": "pending"},
        "build": {"source": {"repository": format!("urn:fixture:{id}"), "revision": "deadbeef"},
            "builder": {"id": "urn:fixture:builder", "identity": {"kind": "fixture"}}},
        "component": {"schema": "openstrata.component/v1alpha1", "id": id,
            "kind": "data", "version": "1.0.0",
            "provides": [{"capability": id, "version": "1.0.0"}], "requires": requires,
            "compatibility": {"targets": ["fixture-target"], "abi": "fixture-abi"},
            "install": [{"source": "share", "destination": format!("share/{id}")}],
            "environment": [{"variable": "FIXTURE_PATH", "operation": "prepend", "values": ["share"]}]
        }
    });
    ost_artifact::generate_evidence(dist_utf8, &mut manifest).unwrap();
    std::fs::write(
        dist.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    (packed.archive_digest, dist)
}

fn composition_fixture(sandbox: &Sandbox) -> PathBuf {
    let (base, base_dist) = component_artifact(sandbox, "base", vec![]);
    let (app, app_dist) = component_artifact(
        sandbox,
        "app",
        vec![serde_json::json!({"capability": "base", "version": ">=1.0"})],
    );
    let source = format!(
        r#"schema = "openstrata.runtime-composition/v1alpha1"
[composition]
name = "locked-fixture"
target = "fixture-target"
[[requirements]]
capability = "app"
[[artifacts]]
artifact = "{base}"
source = "file://{}"
[[artifacts]]
artifact = "{app}"
source = "file://{}"
"#,
        path(&base_dist).replace('\\', "/"),
        path(&app_dist).replace('\\', "/")
    );
    let manifest = sandbox.base.join("composition.toml");
    std::fs::write(&manifest, source).unwrap();
    manifest
}

fn error(output: Output, code: &str) {
    assert!(
        !output.status.success(),
        "unexpected success: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], code, "{json}");
}

#[test]
fn lock_reconstruct_export_and_clean_artifact_consumer_preserve_identity() {
    let producer = Sandbox::new();
    let manifest = composition_fixture(&producer);
    let lock_path = producer.base.join("composition.lock.json");
    let prefix = producer.base.join("prefix");
    let composed = json(producer.ost(&[
        "--json",
        "runtime",
        "compose",
        path(&manifest),
        "--lock",
        path(&lock_path),
        "--output",
        path(&prefix),
    ]));
    let identity = composed["data"]["runtime_digest"].clone();
    let lock: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    assert_eq!(lock["dependencies"][0]["consumer"], "app");
    assert_eq!(lock["dependencies"][0]["provider"], "base");
    assert_eq!(lock["resolved"]["components"][0]["id"], "base");
    assert_eq!(
        lock["inventory"].as_array().unwrap().len(),
        if cfg!(unix) { 3 } else { 2 }
    );
    assert!(lock["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["file"]["executable"] == true));
    let checked = json(producer.ost(&[
        "--json",
        "runtime",
        "compose",
        path(&manifest),
        "--lock",
        path(&lock_path),
        "--locked",
    ]));
    assert_eq!(checked["data"]["runtime_digest"], identity);
    let validation = json(producer.ost(&[
        "--json",
        "runtime",
        "validate",
        "--composition",
        path(&prefix),
    ]));
    assert_eq!(validation["data"]["checks"][3]["status"], "not-run");
    assert_eq!(
        validation["data"]["components"][0]["producer_validation"],
        "pending"
    );

    // Only the lock and immutable file:// distributions are available in this
    // different OST_HOME. Neither the original manifest nor caches are needed.
    std::fs::remove_file(&manifest).unwrap();
    let clean = Sandbox::new();
    let rebuilt = clean.base.join("rebuilt");
    let result = json(clean.ost(&[
        "--json",
        "runtime",
        "reconstruct",
        path(&lock_path),
        "--output",
        path(&rebuilt),
    ]));
    assert_eq!(result["data"]["runtime_digest"], identity);
    let dist = producer.base.join("composed-dist");
    let exported = json(producer.ost(&[
        "--json",
        "runtime",
        "export",
        "--composition",
        path(&prefix),
        "--dist",
        path(&dist),
        "--level",
        "1",
    ]));
    assert_eq!(exported["data"]["runtime_digest"], identity);
    let artifact_digest = exported["data"]["digest"].as_str().unwrap();
    assert!(dist.join("sbom.spdx.json").is_file());
    assert!(dist.join("provenance.intoto.jsonl").is_file());
    let provenance: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dist.join("provenance.intoto.jsonl")).unwrap())
            .unwrap();
    assert_eq!(
        provenance["predicate"]["buildDefinition"]["resolvedDependencies"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        provenance["predicate"]["buildDefinition"]["externalParameters"]["runtime_digest"],
        identity
    );
    let repeated = producer.base.join("repeated-dist");
    let repeated = json(producer.ost(&[
        "--json",
        "runtime",
        "export",
        "--composition",
        path(&prefix),
        "--dist",
        path(&repeated),
        "--level",
        "1",
    ]));
    assert_eq!(repeated["data"]["digest"], artifact_digest);

    let consumer = Sandbox::new();
    let imported = json(consumer.ost(&[
        "--json",
        "artifact",
        "pull",
        &format!("file://{}", path(&dist)),
        "--expect-artifact",
        artifact_digest,
    ]));
    assert_eq!(imported["ok"], true);
    let consumed = consumer.base.join("consumed");
    let pulled = json(consumer.ost(&[
        "--json",
        "runtime",
        "reconstruct",
        "--from-artifact",
        artifact_digest,
        "--output",
        path(&consumed),
    ]));
    assert_eq!(pulled["data"]["runtime_digest"], identity);
    json(consumer.ost(&[
        "--json",
        "runtime",
        "validate",
        "--composition",
        path(&consumed),
    ]));
    assert_eq!(
        std::fs::read(consumed.join("components/app/share/payload.txt")).unwrap(),
        b"immutable payload of app\n"
    );
    let objects = std::fs::read_dir(consumer.home.join("artifacts/objects/sha256"))
        .unwrap()
        .count();
    assert_eq!(objects, 1, "exported composition must be self-contained");

    // Producer labels are checked against the immutable embedded lock too.
    let producer_manifest = dist.join("manifest.json");
    let mut changed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&producer_manifest).unwrap()).unwrap();
    changed["target"] = "wrong-target".into();
    std::fs::write(&producer_manifest, serde_json::to_vec(&changed).unwrap()).unwrap();
    let mislabeled = Sandbox::new();
    json(mislabeled.ost(&["--json", "artifact", "import", path(&dist)]));
    let output = mislabeled.base.join("must-not-exist");
    error(
        mislabeled.ost(&[
            "--json",
            "runtime",
            "reconstruct",
            "--from-artifact",
            artifact_digest,
            "--output",
            path(&output),
        ]),
        "COMPOSITION_LOCK_MISMATCH",
    );
    assert!(!output.exists());
}

#[test]
fn locked_metadata_drift_and_payload_tampering_fail_without_publishing_output() {
    let sandbox = Sandbox::new();
    let manifest = composition_fixture(&sandbox);
    let lock_path = sandbox.base.join("composition.lock.json");
    let prefix = sandbox.base.join("prefix");
    json(sandbox.ost(&[
        "--json",
        "runtime",
        "compose",
        path(&manifest),
        "--lock",
        path(&lock_path),
        "--output",
        path(&prefix),
    ]));
    let original_lock = std::fs::read(&lock_path).unwrap();
    error(
        sandbox.ost(&[
            "--json",
            "runtime",
            "export",
            "--composition",
            path(&prefix),
            "--dist",
            path(&prefix.join("nested-dist")),
        ]),
        "COMPOSITION_OUTPUT_OVERLAP",
    );
    std::fs::write(prefix.join("components/app/share/payload.txt"), "tampered").unwrap();
    error(
        sandbox.ost(&[
            "--json",
            "runtime",
            "validate",
            "--composition",
            path(&prefix),
        ]),
        "COMPOSITION_INVENTORY_MISMATCH",
    );
    let dist = sandbox.base.join("must-not-exist");
    error(
        sandbox.ost(&[
            "--json",
            "runtime",
            "export",
            "--composition",
            path(&prefix),
            "--dist",
            path(&dist),
        ]),
        "COMPOSITION_INVENTORY_MISMATCH",
    );
    assert!(!dist.exists());

    let mut lock: serde_json::Value = serde_json::from_slice(&original_lock).unwrap();
    lock["runtime_digest"] = format!("sha256:{}", "ab".repeat(32)).into();
    let changed_lock = sandbox.base.join("changed.lock.json");
    std::fs::write(&changed_lock, serde_json::to_vec(&lock).unwrap()).unwrap();
    error(
        sandbox.ost(&[
            "--json",
            "runtime",
            "reconstruct",
            path(&changed_lock),
            "--output",
            path(&dist),
        ]),
        "COMPOSITION_LOCK_MISMATCH",
    );
    assert!(!dist.exists());

    // A different producer manifest with the same archive must not satisfy the
    // lock. A clean consumer sees the new metadata, not an earlier cached copy.
    let producer_manifest = sandbox.base.join("dist-app/manifest.json");
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&producer_manifest).unwrap()).unwrap();
    metadata["licenses"] = serde_json::json!(["MIT"]);
    std::fs::write(&producer_manifest, serde_json::to_vec(&metadata).unwrap()).unwrap();
    let clean = Sandbox::new();
    let output = clean.base.join("must-not-exist");
    error(
        clean.ost(&[
            "--json",
            "runtime",
            "reconstruct",
            path(&lock_path),
            "--output",
            path(&output),
        ]),
        "COMPOSITION_LOCK_MISMATCH",
    );
    assert!(!output.exists());
    assert_eq!(std::fs::read(&lock_path).unwrap(), original_lock);
}

#[test]
fn composition_refuses_mutable_sources_existing_destinations_and_changed_manifest() {
    let sandbox = Sandbox::new();
    let manifest = composition_fixture(&sandbox);
    let lock = sandbox.base.join("composition.lock.json");
    let prefix = sandbox.base.join("prefix");
    json(sandbox.ost(&[
        "--json",
        "runtime",
        "compose",
        path(&manifest),
        "--lock",
        path(&lock),
        "--output",
        path(&prefix),
    ]));
    error(
        sandbox.ost(&[
            "--json",
            "runtime",
            "reconstruct",
            path(&lock),
            "--output",
            path(&prefix),
        ]),
        "COMPOSITION_OUTPUT_EXISTS",
    );
    let original = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        original.replace("locked-fixture", "changed-fixture"),
    )
    .unwrap();
    error(
        sandbox.ost(&[
            "--json",
            "runtime",
            "compose",
            path(&manifest),
            "--lock",
            path(&lock),
            "--locked",
        ]),
        "COMPOSITION_LOCK_MISMATCH",
    );
    let mut value: toml::Value = toml::from_str(&original).unwrap();
    value["artifacts"][0]["source"] = "oci://registry.example/fixture:latest".into();
    std::fs::write(&manifest, toml::to_string(&value).unwrap()).unwrap();
    error(
        sandbox.ost(&[
            "--json",
            "runtime",
            "compose",
            path(&manifest),
            "--lock",
            path(&lock),
        ]),
        "COMPOSITION_SOURCE_MUTABLE",
    );
}

#[test]
fn acquisition_relocation_and_candidate_order_do_not_change_runtime_identity() {
    let producer = Sandbox::new();
    let manifest = composition_fixture(&producer);
    let first = producer.base.join("first.lock.json");
    let original = json(producer.ost(&[
        "--json",
        "runtime",
        "compose",
        path(&manifest),
        "--lock",
        path(&first),
    ]));
    let mut declaration: toml::Value =
        toml::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
    declaration["artifacts"].as_array_mut().unwrap().reverse();
    for id in ["app", "base"] {
        std::fs::rename(
            producer.base.join(format!("dist-{id}")),
            producer.base.join(format!("mirror-{id}")),
        )
        .unwrap();
    }
    for artifact in declaration["artifacts"].as_array_mut().unwrap() {
        artifact["source"] = artifact["source"]
            .as_str()
            .unwrap()
            .replace("dist-", "mirror-")
            .into();
    }
    std::fs::write(&manifest, toml::to_string(&declaration).unwrap()).unwrap();
    let clean = Sandbox::new();
    let second = clean.base.join("second.lock.json");
    let relocated = json(clean.ost(&[
        "--json",
        "runtime",
        "compose",
        path(&manifest),
        "--lock",
        path(&second),
    ]));
    assert_eq!(
        original["data"]["runtime_digest"],
        relocated["data"]["runtime_digest"]
    );
    json(clean.ost(&[
        "--json",
        "runtime",
        "compose",
        path(&manifest),
        "--lock",
        path(&first),
        "--locked",
    ]));
}

#[test]
fn missing_extra_files_and_forged_evidence_are_not_validation_success() {
    let sandbox = Sandbox::new();
    let manifest = composition_fixture(&sandbox);
    let prefix = sandbox.base.join("prefix");
    json(sandbox.ost(&[
        "--json",
        "runtime",
        "compose",
        path(&manifest),
        "--output",
        path(&prefix),
    ]));
    let payload = prefix.join("components/base/share/payload.txt");
    let bytes = std::fs::read(&payload).unwrap();
    std::fs::remove_file(&payload).unwrap();
    error(
        sandbox.ost(&[
            "--json",
            "runtime",
            "validate",
            "--composition",
            path(&prefix),
        ]),
        "COMPOSITION_INVENTORY_MISMATCH",
    );
    std::fs::write(&payload, bytes).unwrap();
    let extra = prefix.join("components/base/share/extra.txt");
    std::fs::write(&extra, "not locked").unwrap();
    error(
        sandbox.ost(&[
            "--json",
            "runtime",
            "validate",
            "--composition",
            path(&prefix),
        ]),
        "COMPOSITION_INVENTORY_MISMATCH",
    );
    std::fs::remove_file(&extra).unwrap();
    let evidence = prefix.join("metadata/validation.json");
    let mut report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&evidence).unwrap()).unwrap();
    report["checks"][3]["status"] = "passed".into();
    std::fs::write(&evidence, serde_json::to_vec(&report).unwrap()).unwrap();
    error(
        sandbox.ost(&[
            "--json",
            "runtime",
            "validate",
            "--composition",
            path(&prefix),
        ]),
        "COMPOSITION_EVIDENCE_MISMATCH",
    );
}

#[cfg(unix)]
#[test]
fn metadata_symlinks_and_execute_bit_drift_are_rejected() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let sandbox = Sandbox::new();
    let manifest = composition_fixture(&sandbox);
    let prefix = sandbox.base.join("prefix");
    json(sandbox.ost(&[
        "--json",
        "runtime",
        "compose",
        path(&manifest),
        "--output",
        path(&prefix),
    ]));
    let payload = prefix.join("components/app/share/payload.txt");
    std::fs::set_permissions(&payload, std::fs::Permissions::from_mode(0o644)).unwrap();
    error(
        sandbox.ost(&[
            "--json",
            "runtime",
            "validate",
            "--composition",
            path(&prefix),
        ]),
        "COMPOSITION_INVENTORY_MISMATCH",
    );
    std::fs::set_permissions(&payload, std::fs::Permissions::from_mode(0o755)).unwrap();
    let lock = prefix.join("metadata/composition.lock.json");
    std::fs::rename(&lock, prefix.join("metadata/retained.json")).unwrap();
    symlink("retained.json", &lock).unwrap();
    error(
        sandbox.ost(&[
            "--json",
            "runtime",
            "validate",
            "--composition",
            path(&prefix),
        ]),
        "COMPOSITION_INVENTORY_INVALID",
    );
}
