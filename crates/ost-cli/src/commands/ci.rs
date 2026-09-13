// SPDX-License-Identifier: Apache-2.0
//! `ost ci` — the CI support matrix (Phase 5 MVP).
//!
//! - `init`     scaffold a commented `openstrata.ci.yaml` starter matrix.
//! - `validate` structural checks; `--resolve` additionally requires every
//!   pinned digest to exist in the local artifact registry.
//! - `plan`     preflight execution facts (lanes, runners, billing) without
//!   rendering or estimating money.
//! - `generate github` render the matrix into workflows, one per lane family.
//!
//! The matrix is the single source of truth; generated workflows carry a
//! "regenerate, don't edit" banner. Jenkins generation lands later on the same
//! model.

use camino::{Utf8Path, Utf8PathBuf};
use clap::Subcommand;

use ost_artifact::{ArtifactKind, ArtifactStore};
use ost_ci::{
    generate_github, starter_matrix, Lane, Publish, RequireEvidence, SupportDeclaration,
    SupportMatrix, MATRIX_FILE, RELEASE_WORKFLOW_PATH, SOURCE_WORKFLOW_PATH, WORKFLOW_PATH,
};
use ost_core::{Error, Result};

use crate::output::{self, Format};

#[derive(Debug, Subcommand)]
pub enum CiCmd {
    /// Write a starter openstrata.ci.yaml support matrix.
    Init {
        /// Directory to write into. Defaults to the current directory.
        #[arg(long)]
        dir: Option<String>,
    },
    /// Validate the support matrix.
    Validate {
        /// Path to the matrix file. Defaults to ./openstrata.ci.yaml.
        #[arg(long)]
        matrix: Option<String>,
        /// Also require every pinned digest to resolve in the local registry.
        #[arg(long)]
        resolve: bool,
        /// Check hosted cells against a public support/platform declaration.
        #[arg(long)]
        support: Option<String>,
    },
    /// Report preflight execution facts (lanes, runners, billing).
    Plan {
        /// Path to the matrix file. Defaults to ./openstrata.ci.yaml.
        #[arg(long)]
        matrix: Option<String>,
    },
    /// Emit resolved cells for an externally managed workflow without copying pins.
    Matrix {
        /// Path to the matrix file. Defaults to ./openstrata.ci.yaml.
        #[arg(long)]
        matrix: Option<String>,
        /// Only cells in this lane (pull_request | main | scheduled |
        /// workflow_dispatch). All lanes when omitted.
        #[arg(long)]
        lane: Option<String>,
    },
    /// Generate CI configuration from the support matrix.
    #[command(subcommand)]
    Generate(GenerateCmd),
}

#[derive(Debug, Subcommand)]
pub enum GenerateCmd {
    /// Emit a GitHub Actions workflow (one job per support cell).
    Github {
        /// Path to the matrix file. Defaults to ./openstrata.ci.yaml.
        #[arg(long)]
        matrix: Option<String>,
        /// Output path. Defaults to .github/workflows/ost-support-matrix.yml.
        #[arg(long)]
        out: Option<String>,
        /// Overwrite an existing workflow file.
        #[arg(long)]
        force: bool,
        /// Print the workflow to stdout instead of writing a file.
        #[arg(long)]
        stdout: bool,
        /// Render even if cells still carry the scaffold's all-zero
        /// placeholder digests.
        #[arg(long)]
        allow_placeholders: bool,
        /// Refuse generation when hosted claims exceed this support declaration.
        #[arg(long)]
        support: Option<String>,
    },
}

pub fn run(cmd: CiCmd, fmt: Format) -> Result<()> {
    match cmd {
        CiCmd::Init { dir } => init(dir.as_deref(), fmt),
        CiCmd::Validate {
            matrix,
            resolve,
            support,
        } => validate(matrix.as_deref(), resolve, support.as_deref(), fmt),
        CiCmd::Plan { matrix } => plan(matrix.as_deref(), fmt),
        CiCmd::Matrix { matrix, lane } => resolved_matrix(matrix.as_deref(), lane.as_deref(), fmt),
        CiCmd::Generate(GenerateCmd::Github {
            matrix,
            out,
            force,
            stdout,
            allow_placeholders,
            support,
        }) => generate(
            matrix.as_deref(),
            out.as_deref(),
            force,
            stdout,
            allow_placeholders,
            support.as_deref(),
            fmt,
        ),
    }
}

fn matrix_path(flag: Option<&str>) -> Utf8PathBuf {
    Utf8PathBuf::from(flag.unwrap_or(MATRIX_FILE))
}

fn load_matrix(flag: Option<&str>) -> Result<(Utf8PathBuf, SupportMatrix)> {
    let path = matrix_path(flag);
    if !path.as_std_path().is_file() {
        return Err(
            Error::precondition(format!("no support matrix at '{path}'"))
                .with_hint("scaffold one with `ost ci init`"),
        );
    }
    let src =
        std::fs::read_to_string(path.as_std_path()).map_err(|e| Error::io(path.to_string(), e))?;
    let matrix = SupportMatrix::from_yaml(&src)?;
    Ok((path, matrix))
}

