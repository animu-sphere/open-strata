// SPDX-License-Identifier: Apache-2.0
//! Output rendering helpers.
//!
//! Every command can render for a human terminal or as JSON (§13.2, §18.3).
//! Keeping this in one place ensures consistent error shapes and exit behavior.

use std::sync::atomic::{AtomicBool, Ordering};

static REDACT_PATHS: AtomicBool = AtomicBool::new(false);

pub fn set_redact_paths(enabled: bool) {
    REDACT_PATHS.store(enabled, Ordering::Relaxed);
}

/// Selected output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Human,
    Json,
}

impl Format {
    pub fn from_flag(json: bool) -> Format {
        if json {
            Format::Json
        } else {
            Format::Human
        }
    }

    pub fn is_json(self) -> bool {
        matches!(self, Format::Json)
    }
}

/// The version of the machine-readable output contract (design §14.3). Bump
/// only for a breaking change to the envelope shape.
pub const SCHEMA_VERSION: u64 = 1;

/// Render an error, matching the active format (design §14.3/§14.4).
///
/// In JSON mode the failure envelope is the single document on stdout, so an
/// agent reads one place; in human mode a short line goes to stderr, prefixed
/// with the stable code unless it is the generic legacy code.
pub fn error(err: &ost_core::Error, fmt: Format) {
    match fmt {
        Format::Json => {
            let mut error = serde_json::json!({
                "code": err.code(),
                "category": err.category().as_str(),
                "message": err.to_string(),
            });
            if let Some(hint) = err.hint() {
                error["hint"] = serde_json::Value::String(hint.to_string());
            }
            if let Some(phase) = err.phase() {
                error["phase"] = serde_json::Value::String(phase.to_string());
            }
            json(&serde_json::json!({
                "ok": false,
                "schema": SCHEMA_VERSION,
                "error": error,
                "warnings": [],
            }));
            // A `--json` stdout is routinely redirected to a file for later
            // parsing. When it is, the failure envelope goes with it and the
            // terminal shows nothing at all — a command that failed looks like
            // one that printed nothing. Mirror the identifying line to stderr so
            // the failure is visible wherever stdout ended up.
            //
            // Only the code and message: stderr is a signal here, not a second
            // copy of the document, and stdout remains the one place to parse.
            let message = if REDACT_PATHS.load(Ordering::Relaxed) {
                redact_string(&err.to_string(), "message")
            } else {
                err.to_string()
            };
            eprintln!("error[{}]: {message}", err.code());
        }
        Format::Human => {
            // The generic migration code adds no signal for a human reader.
            let phase = err
                .phase()
                .map(|p| format!(" (phase: {p})"))
                .unwrap_or_default();
            if err.code() == "OPERATION_FAILED" {
                eprintln!("error{phase}: {err}");
            } else {
                eprintln!("error[{}]{phase}: {err}", err.code());
            }
            if let Some(hint) = err.hint() {
                eprintln!("  hint: {hint}");
            }
        }
    }
}

/// Print a value as pretty JSON to stdout. The low-level printer the envelope
/// helpers build on; prefer [`success`]/[`report`] so output carries the
/// `{ok,schema,data,warnings}` contract (design §14.3).
pub fn json(value: &serde_json::Value) {
    let rendered = if REDACT_PATHS.load(Ordering::Relaxed) {
        let mut value = value.clone();
        redact_value(&mut value, None);
        if let Some(object) = value.as_object_mut() {
            object.insert("redacted".into(), serde_json::Value::Bool(true));
            object.insert(
                "redaction_schema".into(),
                serde_json::Value::String("openstrata.redaction/v1".into()),
            );
        }
        value
    } else {
        value.clone()
    };
    match serde_json::to_string_pretty(&rendered) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("error: failed to serialize JSON: {e}"),
    }
}

