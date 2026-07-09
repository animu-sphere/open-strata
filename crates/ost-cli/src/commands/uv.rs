// SPDX-License-Identifier: Apache-2.0
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
//!
//! It also **diagnoses ABI-sensitive shadowing**: a project uv dependency that
//! duplicates a native package the runtime already provides (OpenUSD/Qt/OpenEXR/
//! OpenColorIO/MaterialX bindings) would sit ahead of the runtime's ABI-matched
//! build on `sys.path` and crash at import (double-loaded native libraries, ABI
//! mismatch). `ost uv` warns on such deps, and **refuses** an install-shaped
//! `uv` subcommand (`sync`/`add`/`install`/`pip`/`lock`) that would materialize
//! them, unless `OST_UV_ALLOW_SHADOWED` is set (§9, §20.3).

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;

use camino::{Utf8Path, Utf8PathBuf};
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
        return Err(Error::coded(
            "RUNTIME_NOT_FOUND",
            ost_core::Category::Precondition,
            format!(
                "runtime '{}' not pulled — run `ost runtime pull {} --profile {}` first",
                r.runtime.id(),
                platform,
                profile
            ),
        ));
    }

    // The interpreter uv must use, never one it picks itself.
    let uv_python = runtime_python(&r.artifact_prefix, r.runtime.variant.os);

    // App-local uv deps that duplicate ABI-sensitive runtime packages.
    let shadows = shadowing_deps(&project_uv_packages(&root));

    // No args: show how uv would be pinned, without invoking it.
    if args.args.is_empty() {
        if fmt.is_json() {
            let env: Vec<_> = r
                .env
                .pairs()
                .into_iter()
                .map(|(k, v)| serde_json::json!({ "name": k, "value": v }))
                .collect();
            output::success(&serde_json::json!({
                "runtime": r.runtime.id(),
                "uv_python": uv_python.to_string(),
                "env": env,
                "shadowed_runtime_deps": shadows
                    .iter()
                    .map(|s| serde_json::json!({
                        "dependency": s.dep,
                        "category": s.category,
                        "recommendation": s.recommendation,
                    }))
                    .collect::<Vec<_>>(),
            }));
        } else {
            println!("uv would run pinned to the runtime:");
            println!("  runtime:   {}", r.runtime.id());
            println!("  UV_PYTHON: {uv_python}");
            report_shadows(&shadows);
            println!("\nRun e.g.: ost uv sync --locked");
        }
        return Ok(());
    }

    // Diagnose (always) and refuse an install-shaped subcommand that would
    // materialize the shadowing deps into the environment.
    if !shadows.is_empty() {
        report_shadows(&shadows);
        let installing = args
            .args
            .iter()
            .find(|a| !a.starts_with('-'))
            .map(|a| is_installing_subcommand(a))
            .unwrap_or(false);
        if installing && std::env::var_os("OST_UV_ALLOW_SHADOWED").is_none() {
            return Err(Error::coded(
                "UV_SHADOWED_RUNTIME_DEPS",
                ost_core::Category::Precondition,
                "refusing to install uv dependencies that shadow ABI-sensitive runtime \
                 packages (OpenUSD / Qt / OpenEXR / OpenColorIO / MaterialX bindings)",
            )
            .with_hint(
                "remove them from pyproject.toml and use the runtime's ABI-matched build, \
                 or set OST_UV_ALLOW_SHADOWED=1 to override",
            ));
        }
    }

    let uv = locate_uv().ok_or_else(|| {
        Error::coded(
            "REQUIRED_TOOL_MISSING",
            ost_core::Category::Precondition,
            "`uv` not found — install uv, add it to PATH, or set OST_UV",
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

/// One project uv dependency that shadows an ABI-sensitive runtime package.
struct Shadow {
    /// The dependency as declared/resolved (original spelling).
    dep: String,
    /// The native package family it duplicates (OpenUSD / Qt / …).
    category: &'static str,
    /// What to do instead.
    recommendation: &'static str,
}

/// Print a shadow diagnosis to stderr (human output).
fn report_shadows(shadows: &[Shadow]) {
    if shadows.is_empty() {
        return;
    }
    eprintln!(
        "\nwarning: {} project uv dependency(ies) shadow ABI-sensitive runtime packages:",
        shadows.len()
    );
    for s in shadows {
        eprintln!("  - {} [{}] — {}", s.dep, s.category, s.recommendation);
    }
    eprintln!(
        "  a duplicated native binding sits ahead of the runtime's ABI-matched build on \
         sys.path and crashes at import."
    );
}

/// Whether a `uv` subcommand would install/resolve deps into the environment
/// (and so materialize any shadowing).
fn is_installing_subcommand(sub: &str) -> bool {
    matches!(sub, "sync" | "add" | "install" | "pip" | "lock")
}

/// The project's uv-managed package names: the resolved set from `uv.lock`
/// (authoritative, includes transitive deps) when present, else the declared
/// dependencies in `pyproject.toml`.
fn project_uv_packages(root: &Utf8Path) -> Vec<String> {
    if let Ok(src) = std::fs::read_to_string(root.join("uv.lock").as_std_path()) {
        if let Ok(doc) = toml::from_str::<toml::Value>(&src) {
            if let Some(pkgs) = doc.get("package").and_then(|v| v.as_array()) {
                return pkgs
                    .iter()
                    .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(str::to_string))
                    .collect();
            }
        }
    }
    let Ok(src) = std::fs::read_to_string(root.join("pyproject.toml").as_std_path()) else {
        return Vec::new();
    };
    let Ok(doc) = toml::from_str::<toml::Value>(&src) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    let mut collect = |v: Option<&toml::Value>| {
        if let Some(arr) = v.and_then(|v| v.as_array()) {
            for req in arr.iter().filter_map(|r| r.as_str()) {
                if let Some(name) = requirement_name(req) {
                    names.push(name);
                }
            }
        }
    };
    let project = doc.get("project");
    collect(project.and_then(|p| p.get("dependencies")));
    if let Some(opt) = project
        .and_then(|p| p.get("optional-dependencies"))
        .and_then(|v| v.as_table())
    {
        for group in opt.values() {
            collect(Some(group));
        }
    }
    collect(
        doc.get("tool")
            .and_then(|t| t.get("uv"))
            .and_then(|u| u.get("dev-dependencies")),
    );
    names
}

/// The distribution name from a PEP 508 requirement string
/// (`usd-core>=24.0 ; python_version<'3.13'` → `usd-core`, `foo[extra]` → `foo`).
fn requirement_name(req: &str) -> Option<String> {
    let req = req.trim();
    let end = req
        .find(|c: char| c.is_whitespace() || "[;<>=!~(,@".contains(c))
        .unwrap_or(req.len());
    let name = req[..end].trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// PEP 503 normalization: lowercase, and each run of `-_.` collapses to a
/// single `-`, so `USD_Core` / `usd.core` / `usd-core` compare equal.
fn normalize_dist(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_sep = false;
    for c in name.to_ascii_lowercase().chars() {
        if matches!(c, '-' | '_' | '.') {
            if !prev_sep && !out.is_empty() {
                out.push('-');
            }
            prev_sep = true;
        } else {
            out.push(c);
            prev_sep = false;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// The runtime-provided native family a normalized dist name shadows, if any,
/// as `(category, recommendation)`.
fn shadow_of(normalized: &str) -> Option<(&'static str, &'static str)> {
    let usd = (
        "OpenUSD",
        "the runtime provides ABI-matched pxr/OpenUSD — drop it and use a usd/lookdev profile",
    );
    let qt = (
        "Qt",
        "the runtime provides Qt/PySide via the qt-ui capability — drop it and use the runtime's build",
    );
    let ocio = (
        "OpenColorIO",
        "the runtime provides OpenColorIO via the color-management capability — drop it",
    );
    let exr = (
        "OpenEXR/OpenImageIO",
        "the runtime provides the image-io native libraries via the image-io capability — drop it",
    );
    let mtlx = (
        "MaterialX",
        "the runtime provides MaterialX via the usd-materialx capability — drop it",
    );
    match normalized {
        "usd-core" | "openusd" | "usd" | "pxr" => Some(usd),
        "pyside6" | "pyside2" | "shiboken6" | "shiboken2" | "pyqt5" | "pyqt6" => Some(qt),
        "opencolorio" | "pyopencolorio" => Some(ocio),
        "openexr" | "openimageio" | "oiio" => Some(exr),
        "materialx" => Some(mtlx),
        _ => None,
    }
}

/// The shadowing dependencies among `names` (deduplicated by normalized name).
fn shadowing_deps(names: &[String]) -> Vec<Shadow> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for name in names {
        let norm = normalize_dist(name);
        if let Some((category, recommendation)) = shadow_of(&norm) {
            if seen.insert(norm) {
                out.push(Shadow {
                    dep: name.clone(),
                    category,
                    recommendation,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirement_name_extracts_the_distribution() {
        assert_eq!(
            requirement_name("usd-core>=24.0").as_deref(),
            Some("usd-core")
        );
        assert_eq!(
            requirement_name("PySide6 ; sys_platform=='linux'").as_deref(),
            Some("PySide6")
        );
        assert_eq!(requirement_name("foo[extra]==1.0").as_deref(), Some("foo"));
        assert_eq!(requirement_name("  spaced  ").as_deref(), Some("spaced"));
        assert_eq!(requirement_name(""), None);
    }

    #[test]
    fn normalize_follows_pep503() {
        assert_eq!(normalize_dist("USD_Core"), "usd-core");
        assert_eq!(normalize_dist("usd.core"), "usd-core");
        assert_eq!(normalize_dist("PyOpenColorIO"), "pyopencolorio");
        assert_eq!(normalize_dist("Py--Side6"), "py-side6");
    }

    #[test]
    fn shadowing_deps_flags_abi_sensitive_families_and_dedupes() {
        // Bare names, as the uv.lock path yields them.
        let names = vec![
            "usd-core".to_string(),
            "USD_Core".to_string(), // same dist, different spelling → one hit
            "PySide6".to_string(),
            "numpy".to_string(), // app-owned, not shadowed
            "requests".to_string(),
            "OpenEXR".to_string(),
        ];
        let shadows = shadowing_deps(&names);
        let cats: Vec<_> = shadows.iter().map(|s| s.category).collect();
        assert_eq!(cats, vec!["OpenUSD", "Qt", "OpenEXR/OpenImageIO"]);
        assert_eq!(shadows.len(), 3, "usd-core is counted once");
    }

    #[test]
    fn project_packages_read_uv_lock_then_pyproject() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ost-uv-scan-{}-{nanos}", std::process::id()));
        let root = Utf8PathBuf::from_path_buf(dir.clone()).unwrap();
        std::fs::create_dir_all(root.as_std_path()).unwrap();

        // pyproject only: declared deps (names parsed out of PEP 508 strings).
        std::fs::write(
            root.join("pyproject.toml").as_std_path(),
            "[project]\nname='app'\ndependencies=['usd-core>=24.0', 'rich']\n\
             [project.optional-dependencies]\nui=['PySide6']\n",
        )
        .unwrap();
        let pkgs = project_uv_packages(&root);
        assert!(pkgs.iter().any(|p| p == "usd-core"));
        assert!(pkgs.iter().any(|p| p == "PySide6"));
        assert_eq!(shadowing_deps(&pkgs).len(), 2);

        // uv.lock wins when present (authoritative resolved set).
        std::fs::write(
            root.join("uv.lock").as_std_path(),
            "[[package]]\nname = \"openimageio\"\nversion = \"2.5\"\n\
             [[package]]\nname = \"click\"\nversion = \"8\"\n",
        )
        .unwrap();
        let pkgs = project_uv_packages(&root);
        assert_eq!(pkgs, vec!["openimageio", "click"]);
        assert_eq!(shadowing_deps(&pkgs).len(), 1);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn non_shadowing_project_is_clean() {
        let names = vec!["numpy".to_string(), "rich".to_string(), "click".to_string()];
        assert!(shadowing_deps(&names).is_empty());
    }
}