/// Default OST-owned workflow files that still exist even though the current
/// matrix no longer renders their lane family.
///
/// Generation deliberately does not delete these files: a repository may have
/// reviewed or temporarily edited one, and removing it as a side effect would
/// be too aggressive. The generated banner is the ownership signal, so a
/// hand-authored file at one of the default paths is never diagnosed as stale.
fn stale_generated_workflows(matrix_path: &Utf8Path, matrix: &SupportMatrix) -> Vec<String> {
    const GENERATED_BANNER: &str =
        "# Generated by `ost ci generate github` from openstrata.ci.yaml.";
    // The generated banner identifies only the canonical matrix. An arbitrary
    // `--matrix candidate.yaml` cannot prove ownership of workflow files next
    // to it, so diagnosing those could tell a maintainer to delete a workflow
    // that the real openstrata.ci.yaml still emits. Canonical paths keep
    // equivalent spellings such as `./openstrata.ci.yaml` working.
    let selected = std::fs::canonicalize(matrix_path.as_std_path());
    let canonical = std::fs::canonicalize(Utf8Path::new(MATRIX_FILE).as_std_path());
    if !matches!((selected, canonical), (Ok(selected), Ok(canonical)) if selected == canonical) {
        return Vec::new();
    }
    let emitted = generate_github(matrix);
    [WORKFLOW_PATH, SOURCE_WORKFLOW_PATH, RELEASE_WORKFLOW_PATH]
        .into_iter()
        .filter(|path| !emitted.iter().any(|workflow| workflow.path == *path))
        .filter(|path| {
            std::fs::read_to_string(path).is_ok_and(|source| source.starts_with(GENERATED_BANNER))
        })
        .map(str::to_string)
        .collect()
}

fn stale_workflow_message(path: &str) -> String {
    format!(
        "{path} is OST-generated but is no longer emitted by the current matrix; \
         it may still have an on: trigger — remove it or re-add a matching lane cell"
    )
}

fn init(dir: Option<&str>, fmt: Format) -> Result<()> {
    let dir = Utf8PathBuf::from(dir.unwrap_or("."));
    let path = dir.join(MATRIX_FILE);
    if path.as_std_path().exists() {
        return Err(Error::usage(format!(
            "'{path}' already exists — edit it, or remove it to re-scaffold"
        )));
    }
    std::fs::create_dir_all(dir.as_std_path()).map_err(|e| Error::io(dir.to_string(), e))?;
    std::fs::write(path.as_std_path(), starter_matrix())
        .map_err(|e| Error::io(path.to_string(), e))?;

    if fmt.is_json() {
        output::success(&serde_json::json!({
            "created": true,
            "matrix": path.to_string(),
        }));
        return Ok(());
    }
    println!("Wrote {path}");
    println!("  1. publish artifacts:  ost runtime export … / ost plugin publish …");
    println!("  2. pin their digests in the matrix cells");
    println!("  3. validate:           ost ci validate --resolve");
    println!("  4. generate CI:        ost ci generate github");
    Ok(())
}

/// Pinned artifacts whose stored evidence cannot satisfy the gate the matrix
/// will render for them.
///
/// v0.17.0 shipped a gate no existing artifact could pass and gave no warning
/// at generate time, so the first sign of trouble was a red hosted lane with
/// nothing wrong in the repo. `ost ci generate` warns on these and
/// `ost ci validate` fails fast, so the mismatch is caught locally.
///
/// Digests absent from the local registry are not reported here: that is
/// `--resolve`'s job, and an unknown digest says nothing about its evidence.
fn evidence_gate_gaps(matrix: &SupportMatrix) -> Vec<String> {
    let store = ArtifactStore::discover();
    let mut gaps = Vec::new();
    for cell in &matrix.cells {
        let demand = matrix.require_evidence(cell);
        if demand == RequireEvidence::None {
            continue;
        }
        let mut refs: Vec<(&str, &str)> = Vec::new();
        if let Some(runtime) = &cell.runtime_artifact {
            refs.push(("runtime_artifact", runtime));
        }
        if let Some(plugin) = &cell.plugin_artifact {
            refs.push(("plugin_artifact", plugin));
        }
        for (what, digest) in refs {
            let Ok(record) = store.resolve(digest) else {
                continue;
            };
            let mut missing = Vec::new();
            if demand.requires_sbom() && record.sbom.is_none() {
                missing.push("SBOM");
            }
            if demand.requires_provenance() && record.provenance.is_none() {
                missing.push("provenance");
            }
            if !missing.is_empty() {
                let recovery = if what == "runtime_artifact" {
                    cell.runtime_remote
                        .as_ref()
                        .map_or_else(String::new, |remote| {
                            let openusd = cell.require_openusd.as_ref().map_or_else(
                                String::new,
                                |selector| format!(" --require-openusd {selector}"),
                            );
                            let version = cell.require_openusd_version.as_ref().map_or_else(
                                String::new,
                                |version| format!(" --require-openusd-version {version}"),
                            );
                            format!(
                                "; recover without changing the pinned artifact identity: \
                             `ost artifact pull {} --expect-artifact {} --require-kind runtime{openusd}{version}`",
                                remote.uri, digest,
                            )
                        })
                } else {
                    String::new()
                };
                gaps.push(format!(
                    "{}: {what} {} has no {} but require_evidence is '{}'{}",
                    cell.name,
                    record.short_digest(),
                    missing.join(" or "),
                    demand.as_str(),
                    recovery,
                ));
            }
        }
    }
    gaps
}

