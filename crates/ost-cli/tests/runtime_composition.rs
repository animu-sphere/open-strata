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
