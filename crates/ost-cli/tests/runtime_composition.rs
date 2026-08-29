// SPDX-License-Identifier: Apache-2.0
//! Locked composition, materialization and artifact distribution lifecycle.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_SANDBOX: AtomicU64 = AtomicU64::new(0);

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
        Self::with_timestamp(nanos)
    }

    fn with_timestamp(nanos: u128) -> Self {
        // Wall clocks can return the same timestamp on concurrent test threads
        // (notably on Intel macOS). Never let one sandbox's Drop delete another.
        let serial = NEXT_SANDBOX.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "ost-runtime-composition-{}-{nanos}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir(&base).expect("claim a unique test sandbox");
        let home = base.join("home");
        std::fs::create_dir(&home).unwrap();
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
            .env_remove("PYTHONDONTWRITEBYTECODE")
            .env_remove("PYTHONPYCACHEPREFIX")
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

    fn promote_runtime_with_python_dir(&self, python_dir: &str) {
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
            (format!("{python_dir}/pxr/__init__.py").as_str(), ""),
            // Keep the declared loader directory in the archive even when
            // Python lives under Lib/ on a case-sensitive filesystem.
            ("lib/runtime-fixture.txt", "mock runtime loader directory\n"),
            (
                "lib/usd/usd/resources/codegenTemplates/plugInfo.json",
                r#"{"Plugins":[{"Name":"{{ libraryName }}","Root":"@PLUG_INFO_ROOT@","LibraryPath":"@PLUG_INFO_LIBRARY_PATH@"}]}"#,
            ),
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

#[test]
fn sandboxes_with_identical_timestamps_have_independent_lifetimes() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sandboxes = std::thread::scope(|scope| {
        (0..32)
            .map(|_| scope.spawn(|| Sandbox::with_timestamp(nanos)))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>()
    });
    let paths = sandboxes
        .iter()
        .map(|sandbox| sandbox.base.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(paths.len(), sandboxes.len());
    let mut remaining = sandboxes.into_iter();
    let survivor = remaining.next().unwrap();
    drop(remaining);
    assert!(survivor.home.is_dir());
    assert_eq!(paths.iter().filter(|path| path.exists()).count(), 1);
}

#[track_caller]
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

fn exported_runtime_composition(sandbox: &Sandbox, python_dir: &str) -> (PathBuf, String) {
    json(sandbox.ost(&["--json", "runtime", "pull", "cy2026", "--profile", "usd"]));
    sandbox.promote_runtime_with_python_dir(python_dir);
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
    (pathbuf, digest.to_string())
}

#[test]
fn exported_runtime_resolves_through_the_component_contract() {
    let sandbox = Sandbox::new();
    let (pathbuf, digest) = exported_runtime_composition(&sandbox, "lib/python");

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

#[test]
fn exported_runtime_python_layouts_support_repeated_sdk_execution() {
    let python = ost_core::tools::which("python").or_else(|| ost_core::tools::which("python3"));
    let shell = ost_core::tools::which(if cfg!(windows) { "pwsh" } else { "bash" });
    if std::env::var_os("OST_TEST_REQUIRE_SDK_TOOLS").is_some() {
        assert!(python.is_some(), "SDK CI requires a Python interpreter");
        assert!(shell.is_some(), "SDK CI requires PowerShell/Bash");
    }
    for python_dir in [
        "lib/python",
        "lib/site-packages",
        "Lib/site-packages",
        "lib/python3.13/site-packages",
    ] {
        let sandbox = Sandbox::new();
        let (manifest, _) = exported_runtime_composition(&sandbox, python_dir);
        let prefix = sandbox.base.join("sdk");
        json(sandbox.ost(&[
            "--json",
            "runtime",
            "compose",
            path(&manifest),
            "--output",
            path(&prefix),
        ]));
        let environment =
            json(sandbox.ost(&["--json", "runtime", "env", "--composition", path(&prefix)]));
        let pairs: Vec<(String, String)> =
            serde_json::from_value(environment["data"]["env"].clone()).unwrap();
        let vars = pairs
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(vars["PYTHONDONTWRITEBYTECODE"], "1");
        assert_eq!(vars["PYTHONNOUSERSITE"], "1");
        assert!(
            vars["PYTHONPATH"]
                .to_ascii_lowercase()
                .contains(&python_dir.to_ascii_lowercase()),
            "{vars:?}"
        );

        if let Some(python) = &python {
            for _ in 0..2 {
                let executed = json(sandbox.ost(&[
                    "--json",
                    "runtime",
                    "exec",
                    "--composition",
                    path(&prefix),
                    "--",
                    path(python),
                    "-c",
                    "import pxr; print(pxr.__file__)",
                ]));
                let loaded = executed["data"]["stdout"].as_str().unwrap().trim();
                assert!(Path::new(loaded).starts_with(&prefix), "{loaded}");
            }
            if let Some(shell) = &shell {
                let rendered = sandbox.ost(&[
                    "runtime",
                    "env",
                    "--composition",
                    path(&prefix),
                    "--shell",
                    if cfg!(windows) { "pwsh" } else { "bash" },
                ]);
                assert!(rendered.status.success());
                let mut script = String::from_utf8(rendered.stdout).unwrap();
                script.push_str(if cfg!(windows) {
                    "& $env:SDK_TEST_PYTHON -c 'import pxr; print(pxr.__file__)'\nexit $LASTEXITCODE\n"
                } else {
                    "\"$SDK_TEST_PYTHON\" -c 'import pxr; print(pxr.__file__)'\n"
                });
                let script_path = sandbox.base.join(if cfg!(windows) {
                    "activate.ps1"
                } else {
                    "activate.sh"
                });
                std::fs::write(&script_path, script).unwrap();
                let mut command = Command::new(shell);
                if cfg!(windows) {
                    command.args(["-NoProfile", "-File"]);
                }
                let output = command
                    .arg(script_path)
                    .env("SDK_TEST_PYTHON", python)
                    .env_remove("PYTHONHOME")
                    .env_remove("PYTHONPYCACHEPREFIX")
                    .env_remove("PYTHONDONTWRITEBYTECODE")
                    .output()
                    .unwrap();
                assert!(output.status.success(), "{output:?}");
                let stdout = String::from_utf8(output.stdout).unwrap();
                assert!(Path::new(stdout.trim()).starts_with(&prefix), "{stdout}");
            } else {
                eprintln!("SKIP: shell execution requires PowerShell/Bash");
            }
        } else {
            eprintln!("SKIP: Python execution requires a host interpreter");
        }
        // Both execution modes must leave the full inventory immutable.
        json(sandbox.ost(&[
            "--json",
            "runtime",
            "validate",
            "--composition",
            path(&prefix),
            "--sdk",
        ]));
        json(sandbox.ost(&[
            "--json",
            "runtime",
            "export",
            "--composition",
            path(&prefix),
        ]));
        if python_dir == "lib/python3.13/site-packages" {
            let mut lock: ost_formation::RuntimeCompositionLock = serde_json::from_slice(
                &std::fs::read(prefix.join("metadata/composition.lock.json")).unwrap(),
            )
            .unwrap();
            let mut other_abi = lock
                .inventory
                .iter()
                .find(|entry| entry.file.path.ends_with("pxr/__init__.py"))
                .unwrap()
                .clone();
            other_abi.file.path = other_abi.file.path.replace("python3.13", "python3.12");
            lock.inventory.push(other_abi);
            let sdk = ost_formation::RuntimeSdkLayout::derive(&lock).unwrap();
            assert!(sdk.environment.values().all(|env| env
                .iter()
                .filter(|c| c.key == "PYTHONPATH")
                .flat_map(|c| &c.paths)
                .all(|p| !p.contains("python3.12"))));

            // An explicit declaration wins even if a conventional layout exists.
            lock.resolved
                .environment
                .push(ost_formation::ResolvedEnvironmentContribution {
                    variable: "PYTHONPATH".into(),
                    operation: "prepend".into(),
                    source: lock.resolved.components[0].id.clone(),
                    values: vec!["custom-python".into()],
                });
            let sdk = ost_formation::RuntimeSdkLayout::derive(&lock).unwrap();
            assert!(sdk.environment.values().all(|env| {
                let paths = env
                    .iter()
                    .filter(|c| c.key == "PYTHONPATH")
                    .flat_map(|c| &c.paths)
                    .collect::<Vec<_>>();
                paths.iter().any(|p| p.ends_with("custom-python"))
                    && !paths.iter().any(|p| p.contains("site-packages"))
            }));
        }
    }
}

#[test]
fn codeless_schema_sdk_omits_empty_optional_search_paths() {
    let sandbox = Sandbox::new();
    let (manifest, _) = exported_runtime_composition(&sandbox, "lib/python");
    let plugin = sandbox.base.join("codeless");
    json(sandbox.ost(&[
        "--json",
        "plugin",
        "new",
        "usd-schema",
        "codeless",
        "--dir",
        path(&plugin),
    ]));
    let packaged = json(sandbox.ost(&[
        "--json",
        "plugin",
        "package",
        path(&plugin),
        "--target",
        "cy2026",
        "--profile",
        "usd",
    ]));
    let archive = Path::new(packaged["data"]["archive"].as_str().unwrap());
    let dist = archive.parent().unwrap();
    json(sandbox.ost(&["--json", "artifact", "import", path(dist)]));
    let mut source = std::fs::read_to_string(&manifest).unwrap();
    source.push_str(&format!(
        "\n[[requirements]]\ncapability = 'component:codeless'\n[[artifacts]]\nartifact = '{}'\n",
        packaged["data"]["archive_digest"].as_str().unwrap(),
    ));
    std::fs::write(&manifest, source).unwrap();
    let prefix = sandbox.base.join("sdk");
    json(sandbox.ost(&[
        "--json",
        "runtime",
        "compose",
        path(&manifest),
        "--output",
        path(&prefix),
    ]));
    assert!(!prefix.join("components/codeless/lib").exists());
    assert!(!prefix.join("components/codeless/python").exists());
    let report = json(sandbox.ost(&[
        "--json",
        "runtime",
        "validate",
        "--composition",
        path(&prefix),
        "--sdk",
    ]));
    assert!(report["data"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| { check["name"] == "schema-resource" && check["status"] == "passed" }));

    let lock: ost_formation::RuntimeCompositionLock = serde_json::from_slice(
        &std::fs::read(prefix.join("metadata/composition.lock.json")).unwrap(),
    )
    .unwrap();
    // Conventional defaults alone are optional: missing declared plugin roots,
    // nonstandard library paths, and set operations remain validation candidates.
    for (key, operation, value) in [
        ("PXR_PLUGINPATH_NAME", "prepend", "missing-resources"),
        ("LD_LIBRARY_PATH", "prepend", "missing-libraries"),
        ("CUSTOM_PATH", "set", "python"),
    ] {
        let mut declared = lock.clone();
        declared
            .resolved
            .environment
            .push(ost_formation::ResolvedEnvironmentContribution {
                variable: key.into(),
                operation: operation.into(),
                source: "codeless".into(),
                values: vec![value.into()],
            });
        let sdk = ost_formation::RuntimeSdkLayout::derive(&declared).unwrap();
        assert!(sdk.environment.values().all(|env| env
            .iter()
            .any(|c| { c.key == key && c.paths == [format!("components/codeless/{value}")] })));
    }
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
    assert_eq!(lock["sdk"]["schema"], "openstrata.runtime-sdk/v1alpha1");
    for root in ost_formation::SDK_ROOTS {
        assert!(prefix.join(root).is_dir(), "missing SDK root {root}");
    }
    assert_eq!(
        std::fs::read(prefix.join("share/app/payload.txt")).unwrap(),
        b"immutable payload of app\n"
    );
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
    let consumer_manifest_path = producer.base.join("python-consumer.json");
    let consumer_manifest = json(producer.ost(&[
        "--json",
        "runtime",
        "consumer-manifest",
        "--from-artifact",
        artifact_digest,
        "--kind",
        "python-wheel",
        "--name",
        "fixture-python",
        "--version",
        "1.0.0",
        "--entrypoint",
        "fixture.api",
        "--output",
        path(&consumer_manifest_path),
    ]));
    assert_eq!(
        consumer_manifest["data"]["manifest"]["runtime"]["artifact_digest"],
        artifact_digest
    );
    assert_eq!(
        consumer_manifest["data"]["manifest"]["runtime"]["runtime_digest"],
        identity
    );
    assert!(
        consumer_manifest["data"]["manifest"]["runtime"]["sbom_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(
        consumer_manifest["data"]["manifest"]["runtime"]["provenance_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(
        consumer_manifest["data"]["manifest"]["private_loader"]["scope"],
        "package-private"
    );
    assert_eq!(
        consumer_manifest["data"]["manifest"]["runtime"]["components"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            &std::fs::read(&consumer_manifest_path).unwrap()
        )
        .unwrap(),
        consumer_manifest["data"]["manifest"]
    );
    let verified_consumer = json(producer.ost(&[
        "--json",
        "runtime",
        "consumer-verify",
        "--manifest",
        path(&consumer_manifest_path),
    ]));
    assert_eq!(verified_consumer["data"]["verified"], true);
    assert_eq!(
        verified_consumer["data"]["runtime"],
        consumer_manifest["data"]["manifest"]["runtime"]
    );
    let mut mismatched = consumer_manifest["data"]["manifest"].clone();
    mismatched["runtime"]["runtime_digest"] = format!("sha256:{}", "f0".repeat(32)).into();
    let mismatched_path = producer.base.join("mismatched-consumer.json");
    std::fs::write(
        &mismatched_path,
        serde_json::to_vec_pretty(&mismatched).unwrap(),
    )
    .unwrap();
    error(
        producer.ost(&[
            "--json",
            "runtime",
            "consumer-verify",
            "--manifest",
            path(&mismatched_path),
        ]),
        "CONSUMER_PACKAGE_RUNTIME_MISMATCH",
    );
    let missing_entrypoint = producer.base.join("missing-native-consumer.json");
    error(
        producer.ost(&[
            "--json",
            "runtime",
            "consumer-manifest",
            "--from-artifact",
            artifact_digest,
            "--kind",
            "native-sdk",
            "--name",
            "missing-sdk",
            "--version",
            "1.0.0",
            "--entrypoint",
            "Missing",
            "--output",
            path(&missing_entrypoint),
        ]),
        "CONSUMER_PACKAGE_ENTRYPOINT_MISSING",
    );
    assert!(!missing_entrypoint.exists());
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
    for root in ost_formation::SDK_ROOTS {
        assert!(
            consumed.join(root).is_dir(),
            "empty root must survive export: {root}"
        );
    }
    assert_eq!(
        std::fs::read(consumed.join("metadata/sdk.json")).unwrap(),
        std::fs::read(prefix.join("metadata/sdk.json")).unwrap()
    );
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
    let consumer_output = mislabeled.base.join("consumer-must-not-exist.json");
    error(
        mislabeled.ost(&[
            "--json",
            "runtime",
            "consumer-manifest",
            "--from-artifact",
            artifact_digest,
            "--kind",
            "native-sdk",
            "--name",
            "tiny-sdk",
            "--version",
            "1.0.0",
            "--entrypoint",
            "Tiny",
            "--output",
            path(&consumer_output),
        ]),
        "COMPOSITION_LOCK_MISMATCH",
    );
    assert!(!consumer_output.exists());
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
fn new_locks_check_source_metadata_even_when_the_archive_is_cached() {
    let sandbox = Sandbox::new();
    let manifest = composition_fixture(&sandbox);
    let original = std::fs::read_to_string(&manifest).unwrap();
    let lock = sandbox.base.join("original.lock.json");
    json(sandbox.ost(&[
        "--json",
        "runtime",
        "compose",
        path(&manifest),
        "--lock",
        path(&lock),
    ]));
    let original_lock = std::fs::read(&lock).unwrap();
    let dist = sandbox.base.join("dist-app");

    for change in [
        "producer",
        "sbom",
        "provenance",
        "missing-sbom",
        "missing-provenance",
    ] {
        let mirror = sandbox.base.join(format!("mirror-{change}"));
        std::fs::create_dir(&mirror).unwrap();
        for entry in std::fs::read_dir(&dist).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), mirror.join(entry.file_name())).unwrap();
        }
        match change {
            "producer" => {
                let file = mirror.join("manifest.json");
                let mut value: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
                value["created_unix"] = 123.into();
                std::fs::write(file, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            "sbom" | "provenance" => {
                let name = if change == "sbom" {
                    ost_artifact::SBOM_FILE
                } else {
                    ost_artifact::PROVENANCE_FILE
                };
                let file = mirror.join(name);
                let mut bytes = std::fs::read(&file).unwrap();
                bytes.push(b'\n'); // Same claims, different evidence digest.
                std::fs::write(file, bytes).unwrap();
            }
            _ => {
                let name = if change == "missing-sbom" {
                    ost_artifact::SBOM_FILE
                } else {
                    ost_artifact::PROVENANCE_FILE
                };
                std::fs::remove_file(mirror.join(name)).unwrap();
            }
        }
        // The source is already different when the new manifest is declared;
        // it does not change during or after lock generation.
        std::fs::write(
            &manifest,
            original.replace("dist-app", &format!("mirror-{change}")),
        )
        .unwrap();
        let output = sandbox.base.join("must-not-exist");
        error(
            sandbox.ost(&[
                "--json",
                "runtime",
                "compose",
                path(&manifest),
                "--lock",
                path(&lock),
                "--output",
                path(&output),
            ]),
            "COMPOSITION_SOURCE_MISMATCH",
        );
        assert!(!output.exists());
        assert_eq!(std::fs::read(&lock).unwrap(), original_lock);
    }

    // Matching source metadata still permits a new lock with a warm cache.
    std::fs::write(&manifest, &original).unwrap();
    json(sandbox.ost(&[
        "--json",
        "runtime",
        "compose",
        path(&manifest),
        "--lock",
        path(&lock),
    ]));
    assert_eq!(std::fs::read(&lock).unwrap(), original_lock);
    // Locked operations must not require source access or replace cached pins.
    std::fs::rename(&dist, sandbox.base.join("offline-app")).unwrap();
    std::fs::rename(
        sandbox.base.join("dist-base"),
        sandbox.base.join("offline-base"),
    )
    .unwrap();
    json(sandbox.ost(&[
        "--json",
        "runtime",
        "compose",
        path(&manifest),
        "--lock",
        path(&lock),
        "--locked",
    ]));
    json(sandbox.ost(&[
        "--json",
        "runtime",
        "reconstruct",
        path(&lock),
        "--output",
        path(&sandbox.base.join("offline-prefix")),
    ]));
}

#[test]
fn overlapping_lock_and_output_paths_fail_before_any_output_is_created() {
    let sandbox = Sandbox::new();
    let manifest = composition_fixture(&sandbox);
    let mut cases = vec![
        ("prefix/runtime.lock.json", "prefix"),
        ("prefix/components/app/share/payload.txt", "prefix"),
        ("prefix", "prefix"),
        ("prefix", "prefix/nested"),
        ("other/../prefix/runtime.lock.json", "prefix"),
        ("prefix/runtime.lock.json", "other/../prefix"),
    ];
    #[cfg(windows)]
    cases.extend([
        ("PREFIX/runtime.lock.json", "prefix"),
        ("prefix./runtime.lock.json", "prefix"),
        ("prefix /runtime.lock.json", "prefix"),
    ]);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&sandbox.base, sandbox.base.join("alias")).unwrap();
        cases.push(("alias/prefix/runtime.lock.json", "prefix"));
    }
    for (lock, output) in cases {
        error(
            sandbox.ost(&[
                "--json",
                "runtime",
                "compose",
                path(&manifest),
                "--lock",
                lock,
                "--output",
                output,
            ]),
            "COMPOSITION_OUTPUT_OVERLAP",
        );
        assert!(!sandbox.base.join("prefix").exists());
        assert!(!sandbox.base.join("other").exists());
        assert!(!sandbox.home.join("artifacts").exists());
    }
}

#[test]
fn exported_dependency_evidence_must_match_the_embedded_lock() {
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
    let dist = sandbox.base.join("export");
    let exported = json(sandbox.ost(&[
        "--json",
        "runtime",
        "export",
        "--composition",
        path(&prefix),
        "--dist",
        path(&dist),
    ]));
    let digest = exported["data"]["digest"].as_str().unwrap();
    let producer_path = dist.join("manifest.json");
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&producer_path).unwrap()).unwrap();
    for change in [
        "empty",
        "missing",
        "extra",
        "digest",
        "version",
        "source",
        "reordered",
    ] {
        let mut producer = original.clone();
        let dependencies = producer["build"]["dependencies"].as_array_mut().unwrap();
        match change {
            "empty" => dependencies.clear(),
            "missing" => {
                dependencies.pop();
            }
            "extra" => {
                let mut extra = dependencies[0].clone();
                extra["name"] = "unselected-component".into();
                dependencies.push(extra);
            }
            "digest" => {
                dependencies[0]["archive_digest"] = format!("sha256:{}", "ab".repeat(32)).into()
            }
            "version" => dependencies[0]["version"] = "9.9.9".into(),
            "source" => dependencies[0]["source"]["revision"] = "wrong-revision".into(),
            "reordered" => dependencies.reverse(),
            _ => unreachable!(),
        }
        // Keep outer manifest, SBOM and provenance internally consistent while
        // leaving the archive (and its embedded component lock) untouched.
        ost_artifact::generate_evidence(camino::Utf8Path::from_path(&dist).unwrap(), &mut producer)
            .unwrap();
        std::fs::write(&producer_path, serde_json::to_vec(&producer).unwrap()).unwrap();
        let clean = Sandbox::new();
        json(clean.ost(&[
            "--json",
            "artifact",
            "pull",
            &format!("file://{}", path(&dist).replace('\\', "/")),
            "--expect-artifact",
            digest,
            "--require-sbom",
            "--require-provenance",
        ]));
        let output = clean.base.join("consumer");
        let reconstructed = clean.ost(&[
            "--json",
            "runtime",
            "reconstruct",
            "--from-artifact",
            digest,
            "--output",
            path(&output),
        ]);
        if change == "reordered" {
            json(reconstructed);
        } else {
            error(reconstructed, "COMPOSITION_LOCK_MISMATCH");
            assert!(!output.exists());
        }
    }
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

fn sdk_fixture(sandbox: &Sandbox) -> (PathBuf, ost_formation::RuntimeCompositionLock) {
    let manifest = composition_fixture(sandbox);
    let prefix = sandbox.base.join("sdk $literal ' prefix");
    json(sandbox.ost(&[
        "--json",
        "runtime",
        "compose",
        path(&manifest),
        "--output",
        path(&prefix),
    ]));
    let lock = serde_json::from_slice(
        &std::fs::read(prefix.join("metadata/composition.lock.json")).unwrap(),
    )
    .unwrap();
    (prefix, lock)
}

#[test]
fn sdk_environment_ownership_and_projection_are_verified() {
    let sandbox = Sandbox::new();
    let (prefix, lock) = sdk_fixture(&sandbox);
    let environment =
        json(sandbox.ost(&["--json", "runtime", "env", "--composition", path(&prefix)]));
    assert_eq!(environment["data"]["runtime_digest"], lock.runtime_digest);
    let vars: Vec<(String, String)> =
        serde_json::from_value(environment["data"]["env"].clone()).unwrap();
    let cmake = vars
        .iter()
        .find(|(key, _)| key == "CMAKE_PREFIX_PATH")
        .unwrap();
    assert_eq!(cmake.1, path(&prefix).replace('\\', "/"));
    assert!(!vars
        .iter()
        .any(|(_, value)| value.contains(".ost-composition-")));
    let shell = sandbox.ost(&[
        "runtime",
        "env",
        "--composition",
        path(&prefix),
        "--shell",
        "pwsh",
    ]);
    assert!(shell.status.success());
    assert!(String::from_utf8_lossy(&shell.stdout).contains("`$literal"));
    let report = json(sandbox.ost(&[
        "--json",
        "runtime",
        "validate",
        "--composition",
        path(&prefix),
        "--sdk",
    ]));
    assert!(report["data"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .all(|check| check["status"] != "failed"));

    error(
        sandbox.ost(&[
            "--json",
            "runtime",
            "exec",
            "--composition",
            path(&prefix),
            "--",
            "unknown",
        ]),
        "COMPOSITION_HOST_MISMATCH",
    );
    let sdk_path = prefix.join("metadata/sdk.json");
    let original = std::fs::read(&sdk_path).unwrap();
    std::fs::write(&sdk_path, "{}").unwrap();
    error(
        sandbox.ost(&["--json", "runtime", "env", "--composition", path(&prefix)]),
        "COMPOSITION_EVIDENCE_MISMATCH",
    );
    std::fs::write(sdk_path, original).unwrap();
    std::fs::write(prefix.join("share/app/payload.txt"), "changed").unwrap();
    error(
        sandbox.ost(&["--json", "runtime", "env", "--composition", path(&prefix)]),
        "COMPOSITION_INVENTORY_MISMATCH",
    );
    assert_eq!(
        std::fs::read(prefix.join("components/app/share/payload.txt")).unwrap(),
        b"immutable payload of app\n",
        "SDK copies must not alias component bytes"
    );
}

#[test]
fn sdk_rejects_expanded_collisions_reserved_roots_and_missing_sources() {
    let sandbox = Sandbox::new();
    let (_, lock) = sdk_fixture(&sandbox);
    let mut case_alias = lock.clone();
    case_alias
        .resolved
        .install
        .iter_mut()
        .find(|m| m.component == "base")
        .unwrap()
        .destination = "SHARE/base".into();
    let sdk = ost_formation::RuntimeSdkLayout::derive(&case_alias).unwrap();
    assert!(sdk
        .files
        .iter()
        .any(|e| e.file.path == "share/base/payload.txt"));
    for (source, destination, expected) in [
        ("share", "share/app", "COMPOSITION_INSTALL_PATH_COLLISION"),
        (
            "share",
            "share/app/payload.txt/child",
            "COMPOSITION_INSTALL_PATH_COLLISION",
        ),
        ("share", "metadata/owned", "COMPOSITION_SDK_PATH_INVALID"),
        ("share", "components/owned", "COMPOSITION_SDK_PATH_INVALID"),
        (
            "share",
            "share/unsafe:stream",
            "COMPOSITION_SDK_PATH_INVALID",
        ),
        ("absent", "share/new", "COMPOSITION_INSTALL_SOURCE_MISSING"),
    ] {
        let mut invalid = lock.clone();
        let mapping = invalid
            .resolved
            .install
            .iter_mut()
            .find(|m| m.component == "base")
            .unwrap();
        mapping.source = source.into();
        mapping.destination = destination.into();
        let error = ost_formation::RuntimeSdkLayout::derive(&invalid).unwrap_err();
        assert!(format!("{error:?}").contains(expected), "{error:?}");
    }
    let mut tampered = lock;
    let mut conflicting = tampered.clone();
    for source in ["app", "base"] {
        conflicting
            .resolved
            .environment
            .push(ost_formation::ResolvedEnvironmentContribution {
                variable: "SET_PATH".into(),
                operation: "set".into(),
                source: source.into(),
                values: vec!["share".into()],
            });
    }
    assert!(format!(
        "{:?}",
        ost_formation::RuntimeSdkLayout::derive(&conflicting).unwrap_err()
    )
    .contains("COMPOSITION_ENVIRONMENT_CONFLICT"));
    let mut reserved = tampered.clone();
    reserved
        .resolved
        .environment
        .push(ost_formation::ResolvedEnvironmentContribution {
            variable: "PYTHONDONTWRITEBYTECODE".into(),
            operation: "set".into(),
            source: "app".into(),
            values: vec!["share".into()],
        });
    assert!(format!(
        "{:?}",
        ost_formation::RuntimeSdkLayout::derive(&reserved).unwrap_err()
    )
    .contains("COMPOSITION_ENVIRONMENT_CONFLICT"));
    tampered.sdk.as_mut().unwrap().files[0].component = "forged-owner".into();
    assert!(tampered.validate().is_err());
}

#[test]
fn legacy_component_only_locks_reconstruct_without_identity_migration() {
    let sandbox = Sandbox::new();
    let (_, sdk_lock) = sdk_fixture(&sandbox);
    let legacy = ost_formation::RuntimeCompositionLock::new(
        sdk_lock.manifest,
        sdk_lock.artifacts,
        sdk_lock.inventory,
    )
    .unwrap();
    let value = serde_json::to_value(&legacy).unwrap();
    assert!(value.get("sdk").is_none());
    let lock_path = sandbox.base.join("legacy.json");
    std::fs::write(&lock_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let clean = Sandbox::new();
    let prefix = clean.base.join("legacy");
    let result = json(clean.ost(&[
        "--json",
        "runtime",
        "reconstruct",
        path(&lock_path),
        "--output",
        path(&prefix),
    ]));
    assert_eq!(result["data"]["runtime_digest"], legacy.runtime_digest);
    assert!(!prefix.join("metadata/sdk.json").exists());
    json(clean.ost(&[
        "--json",
        "runtime",
        "validate",
        "--composition",
        path(&prefix),
    ]));
    error(
        clean.ost(&["--json", "runtime", "env", "--composition", path(&prefix)]),
        "COMPOSITION_SDK_REQUIRED",
    );
    let dist = clean.base.join("legacy-dist");
    let exported = json(clean.ost(&[
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
    let consumer_output = clean.base.join("legacy-consumer-must-not-exist.json");
    error(
        clean.ost(&[
            "--json",
            "runtime",
            "consumer-manifest",
            "--from-artifact",
            exported["data"]["digest"].as_str().unwrap(),
            "--kind",
            "native-sdk",
            "--name",
            "legacy-sdk",
            "--version",
            "1.0.0",
            "--entrypoint",
            "Tiny",
            "--output",
            path(&consumer_output),
        ]),
        "COMPOSITION_SDK_REQUIRED",
    );
    assert!(!consumer_output.exists());
}

#[test]
fn sdk_activation_preserves_ordered_prepend_append_and_set_operations() {
    let sandbox = Sandbox::new();
    let (_, mut lock) = sdk_fixture(&sandbox);
    for (operation, value) in [
        ("set", "share/base"),
        ("append", "share/app"),
        ("prepend", "share/first"),
    ] {
        lock.resolved
            .environment
            .push(ost_formation::ResolvedEnvironmentContribution {
                variable: "SDK_TEST".into(),
                operation: operation.into(),
                source: "app".into(),
                values: vec![value.into()],
            });
    }
    let sdk = ost_formation::RuntimeSdkLayout::derive(&lock).unwrap();
    for os in [
        ost_core::host::Os::Linux,
        ost_core::host::Os::Windows,
        ost_core::host::Os::Macos,
    ] {
        let env = sdk.activate(camino::Utf8Path::new("/sdk"), os).unwrap();
        let pairs = env.pairs();
        let (_, value) = pairs.iter().find(|(key, _)| key == "SDK_TEST").unwrap();
        assert_eq!(
            value.split(env.sep).collect::<Vec<_>>(),
            vec![
                "/sdk/components/app/share/first",
                "/sdk/components/app/share/base",
                "/sdk/components/app/share/app"
            ]
        );
    }
}

#[test]
fn sdk_rewrites_relative_symlinks_and_refuses_uninstalled_targets() {
    let sandbox = Sandbox::new();
    let (_, mut lock) = sdk_fixture(&sandbox);
    lock.inventory
        .retain(|e| e.file.path != "components/app/share/alias.txt");
    let mut entry = lock
        .inventory
        .iter()
        .find(|e| e.component == "app")
        .unwrap()
        .clone();
    entry.file.path = "components/app/share/alias.txt".into();
    entry.file.link_target = Some("payload.txt".into());
    entry.file.sha256 = ost_core::digest::sha256_hex(b"payload.txt");
    entry.file.size = 11;
    entry.file.executable = false;
    lock.inventory.push(entry);
    let mut mapping = lock
        .resolved
        .install
        .iter()
        .find(|m| m.component == "app")
        .unwrap()
        .clone();
    lock.resolved.install.retain(|m| m.component != "app");
    mapping.source = "share/payload.txt".into();
    mapping.destination = "lib/app.txt".into();
    lock.resolved.install.push(mapping.clone());
    mapping.source = "share/alias.txt".into();
    mapping.destination = "share/app/alias.txt".into();
    lock.resolved.install.push(mapping);
    let sdk = ost_formation::RuntimeSdkLayout::derive(&lock).unwrap();
    let link = sdk
        .files
        .iter()
        .find(|f| f.file.link_target.is_some())
        .unwrap();
    assert_eq!(link.file.link_target.as_deref(), Some("../../lib/app.txt"));
    assert_eq!(
        link.file.sha256,
        ost_core::digest::sha256_hex(b"../../lib/app.txt")
    );
    lock.resolved
        .install
        .retain(|m| m.destination != "lib/app.txt");
    assert!(format!(
        "{:?}",
        ost_formation::RuntimeSdkLayout::derive(&lock).unwrap_err()
    )
    .contains("COMPOSITION_SDK_LINK_INVALID"));
}

fn checked(command: &mut Command) {
    let output = command.output().expect("spawn native SDK tool");
    assert!(
        output.status.success(),
        "{command:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn relocated_sdk_builds_and_runs_a_clean_native_cmake_consumer() {
    let require_tools = std::env::var_os("OST_TEST_REQUIRE_SDK_TOOLS").is_some();
    let Some(cmake) = ost_core::tools::which("cmake") else {
        assert!(!require_tools, "SDK CI requires CMake");
        eprintln!("SKIP native SDK: CMake unavailable");
        return;
    };
    let Some(ninja) = ost_core::tools::which("ninja") else {
        assert!(!require_tools, "SDK CI requires Ninja");
        eprintln!("SKIP native SDK: Ninja unavailable");
        return;
    };
    let mut build_env = std::collections::BTreeMap::new();
    if cfg!(windows) {
        let Some(msvc) = ost_build::msvc::bootstrap().expect("MSVC bootstrap") else {
            assert!(!require_tools, "SDK CI requires MSVC");
            eprintln!("SKIP native SDK: MSVC unavailable");
            return;
        };
        build_env.extend(msvc.vars);
    } else if !["c++", "clang++", "g++"]
        .iter()
        .any(|p| ost_core::tools::which(p).is_some())
    {
        assert!(!require_tools, "SDK CI requires a C++ compiler");
        eprintln!("SKIP native SDK: C++ compiler unavailable");
        return;
    }
    let sandbox = Sandbox::new();
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/runtime-composition/native-sdk");
    let build = sandbox.base.join("producer-build");
    let stage = sandbox.base.join("producer-install");
    let configure = |src: &Path, build: &Path| {
        let mut command = Command::new(&cmake);
        command
            .args([
                "-S",
                path(src),
                "-B",
                path(build),
                "-G",
                "Ninja",
                "-DCMAKE_BUILD_TYPE=Release",
            ])
            .arg(format!("-DCMAKE_MAKE_PROGRAM={}", ninja.display()))
            .envs(&build_env);
        command
    };
    checked(configure(&source, &build).arg(format!("-DCMAKE_INSTALL_PREFIX={}", stage.display())));
    checked(
        Command::new(&cmake)
            .args(["--build", path(&build), "--target", "install"])
            .envs(&build_env),
    );
    let dist = sandbox.base.join("native-dist");
    std::fs::create_dir_all(&dist).unwrap();
    let stage_utf8 = camino::Utf8Path::from_path(&stage).unwrap();
    let dist_utf8 = camino::Utf8Path::from_path(&dist).unwrap();
    let packed = ost_build::pack_dir_with(
        stage_utf8,
        &dist_utf8.join("native.tar.zst"),
        &ost_build::stage_files(stage_utf8).unwrap(),
        ost_build::PackOptions {
            level: 1,
            ..Default::default()
        },
        &mut |_| {},
    )
    .unwrap();
    let target = ost_core::Host::detect().slug();
    let mut producer = serde_json::json!({
        "schema": 1, "name": "tiny", "version": "1.0.0", "target": target,
        "archive": "native.tar.zst", "archive_digest": packed.archive_digest, "archive_size": packed.archive_size, "total_size": packed.total_size,
        "files": packed.files.iter().map(|f| f.manifest_json()).collect::<Vec<_>>(), "licenses": ["Apache-2.0"],
        "component": {"schema": "openstrata.component/v1alpha1", "id": "tiny", "kind": "library", "version": "1.0.0",
            "provides": [{"capability": "tiny", "version": "1.0.0"}],
            "install": packed.files.iter().map(|f| serde_json::json!({"source": f.path, "destination": f.path})).collect::<Vec<_>>(),
            "environment": [{"variable": "PXR_PLUGINPATH_NAME", "operation": "prepend", "values": ["plugins/Tiny"]}]
        }
    });
    ost_artifact::generate_evidence(dist_utf8, &mut producer).unwrap();
    std::fs::write(
        dist.join("manifest.json"),
        serde_json::to_vec(&producer).unwrap(),
    )
    .unwrap();
    let manifest = sandbox.base.join("native.toml");
    std::fs::write(&manifest, format!("schema = 'openstrata.runtime-composition/v1alpha1'\n[composition]\nname = 'native-sdk'\ntarget = '{target}'\n[[requirements]]\ncapability = 'tiny'\n[[artifacts]]\nartifact = '{}'\nsource = 'file://{}'\n", packed.archive_digest, dist.display())).unwrap();
    let prefix = sandbox.base.join("sdk-original");
    json(sandbox.ost(&[
        "--json",
        "runtime",
        "compose",
        path(&manifest),
        "--output",
        path(&prefix),
    ]));
    let export = sandbox.base.join("sdk-dist");
    let exported = json(sandbox.ost(&[
        "--json",
        "runtime",
        "export",
        "--composition",
        path(&prefix),
        "--dist",
        path(&export),
        "--level",
        "1",
    ]));
    let consumer_manifest = sandbox.base.join("tiny-consumer.json");
    let derived = json(sandbox.ost(&[
        "--json",
        "runtime",
        "consumer-manifest",
        "--from-artifact",
        exported["data"]["digest"].as_str().unwrap(),
        "--kind",
        "native-sdk",
        "--name",
        "tiny-sdk",
        "--version",
        "1.0.0",
        "--entrypoint",
        "Tiny",
        "--output",
        path(&consumer_manifest),
    ]));
    assert_eq!(
        derived["data"]["manifest"]["public_api"]["entrypoints"],
        serde_json::json!(["Tiny"])
    );
    assert!(consumer_manifest.is_file());
    // Delete only this test's own producer output; no source/build prefix is
    // available to hide a non-relocatable package or missing dependency.
    std::fs::remove_dir_all(&stage).unwrap();
    std::fs::remove_dir_all(&build).unwrap();
    std::fs::remove_dir_all(&prefix).unwrap();
    let consumer = Sandbox::new();
    json(consumer.ost(&[
        "--json",
        "artifact",
        "pull",
        &format!("file://{}", export.display()),
    ]));
    let installed = consumer.base.join("installed");
    json(consumer.ost(&[
        "--json",
        "runtime",
        "reconstruct",
        "--from-artifact",
        exported["data"]["digest"].as_str().unwrap(),
        "--output",
        path(&installed),
    ]));
    let relocated = consumer.base.join("relocated SDK ' prefix");
    std::fs::rename(installed, &relocated).unwrap();
    let report = json(consumer.ost(&[
        "--json",
        "runtime",
        "validate",
        "--composition",
        path(&relocated),
        "--sdk",
        "--cmake-package",
        "Tiny",
    ]));
    assert!(report["data"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["name"] == "cmake-package" && c["status"] == "passed"));
    let run = json(consumer.ost(&[
        "--json",
        "runtime",
        "exec",
        "--composition",
        path(&relocated),
        "--",
        "sdk-inspect",
    ]));
    assert_eq!(run["data"]["stdout"].as_str().unwrap().trim(), "tiny=42");
    let build = consumer.base.join("consumer-build");
    let mut command = configure(&source.join("consumer"), &build);
    for key in [
        "CMAKE_PREFIX_PATH",
        "CMAKE_MODULE_PATH",
        "Tiny_DIR",
        "Tiny_ROOT",
    ] {
        command.env_remove(key);
    }
    command
        .arg(format!("-DCMAKE_PREFIX_PATH={}", relocated.display()))
        .args([
            "-DCMAKE_FIND_USE_PACKAGE_REGISTRY=FALSE",
            "-DCMAKE_FIND_USE_SYSTEM_PACKAGE_REGISTRY=FALSE",
        ]);
    checked(&mut command);
    checked(
        Command::new(&cmake)
            .args(["--build", path(&build)])
            .envs(&build_env),
    );
    let executable = build.join(if cfg!(windows) {
        "consumer.exe"
    } else {
        "consumer"
    });
    let run = json(consumer.ost(&[
        "--json",
        "runtime",
        "exec",
        "--composition",
        path(&relocated),
        "--",
        path(&executable),
    ]));
    assert_eq!(run["data"]["stdout"].as_str().unwrap().trim(), "tiny=42");
    let missing = consumer.ost(&[
        "--json",
        "runtime",
        "validate",
        "--composition",
        path(&relocated),
        "--sdk",
        "--cmake-package",
        "MissingTiny",
    ]);
    assert!(!missing.status.success());
    error(
        consumer.ost(&[
            "--json",
            "runtime",
            "exec",
            "--composition",
            path(&relocated),
            "--",
            "cmake",
        ]),
        "COMPOSITION_EXECUTABLE_UNREACHABLE",
    );
    eprintln!("Native SDK: shared library, relocated export, find_package, consumer build and loader execution passed");
}