/// The two ways out of an evidence gap, in the order a reader should try them.
const EVIDENCE_GAP_HINT: &str = "re-import the dist directory to attach its sidecars \
     (`ost artifact rm <digest>` first if it predates evidence), or lower \
     `require_evidence` while the artifact is republished";

/// Source cells must provision every native package declared by their pinned
/// runtime before the generated runtime bootstrap runs. The artifact is the
/// authority; `host_packages` is its lossless CI rendering.
fn host_requirement_gaps(matrix: &SupportMatrix) -> Vec<String> {
    let store = ArtifactStore::discover();
    let mut gaps = Vec::new();
    for cell in matrix.cells.iter().filter(|cell| cell.lane.is_source()) {
        let Some(runtime) = cell.runtime_artifact.as_deref() else {
            continue;
        };
        let Ok(record) = store.resolve(runtime) else {
            continue;
        };
        let requirements =
            match crate::commands::runtime::artifact_host_requirements(&store, &record) {
                Ok(requirements) => requirements,
                Err(error) => {
                    gaps.push(format!(
                        "{}: runtime {} carries invalid host_requirements ({error})",
                        cell.name,
                        record.short_digest()
                    ));
                    continue;
                }
            };
        for requirement in requirements {
            let declared = match requirement.manager {
                ost_runtime::HostPackageManager::Apt => cell
                    .host_packages
                    .as_ref()
                    .is_some_and(|packages| packages.apt.contains(&requirement.name)),
                ost_runtime::HostPackageManager::Brew => cell
                    .host_packages
                    .as_ref()
                    .is_some_and(|packages| packages.brew.contains(&requirement.name)),
            };
            if !declared {
                gaps.push(format!(
                    "{}: runtime {} requires {}:{}, but the source cell does not render it under host_packages.{}",
                    cell.name,
                    record.short_digest(),
                    requirement.manager.as_str(),
                    requirement.name,
                    requirement.manager.as_str()
                ));
            }
        }
    }
    gaps
}

const HOST_REQUIREMENT_GAP_HINT: &str = "copy each pinned runtime requirement into the \
     source cell's host_packages block, then regenerate the workflow";

