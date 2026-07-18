// SPDX-License-Identifier: Apache-2.0
//! `ost validate` — validate a built/packaged target (§8.5, §18.2).
//!
//! Structural checks over a target's build and (optional) artifact state:
//! configured, built, runtime-compatible, and — when packaged — archive
//! integrity. Deterministic exit for CI: `0` when all non-skipped checks pass,
//! `1` otherwise.

use clap::Args;

use camino::Utf8PathBuf;
use ost_build::{
    BuildCompletion, TargetLock, TestCompletion, BUILD_COMPLETION_FILE, TEST_COMPLETION_FILE,
};
use ost_core::paths::STATE_DIR;
use ost_core::{digest, Result};
use ost_manifest::{RendererCheckStatus, RendererManifest, RendererReport, RENDERER_MANIFEST};

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

    /// External/manual build tree whose evidence should be validated without
    /// claiming it was produced by `ost build`.
    #[arg(long)]
    build_dir: Option<Utf8PathBuf>,
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
    name: String,
    status: Status,
    detail: Option<String>,
}

impl Check {
    fn pass(name: impl Into<String>) -> Check {
        Check {
            name: name.into(),
            status: Status::Pass,
            detail: None,
        }
    }
    fn fail(name: impl Into<String>, detail: impl Into<String>) -> Check {
        Check {
            name: name.into(),
            status: Status::Fail,
            detail: Some(detail.into()),
        }
    }
    fn skip(name: impl Into<String>, detail: impl Into<String>) -> Check {
        Check {
            name: name.into(),
            status: Status::Skip,
            detail: Some(detail.into()),
        }
    }
}

