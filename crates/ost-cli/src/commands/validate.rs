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
    BuildCompletion, RendererEvidenceBinding, TargetLock, TestCompletion, BUILD_COMPLETION_FILE,
    TEST_COMPLETION_FILE,
};
use ost_core::paths::STATE_DIR;
use ost_core::{digest, Error, Result};
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

    /// Validate the build produced for this project-declared intent.
    #[arg(long, conflicts_with = "build_dir")]
    intent: Option<String>,

    /// External/manual build tree whose evidence should be validated without
    /// claiming it was produced by `ost build`.
    #[arg(long)]
    build_dir: Option<Utf8PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    let intent = crate::commands::build::resolve_declared_intent(&root, args.intent.as_deref())?;

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
    let relative_build_dir = crate::commands::build::build_dir_for_intent(&id, &intent);
    let managed_build_dir = root.join(&relative_build_dir);
    let build_dir = external_build.as_ref().unwrap_or(&managed_build_dir);
    // The validated build record, kept so the `tested` check can bind against it.
    let mut built_completion: Option<BuildCompletion> = None;
    let mut tested_completion: Option<TestCompletion> = None;
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
                        match crate::commands::build::validate_completed_intent(
                            &completion.intent,
                            &intent,
                        ) {
                            Ok(()) => {
                                checks.push(Check::pass("built"));
                                // `tested` is only meaningful once `built` holds: a test
                                // record bound to a build that no longer validates
                                // describes binaries that are gone.
                                built_completion = Some(completion);
                            }
                            Err(detail) => checks.push(Check::fail("built", detail)),
                        }
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
                    Ok(()) => {
                        checks.push(Check::pass("tested"));
                        tested_completion = Some(tested);
                    }
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
                        verify_managed_renderer_binding(
                            &root,
                            &id,
                            build_dir,
                            &report_path,
                            &report,
                            built_completion.as_ref(),
                            tested_completion.as_ref(),
                        )
                        .map_err(Error::validation)?;
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
                checks.push(viewport_launch_check(&root, &id, build_dir));
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

fn viewport_launch_check(
    root: &camino::Utf8Path,
    target_id: &str,
    build_dir: &camino::Utf8Path,
) -> Check {
    let path = root
        .join(STATE_DIR)
        .join("renderer-viewport")
        .join(target_id)
        .join("launch.json");
    let source = match std::fs::read_to_string(path.as_std_path()) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Check::skip("renderer-viewport", "no durable viewport launch");
        }
        Err(error) => {
            return Check::fail(
                "renderer-viewport",
                format!("cannot read viewport launch record {path}: {error}"),
            );
        }
    };
    let value: serde_json::Value = match serde_json::from_str(&source) {
        Ok(value) => value,
        Err(error) => {
            return Check::fail("renderer-viewport", format!("{path} is invalid: {error}"));
        }
    };
    if value["schema"] != "openstrata.renderer-launch/v1" || value["kind"] != "renderer-viewport" {
        return Check::fail(
            "renderer-viewport",
            format!("{path} is not an openstrata.renderer-launch/v1 viewport record"),
        );
    }
    if value.pointer("/preflight/passed") != Some(&serde_json::Value::Bool(true))
        || value
            .pointer("/preflight/target")
            .and_then(|item| item.as_str())
            != Some(target_id)
    {
        return Check::fail(
            "renderer-viewport",
            "viewport launch preflight is absent, failed, or names another target",
        );
    }
    let bindings: Vec<RendererEvidenceBinding> = match serde_json::from_value(
        value
            .get("renderer_reports")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
    ) {
        Ok(bindings) => bindings,
        Err(error) => {
            return Check::fail(
                "renderer-viewport",
                format!("viewport renderer report bindings are invalid: {error}"),
            );
        }
    };
    for binding in bindings {
        let path_bytes = binding.path.as_bytes();
        if binding.path.is_empty()
            || binding.path.starts_with('/')
            || binding.path.starts_with('\\')
            || binding.path.contains('\\')
            || binding.path.split('/').any(|segment| segment == "..")
            || (path_bytes.len() >= 2
                && path_bytes[0].is_ascii_alphabetic()
                && path_bytes[1] == b':')
        {
            return Check::fail(
                "renderer-viewport",
                format!(
                    "viewport renderer report path '{}' is not build-directory-relative",
                    binding.path
                ),
            );
        }
        let report_path = build_dir.join(&binding.path);
        let bytes = match std::fs::read(report_path.as_std_path()) {
            Ok(bytes) => bytes,
            Err(error) => {
                return Check::fail(
                    "renderer-viewport",
                    format!("bound viewport report '{report_path}' is unavailable: {error}"),
                );
            }
        };
        let observed = digest::sha256_hex(&bytes);
        if observed != binding.sha256 {
            return Check::fail(
                "renderer-viewport",
                format!(
                    "bound viewport report '{}' digest {} != {}",
                    binding.path, observed, binding.sha256
                ),
            );
        }
    }
    match value.pointer("/exit/state").and_then(|item| item.as_str()) {
        Some("success")
            if value["outcome"] == "success"
                && value.pointer("/exit/code").and_then(|item| item.as_i64()) == Some(0)
                && value.pointer("/readiness/reached") == Some(&serde_json::Value::Bool(true)) =>
        {
            Check::pass("renderer-viewport")
        }
        Some("presentation-unavailable") => Check::skip(
            "renderer-viewport",
            "viewport completed but this host cannot present",
        ),
        Some(state @ ("build-failure" | "child-failure")) => Check::fail(
            "renderer-viewport",
            format!("last durable viewport launch ended in {state}"),
        ),
        Some(state) => Check::fail(
            "renderer-viewport",
            format!("viewport launch has unsupported exit state '{state}'"),
        ),
        None => Check::fail("renderer-viewport", "viewport launch has no exit state"),
    }
}