fn validate(
    matrix_flag: Option<&str>,
    resolve: bool,
    support_flag: Option<&str>,
    fmt: Format,
) -> Result<()> {
    let (path, matrix) = load_matrix(matrix_flag)?;

    let (support_path, support_issues) = match support_flag {
        Some(flag) => {
            let path = Utf8PathBuf::from(flag);
            let src = std::fs::read_to_string(path.as_std_path())
                .map_err(|error| Error::io(path.to_string(), error))?;
            let declaration = SupportDeclaration::from_toml(&src)?;
            (Some(path), declaration.matrix_issues(&matrix))
        }
        None => (None, Vec::new()),
    };

    // `--resolve`: every pinned digest must exist in the local registry, so a
    // matrix can be gated before the runners ever see it.
    let mut unresolved: Vec<String> = Vec::new();
    if resolve {
        let store = ArtifactStore::discover();
        for cell in &matrix.cells {
            let mut refs: Vec<(&str, &str, ArtifactKind)> = Vec::new();
            if let Some(runtime) = &cell.runtime_artifact {
                refs.push(("runtime", runtime, ArtifactKind::Runtime));
            }
            if let Some(plugin) = &cell.plugin_artifact {
                refs.push(("plugin", plugin, ArtifactKind::Plugin));
            }
            for (what, digest, expected) in refs {
                match store.resolve(digest) {
                    Ok(record) if record.kind != expected => unresolved.push(format!(
                        "{}: {what} {digest} is a {} artifact, expected {}",
                        cell.name,
                        record.kind.as_str(),
                        expected.as_str()
                    )),
                    Ok(record) => {
                        if what == "runtime" {
                            if let Some(selector) = &cell.require_openusd {
                                let required = ost_platform::resolve_openusd_requirement(selector)?;
                                if let Err(error) = ost_artifact::verify_openusd_requirement(
                                    &record,
                                    &required,
                                    cell.require_openusd_version.as_deref(),
                                ) {
                                    unresolved.push(format!(
                                        "{}: runtime {digest} does not satisfy require_openusd '{selector}': {error}",
                                        cell.name
                                    ));
                                }
                            }
                        }
                    }
                    Err(_) => unresolved.push(format!("{}: {what} {digest}", cell.name)),
                }
            }
        }
    }

    // Placeholder digests are structurally valid, so syntax-only validation
    // used to accept the untouched scaffold in silence — easy to mistake for a
    // usable matrix (dogfooding report #8). Warn without failing.
    let placeholders = matrix.placeholder_digests();

    // Hosted-runner billing: warn while a referenced github-hosted profile
    // lacks `billing.acknowledgement: required`; a publish-capable cell on
    // such a profile is an error (metered infrastructure must be opted into
    // before CI can upload anything).
    let ack_missing = matrix.hosted_ack_missing();
    let ack_errors = matrix.hosted_ack_errors();

    // A gate the pinned artifacts cannot satisfy is a validation failure, not a
    // warning: shipping it means a red lane on every runner.
    let evidence_gaps = evidence_gate_gaps(&matrix);
    let host_requirement_gaps = host_requirement_gaps(&matrix);
    let stale_workflows = stale_generated_workflows(&path, &matrix);

    let ok = unresolved.is_empty()
        && ack_errors.is_empty()
        && support_issues.is_empty()
        && evidence_gaps.is_empty()
        && host_requirement_gaps.is_empty();
    if fmt.is_json() {
        let mut warnings: Vec<serde_json::Value> = placeholders
            .iter()
            .map(|hit| {
                serde_json::json!({
                    "code": "CI_PLACEHOLDER_DIGEST",
                    "message": format!("placeholder digest — {hit}"),
                })
            })
            .collect();
        warnings.extend(ack_missing.iter().map(|name| {
            serde_json::json!({
                "code": "CI_HOSTED_BILLING_UNACKNOWLEDGED",
                "message": format!(
                    "GitHub-hosted runner '{name}' may incur billable usage — set \
                     runners.{name}.billing.acknowledgement: required"
                ),
            })
        }));
        warnings.extend(stale_workflows.iter().map(|path| {
            serde_json::json!({
                "code": "CI_STALE_GENERATED_WORKFLOW",
                "message": stale_workflow_message(path),
            })
        }));
        output::report_with_warnings(
            ok,
            &serde_json::json!({
                "matrix": path.to_string(),
                "cells": matrix.cells.len(),
                "resolved": resolve,
                "support_declaration": support_path.as_ref().map(ToString::to_string),
                "support_issues": support_issues,
                "unresolved": unresolved,
                "placeholders": placeholders,
                "hosted_unacknowledged": ack_missing,
                "billing_errors": ack_errors,
                "evidence_gaps": evidence_gaps,
                "host_requirement_gaps": host_requirement_gaps,
                "stale_workflows": stale_workflows,
            }),
            &warnings,
        );
    } else {
        println!(
            "Matrix {path}: {} cell(s), structure OK",
            matrix.cells.len()
        );
        for hit in &placeholders {
            println!("  WARNING: placeholder digest — {hit}");
        }
        if !placeholders.is_empty() {
            println!("  pin real digests: `ost runtime export`, `ost plugin publish`");
        }
        for name in &ack_missing {
            println!("  WARNING: GitHub-hosted runner '{name}' may incur billable usage");
            println!("           set runners.{name}.billing.acknowledgement: required");
        }
        for stale in &stale_workflows {
            println!("  WARNING: {}", stale_workflow_message(stale));
        }
        for hit in &ack_errors {
            println!("  ERROR: publish-capable cell needs billing acknowledgement — {hit}");
        }
        for miss in &unresolved {
            println!("  UNRESOLVED: {miss}");
        }
        for issue in &support_issues {
            println!("  UNSUPPORTED: {issue}");
        }
        for gap in &evidence_gaps {
            println!("  EVIDENCE GAP: {gap}");
        }
        if !evidence_gaps.is_empty() {
            println!("  {EVIDENCE_GAP_HINT}");
        }
        for gap in &host_requirement_gaps {
            println!("  HOST REQUIREMENT GAP: {gap}");
        }
        if !host_requirement_gaps.is_empty() {
            println!("  {HOST_REQUIREMENT_GAP_HINT}");
        }
        if let Some(support_path) = &support_path {
            if support_issues.is_empty() {
                println!("  public support claims agree with {support_path}");
            }
        }
        if resolve && ok {
            println!("  all pinned digests resolve in the local registry");
        }
    }
    if !ok {
        // The report above is this command's single document (§14.3).
        std::process::exit(ost_core::Category::Validation.exit_code() as i32);
    }
    Ok(())
}

