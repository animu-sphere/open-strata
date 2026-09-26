// SPDX-License-Identifier: Apache-2.0
//! Formation CLI lifecycle over a real digest-pinned runtime artifact.

use std::io::Write;
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
        let base =
            std::env::temp_dir().join(format!("ost-formation-{}-{nanos}", std::process::id()));
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
        let mut entries = std::fs::read_dir(self.home.join("runtimes"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        entries.remove(0)
    }

    fn promote_runtime(&self) {
        let prefix = self.runtime_prefix();
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
    let source = String::from_utf8_lossy(&output.stdout);
    let start = source.find('{').unwrap_or(0);
    serde_json::from_str(&source[start..]).unwrap_or_else(|e| panic!("{e}: {source}"))
}

fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn executable_fixture(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[test]
fn formation_resolves_real_renderer_and_plugin_package_outputs() {
    if ost_core::tools::which("cmake").is_none() || ost_core::tools::which("ninja").is_none() {
        assert!(
            std::env::var_os("OST_TEST_REQUIRE_SDK_TOOLS").is_none(),
            "SDK tools required"
        );
        return;
    }
    let sb = Sandbox::new();
    json(sb.ost(&["--json", "init", "--name", "toon", "--platform", "cy2026"]));
    json(sb.ost(&["--json", "runtime", "pull", "cy2026", "--profile", "usd"]));
    sb.promote_runtime();
    // The fixture models an imaging SDK structurally; actual DLL loading is
    // covered by the native release smoke test, not by these placeholder bytes.
    let prefix = sb.runtime_prefix();
    let header = prefix.join("include/pxr/usdImaging/usdImaging/adapterRegistry.h");
    std::fs::create_dir_all(header.parent().unwrap()).unwrap();
    std::fs::write(header, "// imaging SDK fixture\n").unwrap();
    std::fs::write(
        prefix.join(format!(
            "lib/libusd_usdImaging{}",
            std::env::consts::DLL_SUFFIX
        )),
        "fixture",
    )
    .unwrap();
    let manifest_path = prefix.join("runtime.json");
    let mut runtime_manifest =
        ost_runtime::RuntimeManifest::from_json(&std::fs::read_to_string(&manifest_path).unwrap())
            .unwrap();
    runtime_manifest
        .extensions
        .iter_mut()
        .find(|e| e.id == "openusd")
        .unwrap()
        .version = "26.08".into();
    runtime_manifest.digest = runtime_manifest.compute_digest();
    std::fs::write(manifest_path, runtime_manifest.to_json().unwrap()).unwrap();
    std::fs::write(
        prefix.join("include/pxr/pxr.h"),
        "#define PXR_MAJOR_VERSION 0\n#define PXR_MINOR_VERSION 26\n#define PXR_PATCH_VERSION 8\n",
    )
    .unwrap();
    let exported = json(sb.ost(&["--json", "runtime", "export", "cy2026", "--profile", "usd"]));
    let runtime = exported["data"]["digest"].as_str().unwrap();
    // A tiny install fixture tests the packaging contract without needing a
    // full OpenUSD SDK or claiming to exercise a renderer implementation.
    std::fs::write(sb.base.join("CMakeLists.txt"), "cmake_minimum_required(VERSION 3.23)\nproject(toon NONE)\nif(NOT ENABLE_HYDRA)\nmessage(FATAL_ERROR \"intent was lost\")\nendif()\ninstall(FILES plugInfo.json DESTINATION lib/usd/hdToon/resources)\n").unwrap();
    std::fs::write(sb.base.join("plugInfo.json"), "{\"Plugins\": []}").unwrap();
    std::fs::OpenOptions::new().append(true).open(sb.base.join("openstrata.toml")).unwrap().write_all(b"\n[build.intents.hydra]\ncache = { ENABLE_HYDRA = { type = 'BOOL', value = true } }\n").unwrap();
    std::fs::write(sb.base.join("openstrata.renderer.yaml"), "schema: openstrata.renderer/v1alpha1\nrenderer: { name: toon }\ncomposition:\n  backend: vulkan\n  scene_inputs: [headless]\n  units: { core: toon-core }\nrender_products: { required: [color] }\nframe: { contexts: 1, completion: explicit }\nvalidation:\n  gpu_smoke: false\n  validation_messages_are_errors: true\n  assertions: [renderer.install_tree]\n").unwrap();
    let build = sb.ost(&["build", "--intent", "hydra", "--progress", "plain"]);
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stdout)
    );
    let packaged = json(sb.ost(&["--json", "package", "--intent", "hydra"]));
    let archive = PathBuf::from(packaged["data"]["archive"].as_str().unwrap());
    let dist = archive.parent().unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dist.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["provenance"]["build_intent"]["name"], "hydra");
    assert!(manifest["target"].as_str().unwrap().ends_with("-usd"));
    let contribution = manifest["component"]["environment"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["variable"] == "PXR_PLUGINPATH_NAME")
        .unwrap();
    assert_eq!(
        contribution["values"],
        serde_json::json!(["lib/usd/hdToon/resources"])
    );
    let renderer = json(sb.ost(&["--json", "artifact", "import", path(dist)]));

    json(sb.ost(&["--json", "plugin", "new", "usd-schema", "schema"]));
    json(sb.ost(&[
        "--json",
        "plugin",
        "new",
        "usd-imaging",
        "imaging",
        "--schema-bundle",
        "schema",
        "--schema-type",
        "API",
    ]));
    std::fs::OpenOptions::new()
        .append(true)
        .open(sb.base.join("openstrata.toml"))
        .unwrap()
        .write_all(b"\n[workspace]\nmembers = ['.', 'schema', 'imaging']\n")
        .unwrap();
    let info = sb
        .base
        .join("imaging/plugin/resources/imaging/plugInfo.json");
    let source = std::fs::read_to_string(info.with_extension("json.in"))
        .unwrap()
        .replace("@OPENSTRATA_PLUGIN_LIBRARY_PREFIX@", "lib")
        .replace(
            "@CMAKE_SHARED_LIBRARY_SUFFIX@",
            std::env::consts::DLL_SUFFIX,
        );
    std::fs::write(info, source).unwrap();
    std::fs::create_dir_all(sb.base.join("imaging/lib")).unwrap();
    std::fs::write(
        sb.base.join(format!(
            "imaging/lib/libImagingImaging{}",
            std::env::consts::DLL_SUFFIX
        )),
        "fixture",
    )
    .unwrap();
    let plugin = json(sb.ost(&["--json", "plugin", "package", "imaging"]));
    let plugin_archive = PathBuf::from(plugin["data"]["archive"].as_str().unwrap());
    let plugin = json(sb.ost(&[
        "--json",
        "artifact",
        "import",
        path(plugin_archive.parent().unwrap()),
    ]));
    std::fs::write(sb.base.join("formation.toml"), format!("schema = 'openstrata.formation/v1alpha1'\n[formation]\nname = 'packaged-components'\n[runtime]\nartifact = '{runtime}'\n[[components]]\nid = 'toon'\nkind = 'renderer'\nartifact = '{}'\n[[components]]\nid = 'imaging'\nkind = 'plugin'\nartifact = '{}'\n[command]\nprogram = 'usdview'\n", renderer["data"]["artifact"]["digest"].as_str().unwrap(), plugin["data"]["artifact"]["digest"].as_str().unwrap())).unwrap();
    let resolved = json(sb.ost(&["--json", "formation", "resolve", "formation.toml"]));
    assert!(resolved.to_string().contains("lib/usd/hdToon/resources"));
    json(sb.ost(&["--json", "formation", "lock", "formation.toml"]));
    // Changing the declaration cannot package a stale intent completion.
    let project = sb.base.join("openstrata.toml");
    let source = std::fs::read_to_string(&project)
        .unwrap()
        .replace("value = true", "value = false");
    std::fs::write(project, source).unwrap();
    assert!(!sb.ost(&["package", "--intent", "hydra"]).status.success());
}