fn verify_managed_renderer_binding(
    root: &camino::Utf8Path,
    target_id: &str,
    build_dir: &camino::Utf8Path,
    report_path: &camino::Utf8Path,
    report: &RendererReport,
    build: Option<&BuildCompletion>,
    tested: Option<&TestCompletion>,
) -> std::result::Result<(), String> {
    let Some(producer) = report.producer.as_ref() else {
        return Ok(());
    };
    if !producer.kind.starts_with("ost-") {
        // External producers remain explicitly unverified. Their PASS contract
        // is governed by the producer outcome, not an OST completion record.
        return Ok(());
    }

    let bindings = match producer.kind.as_str() {
        "ost-build" => build
            .map(|completion| completion.renderer_reports.clone())
            .unwrap_or_default(),
        "ost-test" => tested
            .map(|completion| completion.renderer_reports.clone())
            .unwrap_or_default(),
        "ost-renderer-viewport" => {
            let path = root
                .join(STATE_DIR)
                .join("renderer-viewport")
                .join(target_id)
                .join("launch.json");
            let source = std::fs::read_to_string(path.as_std_path()).map_err(|error| {
                format!(
                    "managed producer '{}' has no durable viewport record at {} ({error})",
                    producer.id, path
                )
            })?;
            let value: serde_json::Value = serde_json::from_str(&source)
                .map_err(|error| format!("{path} is invalid: {error}"))?;
            serde_json::from_value(
                value
                    .get("renderer_reports")
                    .cloned()
                    .unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
            )
            .map_err(|error| format!("{path} renderer_reports are invalid: {error}"))?
        }
        kind => {
            return Err(format!(
                "managed renderer producer kind '{kind}' has no supported completion binding"
            ))
        }
    };

    let relative = report_path
        .strip_prefix(build_dir)
        .map_err(|_| {
            format!("renderer report '{report_path}' is outside build directory '{build_dir}'")
        })?
        .as_str()
        .replace('\\', "/");
    let binding = bindings
        .iter()
        .find(|binding: &&RendererEvidenceBinding| {
            binding.path == relative && binding.session == producer.id
        })
        .ok_or_else(|| {
            format!(
                "managed producer '{}' does not bind renderer report '{}' in its completion evidence",
                producer.id, relative
            )
        })?;
    let bytes = std::fs::read(report_path.as_std_path())
        .map_err(|error| format!("cannot digest renderer report '{report_path}': {error}"))?;
    let observed = digest::sha256_hex(&bytes);
    if observed != binding.sha256 {
        return Err(format!(
            "renderer report '{}' digest {} does not match managed producer '{}' completion digest {}",
            relative, observed, producer.id, binding.sha256
        ));
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
    // Verify the suggested import can actually inspect this tree before
    // recommending it. A path with no CMake cache needs configuration, not a
    // circular `external import` instruction.
    let cache = match crate::commands::external::load_cache(build_dir) {
        Ok(cache) => cache,
        Err(error) => {
            let detail = match error.hint() {
                Some(hint) => format!("{error} — {hint}"),
                None => error.to_string(),
            };
            return Check::skip("runtime-compatible", detail);
        }
    };
    let Ok(record) = crate::commands::external::read_provenance(build_dir) else {
        return Check::skip(
            "runtime-compatible",
            format!(
                "external build has no imported provenance — run \
                 `ost external import --build-dir {build_dir} --target {} --profile {}`",
                target.platform, target.profile
            ),
        );
    };
    if !record.scope.profile.is_empty() && record.scope.profile != target.profile {
        return Check::fail(
            "runtime-compatible",
            format!(
                "provenance was imported for profile '{}', not selected profile '{}'",
                record.scope.profile, target.profile
            ),
        );
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_viewport_exit_states_are_validation_evidence() {
        let root = std::env::temp_dir().join(format!(
            "ost-viewport-validate-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = camino::Utf8PathBuf::from_path_buf(root).unwrap();
        let target = "cy2026-windows-x86_64-py313-usd";
        let build = root.join("build").join(target);
        let record_path = root
            .join(STATE_DIR)
            .join("renderer-viewport")
            .join(target)
            .join("launch.json");
        std::fs::create_dir_all(record_path.parent().unwrap().as_std_path()).unwrap();
        std::fs::create_dir_all(build.as_std_path()).unwrap();
        let mut record = serde_json::json!({
            "schema": "openstrata.renderer-launch/v1",
            "kind": "renderer-viewport",
            "preflight": {"passed": true, "target": target},
            "renderer_reports": [],
            "outcome": "success",
            "readiness": {"reached": true},
            "exit": {"state": "success", "code": 0},
        });
        std::fs::write(
            record_path.as_std_path(),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        assert_eq!(
            viewport_launch_check(&root, target, &build).status,
            Status::Pass
        );

        record["outcome"] = "failure".into();
        record["readiness"]["reached"] = false.into();
        record["exit"] = serde_json::json!({"state": "child-failure", "code": 1});
        std::fs::write(
            record_path.as_std_path(),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        assert_eq!(
            viewport_launch_check(&root, target, &build).status,
            Status::Fail
        );
        std::fs::remove_dir_all(root.as_std_path()).unwrap();
    }
}