fn redact_value(value: &mut serde_json::Value, key: Option<&str>) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                redact_value(value, Some(key));
            }
        }
        serde_json::Value::Array(values) if key == Some("runtime_env") => {
            for value in values {
                if let Some(pair) = value.as_array_mut() {
                    if pair.len() >= 2 {
                        pair[1] = serde_json::Value::String("<managed-runtime-env>".into());
                    }
                } else {
                    redact_value(value, key);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_value(value, key);
            }
        }
        serde_json::Value::String(text) => {
            *text = redact_string(text, key.unwrap_or_default());
        }
        _ => {}
    }
}

fn redact_string(value: &str, key: &str) -> String {
    let placeholder = match key {
        "root" | "project_root" | "source_root" => "<project-root>",
        "store" | "runtime_store" => "<runtime-store>",
        "build_dir" => "<build-dir>",
        "scene" | "fixture" => "<scene-path>",
        "executable" | "program" | "cmake" | "ninja" | "cc" | "cxx" => "<tool-path>",
        "prefix" | "report_dir" | "record" | "package" | "archive" | "matrix" => "<path>",
        _ => "<absolute-path>",
    };
    if is_absolute_path(value) {
        return placeholder.into();
    }
    if contains_absolute_path(value) {
        return format!("<redacted-path-bearing-{key}>");
    }
    value.to_string()
}

fn is_absolute_path(value: &str) -> bool {
    let value = value.trim_matches(['\"', '\'']);
    value.starts_with('/')
        || value.starts_with("\\\\")
        || value.starts_with("//")
        || (value.len() >= 3
            && value.as_bytes()[0].is_ascii_alphabetic()
            && value.as_bytes()[1] == b':'
            && matches!(value.as_bytes()[2], b'/' | b'\\'))
}

fn contains_absolute_path(value: &str) -> bool {
    value.split_whitespace().any(|part| {
        is_absolute_path(part.trim_matches(|character: char| {
            matches!(character, '\"' | '\'' | '(' | ')' | '[' | ']' | ',' | ';')
        }))
    })
}

/// Emit a success envelope on stdout: `{ok:true, schema, data, warnings}`
/// (design §14.3). For a command whose result is itself a pass/fail report,
/// use [`report`] so `ok` carries the outcome.
pub fn success(data: &serde_json::Value) {
    report(true, data);
}

/// Emit an envelope whose `ok` carries an explicit outcome (design §14.3),
/// for report-style commands (`validate`, `lock --check`, `doctor`). The
/// command still owns its process exit code (§14.4).
pub fn report(ok: bool, data: &serde_json::Value) {
    report_with_warnings(ok, data, &[]);
}

/// Like [`report`], carrying non-fatal `{code, message}` warnings in the
/// envelope's `warnings` array (§14.3 — consumers must tolerate new codes).
pub fn report_with_warnings(ok: bool, data: &serde_json::Value, warnings: &[serde_json::Value]) {
    json(&serde_json::json!({
        "ok": ok,
        "schema": SCHEMA_VERSION,
        "data": data,
        "warnings": warnings,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_redaction_handles_host_paths_and_managed_env_without_secret_guessing() {
        let mut value = serde_json::json!({
            "root": "C:\\Users\\Alice Smith\\work\\project",
            "build_dir": "\\\\server\\private share\\build",
            "scene": "/home/alice/private scenes/shot.usda",
            "runtime_env": [
                ["PATH", "C:\\Users\\Alice\\.ost\\bin"],
                ["PATH", "/home/alice/.ost/lib"]
            ],
            "runtime_artifact": format!("sha256:{}", "ab".repeat(32)),
            "token_like_but_not_a_path": "sk-example-not-a-real-secret"
        });
        redact_value(&mut value, None);
        assert_eq!(value["root"], "<project-root>");
        assert_eq!(value["build_dir"], "<build-dir>");
        assert_eq!(value["scene"], "<scene-path>");
        assert_eq!(value["runtime_env"][0][0], "PATH");
        assert_eq!(value["runtime_env"][0][1], "<managed-runtime-env>");
        assert_eq!(value["runtime_env"][1][1], "<managed-runtime-env>");
        assert!(value["runtime_artifact"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
        assert_eq!(
            value["token_like_but_not_a_path"],
            "sk-example-not-a-real-secret"
        );
    }
}
