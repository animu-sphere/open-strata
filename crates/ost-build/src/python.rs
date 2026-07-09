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

/// Packages `usdGenSchema` imports at *runtime* but that `build_usd.py` never
/// installs into the USD tree — it needs them only on the build host. Without
/// them on the runtime's `PYTHONPATH`, a published image dies with a bare
/// `ModuleNotFoundError: No module named 'jinja2'` in `ost plugin build`'s
/// schema-generate phase (report Finding D).
pub const SCHEMA_GEN_PACKAGES: &[&str] = &["jinja2", "MarkupSafe"];

/// Top-level imports the schema generator needs at runtime.
pub const SCHEMA_GEN_MODULES: &[&str] = &["jinja2", "markupsafe"];

/// Whether a runtime bundles the `usdGenSchema` schema tool in `bin/`. When it
/// does, the runtime is expected to be able to generate schemas standalone, so
/// its schema-gen Python deps must travel with it.
pub fn bundles_usdgenschema(prefix: &Utf8Path) -> bool {
    let bin = prefix.join("bin");
    [
        "usdGenSchema",
        "usdGenSchema.cmd",
        "usdGenSchema.exe",
        "usdGenSchema.bat",
        "usdGenSchema.py",
    ]
    .iter()
    .any(|n| bin.join(n).as_std_path().is_file())
}

/// Whether an importable top-level `module` is present in `python_lib_dir`
/// (a package directory, a single-file module, or a namespace/`dist-info`).
pub fn module_present(python_lib_dir: &Utf8Path, module: &str) -> bool {
    python_lib_dir.join(module).as_std_path().is_dir()
        || python_lib_dir
            .join(format!("{module}.py"))
            .as_std_path()
            .is_file()
}

/// Outcome of provisioning the schema-gen Python deps into a runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaDepsOutcome {
    /// `usdGenSchema` is not bundled — the runtime never generates schemas, so
    /// nothing is provisioned.
    NotNeeded,
    /// The deps were already importable on the runtime's `PYTHONPATH`.
    AlreadyPresent,
    /// The named packages were installed into the runtime's `PYTHONPATH` dir.
    Installed(Vec<String>),
}

/// Ensure the schema-gen Python deps ([`SCHEMA_GEN_PACKAGES`]) live in
/// `python_lib_dir` — exactly the dir `ost` puts on `PYTHONPATH` for the
/// runtime — when the runtime bundles `usdGenSchema` (report Finding D).
///
/// Runs `pip install --target <python_lib_dir>` with `python_argv` (the resolved
/// run-interpreter). A no-op when `usdGenSchema` is absent or all required imports
/// are already present, so re-pulls and export-time calls stay idempotent. `pip`
/// failure is surfaced as an `Err` for the caller to downgrade to a warning: the
/// runtime is otherwise built and the manual remediation is a single
/// `pip install --target`.
pub fn provision_schema_gen_deps(
    prefix: &Utf8Path,
    python_lib_dir: &Utf8Path,
    python_argv: &[String],
) -> std::io::Result<SchemaDepsOutcome> {
    if !bundles_usdgenschema(prefix) {
        return Ok(SchemaDepsOutcome::NotNeeded);
    }
    if SCHEMA_GEN_MODULES
        .iter()
        .all(|module| module_present(python_lib_dir, module))
    {
        return Ok(SchemaDepsOutcome::AlreadyPresent);
    }
    std::fs::create_dir_all(python_lib_dir.as_std_path())?;
    let (head, rest) = python_argv.split_first().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no interpreter to run pip with",
        )
    })?;
    let status = Command::new(head)
        .args(rest)
        .args(["-m", "pip", "install", "--no-input", "--target"])
        .arg(python_lib_dir.as_str())
        .args(SCHEMA_GEN_PACKAGES)
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other(format!(
            "pip install of schema-gen deps failed ({status})"
        )));
    }
    Ok(SchemaDepsOutcome::Installed(
        SCHEMA_GEN_PACKAGES.iter().map(|s| s.to_string()).collect(),
    ))
}

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