#[test]
fn resolve_lock_inspect_and_run_are_one_digest_pinned_lifecycle() {
    let sandbox = Sandbox::new();
    json(sandbox.ost(&["--json", "runtime", "pull", "cy2026", "--profile", "usd"]));
    sandbox.promote_runtime();
    let exported =
        json(sandbox.ost(&["--json", "runtime", "export", "cy2026", "--profile", "usd"]));
    let runtime_digest = exported["data"]["digest"].as_str().unwrap();
    let program = serde_json::to_string(ost_bin()).unwrap();
    let manifest = format!(
        r#"schema = "openstrata.formation/v1alpha1"

[formation]
name = "runtime-smoke"

[runtime]
artifact = "{runtime_digest}"

[command]
program = {program}
args = ["--version"]
"#
    );
    let formation = sandbox.base.join("formation.toml");
    std::fs::write(&formation, &manifest).unwrap();

    let resolved = json(sandbox.ost(&["--json", "formation", "resolve", path(&formation)]));
    assert_eq!(
        resolved["data"]["schema"],
        "openstrata.formation-resolved/v1alpha1"
    );
    assert_eq!(resolved["data"]["runtime"]["digest"], runtime_digest);
    assert_eq!(resolved["data"]["conflicts"], serde_json::json!([]));

    let locked = json(sandbox.ost(&["--json", "formation", "lock", path(&formation)]));
    assert_eq!(
        locked["data"]["schema"],
        "openstrata.formation-lock/v1alpha1"
    );
    assert!(sandbox.base.join("formation.lock").is_file());

    let inspected = json(sandbox.ost(&["--json", "formation", "inspect", path(&formation)]));
    assert_eq!(inspected["data"]["lock"]["matches_manifest"], true);

    let diagnosed = json(sandbox.ost(&["--json", "formation", "doctor", path(&formation)]));
    assert_eq!(diagnosed["ok"], true);
    assert!(diagnosed["data"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .all(|check| check["status"] == "pass"));

    let environment = json(sandbox.ost(&[
        "--json",
        "formation",
        "env",
        path(&formation),
        "--shell",
        "pwsh",
    ]));
    assert_eq!(environment["data"]["formation"], "runtime-smoke");
    let materialized = PathBuf::from(environment["data"]["materialized"].as_str().unwrap());
    assert!(
        materialized.is_dir(),
        "formation env paths must survive the command that exports them"
    );
    assert!(environment["data"]["env"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["name"] == "PATH"));

    let ran = json(sandbox.ost(&["--json", "formation", "run", path(&formation)]));
    assert_eq!(ran["data"]["run"]["success"], true);
    assert_eq!(ran["data"]["run"]["runtime"]["digest"], runtime_digest);
    assert!(ran["data"]["run"]["stdout"]
        .as_str()
        .unwrap()
        .contains("ost "));
    let evidence = PathBuf::from(ran["data"]["evidence"].as_str().unwrap());
    assert!(evidence.is_file());

    // A host is pinned into the same Formation lock and launched through the
    // same environment/evidence path. The fixture executable is a copy of ost:
    // its --version is safe to invoke without a DCC installation or license.
    let host_root = sandbox.base.join("maya2026");
    let bin = host_root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for name in ["maya", "mayapy"] {
        let executable = if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.into()
        };
        std::fs::copy(ost_bin(), bin.join(executable)).unwrap();
    }
    let header = host_root.join("include/maya/MTypes.h");
    std::fs::create_dir_all(header.parent().unwrap()).unwrap();
    std::fs::write(header, "#define MAYA_APP_VERSION 2026\n").unwrap();
    let discovered = json(sandbox.ost(&[
        "--json",
        "host",
        "discover",
        "--host",
        "maya",
        "--path",
        path(&host_root),
        "--no-environment",
        "--no-known-roots",
    ]));
    let host = &discovered["data"]["records"][0];
    assert_eq!(host["status"], "validated");
    let host_id = host["id"].as_str().unwrap();
    let host_fingerprint = host["fingerprint"]["digest"].as_str().unwrap();

    let unpinned = sandbox.ost(&[
        "--json",
        "host",
        "run",
        host_id,
        "--formation",
        path(&formation),
        "--",
        "--version",
    ]);
    assert!(!unpinned.status.success());
    let unpinned: serde_json::Value = serde_json::from_slice(&unpinned.stdout).unwrap();
    assert_eq!(unpinned["error"]["code"], "HOST_FORMATION_PIN_REQUIRED");

    std::fs::write(
        &formation,
        format!("{manifest}\n[host]\nid = \"{host_id}\"\nfingerprint = \"{host_fingerprint}\"\n"),
    )
    .unwrap();
    json(sandbox.ost(&["--json", "formation", "lock", path(&formation)]));
    let hosted_resolution =
        json(sandbox.ost(&["--json", "formation", "resolve", path(&formation)]));
    let contributions = hosted_resolution["data"]["environment"].as_array().unwrap();
    assert!(contributions.iter().any(|item| {
        item["source"] == format!("host:{host_id}")
            && item["key"] == "PATH"
            && item["paths"] == serde_json::json!(["host/bin"])
    }));
    assert!(contributions.iter().any(|item| {
        item["source"] == format!("host:{host_id}")
            && item["key"] == "MAYA_LOCATION"
            && item["operation"] == "set"
    }));
    let hosted_env = json(sandbox.ost(&["--json", "formation", "env", path(&formation)]));
    let variables = hosted_env["data"]["env"].as_array().unwrap();
    let variable = |name: &str| {
        variables.iter().find(|item| item["name"] == name).unwrap()["value"]
            .as_str()
            .unwrap()
    };
    let canonical_host_root = std::fs::canonicalize(&host_root).unwrap();
    assert_eq!(
        std::fs::canonicalize(variable("MAYA_LOCATION")).unwrap(),
        canonical_host_root
    );
    let canonical_bin = std::fs::canonicalize(&bin).unwrap();
    assert!(std::env::split_paths(variable("PATH"))
        .filter_map(|directory| std::fs::canonicalize(directory).ok())
        .any(|directory| directory == canonical_bin));
    let hosted = json(sandbox.ost(&[
        "--json",
        "host",
        "run",
        host_id,
        "--formation",
        path(&formation),
        "--",
        "--version",
    ]));
    assert_eq!(hosted["data"]["run"]["host"]["id"], host_id);
    assert_eq!(
        hosted["data"]["run"]["host"]["fingerprint"],
        host_fingerprint
    );
    assert!(hosted["data"]["run"]["stdout"]
        .as_str()
        .unwrap()
        .contains("ost "));

    let runtime_manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(sandbox.runtime_prefix().join("runtime.json")).unwrap(),
    )
    .unwrap();
    let runtime_python = runtime_manifest["python"].as_str().unwrap();
    let runtime_minor = runtime_python
        .split('.')
        .take(2)
        .collect::<Vec<_>>()
        .join(".");
    let incompatible_minor = if runtime_minor == "3.11" {
        "3.12"
    } else {
        "3.11"
    };
    let incompatible_dir = host_root.join(format!("lib/python{incompatible_minor}"));
    std::fs::create_dir_all(&incompatible_dir).unwrap();
    let rediscovered = json(sandbox.ost(&[
        "--json",
        "host",
        "discover",
        "--host",
        "maya",
        "--path",
        path(&host_root),
        "--no-environment",
        "--no-known-roots",
        "--refresh",
    ]));
    let incompatible_pin = rediscovered["data"]["records"][0]["fingerprint"]["digest"]
        .as_str()
        .unwrap();
    std::fs::write(
        &formation,
        format!("{manifest}\n[host]\nid = \"{host_id}\"\nfingerprint = \"{incompatible_pin}\"\n"),
    )
    .unwrap();
    let mismatch = sandbox.ost(&["--json", "formation", "lock", path(&formation)]);
    assert!(!mismatch.status.success());
    let mismatch: serde_json::Value = serde_json::from_slice(&mismatch.stdout).unwrap();
    assert_eq!(mismatch["error"]["code"], "HOST_PYTHON_ABI_MISMATCH");

    std::fs::remove_dir_all(host_root.join("lib")).unwrap();
    json(sandbox.ost(&[
        "--json",
        "host",
        "discover",
        "--host",
        "maya",
        "--path",
        path(&host_root),
        "--no-environment",
        "--no-known-roots",
        "--refresh",
    ]));
    std::fs::write(
        &formation,
        format!("{manifest}\n[host]\nid = \"{host_id}\"\nfingerprint = \"{host_fingerprint}\"\n"),
    )
    .unwrap();
    json(sandbox.ost(&["--json", "formation", "lock", path(&formation)]));

    let interpreter = if cfg!(windows) {
        "mayapy.exe"
    } else {
        "mayapy"
    };
    std::fs::OpenOptions::new()
        .append(true)
        .open(bin.join(interpreter))
        .unwrap()
        .write_all(b"changed")
        .unwrap();
    let drifted = sandbox.ost(&[
        "--json",
        "host",
        "run",
        host_id,
        "--formation",
        path(&formation),
        "--",
        "--version",
    ]);
    assert!(!drifted.status.success());
    let drifted: serde_json::Value = serde_json::from_slice(&drifted.stdout).unwrap();
    assert_eq!(drifted["error"]["code"], "HOST_NOT_VALIDATED");

    // Calling Formation directly cannot bypass the host pin either.
    let bypass = sandbox.ost(&[
        "--json",
        "formation",
        "run",
        path(&formation),
        "--",
        ost_bin(),
        "--version",
    ]);
    assert!(!bypass.status.success());
    let bypass: serde_json::Value = serde_json::from_slice(&bypass.stdout).unwrap();
    assert_eq!(bypass["error"]["code"], "HOST_NOT_VALIDATED");
}

