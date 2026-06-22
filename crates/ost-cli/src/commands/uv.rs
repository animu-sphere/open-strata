//! `ost uv` — run `uv` against the project's certified runtime Python (§9.3).
//!
//! OpenStrata selects the Python interpreter; `uv` must not silently replace it
//! (§9.1, §20.3). This wrapper resolves the project's runtime, applies its
//! environment, pins `UV_PYTHON` to the runtime interpreter, and execs `uv` with
//! the passed-through arguments:
//!
//! ```bash
//! ost uv sync --locked      # == UV_PYTHON=<runtime>/bin/python uv sync --locked
//! ost uv run pytest
//! ```
//!
//! With no arguments it prints the environment it would set, so you can see how
//! `uv` is pinned without running it.

use std::path::PathBuf;
use std::process::Command;

use camino::Utf8PathBuf;
use clap::Args;

use ost_core::host::Os;
use ost_core::{tools, Error, Result};

use crate::commands::configure::resolve_selection;
use crate::commands::resolve as resolve_runtime;
use crate::output::{self, Format};

#[derive(Debug, Args)]
pub struct UvArgs {
    /// Arguments passed through to `uv` (e.g. `sync --locked`).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

pub fn run(args: UvArgs, fmt: Format) -> Result<()> {
    let (root, platform, profile) = resolve_selection(None, None)?;
    let r = resolve_runtime(&platform, &profile)?;

    if !r.pulled {
        return Err(Error::Operation(format!(
            "runtime '{}' not pulled — run `ost runtime pull {} --profile {}` first",
            r.runtime.id(),
            platform,
            profile
        )));
    }

    // The interpreter uv must use, never one it picks itself.
    let uv_python = runtime_python(&r.artifact_prefix, r.runtime.variant.os);

    // No args: show how uv would be pinned, without invoking it.
    if args.args.is_empty() {
        if fmt.is_json() {
            let env: Vec<_> = r
                .env
                .pairs()
                .into_iter()
                .map(|(k, v)| serde_json::json!({ "name": k, "value": v }))
                .collect();
            output::json(&serde_json::json!({
                "runtime": r.runtime.id(),
                "uv_python": uv_python.to_string(),
                "env": env,
            }));
        } else {
            println!("uv would run pinned to the runtime:");
            println!("  runtime:   {}", r.runtime.id());
            println!("  UV_PYTHON: {uv_python}");
            println!("\nRun e.g.: ost uv sync --locked");
        }
        return Ok(());
    }

    let uv = locate_uv().ok_or_else(|| {
        Error::Operation(
            "`uv` not found — install uv, add it to PATH, or set OST_UV".to_string(),
        )
    })?;

    let mut cmd = Command::new(&uv);
    cmd.args(&args.args);
    // Run uv from the project root so it finds pyproject.toml / uv.lock.
    cmd.current_dir(root.as_std_path());
    // Apply the runtime environment, then pin the interpreter on top.
    r.env.apply(&mut cmd);
    cmd.env("UV_PYTHON", uv_python.as_std_path());

    let status = cmd
        .status()
        .map_err(|e| Error::io(format!("run {}", uv.display()), e))?;
    std::process::exit(status.code().unwrap_or(1));
}

/// The runtime's Python interpreter path.
fn runtime_python(prefix: &Utf8PathBuf, os: Os) -> Utf8PathBuf {
    match os {
        Os::Windows => prefix.join("python.exe"),
        _ => prefix.join("bin").join("python"),
    }
}

/// Find uv: an `OST_UV` override first, then PATH.
fn locate_uv() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("OST_UV") {
        let pb = PathBuf::from(path);
        if pb.is_file() {
            return Some(pb);
        }
    }
    tools::which("uv")
}
