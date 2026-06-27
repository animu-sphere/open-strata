// SPDX-License-Identifier: Apache-2.0
//! JSON output contract tests (design §14.3/§14.4, docs/json-schema.md).
//!
//! These drive the real `ost` binary and assert the *contract*, not prose:
//!
//! - `--json` stdout is a single parseable JSON document (no report-then-error
//!   double emission, no log lines mixed in);
//! - every envelope carries `ok` / `schema` / `warnings`, with `data` on success
//!   and `error{code,category}` on failure;
//! - the process exit code is the failure category's normalized code.
//!
//! Assertions are field-level, so adding fields never breaks them.

use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

fn ost_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ost")
}

/// A throwaway store + project dir, cleaned up on drop. The work dir name is a
/// valid project identifier so `ost init` (which derives the name from it)
/// succeeds.
struct Sandbox {
    home: PathBuf,
    work: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let base = std::env::temp_dir().join(format!("ost-jc-{}-{nanos}", std::process::id()));
        let home = base.join("home");
        // A dir name that is a valid project identifier (letters/digits/-/_).
        let work = base.join(format!("proj-{tag}"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        Sandbox { home, work }
    }

    fn ost(&self, args: &[&str]) -> Output {
        Command::new(ost_bin())
            .args(args)
            .current_dir(&self.work)
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
        if let Some(base) = self.home.parent() {
            let _ = std::fs::remove_dir_all(base);
        }
    }
}

/// Parse stdout as a single JSON document. Fails if stdout is empty, not JSON,
/// or carries trailing content after the first document (the contract is exactly
/// one document on stdout).
fn parse_stdout(o: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&o.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not a single JSON document: {e}\n--- stdout ---\n{stdout}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&o.stderr)
        )
    })
}

fn exit_code(o: &Output) -> i32 {
    o.status.code().expect("process exited via code, not signal")
}

/// Every envelope has these, regardless of success or failure.
fn assert_envelope_common(v: &Value) {
    assert_eq!(v["schema"], 1, "schema must be 1");
    assert!(v["ok"].is_boolean(), "ok must be a boolean: {v}");
    assert!(v["warnings"].is_array(), "warnings must be an array: {v}");
}

#[test]
fn success_envelope_wraps_data() {
    let sb = Sandbox::new("ok");
    let out = sb.ost(&["--json", "platform", "list"]);
    assert_eq!(exit_code(&out), 0, "platform list should succeed");

    let v = parse_stdout(&out);
    assert_envelope_common(&v);
    assert_eq!(v["ok"], true);
    assert!(v["data"].is_object(), "success carries a data object: {v}");
    assert!(
        v["data"]["platforms"].is_array(),
        "platform list data.platforms is an array: {v}"
    );
    assert!(v.get("error").is_none(), "success has no error: {v}");
}

#[test]
fn failure_envelope_carries_code_and_category() {
    let sb = Sandbox::new("usage");
    let out = sb.ost(&["--json", "platform", "show", "cy2099"]);

    // Unknown platform is a usage error → exit 2.
    assert_eq!(exit_code(&out), 2, "unknown platform is a usage error");

    let v = parse_stdout(&out);
    assert_envelope_common(&v);
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "PLATFORM_NOT_FOUND");
    assert_eq!(v["error"]["category"], "usage");
    assert!(
        v["error"]["message"].is_string(),
        "error carries a human message: {v}"
    );
    assert!(v.get("data").is_none(), "a failure has no data: {v}");
}

#[test]
fn precondition_failure_exits_4() {
    // No project in an empty dir → PROJECT_NOT_FOUND (precondition, exit 4).
    let sb = Sandbox::new("precond");
    let out = sb.ost(&["--json", "build"]);

    assert_eq!(exit_code(&out), 4, "missing project is a precondition");
    let v = parse_stdout(&out);
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "PROJECT_NOT_FOUND");
    assert_eq!(v["error"]["category"], "precondition");
}

#[test]
fn report_command_is_a_single_document_on_failure() {
    // `build --check` with no runtime pulled fails a required preflight check.
    // The report and the failure must not produce two stdout documents.
    let sb = Sandbox::new("report");
    let init = sb.ost(&["init", "--platform", "cy2026"]);
    assert!(
        init.status.success(),
        "init failed: {}{}",
        String::from_utf8_lossy(&init.stdout),
        String::from_utf8_lossy(&init.stderr)
    );

    let out = sb.ost(&["--json", "build", "--check"]);
    // A missing runtime is a precondition (exit 4); the report's ok is false.
    assert_eq!(exit_code(&out), 4, "a failed preflight check exits 4");

    let v = parse_stdout(&out); // panics if stdout is not exactly one document
    assert_envelope_common(&v);
    assert_eq!(v["ok"], false, "the report carries the failed outcome: {v}");
    assert!(
        v["data"]["checks"].is_array(),
        "the report carries its checks under data: {v}"
    );
    // The failure is conveyed by ok + exit code, not a second error envelope.
    assert!(
        v.get("error").is_none(),
        "a report failure stays in data, not a duplicate error doc: {v}"
    );
}

#[test]
fn json_mode_keeps_stdout_free_of_log_noise() {
    // Even on failure, nothing but the single JSON document lands on stdout;
    // any human/diagnostic noise belongs on stderr.
    let sb = Sandbox::new("clean");
    let out = sb.ost(&["--json", "platform", "show", "cy2099"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let trimmed = stdout.trim();
    assert!(
        trimmed.starts_with('{') && trimmed.ends_with('}'),
        "stdout must be exactly the JSON object:\n{stdout}"
    );
    // Parses as one document (no trailing tokens).
    let _: Value = serde_json::from_str(&stdout).expect("single JSON document");
}
