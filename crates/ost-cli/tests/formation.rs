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
    serde_json::from_slice(&output.stdout).unwrap()
}

fn path(path: &Path) -> &str {
    path.to_str().unwrap()
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
