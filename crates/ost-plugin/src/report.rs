// SPDX-License-Identifier: Apache-2.0
//! Report generation (harness §13).
//!
//! Every run writes a report under `.strata/reports/<plugin>/<UTC-timestamp>/`
//! so local and CI failures share one structure. Phase 4a writes the static
//! subset: `report.json` (the staged diagnostics), `summary.txt` (a human
//! digest), and `environment.json` (the session env preview). The execution
//! logs (`stdout.log`, `normalized-output.usda`, `diff.txt`) join in 4b when the
//! levels that produce them light up.
//!
//! The crate builds the values and owns the directory layout; timestamps are
//! formatted to a stable, sortable UTC form with no external date dependency.

use camino::{Utf8Path, Utf8PathBuf};

use ost_core::{Error, Result};
use ost_runtime::EnvSet;

use crate::bundle::Bundle;
use crate::doctor::{DoctorReport, Status};

/// Schema version of `report.json` (bumped on breaking shape changes).
pub const REPORT_SCHEMA: u32 = 1;

/// CI evidence from the `OST_CI_*` contract (Phase 5). Generated workflows
/// export these job-level variables so every report written inside a CI job
/// records *which support claim* it proves: the cell, its lane, the runner
/// profile and resolved `runs-on`, pinned artifact digests, and the effective
/// artifact trust floor. Returns
/// `None` outside CI (no `OST_CI_CELL`), so local reports are unchanged.
pub fn ci_evidence_from_env() -> Option<serde_json::Value> {
    let cell = std::env::var("OST_CI_CELL")
        .ok()
        .filter(|s| !s.is_empty())?;
    let get = |key: &str| match std::env::var(key).ok().filter(|s| !s.is_empty()) {
        Some(value) => serde_json::Value::String(value),
        None => serde_json::Value::Null,
    };
    Some(serde_json::json!({
        "cell": cell,
        "lane": get("OST_CI_LANE"),
        "runner_profile": get("OST_CI_RUNNER_PROFILE"),
        "runs_on": get("OST_CI_RUNS_ON"),
        "runtime_artifact": get("OST_CI_RUNTIME_ARTIFACT"),
        "plugin_artifact": get("OST_CI_PLUGIN_ARTIFACT"),
        "minimum_trust": get("OST_CI_MINIMUM_TRUST"),
    }))
}

/// Build the machine-readable `report.json` body for a doctor run.
pub fn report_json(bundle: &Bundle, report: &DoctorReport) -> serde_json::Value {
    let diagnostics: Vec<_> = report
        .diagnostics
        .iter()
        .map(|d| {
            serde_json::json!({
                "id": d.id,
                "level": d.level,
                "status": d.status.as_str(),
                "observed": d.observed,
                "suggested_actions": d.suggested_actions,
            })
        })
        .collect();

    let mut body = serde_json::json!({
        "schema": REPORT_SCHEMA,
        "plugin": bundle.manifest.plugin.name,
        "version": bundle.manifest.plugin.version,
        "kind": bundle.manifest.kind().as_str(),
        "license": bundle.manifest.license,
        "passed": report.passed(),
        "summary": {
            "pass": report.count(Status::Pass),
            "fail": report.count(Status::Fail),
            "skip": report.count(Status::Skip),
        },
        "diagnostics": diagnostics,
    });
    // Additive: present only inside a CI job that exports the OST_CI_* vars.
    if let Some(ci) = ci_evidence_from_env() {
        body["ci"] = ci;
    }
    body
}

/// Build the `environment.json` body: the session env the run would set.
pub fn environment_json(session_env: &EnvSet) -> serde_json::Value {
    let vars: Vec<_> = session_env
        .pairs()
        .into_iter()
        .map(|(k, v)| serde_json::json!({ "key": k, "value": v }))
        .collect();
    serde_json::json!({
        "separator": session_env.sep.to_string(),
        "vars": vars,
    })
}