#[test]
fn python_entrypoints_and_deep_projects_use_the_same_launch_contract() {
    let sandbox = Sandbox::new();
    let empty = camino::Utf8Path::from_path(&sandbox.base).unwrap();
    let python = ost_build::resolve_run_python(empty, "").expect("test requires Python");
    let version = Command::new(&python[0])
        .args(["-c", "import sys; print('%d.%d' % sys.version_info[:2])"])
        .output()
        .unwrap();
    assert!(version.status.success());
    let minor = String::from_utf8(version.stdout)
        .unwrap()
        .trim()
        .to_string();
    json(sandbox.ost(&["--json", "runtime", "pull", "cy2026", "--profile", "usd"]));
    sandbox.promote_runtime();
    let prefix = sandbox.runtime_prefix();
    let manifest_path = prefix.join("runtime.json");
    let mut runtime: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    runtime["python"] = minor.clone().into();
    runtime["variant"]["python"] = minor.replace('.', "").into();
    let typed: ost_runtime::RuntimeManifest = serde_json::from_value(runtime.clone()).unwrap();
    runtime["digest"] = typed.compute_digest().into();
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&runtime).unwrap()).unwrap();
    // An extensionless script with an unusable producer shebang
    // must run through the selected host Python on Windows, Linux and macOS.
    std::fs::write(prefix.join("bin/testusdview"), "#!/missing/producer/python\nimport os, sys, json\nprint(json.dumps({'python': sys.executable, 'path': os.environ['PATH'], 'args': sys.argv[1:]}))\n").unwrap();
    std::fs::write(
        prefix.join("bin/testusdview.cmd"),
        "@python \"%~dp0testusdview\" %*\r\n",
    )
    .unwrap();
    executable_fixture(&prefix.join("bin/testusdview"));
    executable_fixture(&prefix.join("bin/testusdview.cmd"));
    let plugin = "lib/python/PySide6/plugins/platforms/qwindows.dll";
    std::fs::create_dir_all(prefix.join(plugin).parent().unwrap()).unwrap();
    std::fs::write(prefix.join(plugin), "Qt path length fixture").unwrap();
    let exported =
        json(sandbox.ost(&["--json", "runtime", "export", "cy2026", "--profile", "usd"]));
    let digest = exported["data"]["digest"].as_str().unwrap();
    let project = sandbox.base.join("deep-project-name-".repeat(5));
    std::fs::create_dir_all(&project).unwrap();
    let formation = project.join("formation.toml");
    let manifest = format!("schema = 'openstrata.formation/v1alpha1'\n[formation]\nname = 'python-entrypoints'\n[runtime]\nartifact = '{digest}'\n[command]\nprogram = 'testusdview'\nargs = ['argument with spaces']\n");
    std::fs::write(&formation, &manifest).unwrap();
    json(sandbox.ost(&["--json", "formation", "lock", path(&formation)]));
    let lock = std::fs::read(project.join("formation.lock")).unwrap();
    assert!(!String::from_utf8_lossy(&lock).contains(&python[0]));
    json(sandbox.ost(&["--json", "formation", "doctor", path(&formation)]));
    let environment = json(sandbox.ost(&["--json", "formation", "env", path(&formation)]));
    let root = PathBuf::from(environment["data"]["materialized"].as_str().unwrap());
    assert!(root.starts_with(sandbox.home.join("artifacts/f")));
    assert_eq!(root.file_name().unwrap().len(), 16);
    assert!(root.join("runtime").join(plugin).is_file());
    let env_path = environment["data"]["env"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "PATH")
        .unwrap()["value"]
        .as_str()
        .unwrap();
    assert!(
        std::env::split_paths(env_path).any(|dir| dir == Path::new(&python[0]).parent().unwrap())
    );
    for command in [vec![], vec!["--", "testusdview.cmd"]] {
        let mut args = vec!["--json", "formation", "run", path(&formation)];
        args.extend(command);
        let result = json(sandbox.ost(&args));
        let run = &result["data"]["run"];
        assert_eq!(run["success"], true);
        assert!(run["script"].as_str().unwrap().ends_with("testusdview"));
        let child: serde_json::Value =
            serde_json::from_str(run["stdout"].as_str().unwrap()).unwrap();
        assert_eq!(child["python"], run["executable"]);
        if args.len() == 4 {
            assert_eq!(child["args"], serde_json::json!(["argument with spaces"]));
        }
        // Each run owns a fresh tree and removes it without invalidating `env`.
        assert!(!Path::new(run["script"].as_str().unwrap()).exists());
        assert!(root.is_dir());
    }
    assert_eq!(std::fs::read(project.join("formation.lock")).unwrap(), lock);
    // A retained tree is not a trusted cache; doctor/run must extract anew.
    std::fs::write(root.join("runtime/bin/testusdview"), "broken retained tree").unwrap();
    json(sandbox.ost(&["--json", "formation", "run", path(&formation)]));
    // Runtime export rejects non-executable bin entries on Unix, so exercise
    // command rejection with an explicit project-local file instead.
    let non_executable = project.join("not-executable");
    std::fs::write(&non_executable, "plain text, not an executable").unwrap();
    std::fs::write(
        &formation,
        manifest.replace(
            "program = 'testusdview'",
            &format!(
                "program = {}",
                serde_json::to_string(path(&non_executable)).unwrap()
            ),
        ),
    )
    .unwrap();
    json(sandbox.ost(&["--json", "formation", "lock", path(&formation)]));
    assert!(!sandbox
        .ost(&["--json", "formation", "doctor", path(&formation)])
        .status
        .success());
    assert!(!sandbox
        .ost(&["--json", "formation", "run", path(&formation)])
        .status
        .success());
}

