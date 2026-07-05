// SPDX-License-Identifier: Apache-2.0
//! Output rendering helpers.
//!
//! Every command can render for a human terminal or as JSON (§13.2, §18.3).
//! Keeping this in one place ensures consistent error shapes and exit behavior.

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
            json(&serde_json::json!({
                "ok": false,
                "schema": SCHEMA_VERSION,
                "error": error,
                "warnings": [],
            }));
        }
        Format::Human => {
            // The generic migration code adds no signal for a human reader.
            if err.code() == "OPERATION_FAILED" {
                eprintln!("error: {err}");
            } else {
                eprintln!("error[{}]: {err}", err.code());
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
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("error: failed to serialize JSON: {e}"),
    }
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