/// `ost ci plan` — cost/trust/execution facts a maintainer can preflight
/// before generating workflows. Facts only: counts, referenced runner
/// classes, and whether billing acknowledgement is still missing — never a
/// currency estimate (billing depends on plan/visibility/runner size).
fn plan(matrix_flag: Option<&str>, fmt: Format) -> Result<()> {
    let (path, matrix) = load_matrix(matrix_flag)?;
    let stale_workflows = stale_generated_workflows(&path, &matrix);

    // Referenced runner classes, partitioned by kind (deterministic order:
    // first reference wins, cells are ordered). Named runner profiles report
    // their profile name; legacy `host` cells report the rendered runs-on
    // labels so `plan` stays faithful to generated workflows.
    let mut metered: Vec<String> = Vec::new();
    let mut operator: Vec<String> = Vec::new();
    let push_unique = |list: &mut Vec<String>, value: String| {
        if !list.iter().any(|n| n == &value) {
            list.push(value);
        }
    };
    for cell in &matrix.cells {
        if let Some(name) = cell.runner.as_deref() {
            let Some(profile) = matrix.runners.get(name) else {
                continue;
            };
            let list = if profile.is_hosted() {
                &mut metered
            } else {
                &mut operator
            };
            push_unique(list, name.to_string());
        } else if cell.host.labels.is_empty() {
            push_unique(&mut metered, cell.host.runs_on().join(", "));
        } else {
            push_unique(&mut operator, cell.host.runs_on().join(", "));
        }
    }

    // Draft mode renders no publisher job; only publish mode adds its runner.
    if let Some(name) = matrix
        .release
        .as_ref()
        .filter(|release| release.mode == ost_ci::ReleaseMode::Publish)
        .and_then(|release| release.publisher_runner.as_deref())
    {
        if let Some(profile) = matrix.runners.get(name) {
            let list = if profile.is_hosted() {
                &mut metered
            } else {
                &mut operator
            };
            push_unique(list, name.to_string());
        }
    }

    let hosted_jobs = matrix.cells.iter().filter(|c| matrix.is_hosted(c)).count()
        + matrix
            .release
            .as_ref()
            .filter(|release| {
                release.mode == ost_ci::ReleaseMode::Publish
                    && release.publisher_runner.as_deref().is_some_and(|name| {
                        matrix
                            .runners
                            .get(name)
                            .is_some_and(|profile| profile.is_hosted())
                    })
            })
            .map_or(0, |_| 1);
    let publish_capable = matrix
        .cells
        .iter()
        .filter(|c| c.publish != Publish::Never)
        .count();
    // Remote transport facts (v0.9.0): which cells pull their runtime from a
    // remote registry, and which pinned `ost` the hosted bootstrap installs.
    let remote_runtime_cells: Vec<&str> = matrix
        .cells
        .iter()
        .filter(|c| c.runtime_remote.is_some())
        .map(|c| c.name.as_str())
        .collect();
    let air_gapped_source_cells: Vec<&str> = matrix
        .cells
        .iter()
        .filter(|c| c.lane.is_source() && c.runtime_remote.is_none())
        .map(|c| c.name.as_str())
        .collect();
    let bootstrap = matrix.bootstrap.as_ref().map(|b| {
        serde_json::json!({
            "ost_version": b.ost.version,
            "repository": b.ost.repository,
            "sha256_pinned_targets": b.ost.sha256.keys().collect::<Vec<_>>(),
        })
    });
    let hosted_unacknowledged = matrix.hosted_ack_missing();
    let lane_count = |lane: Lane| matrix.cells.iter().filter(|c| c.lane == lane).count();
    let lanes = serde_json::json!({
        "pull_request": lane_count(Lane::PullRequest),
        "main": lane_count(Lane::Main),
        "scheduled": lane_count(Lane::Scheduled),
        "workflow_dispatch": lane_count(Lane::WorkflowDispatch),
    });
    let mut workflows: Vec<&str> = Vec::new();
    if !matrix.support_cells().is_empty() {
        workflows.push(WORKFLOW_PATH);
    }
    if !matrix.source_cells().is_empty() {
        workflows.push(SOURCE_WORKFLOW_PATH);
    }
    if matrix.release.is_some() {
        workflows.push(RELEASE_WORKFLOW_PATH);
    }
    let trust_cells = matrix
        .cells
        .iter()
        .map(|cell| {
            serde_json::json!({
                "name": cell.name,
                "target": cell.trust,
                "effective_minimum": matrix.minimum_trust(cell),
            })
        })
        .collect::<Vec<_>>();
    let trust = serde_json::json!({
        "policy": matrix.trust.policy.as_deref(),
        "pr_min_trust": matrix.trust.pr_min_trust,
        "main_min_trust": matrix.trust.main_min_trust,
        "release_min_trust": matrix.trust.release_min_trust,
        "cells": trust_cells,
    });
    let release = matrix.release.as_ref().map(|release| {
        serde_json::json!({
            "version": release.version,
            "mode": release.mode,
            "destination": release.destination,
            "publisher_runner": release.publisher_runner,
            "environment": release.environment,
            "reproducible": release.reproducible,
            "from_package": release.from_package,
            "candidate_cells": matrix.candidate_cells().iter().map(|cell| cell.name.as_str()).collect::<Vec<_>>(),
        })
    });

    if fmt.is_json() {
        let warnings = stale_workflows
            .iter()
            .map(|path| {
                serde_json::json!({
                    "code": "CI_STALE_GENERATED_WORKFLOW",
                    "message": stale_workflow_message(path),
                })
            })
            .collect::<Vec<_>>();
        output::report_with_warnings(
            true,
            &serde_json::json!({
                "matrix": path.to_string(),
                "cells": matrix.cells.len(),
                "lanes": lanes,
                "workflows": workflows,
                "stale_workflows": stale_workflows,
                "hosted_jobs": hosted_jobs,
                "metered_runner_classes": metered,
                "operator_managed_runner_classes": operator,
                "hosted_unacknowledged": hosted_unacknowledged,
                "requires_billing_acknowledgement": !hosted_unacknowledged.is_empty(),
                "publish_capable_jobs": publish_capable,
                "trust": trust,
                "release": release,
                "bootstrap": bootstrap,
                "remote_runtime_cells": remote_runtime_cells,
                "air_gapped_source_cells": air_gapped_source_cells,
            }),
            &warnings,
        );
        return Ok(());
    }

    println!("Plan for {path}: {} cell(s)", matrix.cells.len());
    println!(
        "  lanes:            pull_request {}, main {}, scheduled {}, workflow_dispatch {}",
        lane_count(Lane::PullRequest),
        lane_count(Lane::Main),
        lane_count(Lane::Scheduled),
        lane_count(Lane::WorkflowDispatch),
    );
    println!("  workflows:        {}", workflows.join(", "));
    for stale in &stale_workflows {
        println!("  WARNING: {}", stale_workflow_message(stale));
    }
    println!("  hosted jobs:      {hosted_jobs} (metered classes: {})", {
        if metered.is_empty() {
            "none".to_string()
        } else {
            metered.join(", ")
        }
    });
    println!(
        "  operator-managed: {}",
        if operator.is_empty() {
            "none".to_string()
        } else {
            operator.join(", ")
        }
    );
    println!("  publish-capable:  {publish_capable} job(s)");
    if let Some(release) = &matrix.release {
        println!(
            "  release:          v{} ({:?}, {} candidate cell(s))",
            release.version,
            release.mode,
            matrix.candidate_cells().len()
        );
    }
    println!(
        "  trust floors:     PR {}, main {}, release {} (policy: {})",
        matrix.trust.pr_min_trust,
        matrix.trust.main_min_trust,
        matrix.trust.release_min_trust,
        matrix.trust.policy.as_deref().unwrap_or("none")
    );
    match &matrix.bootstrap {
        Some(b) => println!(
            "  bootstrap:        ost {} from {} ({} exact-byte pin(s))",
            b.ost.version,
            b.ost.repository,
            b.ost.sha256.len()
        ),
        None => println!("  bootstrap:        none (runners provide their own ost)"),
    }
    println!(
        "  remote runtime:   {}",
        if remote_runtime_cells.is_empty() {
            "no cells pull from a remote registry".to_string()
        } else {
            remote_runtime_cells.join(", ")
        }
    );
    if !air_gapped_source_cells.is_empty() {
        println!(
            "  air-gapped source: {} (runtime comes from the runner's local registry)",
            air_gapped_source_cells.join(", ")
        );
    }
    if !hosted_unacknowledged.is_empty() {
        println!(
            "  NOTE: billing acknowledgement missing for: {}",
            hosted_unacknowledged.join(", ")
        );
        println!("        set runners.<name>.billing.acknowledgement: required");
    }
    Ok(())
}

