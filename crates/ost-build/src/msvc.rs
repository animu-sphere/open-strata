// SPDX-License-Identifier: Apache-2.0
//! MSVC developer-environment bootstrap (Windows adapter, §11.2).
//!
//! On Windows the C++ compiler (`cl.exe`) and the Windows SDK are only on `PATH`
//! / `INCLUDE` / `LIB` after running `vcvars64.bat`. Rather than require the user
//! to launch a "Developer Command Prompt", `ost build` locates that script,
//! captures the environment it sets, and injects the *delta* into the CMake and
//! Ninja child processes.
//!
//! Everything here degrades to `Ok(None)` off Windows (no script is found), so
//! the module compiles and links on every platform without `cfg` noise.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The captured MSVC environment.
pub struct MsvcEnv {
    /// The `vcvars64.bat` that was used.
    pub vcvars: PathBuf,
    /// Variables vcvars adds or changes relative to the current environment.
    pub vars: Vec<(String, String)>,
}

/// Locate `vcvars64.bat` and capture the environment it would set, returned as a
/// delta over the current process environment. `Ok(None)` means no MSVC install
/// was found (e.g. on a non-Windows host).
pub fn bootstrap() -> io::Result<Option<MsvcEnv>> {
    let vcvars = match locate_vcvars() {
        Some(p) => p,
        None => return Ok(None),
    };
    let captured = capture_env(&vcvars)?;
    let vars = delta_from_current(captured);
    Ok(Some(MsvcEnv { vcvars, vars }))
}

fn locate_vcvars() -> Option<PathBuf> {
    if let Some(p) = vswhere_vcvars() {
        return Some(p);
    }
    for base in known_install_bases() {
        let p = Path::new(&base).join(r"VC\Auxiliary\Build\vcvars64.bat");
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Ask `vswhere` for the latest VS install that has the C++ toolset.
fn vswhere_vcvars() -> Option<PathBuf> {
    let pf86 = std::env::var("ProgramFiles(x86)").ok()?;
    let vswhere = Path::new(&pf86).join(r"Microsoft Visual Studio\Installer\vswhere.exe");
    if !vswhere.is_file() {
        return None;
    }
    let out = Command::new(&vswhere)
        .args([
            "-latest",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "installationPath",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    if path.is_empty() {
        return None;
    }
    let p = Path::new(&path).join(r"VC\Auxiliary\Build\vcvars64.bat");
    p.is_file().then_some(p)
}

/// Well-known install locations to probe when `vswhere` is unavailable.
fn known_install_bases() -> Vec<String> {
    let mut bases = Vec::new();
    let roots = [
        std::env::var("ProgramFiles").ok(),
        std::env::var("ProgramFiles(x86)").ok(),
    ];
    for root in roots.into_iter().flatten() {
        for year in ["2022", "2019"] {
            for edition in ["Community", "Professional", "Enterprise", "BuildTools"] {
                bases.push(format!(r"{root}\Microsoft Visual Studio\{year}\{edition}"));
            }
        }
    }
    bases
}

/// Run `cmd /C call vcvars64 & set` and parse the resulting environment.
fn capture_env(vcvars: &Path) -> io::Result<BTreeMap<String, String>> {
    // vcvars prints a banner and may complain (and even set a nonzero errorlevel)
    // if vswhere is missing, while still configuring the environment. Sequence
    // `set` with `&` — not `&&` — so it always runs and we read the result.
    //
    // The command line is passed verbatim via `raw_arg`: letting Command quote
    // it would wrap our already-quoted script path in another pair of quotes and
    // break cmd's parsing (the symptom is vcvars silently not running).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let cmdline = format!("/C call \"{}\" >nul 2>&1 & set", vcvars.display());
        let out = Command::new("cmd").raw_arg(cmdline).output()?;
        Ok(parse_set_output(&String::from_utf8_lossy(&out.stdout)))
    }
    #[cfg(not(windows))]
    {
        // Unreachable: locate_vcvars only finds a script on Windows.
        let _ = vcvars;
        Ok(BTreeMap::new())
    }
}

fn parse_set_output(text: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once('=') {
            if !key.is_empty() {
                map.insert(key.to_string(), value.to_string());
            }
        }
    }
    map
}

/// Keep only the variables that differ from the current process environment.
fn delta_from_current(captured: BTreeMap<String, String>) -> Vec<(String, String)> {
    captured
        .into_iter()
        .filter(|(k, v)| std::env::var(k).ok().as_deref() != Some(v.as_str()))
        .collect()
}