/// Render the human-facing `summary.txt`.
pub fn summary_text(bundle: &Bundle, report: &DoctorReport) -> String {
    let m = &bundle.manifest;
    let mut out = String::new();
    out.push_str(&format!(
        "Plugin {} {} ({})\n",
        m.plugin.name,
        m.plugin.version,
        m.kind().as_str()
    ));
    if let Some(license) = &m.license {
        out.push_str(&format!("License: {license}\n"));
    }
    out.push_str(&format!("Root:   {}\n\n", bundle.root));
    for d in &report.diagnostics {
        let mark = match d.status {
            Status::Pass => "PASS",
            Status::Fail => "FAIL",
            Status::Skip => "SKIP",
        };
        out.push_str(&format!(
            "[{mark}] L{} {} — {}\n",
            d.level, d.id, d.observed
        ));
        for action in &d.suggested_actions {
            out.push_str(&format!("        ↳ {action}\n"));
        }
    }
    out.push_str(&format!(
        "\nResult: {} ({} pass, {} fail, {} skip)\n",
        if report.passed() { "OK" } else { "FAILED" },
        report.count(Status::Pass),
        report.count(Status::Fail),
        report.count(Status::Skip),
    ));
    out
}

/// Write a full report directory under `reports_root` and return its path.
///
/// Layout: `reports_root/<plugin>/<UTC-timestamp>/{report.json, summary.txt,
/// environment.json}`. `now_unix` is the wall-clock seconds since the epoch
/// (injected so callers control the clock and tests stay deterministic).
pub fn write_report(
    reports_root: &Utf8Path,
    bundle: &Bundle,
    report: &DoctorReport,
    session_env: &EnvSet,
    now_unix: u64,
) -> Result<Utf8PathBuf> {
    let dir = reports_root
        .join(&bundle.manifest.plugin.name)
        .join(utc_stamp(now_unix));
    std::fs::create_dir_all(dir.as_std_path()).map_err(|e| Error::io(dir.to_string(), e))?;

    let report_path = dir.join("report.json");
    let body = serde_json::to_string_pretty(&report_json(bundle, report))
        .map_err(|e| Error::parse("report.json", anyhow::Error::new(e)))?;
    write_file(&report_path, &format!("{body}\n"))?;

    write_file(&dir.join("summary.txt"), &summary_text(bundle, report))?;

    let env_body = serde_json::to_string_pretty(&environment_json(session_env))
        .map_err(|e| Error::parse("environment.json", anyhow::Error::new(e)))?;
    write_file(&dir.join("environment.json"), &format!("{env_body}\n"))?;

    Ok(dir)
}

fn write_file(path: &Utf8Path, contents: &str) -> Result<()> {
    std::fs::write(path.as_std_path(), contents).map_err(|e| Error::io(path.to_string(), e))
}

/// Format epoch seconds as a sortable, filesystem-safe UTC stamp
/// (`YYYYMMDDTHHMMSSZ`). Uses Howard Hinnant's civil-from-days algorithm so we
/// need no date crate (and stay on the pinned dependency tree).
fn utc_stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}{m:02}{d:02}T{hh:02}{mm:02}{ss:02}Z")
}

/// Days since 1970-01-01 -> (year, month, day). Hinnant, "chrono-Compatible
/// Low-Level Date Algorithms".
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_schema_accepts_every_plugin_kind() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../schemas/plugin-report.schema.json"))
                .expect("plugin report schema parses");
        let kinds = schema["properties"]["kind"]["enum"]
            .as_array()
            .expect("kind enum")
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>();
        let modeled = crate::PluginKind::ALL
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>();
        assert_eq!(kinds, modeled);
    }

    #[test]
    fn utc_stamp_is_correct_and_sortable() {
        // 2021-11-14T22:00:00Z = 1_636_927_200
        assert_eq!(utc_stamp(1_636_927_200), "20211114T220000Z");
        // Epoch.
        assert_eq!(utc_stamp(0), "19700101T000000Z");
        // Monotonic input -> lexicographically increasing output.
        assert!(utc_stamp(1_636_927_200) < utc_stamp(1_636_927_201));
    }
}