/// The Python include directory baked into `pxrConfig.cmake` at USD build
/// time (`set(Python3_INCLUDE_DIR [[<path>]])`), in its original (native)
/// separator form. This is the exact string USD also embeds into its exported
/// targets' `INTERFACE_INCLUDE_DIRECTORIES` — the source path for relocation.
fn baked_python_include(pxr_config: &str) -> Option<String> {
    let idx = pxr_config.find("set(Python3_INCLUDE_DIR [[")?;
    let rest = &pxr_config[idx + "set(Python3_INCLUDE_DIR [[".len()..];
    let path = rest.split("]]").next()?.trim();
    (!path.is_empty()).then(|| path.to_string())
}

/// The CMake files a consumer actually loads via `find_package(pxr)`: the
/// config package root plus its `cmake/` directory. USD's `build/` subtree
/// (a full build tree, when the runtime was adopted from one) is never loaded
/// by find_package, so it is deliberately excluded — rewriting it would be
/// both pointless and slow (hundreds of files).
fn loaded_cmake_files(prefix: &Utf8Path) -> Vec<Utf8PathBuf> {
    let mut files = Vec::new();
    let root = prefix.join("pxrConfig.cmake");
    if root.as_std_path().is_file() {
        files.push(root);
    }
    if let Ok(entries) = std::fs::read_dir(prefix.join("cmake").as_std_path()) {
        for e in entries.flatten() {
            if let Ok(p) = Utf8PathBuf::from_path_buf(e.path()) {
                if p.extension() == Some("cmake") {
                    files.push(p);
                }
            }
        }
    }
    files
}

