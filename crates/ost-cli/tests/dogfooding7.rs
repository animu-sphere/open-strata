// SPDX-License-Identifier: Apache-2.0
//! End-to-end tests for the dogfooding-#7 follow-ups (v0.6.0):
//!
//! - `ost plugin schema add` wires a co-located schema into an existing
//!   file-format bundle (starter `schema/schema.usda` + manifest `provides` +
//!   `schema.source`), and the bundle still passes the static L0 doctor.
//! - `ost runtime repair` re-adopts a drifted `local` runtime from its
//!   recorded USD root: `runtime show` reports the drift with the exact repair
//!   command, repair refreshes the recorded OpenUSD version, and the drift is
//!   gone. Non-local sources are refused with the right per-source hint.

use std::path::PathBuf;
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
            std::env::temp_dir().join(format!("ost-df7-{tag}-{}-{nanos}", std::process::id()));
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

    /// A fake USD install root with the markers adopt/validate/drift read.
    fn make_usd_root(&self, minor: u32, patch: u32) -> PathBuf {
        let root = self.base.join("usd-root");
        for rel in ["plugin/usd", "lib/python/pxr", "include/pxr"] {
            std::fs::create_dir_all(root.join(rel)).unwrap();
        }
        std::fs::write(root.join("plugin/usd/plugInfo.json"), "{}").unwrap();
        std::fs::write(root.join("lib/python/pxr/__init__.py"), "").unwrap();
        self.set_usd_version(minor, patch);
        root
    }

    /// (Re)write the install's pxr.h version — the drift lever.
    fn set_usd_version(&self, minor: u32, patch: u32) {
        std::fs::write(
            self.base.join("usd-root/include/pxr/pxr.h"),
            format!(
                "#define PXR_MAJOR_VERSION 0\n\
                 #define PXR_MINOR_VERSION {minor}\n\
                 #define PXR_PATCH_VERSION {patch}\n"
            ),
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

#[test]
fn schema_add_wires_a_bundle_that_still_passes_doctor_l0() {
    let sb = Sandbox::new("schema-add");
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

    let v = stdout_json(&sb.ost(&["--json", "plugin", "schema", "add", "toy"]));
    assert_eq!(v["data"]["schema_type"], "ToyAPI");
    assert_eq!(v["data"]["provides"], "usd-schema:ToyAPI");
    assert_eq!(v["data"]["source"], "schema/schema.usda");
    assert_eq!(v["data"]["codeless"], false);
    assert!(sb.base.join("toy/schema/schema.usda").is_file());

    // Adding the same type twice is refused (exit 3, configuration).
    let out = sb.ost(&["--json", "plugin", "schema", "add", "toy"]);
    assert_eq!(out.status.code(), Some(3));

    // The manifest is wired (provides + schema.source), with the template's
    // comments preserved by the textual edit.
    let manifest = std::fs::read_to_string(sb.base.join("toy/openstrata.plugin.yaml")).unwrap();
    assert!(manifest.contains("- usd-schema:ToyAPI"), "{manifest}");
    assert!(
        manifest.contains("source: schema/schema.usda"),
        "{manifest}"
    );
    assert!(manifest.contains("# OpenStrata plugin bundle manifest."));

    // The wired bundle still loads for inspect, and schema add introduced no
    // *new* structural failure (an unbuilt file-format bundle legitimately
    // fails only the `plugin.shared_library` check).
    let out = sb.ost(&["--json", "plugin", "inspect", "toy"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let failing: Vec<&str> = v["data"]["diagnostics"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|d| d["status"] == "fail")
                .filter_map(|d| d["id"].as_str())
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        failing,
        vec!["plugin.shared_library"],
        "only the not-yet-built library check may fail"
    );
}

#[test]
fn runtime_repair_fixes_openusd_version_drift() {
    let sb = Sandbox::new("repair");
    let root = sb.make_usd_root(25, 5);
    let root_str = root.to_str().unwrap();

    // Adopt at 25.05 (same release as the catalog default, so no drift yet).
    let out = sb.ost(&[
        "--json",
        "runtime",
        "pull",
        "cy2026",
        "--profile",
        "usd",
        "--from-usd",
        root_str,
    ]);
    let v = stdout_json(&out);
    assert_eq!(v["data"]["source"], "local");

    // The install moves underneath the manifest: pxr.h now reports 26.08.
    sb.set_usd_version(26, 8);
    let v = stdout_json(&sb.ost(&["--json", "runtime", "show", "cy2026", "--profile", "usd"]));
    let drift = &v["data"]["openusd_version_drift"];
    assert!(!drift.is_null(), "drift should be reported");
    assert_eq!(drift["detected"], "26.08");
    // The repair pointer is the exact one-step command, no blanks to fill.
    assert_eq!(drift["repair"], "ost runtime repair cy2026 --profile usd");

    // Repair re-adopts from the recorded root and refreshes the version.
    let v = stdout_json(&sb.ost(&["--json", "runtime", "repair", "cy2026", "--profile", "usd"]));
    assert_eq!(v["data"]["repaired"], true);
    assert_eq!(v["data"]["openusd_after"], "26.08");
    assert_eq!(
        v["data"]["usd_root"].as_str().unwrap(),
        root_str.replace('\\', "/")
    );

    // Drift is gone.
    let v = stdout_json(&sb.ost(&["--json", "runtime", "show", "cy2026", "--profile", "usd"]));
    assert!(v["data"]["openusd_version_drift"].is_null());
    assert_eq!(v["data"]["source"], "local");
}

#[test]
fn runtime_repair_refuses_non_local_sources_with_the_right_hint() {
    let sb = Sandbox::new("repair-mock");
    stdout_json(&sb.ost(&["--json", "runtime", "pull", "cy2026", "--profile", "usd"]));

    let out = sb.ost(&["--json", "runtime", "repair", "cy2026", "--profile", "usd"]);
    assert_eq!(out.status.code(), Some(4));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["code"], "REPAIR_UNSUPPORTED_SOURCE");
    // The hint names the exact refresh command for this source.
    assert!(
        v["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("ost runtime pull"),
        "hint: {}",
        v["error"]["hint"]
    );
}
