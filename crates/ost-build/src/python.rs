// SPDX-License-Identifier: Apache-2.0
//! Host-Python resolution for generated toolchains.
//!
//! USD's generated `pxrConfig.cmake` bakes the *build machine's* Python paths
//! (`Python3_EXECUTABLE` / `Python3_LIBRARY` / `Python3_INCLUDE_DIR`) behind
//! `if (NOT DEFINED …)` guards. On any other host those paths are dead, and
//! `find_dependency(Python3 … COMPONENTS Development)` fails — the exact
//! failure a clean CI runner hits with an adopted runtime artifact. Every
//! guard yields to a predefined variable, so the fix is to resolve a matching
//! host interpreter up front and pin all three variables in `toolchain.cmake`.
//!
//! The *required* version comes from `pxrConfig.cmake` itself when present
//! (`find_dependency(Python3 "3.10" EXACT …)`) — it is the ground truth of
//! the USD build, and adopted runtimes have been observed to declare a
//! different Python in `runtime.json` than the one USD was actually built
//! against. The runtime declaration is only the fallback.

use std::process::Command;

use camino::{Utf8Path, Utf8PathBuf};

/// Where the resolved interpreter came from (recorded as a toolchain comment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PythonSource {
    /// Bundled inside the runtime prefix — self-contained runtime.
    RuntimePrefix,
    /// Found on the host (py launcher, tool cache, or PATH).
    Host,
}

/// Resolved CPython paths pinned into the generated `toolchain.cmake`.
#[derive(Debug, Clone)]
pub struct PythonHints {
    /// Interpreter path (native separators; the renderer normalizes).
    pub executable: String,
    /// Import/embed library (`pythonXY.lib` on Windows, `libpythonX.Y.so`
    /// elsewhere). Required: without it `Development` resolution falls back
    /// to whatever the config package baked.
    pub library: String,
    /// Directory containing `Python.h`.
    pub include_dir: String,
    /// `major.minor`, e.g. `3.10`.
    pub version: String,
    pub source: PythonSource,
}

/// The exact Python version the runtime's USD build requires, parsed from
/// `<prefix>/pxrConfig.cmake` (`find_dependency(Python3 "3.10" EXACT …)`).
pub fn usd_python_requirement(prefix: &Utf8Path) -> Option<String> {
    let config = std::fs::read_to_string(prefix.join("pxrConfig.cmake").as_std_path()).ok()?;
    parse_pxr_python_requirement(&config)
}

/// Pure parser behind [`usd_python_requirement`], for tests.
fn parse_pxr_python_requirement(pxr_config: &str) -> Option<String> {
    let idx = pxr_config.find("find_dependency(Python3 \"")?;
    let rest = &pxr_config[idx + "find_dependency(Python3 \"".len()..];
    let version = rest.split('"').next()?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next()?.parse::<u32>().ok()?;
    Some(format!("{major}.{minor}"))
}

/// Reduce a declared runtime Python ("3.13.x", "3.13.0", "313") to
/// `major.minor`, or `None` if it doesn't look like a version.
pub fn major_minor(declared: &str) -> Option<String> {
    let d = declared.trim();
    if let Some((maj, rest)) = d.split_once('.') {
        let major = maj.parse::<u32>().ok()?;
        let minor = rest.split('.').next()?.parse::<u32>().ok()?;
        return Some(format!("{major}.{minor}"));
    }
    // Compact variant form, e.g. "313" → 3.13.
    if d.len() >= 2 && d.chars().all(|c| c.is_ascii_digit()) {
        let (maj, min) = d.split_at(1);
        return Some(format!("{maj}.{}", min.parse::<u32>().ok()?));
    }
    None
}

/// Resolve Python Development artifacts to pin in a toolchain for a build
/// against `prefix`, given the runtime's `declared` Python (e.g. from
/// `runtime.json`). The required `major.minor` is taken from the runtime's
/// `pxrConfig.cmake` when it declares one (ground truth of the USD build),
/// else from `declared`. `None` if nothing matching is found — the caller
/// then renders a toolchain without pins (prior behavior).
pub fn resolve_for_runtime(prefix: &Utf8Path, declared: &str) -> Option<PythonHints> {
    let required = usd_python_requirement(prefix).or_else(|| major_minor(declared));
    resolve_python_hints(prefix, required.as_deref())
}

/// Resolve a host CPython matching `required` (`major.minor`; `None` accepts
/// any) with usable Development artifacts. Candidates, in order: an
/// interpreter bundled in the runtime prefix, the Windows `py` launcher, a
/// CI tool cache (`RUNNER_TOOL_CACHE`), then PATH.
pub fn resolve_python_hints(prefix: &Utf8Path, required: Option<&str>) -> Option<PythonHints> {
    for (candidate, source) in candidates(prefix, required) {
        if let Some(hints) = probe(&candidate, source) {
            if required.is_none_or(|r| hints.version == r) {
                return Some(hints);
            }
        }
    }
    None
}