#[test]
fn missing_runtime_python_fails_doctor_and_run_but_not_portable_locking() {
    let sandbox = Sandbox::new();
    json(sandbox.ost(&["--json", "runtime", "pull", "cy2026", "--profile", "usd"]));
    sandbox.promote_runtime();
    let prefix = sandbox.runtime_prefix();
    let manifest_path = prefix.join("runtime.json");
    let mut runtime: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    runtime["python"] = "9.99.x".into();
    let typed: ost_runtime::RuntimeManifest = serde_json::from_value(runtime.clone()).unwrap();
    runtime["digest"] = typed.compute_digest().into();
    std::fs::write(manifest_path, serde_json::to_vec_pretty(&runtime).unwrap()).unwrap();
    std::fs::write(
        prefix.join("bin/usdview"),
        "#!/usr/bin/env python3\nraise Exception('must not execute')\n",
    )
    .unwrap();
    executable_fixture(&prefix.join("bin/usdview"));
    let exported =
        json(sandbox.ost(&["--json", "runtime", "export", "cy2026", "--profile", "usd"]));
    let digest = exported["data"]["digest"].as_str().unwrap();
    std::fs::write(sandbox.base.join("formation.toml"), format!("schema = 'openstrata.formation/v1alpha1'\n[formation]\nname = 'missing-python'\n[runtime]\nartifact = '{digest}'\n[command]\nprogram = 'usdview'\n")).unwrap();
    json(sandbox.ost(&["--json", "formation", "lock"]));
    let doctor = sandbox.ost(&["--json", "formation", "doctor"]);
    assert!(!doctor.status.success());
    let report: serde_json::Value = serde_json::from_slice(&doctor.stdout).unwrap();
    let check = report["data"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["id"] == "formation.command")
        .unwrap();
    assert_eq!(check["status"], "fail");
    assert!(check["detail"]
        .as_str()
        .unwrap()
        .contains("no Python matching"));
    let run = sandbox.ost(&["--json", "formation", "run"]);
    assert!(!run.status.success());
    assert!(String::from_utf8_lossy(&run.stdout).contains("FORMATION_PYTHON_UNAVAILABLE"));
    let native = json(sandbox.ost(&[
        "--json",
        "formation",
        "run",
        "formation.toml",
        "--",
        ost_bin(),
        "--version",
    ]));
    assert_eq!(native["data"]["run"]["success"], true);
    assert!(native["data"]["run"]["python"].is_null());
}