pub fn run(args: ValidateArgs, fmt: Format) -> Result<()> {
    let (root, platform, profile) = resolve_selection(args.target, args.profile)?;
    let project = load_project(&root)?;
    let project_version = project.effective_version(&root)?;
    let (target, r) = build_target(&platform, &profile)?;
    let id = target.id();

    let mut checks = Vec::new();

    let external_build = args.build_dir.map(|path| {
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    });

    // 1. configured — target.lock.json exists and parses. An explicit external
    // tree is manual evidence, so this OST-managed claim is intentionally SKIP.
    let lock_path = root
        .join(STATE_DIR)
        .join("targets")
        .join(&id)
        .join("target.lock.json");
    let lock: Option<TargetLock> = std::fs::read_to_string(lock_path.as_std_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    match (&lock, &external_build) {
        (_, Some(build_dir)) => checks.push(Check::skip(
            "configured",
            format!("external/manual build selected at {build_dir}"),
        )),
        (Some(_), None) => checks.push(Check::pass("configured")),
        (None, None) => checks.push(Check::fail(
            "configured",
            "target.lock.json missing or invalid — run `ost configure`",
        )),
    }

    // 2. built — a build directory, cache, object, or copied renderer report is
    // not completion evidence. Only the atomic record written after configure,
    // build and output verification can satisfy this check.
    let relative_build_dir = Utf8PathBuf::from(format!("build/{id}"));
    let managed_build_dir = root.join(&relative_build_dir);
    let build_dir = external_build.as_ref().unwrap_or(&managed_build_dir);
    // The validated build record, kept so the `tested` check can bind against it.
    let mut built_completion: Option<BuildCompletion> = None;
    if external_build.is_some() {
        checks.push(Check::skip(
            "built",
            "external/manual evidence does not claim `ost build` completion",
        ));
        if build_dir.as_std_path().is_dir() {
            checks.push(Check::pass("external-build"));
        } else {
            checks.push(Check::fail(
                "external-build",
                format!("external build directory is missing: {build_dir}"),
            ));
        }
    } else {
        let completion_path = build_dir.join(BUILD_COMPLETION_FILE);
        let completion = std::fs::read_to_string(completion_path.as_std_path())
            .map_err(|error| error.to_string())
            .and_then(|source| {
                serde_json::from_str::<BuildCompletion>(&source).map_err(|error| error.to_string())
            });
        match (&lock, completion) {
            (Some(lock), Ok(completion)) => {
                match completion.validate_against(
                    lock,
                    &project.project.name,
                    &project_version,
                    &relative_build_dir,
                ) {
                    Ok(()) if build_dir.as_std_path().is_dir() => {
                        checks.push(Check::pass("built"));
                        // `tested` is only meaningful once `built` holds: a test
                        // record bound to a build that no longer validates
                        // describes binaries that are gone.
                        built_completion = Some(completion);
                    }
                    Ok(()) => checks.push(Check::fail(
                        "built",
                        format!("completed build directory is missing: {build_dir}"),
                    )),
                    Err(detail) => checks.push(Check::fail("built", detail)),
                }
            }
            (Some(_), Err(detail)) => checks.push(Check::fail(
                "built",
                format!(
                    "{} missing or invalid ({detail}) — run `ost build`",
                    completion_path
                ),
            )),
            (None, _) => checks.push(Check::fail(
                "built",
                "target is not configured — run `ost build`",
            )),
        }
    }

    // 2b. tested — a claim of its own. It is not implied by `built`, does not
    // follow from `packaged`, and is not the same as a host-side plugin or
    // renderer check: those exercise an installed artifact on a host, this says
    // the target's own test suite ran against the build recorded above.
    if external_build.is_some() {
        checks.push(Check::skip(
            "tested",
            "external/manual evidence does not claim an `ost test` run",
        ));
    } else {
        let test_path = build_dir.join(TEST_COMPLETION_FILE);
        match (
            &built_completion,
            std::fs::read_to_string(test_path.as_std_path()),
        ) {
            (Some(build), Ok(source)) => match serde_json::from_str::<TestCompletion>(&source) {
                Ok(tested) => match tested.validate_against(build) {
                    Ok(()) => checks.push(Check::pass("tested")),
                    Err(detail) => checks.push(Check::fail("tested", detail)),
                },
                Err(error) => checks.push(Check::fail(
                    "tested",
                    format!("{test_path} is invalid: {error}"),
                )),
            },
            (Some(_), Err(_)) => checks.push(Check::skip("tested", "not tested — run `ost test`")),
            (None, _) => checks.push(Check::skip("tested", "target is not built")),
        }
    }

    // Renderer-specific composition/evidence is additive to the generic target
    // lifecycle. Ordinary projects without openstrata.renderer.yaml keep the
    // exact existing checks.
    let renderer_manifest_path = root.join(RENDERER_MANIFEST);
    if renderer_manifest_path.as_std_path().is_file() {
        match RendererManifest::load(&root) {
            Ok(manifest) => {
                if manifest.renderer.name == project.project.name {
                    checks.push(Check::pass("renderer-manifest"));
                } else {
                    checks.push(Check::fail(
                        "renderer-manifest",
                        format!(
                            "renderer name '{}' does not match project name '{}'",
                            manifest.renderer.name, project.project.name
                        ),
                    ));
                }
                let report_path = manifest.report_path(build_dir);
                if report_path.as_std_path().is_file() {
                    match RendererReport::load(&report_path).and_then(|report| {
                        report.validate_against(&manifest)?;
                        Ok(report)
                    }) {
                        Ok(report) => {
                            checks.push(Check::pass("renderer-evidence"));
                            for renderer_check in report.checks {
                                // Name the producer behind the assertion. A
                                // merged report is several producers' evidence;
                                // presenting it as one anonymous verdict is how
                                // an unowned PASS goes unnoticed.
                                let detail = match (
                                    renderer_check.detail,
                                    renderer_check
                                        .producer
                                        .or_else(|| report.producer.as_ref().map(|s| s.id.clone())),
                                ) {
                                    (Some(detail), Some(producer)) => {
                                        Some(format!("{detail} (producer {producer})"))
                                    }
                                    (Some(detail), None) => Some(detail),
                                    (None, Some(producer)) => Some(format!("producer {producer}")),
                                    (None, None) => None,
                                };
                                checks.push(Check {
                                    name: renderer_check.id,
                                    status: match renderer_check.status {
                                        RendererCheckStatus::Pass => Status::Pass,
                                        RendererCheckStatus::Fail => Status::Fail,
                                        RendererCheckStatus::Skip => Status::Skip,
                                    },
                                    detail,
                                });
                            }
                        }
                        Err(error) => checks.push(Check::fail(
                            "renderer-evidence",
                            format!("{} is invalid: {error}", report_path),
                        )),
                    }
                } else {
                    checks.push(Check::skip(
                        "renderer-evidence",
                        format!(
                            "{} is absent — build the renderer on the target host",
                            report_path
                        ),
                    ));
                }
            }
            Err(error) => checks.push(Check::fail(
                "renderer-manifest",
                format!("{} is invalid: {error}", renderer_manifest_path),
            )),
        }
    }

    // 3. runtime-compatible — the runtime is pulled and its digest matches the
    //    one recorded at configure time (detects drift).
    if let Some(external) = &external_build {
        // An imported record can upgrade this check — but only on a *full*
        // identity match against the tree's own CMake cache and the runtime
        // resolved right now. Anything less stays a refusal: a partial match is
        // what makes an external claim dangerous, since a tree reconfigured
        // against a newer runtime looks identical except in the one place that
        // decides whether its binaries still load.
        checks.push(external_runtime_check(external, &target, &r));
    } else if !r.pulled {
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
        .join(&project_version)
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

/// Decide `runtime-compatible` for an external tree from its imported record.
///
/// With no record the check stays SKIP, exactly as before: an un-imported tree
/// makes no claim about any runtime, and inventing one from the directory's mere
/// existence is what this whole path is designed not to do.
fn external_runtime_check(
    build_dir: &camino::Utf8Path,
    target: &ost_build::Target,
    resolved: &crate::commands::Resolved,
) -> Check {
    let Ok(record) = crate::commands::external::read_provenance(build_dir) else {
        return Check::skip(
            "runtime-compatible",
            format!(
                "external build has no imported provenance — run \
                 `ost external import --build-dir {build_dir}`"
            ),
        );
    };
    let cache = match crate::commands::external::load_cache(build_dir) {
        Ok(cache) => cache,
        Err(error) => return Check::fail("runtime-compatible", error.to_string()),
    };
    let current = ost_build::ExternalRuntime {
        id: target.runtime_id.clone(),
        digest: target.runtime_digest.clone(),
        root: resolved.artifact_prefix.to_string().replace('\\', "/"),
    };
    match record.verify_against(&cache, build_dir, &current) {
        Ok(()) => Check {
            name: "runtime-compatible".into(),
            status: Status::Pass,
            detail: Some(record.describe()),
        },
        Err(detail) => Check::fail("runtime-compatible", detail),
    }
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
