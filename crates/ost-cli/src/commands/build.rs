// SPDX-License-Identifier: Apache-2.0
//! `ost build` — configure and build a target with CMake + Ninja (§8.2).
//!
//! `ost build` regenerates the target's CMake files (same as `ost configure`),
//! then drives CMake: `cmake --preset <id>` to configure, `cmake --build` to
//! compile. Ninja is located on PATH, via `OST_NINJA`, or `--ninja <path>`, and
//! passed to CMake as `CMAKE_MAKE_PROGRAM` so it works even off PATH.
//!
//! OpenStrata decides *what* to build; CMake/Ninja remain the build truth.

use std::path::{Path, PathBuf};
use std::process::Command;

use camino::Utf8Path;
use clap::Args;

use ost_core::host::Os;
use ost_core::{tools, Error, Result};

use crate::commands::configure::{generate, resolve_selection};
use crate::output::Format;

#[derive(Debug, Args)]
pub struct BuildArgs {
    /// Platform target, e.g. `cy2026`. Defaults to the project's platform.
    #[arg(long)]
    target: Option<String>,

    /// Profile to build. Defaults to the project's profile.
    #[arg(long)]
    profile: Option<String>,

    /// Print the commands that would run, without executing them.
    #[arg(long)]
    dry_run: bool,

    /// Parallel jobs: a number, or `auto` to let Ninja decide.
    #[arg(long)]
    jobs: Option<String>,

    /// Path to the ninja executable if it is not on PATH.
    #[arg(long)]
    ninja: Option<String>,

    /// Do not auto-load the MSVC developer environment (Windows).
    #[arg(long)]
    no_vcvars: bool,
}

pub fn run(args: BuildArgs, _fmt: Format) -> Result<()> {
    let (root, platform, profile) = resolve_selection(args.target.clone(), args.profile.clone())?;
    let g = generate(&root, &platform, &profile)?;
    let id = g.id.clone();

    let cmake = locate("cmake", None);
    let ninja = locate(
        "ninja",
        args.ninja.clone().or_else(|| std::env::var("OST_NINJA").ok()),
    );

    // On Windows we may auto-load the MSVC dev environment, which also puts a
    // Ninja on PATH — so an explicit ninja is not strictly required there.
    let will_bootstrap_msvc =
        g.target.os() == Os::Windows && !args.no_vcvars && tools::which("cl").is_none();

    if !args.dry_run {
        if !g.pulled {
            return Err(Error::Operation(format!(
                "runtime '{}' not pulled — run `ost runtime pull {} --profile {}` first",
                g.target.runtime_id, platform, profile
            )));
        }
        if cmake.is_none() {
            return Err(Error::Operation("`cmake` not found on PATH".to_string()));
        }
        if ninja.is_none() && !will_bootstrap_msvc {
            return Err(Error::Operation(
                "`ninja` not found — add it to PATH, set OST_NINJA, or pass --ninja <path>"
                    .to_string(),
            ));
        }
    }

    let cmake_prog = cmake.unwrap_or_else(|| PathBuf::from("cmake"));
    // CMake wants forward slashes even on Windows.
    let ninja_arg = ninja
        .as_ref()
        .map(|p| p.display().to_string().replace('\\', "/"));

    let mut configure_args = vec!["--preset".to_string(), id.clone()];
    if let Some(np) = &ninja_arg {
        configure_args.push(format!("-DCMAKE_MAKE_PROGRAM={np}"));
    }

    let mut build_args = vec!["--build".to_string(), format!("build/{id}")];
    if let Some(jobs) = &args.jobs {
        if let Ok(n) = jobs.parse::<u32>() {
            build_args.push("-j".to_string());
            build_args.push(n.to_string());
        }
    }

    if args.dry_run {
        println!("# dry run — would execute in {root}:");
        if will_bootstrap_msvc {
            println!("# (would auto-load the MSVC environment via vcvars64.bat)");
        }
        println!("{}", render_cmd(&cmake_prog, &configure_args));
        println!("{}", render_cmd(&cmake_prog, &build_args));
        return Ok(());
    }

    // Inject the MSVC developer environment (cl.exe, Windows SDK) if needed.
    let mut extra_env: Vec<(String, String)> = Vec::new();
    if will_bootstrap_msvc {
        match ost_build::msvc::bootstrap() {
            Ok(Some(env)) => {
                println!(
                    "==> msvc env   {} ({} vars)",
                    env.vcvars.display(),
                    env.vars.len()
                );
                extra_env = env.vars;
            }
            Ok(None) => eprintln!(
                "warning: MSVC not found; relying on the current environment (cl must be on PATH)"
            ),
            Err(e) => eprintln!("warning: failed to load the MSVC environment: {e}"),
        }
    }

    println!("==> configure  {}", render_cmd(&cmake_prog, &configure_args));
    run_step(&cmake_prog, &configure_args, &root, &extra_env)?;
    println!("==> build      {}", render_cmd(&cmake_prog, &build_args));
    run_step(&cmake_prog, &build_args, &root, &extra_env)?;
    println!("\nBuilt target {id}");
    Ok(())
}

/// Find an executable: an explicit override path first, then PATH.
fn locate(program: &str, override_path: Option<String>) -> Option<PathBuf> {
    if let Some(p) = override_path {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    tools::which(program)
}

fn render_cmd(program: &Path, args: &[String]) -> String {
    let mut s = quote(&program.display().to_string());
    for a in args {
        s.push(' ');
        s.push_str(&quote(a));
    }
    s
}

fn quote(s: &str) -> String {
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s.to_string()
    }
}

fn run_step(program: &Path, args: &[String], cwd: &Utf8Path, env: &[(String, String)]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd.as_std_path())
        .envs(env.iter().map(|(k, v)| (k.clone(), v.clone())))
        .status()
        .map_err(|e| Error::io(format!("run {}", program.display()), e))?;
    if !status.success() {
        // Propagate the underlying build failure code for CI.
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}