/// Replace every occurrence of `old` (in either separator form) with `new`
/// (forward slashes) across `files`. Returns the number of files changed.
fn replace_in_files(files: &[Utf8PathBuf], old: &str, new: &str) -> std::io::Result<usize> {
    let variants = [old.replace('\\', "/"), old.replace('/', "\\")];
    let new = new.replace('\\', "/");
    let mut changed = 0usize;
    for file in files {
        let Ok(text) = std::fs::read_to_string(file.as_std_path()) else {
            continue;
        };
        let mut out = text.clone();
        for v in &variants {
            if !v.is_empty() {
                out = out.replace(v.as_str(), &new);
            }
        }
        if out != text {
            std::fs::write(file.as_std_path(), out)?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// Relocate an adopted runtime's baked Python include path in its exported
/// CMake files to `replacement`, but only when the baked path is **stale**
/// (absent on this host). USD embeds the build machine's Python include into
/// `pxrTargets.cmake`'s `INTERFACE_INCLUDE_DIRECTORIES` /
/// `INTERFACE_SYSTEM_INCLUDE_DIRECTORIES`; CMake hard-errors at generate time
/// if an imported target's include path does not exist ("Imported target …
/// includes non-existent path"). Pinning `Python3_INCLUDE_DIR` in the
/// toolchain does not help — the property is baked into the target itself.
///
/// Guard: if the baked path exists on this host (the export machine, or an
/// identical layout), nothing is rewritten — so a developer's real USD tree
/// is never mutated. Returns the number of files changed.
pub fn relocate_baked_python(prefix: &Utf8Path, replacement: &str) -> std::io::Result<usize> {
    let Ok(config) = std::fs::read_to_string(prefix.join("pxrConfig.cmake").as_std_path()) else {
        return Ok(0);
    };
    let Some(baked) = baked_python_include(&config) else {
        return Ok(0);
    };
    // Only relocate a path that is genuinely stale on this host.
    if std::path::Path::new(&baked).is_dir() {
        return Ok(0);
    }
    replace_in_files(&loaded_cmake_files(prefix), &baked, replacement)
}

/// Relocate an adopted runtime's own baked install prefix to `prefix` (its
/// current on-host location) in the exported CMake files.
///
/// A runtime adopted from a full USD **build tree** bakes that tree's absolute
/// path into the external-dependency imported targets in `pxrConfig.cmake`
/// (TBB / MaterialX `INTERFACE_INCLUDE_DIRECTORIES`, `IMPORTED_IMPLIB`,
/// `IMPORTED_LOCATION`, `MaterialX_DIR`, …). The config package anchors *its
/// own* targets relatively (`get_filename_component(PXR_CMAKE_DIR …)` /
/// `_IMPORT_PREFIX`), but these dependency paths are absolute, so on a
/// different host they point nowhere and CMake fails at generate/link time.
///
/// The old prefix is not recorded in metadata (an imported artifact is
/// self-contained), so it is **discovered** from the baked files: an absolute
/// directory `X`, absent on this host, whose `X/{include,lib,bin}` is
/// referenced while the same subdir exists under the current `prefix` (the
/// export bundled the layout). That guard makes the rewrite safe — it fires
/// only for a genuinely relocated runtime, never a developer's in-place tree.
/// Call *after* [`relocate_baked_python`] so host-relocated Python paths
/// (which now exist) are not mistaken for the stale install prefix.
pub fn relocate_baked_prefix(prefix: &Utf8Path) -> std::io::Result<usize> {
    let current = prefix.as_str().trim_end_matches('/').replace('\\', "/");
    let files = loaded_cmake_files(prefix);
    let Some(old) = discover_stale_prefix(&files, &current) else {
        return Ok(0);
    };
    replace_in_files(&files, &old, &current)
}

/// Discover a stale baked install prefix in `files`: an absolute directory
/// that is absent on this host but whose `include`/`lib`/`bin` subdir exists
/// under `current` (so replacing it yields real paths). Returns `None` when
/// nothing qualifies. See [`relocate_baked_prefix`].
fn discover_stale_prefix(files: &[Utf8PathBuf], current: &str) -> Option<String> {
    const SUBDIRS: [&str; 3] = ["/include", "/lib", "/bin"];
    for file in files {
        let Ok(text) = std::fs::read_to_string(file.as_std_path()) else {
            continue;
        };
        for raw in text.split(|c: char| {
            matches!(
                c,
                '"' | ';' | '<' | '>' | '$' | '(' | ')' | '\n' | '\r' | '\t' | ' ' | ','
            )
        }) {
            let tok = raw.replace('\\', "/");
            if !is_absolute_path(&tok) || tok.starts_with(current) {
                continue;
            }
            for seg in SUBDIRS {
                let Some(idx) = tok.find(seg) else { continue };
                // Require a real path boundary after the segment.
                let after = &tok[idx + seg.len()..];
                if !after.is_empty() && !after.starts_with('/') {
                    continue;
                }
                let base = &tok[..idx];
                if base.is_empty() || std::path::Path::new(base).exists() {
                    continue; // empty, or not actually stale
                }
                // The counterpart must exist under the current prefix.
                if std::path::Path::new(&format!("{current}{seg}")).exists() {
                    return Some(base.to_string());
                }
            }
        }
    }
    None
}

/// A drive-rooted (`C:/…`) or POSIX-absolute (`/…`) path with real content.
fn is_absolute_path(s: &str) -> bool {
    let b = s.as_bytes();
    (b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'/')
        || (b.len() > 1 && b[0] == b'/')
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

/// Resolve an interpreter *to run a script with* (e.g. `usdGenSchema`), returned
/// as an argv whose head is the program and whose tail is any leading args (the
/// Windows `py -3.11` launcher form). Unlike [`resolve_for_runtime`], this does
/// **not** require Development artifacts — running a script only needs a working
/// interpreter — and it prefers the runtime's own bundled interpreter so the
/// script's `import pxr` matches the runtime ABI. Falls back to a host
/// `python{ver}`/`py`, then `python3`, then `python` (never a bare `python`
/// first, which macOS/modern Linux lack). Host fallbacks must report the
/// required `major.minor` when one is known, so `import pxr` is not attempted
/// with a mismatched ABI. Returns `None` if nothing suitable is found — the
/// caller reports a precondition naming what was searched.
///
/// The required `major.minor` (used to prefer a version-matched host
/// interpreter) is taken from the runtime's `pxrConfig.cmake`, else `declared`.
pub fn resolve_run_python(prefix: &Utf8Path, declared: &str) -> Option<Vec<String>> {
    let required = usd_python_requirement(prefix).or_else(|| major_minor(declared));
    for (argv, source) in candidates(prefix, required.as_deref()) {
        if runnable_for_run(&argv, source, required.as_deref()) {
            return Some(argv);
        }
    }
    None
}

/// A human list of the interpreters [`resolve_run_python`] would try, for a
/// precondition error message when none is runnable.
pub fn run_python_search_paths(prefix: &Utf8Path, declared: &str) -> Vec<String> {
    let required = usd_python_requirement(prefix).or_else(|| major_minor(declared));
    candidates(prefix, required.as_deref())
        .into_iter()
        .map(|(argv, _)| argv.join(" "))
        .collect()
}

/// Whether an interpreter argv responds to `--version` (a cheap runnable probe
/// that does not need Development artifacts).
fn runnable(argv: &[String]) -> bool {
    let Some((head, rest)) = argv.split_first() else {
        return false;
    };
    Command::new(head)
        .args(rest)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Whether an interpreter argv is suitable for running USD scripts. Runtime
/// bundled interpreters only need the cheap runnable probe; host interpreters
/// must match USD's Python ABI when that `major.minor` is known.
fn runnable_for_run(argv: &[String], source: PythonSource, required: Option<&str>) -> bool {
    if source == PythonSource::Host {
        if let Some(required) = required {
            return run_python_major_minor(argv).as_deref() == Some(required);
        }
    }
    runnable(argv)
}

fn run_python_major_minor(argv: &[String]) -> Option<String> {
    let (head, rest) = argv.split_first()?;
    let out = Command::new(head)
        .args(rest)
        .arg("-c")
        .arg("import sys; print('%d.%d' % sys.version_info[:2])")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout)
        .ok()
        .and_then(|s| s.lines().next().map(str::trim).map(str::to_string))
        .filter(|s| !s.is_empty())
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
    fn run_python_search_order_prefers_runtime_and_never_bare_python_first() {
        // No bundled interpreter in an empty prefix: the ordered candidates must
        // still end with `python3` before a bare `python` (the macOS/Linux
        // reality that motivated the fix), never leading with `python`.
        let prefix = Utf8PathBuf::from("/nonexistent/runtime/prefix");
        let searched = run_python_search_paths(&prefix, "3.11");
        let py3 = searched.iter().position(|p| p == "python3");
        let py = searched.iter().position(|p| p == "python");
        assert!(py3.is_some() && py.is_some(), "got {searched:?}");
        assert!(py3 < py, "python3 must precede bare python: {searched:?}");
        assert_ne!(searched.first().map(String::as_str), Some("python"));
        // A version-matched host candidate is offered ahead of the bare
        // fallbacks — `py -3.11` on Windows, `python3.11` elsewhere.
        let version_matched = if cfg!(windows) {
            "py -3.11"
        } else {
            "python3.11"
        };
        assert!(
            searched.iter().any(|p| p == version_matched),
            "expected a version-matched candidate ({version_matched}): {searched:?}"
        );
    }

    #[test]
    fn run_python_prefers_bundled_runtime_interpreter() {
        // A prefix that bundles bin/python3 is offered first, as an existing file.
        let dir = std::env::temp_dir().join(format!("ost-runpy-{}", std::process::id()));
        let prefix = Utf8PathBuf::from_path_buf(dir.clone()).unwrap();
        let bin = if cfg!(windows) {
            prefix.join("python.exe")
        } else {
            prefix.join("bin/python3")
        };
        std::fs::create_dir_all(bin.parent().unwrap().as_std_path()).unwrap();
        std::fs::write(bin.as_std_path(), b"#!/bin/sh\n").unwrap();
        let searched = run_python_search_paths(&prefix, "3.11");
        std::fs::remove_dir_all(dir).ok();
        assert_eq!(
            searched.first().map(String::as_str),
            Some(bin.to_string().as_str()),
            "bundled interpreter must be tried first: {searched:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_python_rejects_host_fallback_with_wrong_minor() {
        use std::os::unix::fs::PermissionsExt;

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("ost-runpy-version-{}-{nanos}", std::process::id()));
        let fake = Utf8PathBuf::from_path_buf(dir.clone())
            .unwrap()
            .join("fake-python");
        std::fs::create_dir_all(fake.parent().unwrap().as_std_path()).unwrap();
        std::fs::write(
            fake.as_std_path(),
            b"#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then exit 0; fi\nif [ \"$1\" = \"-c\" ]; then printf '3.10\\n'; exit 0; fi\nexit 1\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(fake.as_std_path()).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(fake.as_std_path(), perms).unwrap();

        let argv = vec![fake.to_string()];
        assert!(runnable_for_run(&argv, PythonSource::Host, Some("3.10")));
        assert!(!runnable_for_run(&argv, PythonSource::Host, Some("3.11")));
        assert!(runnable_for_run(&argv, PythonSource::Host, None));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_bundled_usdgenschema_and_module_presence() {
        let dir = std::env::temp_dir().join(format!("ost-schemadeps-{}", std::process::id()));
        let prefix = Utf8PathBuf::from_path_buf(dir.clone()).unwrap();
        let bin = prefix.join("bin");
        let pylib = prefix.join("lib/python");
        std::fs::create_dir_all(bin.as_std_path()).unwrap();
        std::fs::create_dir_all(pylib.as_std_path()).unwrap();

        assert!(!bundles_usdgenschema(&prefix));
        std::fs::write(bin.join("usdGenSchema").as_std_path(), b"").unwrap();
        assert!(bundles_usdgenschema(&prefix));

        // jinja2/markupsafe as package dirs; a single-file module also counts.
        assert!(!module_present(&pylib, "jinja2"));
        std::fs::create_dir_all(pylib.join("jinja2").as_std_path()).unwrap();
        assert!(module_present(&pylib, "jinja2"));
        assert!(!SCHEMA_GEN_MODULES
            .iter()
            .all(|module| module_present(&pylib, module)));
        std::fs::write(pylib.join("markupsafe.py").as_std_path(), b"").unwrap();
        assert!(module_present(&pylib, "markupsafe"));
        assert!(SCHEMA_GEN_MODULES
            .iter()
            .all(|module| module_present(&pylib, module)));

        // Already-present deps are a no-op even though the interpreter argv is
        // bogus — the pip subprocess must never be reached.
        assert_eq!(
            provision_schema_gen_deps(&prefix, &pylib, &["definitely-not-python".into()]).unwrap(),
            SchemaDepsOutcome::AlreadyPresent
        );

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn provision_is_not_needed_without_usdgenschema() {
        let dir = std::env::temp_dir().join(format!("ost-noschemadeps-{}", std::process::id()));
        let prefix = Utf8PathBuf::from_path_buf(dir.clone()).unwrap();
        let pylib = prefix.join("lib/python");
        std::fs::create_dir_all(pylib.as_std_path()).unwrap();
        assert_eq!(
            provision_schema_gen_deps(&prefix, &pylib, &["definitely-not-python".into()]).unwrap(),
            SchemaDepsOutcome::NotNeeded
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn probe_rejects_missing_interpreter() {
        assert!(probe(
            &["definitely-not-a-python-anywhere".to_string()],
            PythonSource::Host
        )
        .is_none());
    }

    #[test]
    fn parses_baked_python_include_native_form() {
        let config =
            "set(Python3_INCLUDE_DIR [[C:\\Users\\bob\\Python310\\Include]] CACHE PATH \"\")";
        assert_eq!(
            baked_python_include(config),
            Some("C:\\Users\\bob\\Python310\\Include".to_string())
        );
        assert_eq!(baked_python_include("find_package(pxr)"), None);
    }

    #[test]
    fn relocate_rewrites_stale_include_in_both_separator_forms() {
        let dir = std::env::temp_dir().join(format!("ost-reloc-{}", std::process::id()));
        let prefix = Utf8PathBuf::from_path_buf(dir.clone()).unwrap();
        let cmake = prefix.join("cmake");
        std::fs::create_dir_all(cmake.as_std_path()).unwrap();
        // A stale (non-existent) export-machine include, in native form in
        // pxrConfig and forward-slash form in pxrTargets.
        let stale = "C:\\Users\\ghost\\Py310\\Include";
        std::fs::write(
            prefix.join("pxrConfig.cmake").as_std_path(),
            format!("set(Python3_INCLUDE_DIR [[{stale}]] CACHE PATH \"\")\n"),
        )
        .unwrap();
        std::fs::write(
            cmake.join("pxrTargets.cmake").as_std_path(),
            "INTERFACE_INCLUDE_DIRECTORIES \"${_IMPORT_PREFIX}/include;C:/Users/ghost/Py310/Include\"\n",
        )
        .unwrap();

        let changed = relocate_baked_python(&prefix, "D:/host/py/include").unwrap();
        assert_eq!(changed, 2);
        let targets =
            std::fs::read_to_string(cmake.join("pxrTargets.cmake").as_std_path()).unwrap();
        assert!(targets.contains("D:/host/py/include"));
        assert!(!targets.contains("ghost"));

        // Idempotent: a second pass (stale path now gone) changes nothing.
        assert_eq!(
            relocate_baked_python(&prefix, "D:/host/py/include").unwrap(),
            0
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn relocate_leaves_existing_include_untouched() {
        let dir = std::env::temp_dir().join(format!("ost-reloc-keep-{}", std::process::id()));
        let prefix = Utf8PathBuf::from_path_buf(dir.clone()).unwrap();
        std::fs::create_dir_all(prefix.as_std_path()).unwrap();
        // Point the baked include at a real directory (this temp dir) — a
        // developer's own tree must never be rewritten.
        let real = prefix.as_std_path().to_string_lossy();
        std::fs::write(
            prefix.join("pxrConfig.cmake").as_std_path(),
            format!("set(Python3_INCLUDE_DIR [[{real}]] CACHE PATH \"\")\n"),
        )
        .unwrap();
        assert_eq!(
            relocate_baked_python(&prefix, "D:/host/include").unwrap(),
            0
        );
        std::fs::remove_dir_all(dir).ok();
    }

    fn temp_prefix(tag: &str) -> (std::path::PathBuf, Utf8PathBuf) {
        let dir = std::env::temp_dir().join(format!("ost-{tag}-{}", std::process::id()));
        let prefix = Utf8PathBuf::from_path_buf(dir.clone()).unwrap();
        // A relocated runtime bundles its own include/lib layout.
        std::fs::create_dir_all(prefix.join("include").as_std_path()).unwrap();
        std::fs::create_dir_all(prefix.join("lib").as_std_path()).unwrap();
        (dir, prefix)
    }

    #[test]
    fn relocate_prefix_rewrites_stale_install_prefix() {
        let (dir, prefix) = temp_prefix("reloc-prefix");
        // pxrConfig bakes a stale build-tree prefix into dependency targets.
        std::fs::write(
            prefix.join("pxrConfig.cmake").as_std_path(),
            "_add_property(INTERFACE_INCLUDE_DIRECTORIES \"C:/old/build/tree/include\")\n\
             _add_property(IMPORTED_IMPLIB \"C:/old/build/tree/lib/tbb.lib\")\n\
             set(MaterialX_DIR [[C:\\old\\build\\tree\\lib\\cmake\\MaterialX]])\n",
        )
        .unwrap();
        let current = prefix.as_str().trim_end_matches('/').replace('\\', "/");

        let changed = relocate_baked_prefix(&prefix).unwrap();
        assert_eq!(changed, 1);
        let cfg = std::fs::read_to_string(prefix.join("pxrConfig.cmake").as_std_path()).unwrap();
        // Both separator forms of the stale prefix are gone; forward-slash
        // paths get a clean host prefix, and the backslash MaterialX_DIR at
        // least has its (stale) prefix replaced (a mixed-separator tail is
        // CMake-tolerated).
        assert!(!cfg.contains("C:/old/build/tree"));
        assert!(!cfg.contains("C:\\old\\build\\tree"));
        assert!(cfg.contains(&format!("{current}/include")));
        assert!(cfg.contains(&format!("{current}/lib/tbb.lib")));
        assert!(cfg.contains(current.as_str()));

        // Idempotent once the stale prefix is gone.
        assert_eq!(relocate_baked_prefix(&prefix).unwrap(), 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn relocate_prefix_noop_without_bundled_counterpart() {
        // No include/lib created under the prefix → a discovered stale path has
        // no counterpart to relocate to, so nothing is rewritten.
        let dir = std::env::temp_dir().join(format!("ost-reloc-nocp-{}", std::process::id()));
        let prefix = Utf8PathBuf::from_path_buf(dir.clone()).unwrap();
        std::fs::create_dir_all(prefix.as_std_path()).unwrap();
        std::fs::write(
            prefix.join("pxrConfig.cmake").as_std_path(),
            "_add_property(INTERFACE_INCLUDE_DIRECTORIES \"C:/old/build/tree/include\")\n",
        )
        .unwrap();
        assert_eq!(relocate_baked_prefix(&prefix).unwrap(), 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn is_absolute_path_recognizes_windows_and_posix() {
        assert!(is_absolute_path("C:/dev/build"));
        assert!(is_absolute_path("/usr/local"));
        assert!(!is_absolute_path("relative/path"));
        assert!(!is_absolute_path("${_IMPORT_PREFIX}/include"));
        assert!(!is_absolute_path(""));
    }
}
