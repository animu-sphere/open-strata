// SPDX-License-Identifier: Apache-2.0
//! `ost validate` — validate a built/packaged target (§8.5, §18.2).
//!
//! Structural checks over a target's build and (optional) artifact state:
//! configured, built, runtime-compatible, and — when packaged — archive
//! integrity. Deterministic exit for CI: `0` when all non-skipped checks pass,
//! `1` otherwise.

use clap::Args;

use ost_build::TargetLock;
use ost_core::paths::STATE_DIR;
use ost_core::{digest, Result};

use crate::commands::configure::{build_target, load_project, resolve_selection};
use crate::output::{self, Format};

#[derive(Debug, Args)]
pub struct ValidateArgs {
    /// Platform target, e.g. `cy2026`. Defaults to the project's platform.
    #[arg(long)]
    target: Option<String>,

    /// Profile to validate. Defaults to the project's profile.
    #[arg(long)]
    profile: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Pass,
    Fail,
    Skip,
}

impl Status {
    fn mark(self) -> &'static str {
        match self {
            Status::Pass => "ok  ",
            Status::Fail => "FAIL",
            Status::Skip => "skip",
        }
    }
    fn word(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Fail => "fail",
            Status::Skip => "skip",
        }
    }
}

struct Check {
    name: &'static str,
    status: Status,
    detail: Option<String>,
}

impl Check {
    fn pass(name: &'static str) -> Check {
        Check {
            name,
            status: Status::Pass,
            detail: None,
        }
    }
    fn fail(name: &'static str, detail: impl Into<String>) -> Check {
        Check {
            name,
            status: Status::Fail,
            detail: Some(detail.into()),
        }
    }
    fn skip(name: &'static str, detail: impl Into<String>) -> Check {
        Check {
            name,
            status: Status::Skip,
            detail: Some(detail.into()),
        }
    }
}

pub fn run(args: ValidateArgs, fmt: Format) -> Result<()> {
    let (root, platform, profile) = resolve_selection(args.target, args.profile)?;
    let project = load_project(&root)?;
    let (target, r) = build_target(&platform, &profile)?;
    let id = target.id();

    let mut checks = Vec::new();

    // 1. configured — target.lock.json exists and parses.
    let lock_path = root
        .join(STATE_DIR)
        .join("targets")
        .join(&id)
        .join("target.lock.json");
    let lock: Option<TargetLock> = std::fs::read_to_string(lock_path.as_std_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    match &lock {
        Some(_) => checks.push(Check::pass("configured")),
        None => checks.push(Check::fail(
            "configured",
            "target.lock.json missing or invalid — run `ost configure`",
        )),
    }

    // 2. built — the build directory exists.
    let build_dir = root.join("build").join(&id);
    if build_dir.as_std_path().is_dir() {
        checks.push(Check::pass("built"));
    } else {
        checks.push(Check::fail(
            "built",
            "build directory missing — run `ost build`",
        ));
    }

    // 3. runtime-compatible — the runtime is pulled and its digest matches the
    //    one recorded at configure time (detects drift).
    if !r.pulled {
        checks.push(Check::fail(
            "runtime-compatible",
            format!("runtime '{}' not pulled", target.runtime_id),
        ));
    } else if let Some(lock) = &lock {
        if lock.runtime.digest.is_empty() {
            checks.push(Check::fail(
                "runtime-compatible",
                "configured before the runtime was pulled — re-run `ost configure`",
            ));
        } else if lock.runtime.digest == target.runtime_digest {
            checks.push(Check::pass("runtime-compatible"));
        } else {
            checks.push(Check::fail(
                "runtime-compatible",
                format!(
                    "runtime digest drift: locked {} != current {}",
                    short(&lock.runtime.digest),
                    short(&target.runtime_digest)
                ),
            ));
        }
    } else {
        checks.push(Check::skip("runtime-compatible", "not configured"));
    }

    // 4. artifact-integrity — only when the target has been packaged.
    let dist_dir = root
        .join("dist")
        .join(&project.project.name)
        .join(&project.project.version)
        .join(&id);
    let manifest_path = dist_dir.join("manifest.json");
    match std::fs::read_to_string(manifest_path.as_std_path()) {
        Ok(src) => match serde_json::from_str::<serde_json::Value>(&src) {
            Ok(manifest) => {
                checks.push(check_artifact(&dist_dir, &manifest));
            }
            Err(e) => checks.push(Check::fail(
                "artifact-integrity",
                format!("manifest.json invalid: {e}"),
            )),
        },
        Err(_) => checks.push(Check::skip("artifact-integrity", "not packaged")),
    }

    let failed = checks.iter().any(|c| c.status == Status::Fail);
    emit(&id, &checks, fmt);

    // A failed check is a validation mismatch (§14.4); emit() already produced
    // this command's report, so exit with that category code directly.
    if failed {
        std::process::exit(ost_core::Category::Validation.exit_code() as i32);
    }
    Ok(())
}

/// Recompute the archive digest and compare it to the manifest.
fn check_artifact(dist_dir: &camino::Utf8Path, manifest: &serde_json::Value) -> Check {
    if manifest.get("schema").and_then(|v| v.as_u64()) != Some(1) {
        return Check::fail("artifact-integrity", "unexpected manifest schema");
    }
    let archive_name = match manifest.get("archive").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return Check::fail("artifact-integrity", "manifest has no `archive`"),
    };
    let expected = match manifest.get("archive_digest").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return Check::fail("artifact-integrity", "manifest has no `archive_digest`"),
    };
    let archive_path = dist_dir.join(archive_name);
    let bytes = match std::fs::read(archive_path.as_std_path()) {
        Ok(b) => b,
        Err(e) => return Check::fail("artifact-integrity", format!("cannot read archive: {e}")),
    };
    let actual = digest::sha256_hex(&bytes);
    if actual == expected {
        Check::pass("artifact-integrity")
    } else {
        Check::fail(
            "artifact-integrity",
            format!(
                "archive digest mismatch: {} != {}",
                short(&actual),
                short(expected)
            ),
        )
    }
}

fn short(digest: &str) -> String {
    match digest.split_once(':') {
        Some((algo, hex)) => format!("{algo}:{}", &hex[..hex.len().min(12)]),
        None => digest.to_string(),
    }
}

fn emit(id: &str, checks: &[Check], fmt: Format) {
    if fmt.is_json() {
        let items: Vec<_> = checks
            .iter()
            .map(|c| {
                serde_json::json!({
                    "name": c.name,
                    "status": c.status.word(),
                    "detail": c.detail,
                })
            })
            .collect();
        let ok = !checks.iter().any(|c| c.status == Status::Fail);
        output::report(
            ok,
            &serde_json::json!({
                "target": id,
                "checks": items,
            }),
        );
        return;
    }

    println!("Validating target {id}");
    for c in checks {
        match &c.detail {
            Some(d) => println!("  [{}] {} — {d}", c.status.mark(), c.name),
            None => println!("  [{}] {}", c.status.mark(), c.name),
        }
    }
    let failed = checks.iter().any(|c| c.status == Status::Fail);
    println!(
        "\n{}",
        if failed {
            "Result: FAILED"
        } else {
            "Result: passed"
        }
    );
}