/// Candidate interpreter invocations: `(argv, source)`. Each argv's head may
/// be a bare command resolved via PATH.
fn candidates(prefix: &Utf8Path, required: Option<&str>) -> Vec<(Vec<String>, PythonSource)> {
    let mut out: Vec<(Vec<String>, PythonSource)> = Vec::new();
    let bundled = if cfg!(windows) {
        vec![prefix.join("python.exe"), prefix.join("bin/python.exe")]
    } else {
        vec![prefix.join("bin/python3"), prefix.join("bin/python")]
    };
    for p in bundled {
        if p.as_std_path().is_file() {
            out.push((vec![p.to_string()], PythonSource::RuntimePrefix));
        }
    }
    if cfg!(windows) {
        if let Some(v) = required {
            out.push((vec!["py".into(), format!("-{v}")], PythonSource::Host));
        }
    } else if let Some(v) = required {
        out.push((vec![format!("python{v}")], PythonSource::Host));
    }
    // GitHub-hosted runners keep versioned interpreters in the tool cache
    // without putting them on PATH; a hosted lane must find them anyway.
    if let (Some(v), Ok(cache)) = (required, std::env::var("RUNNER_TOOL_CACHE")) {
        out.extend(
            tool_cache_pythons(Utf8Path::new(&cache), v)
                .into_iter()
                .map(|p| (vec![p.to_string()], PythonSource::Host)),
        );
    }
    out.push((vec!["python3".into()], PythonSource::Host));
    out.push((vec!["python".into()], PythonSource::Host));
    out
}

/// `<tool-cache>/Python/<required>.*/x64/python[.exe]`, newest patch first.
fn tool_cache_pythons(cache: &Utf8Path, required: &str) -> Vec<Utf8PathBuf> {
    let base = cache.join("Python");
    let Ok(entries) = std::fs::read_dir(base.as_std_path()) else {
        return Vec::new();
    };
    let mut versions: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name.starts_with(&format!("{required}.")))
        .collect();
    versions.sort();
    versions.reverse();
    versions
        .into_iter()
        .filter_map(|v| {
            let exe = if cfg!(windows) {
                base.join(&v).join("x64/python.exe")
            } else {
                base.join(&v).join("x64/bin/python3")
            };
            exe.as_std_path().is_file().then_some(exe)
        })
        .collect()
}

/// Interrogate one interpreter for its version and Development artifacts.
/// Rejects interpreters whose include dir or import library don't exist
/// (e.g. embeddable distributions) — pinning those would only move the
/// failure.
fn probe(argv: &[String], source: PythonSource) -> Option<PythonHints> {
    // Flat (no indented blocks): the whole thing is one -c argument, and any
    // indentation would risk breakage across how shells/launchers forward it.
    // Everything is expressed with ternaries so each statement stands alone.
    let script = [
        "import os, sys, sysconfig",
        "print(sys.executable)",
        "print('%d.%d' % sys.version_info[:2])",
        "print(sysconfig.get_path('include') or '')",
        "v = '%d%d' % sys.version_info[:2]",
        "nt = os.path.join(sys.base_prefix, 'libs', 'python' + v + '.lib')",
        "d = sysconfig.get_config_var('LIBDIR') or ''",
        "n = sysconfig.get_config_var('LDLIBRARY') or ''",
        "posix = os.path.join(d, n) if d and n else ''",
        "ma = sysconfig.get_config_var('MULTIARCH') or ''",
        "alt = os.path.join(d, ma, n) if ma and d and n else ''",
        "posix = alt if (posix and not os.path.exists(posix) and alt and os.path.exists(alt)) else posix",
        "print(nt if os.name == 'nt' else posix)",
    ]
    .join("\n");
    let (head, rest) = argv.split_first()?;
    let out = Command::new(head)
        .args(rest)
        .arg("-c")
        .arg(&script)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let mut lines = text.lines().map(str::trim);
    let executable = lines.next()?.to_string();
    let version = lines.next()?.to_string();
    let include_dir = lines.next()?.to_string();
    let library = lines.next()?.to_string();
    if executable.is_empty()
        || include_dir.is_empty()
        || library.is_empty()
        || !std::path::Path::new(&include_dir).is_dir()
        || !std::path::Path::new(&library).is_file()
    {
        return None;
    }
    Some(PythonHints {
        executable,
        library,
        include_dir,
        version,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pxr_exact_python_requirement() {
        let config = r#"
    if (NOT DEFINED Python3_VERSION)
        find_dependency(Python3 "3.10" EXACT COMPONENTS Development)
    else()
        find_dependency(Python3 COMPONENTS Development)
    endif()
"#;
        assert_eq!(
            parse_pxr_python_requirement(config),
            Some("3.10".to_string())
        );
    }

    #[test]
    fn pxr_requirement_missing_yields_none() {
        assert_eq!(parse_pxr_python_requirement("find_package(pxr)"), None);
        assert_eq!(
            parse_pxr_python_requirement("find_dependency(Python3 \"bogus\" EXACT)"),
            None
        );
    }

    #[test]
    fn major_minor_accepts_declared_forms() {
        assert_eq!(major_minor("3.13.x"), Some("3.13".into()));
        assert_eq!(major_minor("3.10.11"), Some("3.10".into()));
        assert_eq!(major_minor("3.10"), Some("3.10".into()));
        assert_eq!(major_minor("310"), Some("3.10".into()));
        assert_eq!(major_minor("313"), Some("3.13".into()));
        assert_eq!(major_minor(""), None);
        assert_eq!(major_minor("python"), None);
    }

    #[test]
    fn probe_rejects_missing_interpreter() {
        assert!(probe(
            &["definitely-not-a-python-anywhere".to_string()],
            PythonSource::Host
        )
        .is_none());
    }
}