/// Parse a `--lane` value into a [`Lane`], naming the accepted set on a typo
/// rather than silently matching nothing. The mapping itself lives beside
/// `Lane::as_str` so the two cannot disagree.
fn parse_lane(value: &str) -> Result<Lane> {
    Lane::parse(value).ok_or_else(|| {
        let accepted = Lane::ALL
            .iter()
            .map(|lane| lane.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Error::usage(format!(
            "unknown lane '{value}' — expected one of: {accepted}"
        ))
    })
}

/// `ost ci matrix` — emit the resolved cells as data.
///
/// Generated CI now covers bundle jobs and runtime-backed/runtime-free workspace
/// build intents. Repositories may still own specialized lanes outside that
/// typed contract. Such a lane must not copy runtime digest pins out of
/// `openstrata.ci.yaml`: two places that need re-pinning on every runtime
/// republish will drift, and a lane silently left on an older OpenUSD looks like
/// coverage while proving nothing (report 32 §2).
///
/// This is read-only projection: the matrix stays the single source of truth and
/// a hand-written lane consumes it instead of duplicating it.
fn resolved_matrix(matrix_flag: Option<&str>, lane_flag: Option<&str>, fmt: Format) -> Result<()> {
    let (path, matrix) = load_matrix(matrix_flag)?;
    let lane = lane_flag.map(parse_lane).transpose()?;
    let cells: Vec<&ost_ci::SupportCell> = matrix
        .cells
        .iter()
        .filter(|c| lane.is_none_or(|l| c.lane == l))
        .collect();

    if fmt.is_json() {
        let rendered: Vec<serde_json::Value> = cells
            .iter()
            .map(|cell| {
                serde_json::json!({
                    "name": cell.name,
                    "lane": cell.lane.as_str(),
                    "runner": cell.runner,
                    "runs_on": matrix.runs_on(cell),
                    "hosted": matrix.is_hosted(cell),
                    "os": matrix.resolved_os(cell).map(|os| os.as_str()),
                    "platform": cell.platform,
                    "profile": cell.profile,
                    "kind": cell.kind.as_str(),
                    // A workspace cell addresses no bundle. Its upper rungs do
                    // reuse `up_to` for the per-member plugin pyramid.
                    "bundle": (!cell.is_workspace())
                        .then(|| cell.bundle.as_deref().unwrap_or(".")),
                    "up_to": (!cell.is_workspace()
                        || matches!(
                            cell.verify(),
                            ost_ci::WorkspaceVerify::Pyramid
                                | ost_ci::WorkspaceVerify::Package
                        ))
                    .then(|| cell.up_to()),
                    "verify": cell.is_workspace().then(|| cell.verify().as_str()),
                    "intent": cell.intent,
                    "runtime_artifact": cell.runtime_artifact,
                    "require_openusd": cell.require_openusd,
                    "require_openusd_version": cell.require_openusd_version,
                    "runtime_remote": cell.runtime_remote.as_ref().map(|r| r.uri.as_str()),
                    "expected_oci_digest": cell
                        .runtime_remote
                        .as_ref()
                        .and_then(|r| r.pinned_oci_digest()),
                    "plugin_artifact": cell.plugin_artifact,
                    "host_python": cell.host_python,
                    "host_packages": cell.host_packages,
                    "minimum_trust": matrix.minimum_trust(cell),
                    "require_evidence": matrix.require_evidence(cell).as_str(),
                })
            })
            .collect();
        // The bootstrap pin travels with the cells: a hand-written lane needs
        // the same checksum-verified `ost` the generated one installs, and that
        // was the largest copied block of all (report 32 §2, ~40 lines).
        let bootstrap = matrix.bootstrap.as_ref().map(|b| {
            serde_json::json!({
                "version": b.ost.version,
                "repository": b.ost.repository,
                "sha256": b.ost.sha256,
            })
        });
        output::report(
            true,
            &serde_json::json!({
                "schema": ost_ci::MATRIX_SCHEMA,
                "matrix": path.to_string(),
                "lane": lane.map(|l| l.as_str()),
                "bootstrap": bootstrap,
                "cells": rendered,
            }),
        );
        return Ok(());
    }

    println!(
        "Matrix {path}: {} cell(s){}",
        cells.len(),
        lane.map(|l| format!(" in lane {}", l.as_str()))
            .unwrap_or_default()
    );
    for cell in &cells {
        let builds = if cell.is_workspace() {
            format!("workspace, verify {}", cell.verify().as_str())
        } else {
            format!(
                "bundle {}, up to L{}",
                cell.bundle.as_deref().unwrap_or("."),
                cell.up_to()
            )
        };
        println!(
            "  {} [{}] {}/{} on {} — {builds} — runtime {}",
            cell.name,
            cell.lane.as_str(),
            cell.platform,
            cell.profile,
            matrix.runs_on(cell).join(", "),
            cell.runtime_artifact.as_deref().unwrap_or("none")
        );
        if let Some(remote) = &cell.runtime_remote {
            println!("      remote: {}", remote.uri);
        }
        if let Some(selector) = &cell.require_openusd {
            println!(
                "      OpenUSD: {}{}",
                selector,
                cell.require_openusd_version
                    .as_deref()
                    .map(|version| format!(" @ {version}"))
                    .unwrap_or_default()
            );
        }
    }
    if cells.is_empty() {
        println!("  (no cell matches; `ost ci plan` lists the lanes that exist)");
    }
    Ok(())
}

fn generate(
    matrix_flag: Option<&str>,
    out: Option<&str>,
    force: bool,
    to_stdout: bool,
    allow_placeholders: bool,
    support_flag: Option<&str>,
    fmt: Format,
) -> Result<()> {
    let (path, matrix) = load_matrix(matrix_flag)?;

    if let Some(flag) = support_flag {
        let support_path = Utf8PathBuf::from(flag);
        let src = std::fs::read_to_string(support_path.as_std_path())
            .map_err(|error| Error::io(support_path.to_string(), error))?;
        let issues = SupportDeclaration::from_toml(&src)?.matrix_issues(&matrix);
        if !issues.is_empty() {
            return Err(Error::coded(
                "CI_SUPPORT_CLAIM_UNSUPPORTED",
                ost_core::Category::Validation,
                format!(
                    "CI cells exceed or omit public support claims ({})",
                    issues.join("; ")
                ),
            )
            .with_hint(format!(
                "correct cells[].support or update {support_path} deliberately"
            )));
        }
    }

    // A workflow rendered from placeholder digests can only fail on a runner
    // (or worse, be committed as if it were a real support claim) — refuse
    // unless explicitly overridden.
    let placeholders = matrix.placeholder_digests();
    if !placeholders.is_empty() && !allow_placeholders {
        return Err(Error::coded(
            "CI_PLACEHOLDER_DIGESTS",
            ost_core::Category::Validation,
            format!(
                "the matrix still carries the scaffold's placeholder digests ({})",
                placeholders.join("; ")
            ),
        )
        .with_hint(
            "pin real digests (`ost runtime export`, `ost plugin publish`), \
             or pass --allow-placeholders to render anyway",
        ));
    }
    let billing_errors = matrix.hosted_ack_errors();
    if !billing_errors.is_empty() {
        return Err(Error::coded(
            "CI_HOSTED_BILLING_UNACKNOWLEDGED",
            ost_core::Category::Validation,
            format!(
                "publish-capable hosted jobs require billing acknowledgement ({})",
                billing_errors.join("; ")
            ),
        )
        .with_hint("set runners.<name>.billing.acknowledgement: required"));
    }

    let workflows = generate_github(&matrix);
    let stale_workflows = stale_generated_workflows(&path, &matrix);

    // Warn — do not fail — when a pin cannot satisfy the gate just rendered:
    // generation may legitimately run on a machine whose registry is not the
    // one the lane will use. `ost ci validate` is the gate that fails.
    let evidence_gaps = evidence_gate_gaps(&matrix);
    let host_requirement_gaps = host_requirement_gaps(&matrix);

    if to_stdout {
        // The workflows themselves are the output; a multi-workflow matrix
        // prints a `---`-separated YAML stream (one document per workflow).
        let mut first = true;
        for wf in &workflows {
            if !first {
                println!("---");
            }
            print!("{}", wf.yaml);
            first = false;
        }
        // stdout is the workflow here — often piped to a file — so the warning
        // goes to stderr rather than being swallowed by the redirect.
        for gap in &evidence_gaps {
            eprintln!("WARNING: the rendered gate cannot pass — {gap}");
        }
        if !evidence_gaps.is_empty() {
            eprintln!("{EVIDENCE_GAP_HINT}");
        }
        for gap in &host_requirement_gaps {
            eprintln!("WARNING: the rendered host provisioning is incomplete — {gap}");
        }
        if !host_requirement_gaps.is_empty() {
            eprintln!("{HOST_REQUIREMENT_GAP_HINT}");
        }
        for stale in &stale_workflows {
            eprintln!("WARNING: {}", stale_workflow_message(stale));
        }
        return Ok(());
    }

    // `--out` targets exactly one file, so it only fits a one-workflow matrix.
    let out_paths: Vec<Utf8PathBuf> = match out {
        Some(out) if workflows.len() > 1 => {
            return Err(Error::usage(format!(
                "the matrix renders {} workflows (source/support/release lanes) — \
                 --out '{out}' targets a single file; use the default paths",
                workflows.len()
            )));
        }
        Some(out) => vec![Utf8PathBuf::from(out)],
        None => workflows
            .iter()
            .map(|wf| Utf8PathBuf::from(wf.path))
            .collect(),
    };

    // Check every destination before writing any, so --force is all-or-nothing.
    for out_path in &out_paths {
        if out_path.as_std_path().exists() && !force {
            return Err(Error::usage(format!(
                "'{out_path}' already exists (pass --force to regenerate over it)"
            )));
        }
    }
    for (wf, out_path) in workflows.iter().zip(&out_paths) {
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent.as_std_path())
                .map_err(|e| Error::io(parent.to_string(), e))?;
        }
        std::fs::write(out_path.as_std_path(), &wf.yaml)
            .map_err(|e| Error::io(out_path.to_string(), e))?;
    }

    if fmt.is_json() {
        let mut warnings: Vec<serde_json::Value> = evidence_gaps
            .iter()
            .map(|gap| {
                serde_json::json!({
                    "code": "CI_EVIDENCE_GATE_UNSATISFIABLE",
                    "message": gap,
                })
            })
            .collect();
        warnings.extend(host_requirement_gaps.iter().map(|gap| {
            serde_json::json!({
                "code": "CI_HOST_REQUIREMENT_UNPROVISIONED",
                "message": gap,
            })
        }));
        warnings.extend(stale_workflows.iter().map(|path| {
            serde_json::json!({
                "code": "CI_STALE_GENERATED_WORKFLOW",
                "message": stale_workflow_message(path),
            })
        }));
        output::report_with_warnings(
            true,
            &serde_json::json!({
                "generated": true,
                "matrix": path.to_string(),
                // `workflow` predates the lane split; keep it as the first path.
                "workflow": out_paths[0].to_string(),
                "workflows": out_paths.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
                "cells": matrix.cells.len(),
                "evidence_gaps": evidence_gaps,
                "host_requirement_gaps": host_requirement_gaps,
                "stale_workflows": stale_workflows,
            }),
            &warnings,
        );
        return Ok(());
    }
    for out_path in &out_paths {
        println!("Generated {out_path} from {path}");
    }
    println!("  {} cell(s) total", matrix.cells.len());
    println!("  runners need `ost` on PATH and the pinned artifacts in their OST_HOME registry");
    for gap in &evidence_gaps {
        println!("  WARNING: the rendered gate cannot pass — {gap}");
    }
    if !evidence_gaps.is_empty() {
        println!("  {EVIDENCE_GAP_HINT}");
    }
    for gap in &host_requirement_gaps {
        println!("  WARNING: the rendered host provisioning is incomplete — {gap}");
    }
    if !host_requirement_gaps.is_empty() {
        println!("  {HOST_REQUIREMENT_GAP_HINT}");
    }
    for stale in &stale_workflows {
        println!("  WARNING: {}", stale_workflow_message(stale));
    }
    Ok(())
}
