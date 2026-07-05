// SPDX-License-Identifier: Apache-2.0
//! `ost ci` — the CI support matrix (Phase 5 MVP).
//!
//! - `init`     scaffold a commented `openstrata.ci.yaml` starter matrix.
//! - `validate` structural checks; `--resolve` additionally requires every
//!   pinned digest to exist in the local artifact registry.
//! - `generate github` render the matrix into a scheduled GitHub Actions
//!   workflow with one job per support cell.
//!
//! The matrix is the single source of truth; generated workflows carry a
//! "regenerate, don't edit" banner. Jenkins generation lands later on the same
//! model.

use camino::Utf8PathBuf;
use clap::Subcommand;

use ost_artifact::{ArtifactKind, ArtifactStore};
use ost_ci::{generate_github, starter_matrix, SupportMatrix, MATRIX_FILE, WORKFLOW_PATH};
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
            for (what, digest, expected) in [
                ("runtime", &cell.runtime_artifact, ArtifactKind::Runtime),
                ("plugin", &cell.plugin_artifact, ArtifactKind::Plugin),
            ] {
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

    let ok = unresolved.is_empty();
    if fmt.is_json() {
        let warnings: Vec<serde_json::Value> = placeholders
            .iter()
            .map(|hit| {
                serde_json::json!({
                    "code": "CI_PLACEHOLDER_DIGEST",
                    "message": format!("placeholder digest — {hit}"),
                })
            })
            .collect();
        output::report_with_warnings(
            ok,
            &serde_json::json!({
                "matrix": path.to_string(),
                "cells": matrix.cells.len(),
                "resolved": resolve,
                "unresolved": unresolved,
                "placeholders": placeholders,
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

    let workflow = generate_github(&matrix);

    if to_stdout {
        // The workflow itself is the output document; keep it uncorrupted.
        print!("{workflow}");
        return Ok(());
    }

    let out_path = Utf8PathBuf::from(out.unwrap_or(WORKFLOW_PATH));
    if out_path.as_std_path().exists() && !force {
        return Err(Error::usage(format!(
            "'{out_path}' already exists (pass --force to regenerate over it)"
        )));
    }
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent.as_std_path())
            .map_err(|e| Error::io(parent.to_string(), e))?;
    }
    std::fs::write(out_path.as_std_path(), &workflow)
        .map_err(|e| Error::io(out_path.to_string(), e))?;

    if fmt.is_json() {
        output::success(&serde_json::json!({
            "generated": true,
            "matrix": path.to_string(),
            "workflow": out_path.to_string(),
            "cells": matrix.cells.len(),
        }));
        return Ok(());
    }
    println!(
        "Generated {out_path} from {path} ({} cell(s))",
        matrix.cells.len()
    );
    println!("  runners need `ost` on PATH and the pinned artifacts in their OST_HOME registry");
    Ok(())
}
