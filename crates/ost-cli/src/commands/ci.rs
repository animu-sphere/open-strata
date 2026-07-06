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

use camino::Utf8PathBuf;
use clap::Subcommand;

use ost_artifact::{ArtifactKind, ArtifactStore};
use ost_ci::{
    generate_github, starter_matrix, Lane, Publish, SupportMatrix, MATRIX_FILE,
    SOURCE_WORKFLOW_PATH, WORKFLOW_PATH,
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
    },
    /// Report preflight execution facts (lanes, runners, billing).
    Plan {
        /// Path to the matrix file. Defaults to ./openstrata.ci.yaml.
        #[arg(long)]
        matrix: Option<String>,
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
    },
}

pub fn run(cmd: CiCmd, fmt: Format) -> Result<()> {
    match cmd {
        CiCmd::Init { dir } => init(dir.as_deref(), fmt),
        CiCmd::Validate { matrix, resolve } => validate(matrix.as_deref(), resolve, fmt),
        CiCmd::Plan { matrix } => plan(matrix.as_deref(), fmt),
        CiCmd::Generate(GenerateCmd::Github {
            matrix,
            out,
            force,
            stdout,
            allow_placeholders,
        }) => generate(
            matrix.as_deref(),
            out.as_deref(),
            force,
            stdout,
            allow_placeholders,
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

fn validate(matrix_flag: Option<&str>, resolve: bool, fmt: Format) -> Result<()> {
    let (path, matrix) = load_matrix(matrix_flag)?;

    // `--resolve`: every pinned digest must exist in the local registry, so a
    // matrix can be gated before the runners ever see it.
    let mut unresolved: Vec<String> = Vec::new();
    if resolve {
        let store = ArtifactStore::discover();
        for cell in &matrix.cells {
            let mut refs = vec![("runtime", &cell.runtime_artifact, ArtifactKind::Runtime)];
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
                    Ok(_) => {}
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

    let ok = unresolved.is_empty() && ack_errors.is_empty();
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
        output::report_with_warnings(
            ok,
            &serde_json::json!({
                "matrix": path.to_string(),
                "cells": matrix.cells.len(),
                "resolved": resolve,
                "unresolved": unresolved,
                "placeholders": placeholders,
                "hosted_unacknowledged": ack_missing,
                "billing_errors": ack_errors,
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
        for hit in &ack_errors {
            println!("  ERROR: publish-capable cell needs billing acknowledgement — {hit}");
        }
        for miss in &unresolved {
            println!("  UNRESOLVED: {miss}");
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

    let hosted_jobs = matrix.cells.iter().filter(|c| matrix.is_hosted(c)).count();
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

    if fmt.is_json() {
        output::success(&serde_json::json!({
            "matrix": path.to_string(),
            "cells": matrix.cells.len(),
            "lanes": lanes,
            "workflows": workflows,
            "hosted_jobs": hosted_jobs,
            "metered_runner_classes": metered,
            "operator_managed_runner_classes": operator,
            "hosted_unacknowledged": hosted_unacknowledged,
            "requires_billing_acknowledgement": !hosted_unacknowledged.is_empty(),
            "publish_capable_jobs": publish_capable,
            "bootstrap": bootstrap,
            "remote_runtime_cells": remote_runtime_cells,
            "air_gapped_source_cells": air_gapped_source_cells,
        }));
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

fn generate(
    matrix_flag: Option<&str>,
    out: Option<&str>,
    force: bool,
    to_stdout: bool,
    allow_placeholders: bool,
    fmt: Format,
) -> Result<()> {
    let (path, matrix) = load_matrix(matrix_flag)?;

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

    let workflows = generate_github(&matrix);

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
        return Ok(());
    }

    // `--out` targets exactly one file, so it only fits a one-workflow matrix.
    let out_paths: Vec<Utf8PathBuf> = match out {
        Some(out) if workflows.len() > 1 => {
            return Err(Error::usage(format!(
                "the matrix renders {} workflows (source CI + support matrix) — \
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
        output::success(&serde_json::json!({
            "generated": true,
            "matrix": path.to_string(),
            // `workflow` predates the lane split; keep it as the first path.
            "workflow": out_paths[0].to_string(),
            "workflows": out_paths.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
            "cells": matrix.cells.len(),
        }));
        return Ok(());
    }
    for out_path in &out_paths {
        println!("Generated {out_path} from {path}");
    }
    println!("  {} cell(s) total", matrix.cells.len());
    println!("  runners need `ost` on PATH and the pinned artifacts in their OST_HOME registry");
    Ok(())
}
