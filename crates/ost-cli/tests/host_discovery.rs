// SPDX-License-Identifier: Apache-2.0
//! `ost host discover|list|inspect` over fixture install trees.
//!
//! These assert the *command* contract rather than the discovery logic (which
//! `ost-host` covers): the diagnostic exit convention, the JSON envelope, the
//! manifest-driven roots, and the selector rules. Every run is sandboxed by
//! `OST_HOME` and asks for no ambient provider, so a developer machine with a
//! real Maya on it cannot change a result.

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
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let base =
            std::env::temp_dir().join(format!("ost-host-{tag}-{}-{nanos}", std::process::id()));
        let home = base.join("home");
        std::fs::create_dir_all(&home).unwrap();
        Sandbox { base, home }
    }

    fn ost(&self, args: &[&str]) -> Output {
        Command::new(ost_bin())
            .args(args)
            .current_dir(&self.base)
            .env("OST_HOME", &self.home)
            // A vendor variable on the developer's machine must not reach in.
            .env_remove("MAYA_LOCATION")
            .env_remove("HFS")
            .output()
            .expect("spawn ost")
    }

    /// Run and parse the JSON envelope, asserting the exit code.
    fn json(&self, args: &[&str], expected_exit: i32) -> serde_json::Value {
        let mut full = vec!["--json"];
        full.extend_from_slice(args);
        let output = self.ost(&full);
        assert_eq!(
            output.status.code(),
            Some(expected_exit),
            "`ost {}` exit; stdout: {} stderr: {}",
            full.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "`ost {}` did not emit one JSON envelope ({error}): {}",
                full.join(" "),
                String::from_utf8_lossy(&output.stdout)
            )
        })
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.base.join(relative)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// A Maya-shaped install carrying the devkit header, so the fixture has a
/// high-confidence version rather than one read off its directory name.
fn maya_install(root: &Path, version: &str) {
    for name in ["maya", "mayapy"] {
        write(&root.join("bin").join(exe(name)), "fixture");
    }
    std::fs::create_dir_all(root.join("modules")).unwrap();
    std::fs::create_dir_all(root.join("lib").join("python3.11")).unwrap();
    write(
        &root.join("include").join("maya").join("MTypes.h"),
        &format!("#define MAYA_APP_VERSION {version}\n"),
    );
}

fn houdini_install(root: &Path, version: &str) {
    for name in ["houdini", "hython", "husk"] {
        write(&root.join("bin").join(exe(name)), "fixture");
    }
    std::fs::create_dir_all(root.join("houdini").join("python3.11libs")).unwrap();
    write(
        &root
            .join("toolkit")
            .join("include")
            .join("SYS")
            .join("SYS_Version.h"),
        &format!("#define SYS_VERSION_FULL \"{version}\"\n"),
    );
}

/// Flags that keep a run from consulting anything but what the test names.
const HERMETIC: [&str; 2] = ["--no-environment", "--no-known-roots"];

#[test]
fn discover_reports_an_explicit_install_and_list_serves_it_back() {
    let sandbox = Sandbox::new("explicit");
    let root = sandbox.path("dcc/maya2026");
    maya_install(&root, "2026");
    let root = root.to_string_lossy().into_owned();

    let discovered = sandbox.json(
        &[
            "host",
            "discover",
            "--host",
            "maya",
            "--path",
            &root,
            HERMETIC[0],
            HERMETIC[1],
        ],
        0,
    );
    assert_eq!(discovered["ok"], true);
    assert_eq!(
        discovered["data"]["schema_version"],
        "openstrata.host-record/v1alpha1"
    );
    let records = discovered["data"]["records"].as_array().unwrap();
    assert_eq!(records.len(), 1, "{records:#?}");
    assert_eq!(records[0]["status"], "validated");
    assert_eq!(records[0]["family"], "maya");
    assert_eq!(records[0]["version"]["raw"], "2026");
    assert_eq!(records[0]["version"]["confidence"], "high");
    assert_eq!(records[0]["python"]["abi_tag"], "cp311");
    let id = records[0]["id"].as_str().unwrap().to_string();

    // The cache is what makes `list` cheap, and it must survive the process.
    let listed = sandbox.json(&["host", "list"], 0);
    let listed = listed["data"]["records"].as_array().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["id"], id.as_str());

    let inspected = sandbox.json(&["host", "inspect", &id], 0);
    assert_eq!(inspected["data"]["id"], id.as_str());
    // Both are published contracts (host-record.schema.json): an id is
    // `[A-Za-z0-9._-]` and a digest is exactly `sha256:<64 hex>`.
    assert!(
        id.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')),
        "{id}"
    );
    let digest = inspected["data"]["fingerprint"]["digest"].as_str().unwrap();
    let hex = digest
        .strip_prefix("sha256:")
        .unwrap_or_else(|| panic!("{digest}"));
    assert_eq!(hex.len(), 64, "{digest}");
    assert!(hex.chars().all(|c| c.is_ascii_hexdigit()), "{digest}");
}

#[test]
fn finding_nothing_is_an_answer_not_a_failure() {
    let sandbox = Sandbox::new("empty");

    // `discover` is diagnostic, like `doctor`: an empty machine exits 0 and
    // says which providers it consulted, rather than failing.
    let discovered = sandbox.json(&["host", "discover", HERMETIC[0], HERMETIC[1]], 0);
    assert_eq!(discovered["ok"], true);
    assert!(discovered["data"]["records"].as_array().unwrap().is_empty());
    let codes: Vec<&str> = discovered["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|warning| warning["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"HOST_NONE_VALIDATED"), "{codes:?}");

    let listed = sandbox.json(&["host", "list"], 0);
    assert_eq!(listed["ok"], true);
    assert!(listed["data"]["records"].as_array().unwrap().is_empty());
}

#[test]
fn a_named_path_that_is_not_a_host_is_refused_with_the_reason() {
    let sandbox = Sandbox::new("reject");
    let root = sandbox.path("not-a-host");
    std::fs::create_dir_all(root.join("bin")).unwrap();
    let root = root.to_string_lossy().into_owned();

    let discovered = sandbox.json(
        &[
            "host",
            "discover",
            "--host",
            "houdini",
            "--path",
            &root,
            HERMETIC[0],
            HERMETIC[1],
        ],
        0,
    );
    let records = discovered["data"]["records"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["status"], "rejected");
    assert_eq!(records[0]["rejection"]["code"], "HOST_MARKERS_MISSING");
    assert!(
        records[0]["rejection"]["message"]
            .as_str()
            .unwrap()
            .contains("hython"),
        "the refusal names what it looked for: {}",
        records[0]["rejection"]["message"]
    );
}

#[test]
fn selectors_resolve_exactly_or_fail_with_the_candidates() {
    let sandbox = Sandbox::new("selector");
    maya_install(&sandbox.path("dcc/maya2025"), "2025");
    maya_install(&sandbox.path("dcc/maya2026"), "2026");
    let roots = sandbox.path("dcc").to_string_lossy().into_owned();

    sandbox.json(
        &[
            "host",
            "discover",
            "--root",
            &roots,
            "--depth",
            "1",
            HERMETIC[0],
            HERMETIC[1],
        ],
        0,
    );

    // A family selector naming two installs must not be resolved by scan order.
    let ambiguous = sandbox.json(&["host", "inspect", "maya"], 2);
    assert_eq!(ambiguous["ok"], false);
    assert_eq!(ambiguous["error"]["code"], "HOST_SELECTOR_AMBIGUOUS");
    assert_eq!(ambiguous["error"]["category"], "usage");

    // An id prefix that names exactly one does resolve.
    let resolved = sandbox.json(&["host", "inspect", "maya-2026"], 0);
    assert_eq!(resolved["data"]["version"]["raw"], "2026");

    let missing = sandbox.json(&["host", "inspect", "nuke"], 4);
    assert_eq!(missing["error"]["code"], "HOST_NOT_FOUND");
    assert_eq!(missing["error"]["category"], "precondition");
}

#[test]
fn the_project_manifest_supplies_discovery_roots_and_register_writes_the_inventory() {
    let sandbox = Sandbox::new("project");
    maya_install(&sandbox.path("dcc/maya2026"), "2026");
    houdini_install(&sandbox.path("dcc/hfs20.5.550"), "20.5.550");
    let roots = sandbox.path("dcc").to_string_lossy().replace('\\', "/");
    write(
        &sandbox.path("openstrata.toml"),
        &format!(
            "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n\n\
             [requires]\nplatform = \"cy2026\"\n\n\
             [host.discovery]\nroots = [\"{roots}\"]\nmax_depth = 1\n"
        ),
    );

    let discovered = sandbox.json(
        &["host", "discover", "--register", HERMETIC[0], HERMETIC[1]],
        0,
    );
    let records = discovered["data"]["records"].as_array().unwrap();
    let families: Vec<&str> = records
        .iter()
        .map(|record| record["family"].as_str().unwrap())
        .collect();
    assert_eq!(families, vec!["maya", "houdini"], "{records:#?}");

    let inventory = sandbox.path(".strata/hosts/inventory.json");
    assert!(
        inventory.is_file(),
        "--register writes a reviewable project inventory"
    );
    let inventory: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&inventory).unwrap()).unwrap();
    assert_eq!(inventory["schema"], "openstrata.host-inventory/v1alpha1");
    assert_eq!(inventory["records"].as_array().unwrap().len(), 2);
}

#[test]
fn a_discovery_root_that_would_scan_the_disk_fails_the_manifest() {
    let sandbox = Sandbox::new("unbounded");
    write(
        &sandbox.path("openstrata.toml"),
        "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n\n\
         [requires]\nplatform = \"cy2026\"\n\n\
         [host.discovery]\nroots = [\"/\"]\n",
    );

    let failed = sandbox.json(&["host", "discover", HERMETIC[0], HERMETIC[1]], 3);
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"]["category"], "configuration");
    assert!(
        failed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("filesystem root"),
        "{}",
        failed["error"]["message"]
    );
}

#[test]
fn an_unknown_family_names_the_supported_ones() {
    let sandbox = Sandbox::new("family");
    let failed = sandbox.json(&["host", "discover", "--host", "blender"], 2);
    assert_eq!(failed["error"]["code"], "HOST_FAMILY_UNKNOWN");
    assert!(
        failed["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("houdini"),
        "{}",
        failed["error"]["hint"]
    );
}
