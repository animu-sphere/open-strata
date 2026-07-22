// SPDX-License-Identifier: Apache-2.0
pub mod artifact;
pub mod build;
pub mod ci;
pub mod compiler;
pub mod configure;
pub mod devshell;
pub mod doctor;
pub mod env;
pub mod extension;
pub mod external;
pub mod formation;
pub mod init;
pub mod internal;
pub mod lock;
pub mod package;
pub mod platform;
pub mod plugin;
pub mod presets;
pub mod renderer;
pub mod runtime;
pub mod test;
pub mod uv;
pub mod validate;

use camino::Utf8PathBuf;

use ost_core::paths::Store;
use ost_core::{Error, Host, Result};
use ost_platform::Catalog;
use ost_runtime::{
    python_minor, EnvOp, EnvSet, EnvVar, ProfileCatalog, Runtime, RuntimeManifest, MANIFEST_FILE,
};

/// Everything needed to activate a runtime, shared by `env`, `devshell`, `runtime`.
pub struct Resolved {
    pub runtime: Runtime,
    /// Store location of the runtime: where its `runtime.json` manifest lives.
    pub prefix: Utf8PathBuf,
    /// Where the runtime's real artifacts (bin/lib/python) live. Equals `prefix`
    /// for mock/build runtimes; for an adopted `local` runtime it is the external
    /// USD install root recorded in the manifest.
    pub artifact_prefix: Utf8PathBuf,
    pub env: EnvSet,
    /// Platform Python version, e.g. `3.13.x`.
    pub python_version: String,
    /// C++ standard from the platform, e.g. `20`.
    pub cxx_standard: String,
    /// Capabilities provided by the selected profile.
    pub capabilities: Vec<String>,
    /// Whether the runtime has been pulled (its manifest exists on disk).
    pub pulled: bool,
}

/// Resolve a platform + profile selection into a runtime and its environment.
///
/// This does not pull artifacts; the prefix is the prospective store location.
pub fn resolve(platform_id: &str, profile_id: &str) -> Result<Resolved> {
    let platforms = Catalog::load()?;
    let platform = platforms.get(platform_id)?;
    let python_version = platform.component("python").ok_or_else(|| {
        Error::InvalidManifest(format!(
            "platform '{}' does not define a 'python' version",
            platform.id
        ))
    })?;

    let profiles = ProfileCatalog::load()?;
    let profile = profiles.get(profile_id)?;

    let host = Host::detect();
    let runtime = Runtime::resolve(&platform.id, &profile.id, &host, python_version);
    let store = Store::discover();
    let prefix = runtime.prefix(&store);

    let capabilities = profile.capabilities().to_vec();
    let usd_plugins = capabilities.iter().any(|c| c.starts_with("usd"));
    let pulled = prefix.join(MANIFEST_FILE).as_std_path().is_file();

    // An adopted (`local`) runtime keeps its manifest in the store but its real
    // artifacts live at an external root with USD's own layout. Read the manifest
    // to learn the source, then build the env against the effective prefix.
    let manifest = if pulled {
        std::fs::read_to_string(prefix.join(MANIFEST_FILE).as_std_path())
            .ok()
            .and_then(|s| RuntimeManifest::from_json(&s).ok())
    } else {
        None
    };

    // Real OpenUSD runtimes (adopted `local`, built `build`, fetched `artifact`)
    // carry USD's own install layout; the mock backend uses the OpenStrata layout.
    let (artifact_prefix, env) = match &manifest {
        Some(m) if m.source.is_real() => {
            let ep = Utf8PathBuf::from(m.effective_prefix(&prefix));
            let mut env = EnvSet::for_usd_install(&ep, host.os);
            // A build linked against external deps needs their lib dirs at runtime.
            for dep in &m.runtime_deps {
                env.add_dep_libs(camino::Utf8Path::new(dep), host.os);
            }
            (ep, env)
        }
        _ => {
            let env =
                EnvSet::for_runtime(&prefix, host.os, &python_minor(python_version), usd_plugins);
            (prefix.clone(), env)
        }
    };

    let cxx_standard = platform
        .component("cxx_standard")
        .unwrap_or("17")
        .to_string();

    Ok(Resolved {
        runtime,
        prefix,
        artifact_prefix,
        env,
        python_version: python_version.to_string(),
        cxx_standard,
        capabilities,
        pulled,
    })
}

/// Whether a capability needs a real OpenUSD behind it.
///
/// Not every OpenUSD-dependent capability is spelled `usd-*`: `hydra-preview`
/// enables USD imaging and is just as dependent, so a name-prefix test alone
/// reports a lookdev profile as needing nothing. This lives here because two
/// commands ask the same question and must not answer it differently — `uv`
/// deciding which native families the runtime already provides, and `doctor`
/// deciding whether a mock runtime is a real limitation or the profile working
/// as specified.
pub(crate) fn needs_openusd(capability: &str) -> bool {
    capability.starts_with("usd-") || capability == "hydra-preview"
}

/// Relocate an adopted runtime's baked absolute paths in USD's exported CMake
/// files to this host, when stale. Two independent bakes are handled: the
/// build machine's Python include (→ the resolved host include) and the
/// runtime's own install prefix embedded in external-dependency imported
/// targets (→ its current on-host location). Both are no-ops when the baked
/// paths already exist (the export machine), so a developer's own USD tree is
/// never touched. Python is relocated first so its now-valid host paths are
/// not mistaken for the stale install prefix. Shared by `configure` and
/// `plugin build`.
pub(crate) fn relocate_baked_python_if_stale(
    artifact_prefix: &camino::Utf8Path,
    python: Option<&ost_build::PythonHints>,
) {
    if let Some(h) = python {
        if let Ok(n) = ost_build::relocate_baked_python(artifact_prefix, &h.include_dir) {
            if n > 0 {
                println!("==> relocated baked Python include in {n} runtime CMake file(s)");
            }
        }
    }
    if let Ok(n) = ost_build::relocate_baked_prefix(artifact_prefix) {
        if n > 0 {
            println!("==> relocated baked runtime install prefix in {n} CMake file(s)");
        }
    }
}

/// Put a matching host Python interpreter's directory on the session's
/// dynamic-library path when the runtime does not bundle one.
///
/// An adopted USD runtime links its tools (`usdcat`, `usdview`) and its `pxr`
/// bindings against `pythonXY.dll`, but a build-tree adoption does not ship the
/// interpreter — so on a clean host those tools fail to start (`usdcat` exits
/// silently; `import pxr` fails loading `_tf`). Prepending the resolved host
/// interpreter's directory to the loader path (PATH on Windows) supplies
/// `pythonXY.dll` and makes a version-matched `python` discoverable for the
/// pyramid's Python levels. A no-op when nothing matching is found. Returns the
/// augmented set unchanged otherwise.
pub(crate) fn with_host_python_on_path(
    mut env: EnvSet,
    artifact_prefix: &camino::Utf8Path,
    python_version: &str,
    os: ost_core::host::Os,
) -> EnvSet {
    let Some(hints) = ost_build::resolve_for_runtime(artifact_prefix, python_version) else {
        return env;
    };
    let Some(dir) = camino::Utf8Path::new(&hints.executable).parent() else {
        return env;
    };
    let key = match os {
        ost_core::host::Os::Windows => "PATH",
        ost_core::host::Os::Macos => "DYLD_LIBRARY_PATH",
        ost_core::host::Os::Linux => "LD_LIBRARY_PATH",
    };
    // Pushed last so it wins priority (front of the path): a version-matched
    // interpreter and its DLLs are found ahead of any host default.
    env.vars.push(EnvVar {
        key: key.into(),
        op: EnvOp::Prepend(dir.to_string().replace('\\', "/")),
    });
    // The interpreter directory itself also needs to be on PATH so a bare
    // `python` resolves to the matched one (macOS/Linux route DLLs elsewhere).
    if os != ost_core::host::Os::Windows {
        env.vars.push(EnvVar {
            key: "PATH".into(),
            op: EnvOp::Prepend(dir.to_string().replace('\\', "/")),
        });
    }
    env
}

/// Prepare a package staging directory and translate the outcome into
/// warning/note JSON shared by `ost package` and `ost plugin package`.
///
/// A rerun must not fail on a stage a previous run left temporarily undeletable
/// (scanner-held handles, dogfooding report #9): [`prepare_staging_dir`] stages
/// into a fresh sibling instead. This surfaces that as an actionable
/// `STAGE_FALLBACK` warning — naming any stale siblings still locked and the
/// `--clean-stage` escape hatch — rather than the previous recurring, opaque
/// note (dogfooding report 2026-07-10 ask #4). `clean` is `--clean-stage`: it
/// reclaims the stable name harder and, even on success, reports what it swept.
pub(crate) fn prepare_package_stage(
    preferred: &camino::Utf8Path,
    clean: bool,
) -> Result<(Utf8PathBuf, Vec<serde_json::Value>)> {
    let outcome = ost_core::fs::prepare_staging_dir(preferred.as_std_path(), clean)?;
    let stage = Utf8PathBuf::from_path_buf(outcome.path.clone())
        .map_err(|p| Error::Operation(format!("non-UTF-8 staging path: {}", p.display())))?;

    let mut warnings = Vec::new();
    if outcome.fell_back(preferred.as_std_path()) {
        let leftover_note = if outcome.leftover > 0 {
            format!(
                "; {} stale fallback stage(s) are still locked — rerun with --clean-stage \
                 once the holding process exits to reclaim them",
                outcome.leftover
            )
        } else {
            "; a later run (or --clean-stage) sweeps it".to_string()
        };
        warnings.push(serde_json::json!({
            "code": "STAGE_FALLBACK",
            "message": format!(
                "previous package stage '{preferred}' is held open by another process; \
                 staged into '{stage}' instead{leftover_note}"
            ),
            "fallback_stage": stage.to_string(),
            "leftover": outcome.leftover,
        }));
    } else if clean && (outcome.swept > 0 || outcome.leftover > 0) {
        // The operator asked to clean and something was there to clean: report
        // the accounting even though staging itself did not fall back.
        warnings.push(serde_json::json!({
            "code": "STAGE_CLEANED",
            "message": format!(
                "reclaimed the package stage '{preferred}'; swept {} stale fallback stage(s), \
                 {} still locked",
                outcome.swept, outcome.leftover
            ),
            "swept": outcome.swept,
            "leftover": outcome.leftover,
        }));
    }
    Ok((stage, warnings))
}

#[cfg(test)]
mod stage_tests {
    use super::*;

    fn scratch(tag: &str) -> Utf8PathBuf {
        let dir = std::env::temp_dir().join(format!("ost-stage-{tag}-{}", std::process::id()));
        Utf8PathBuf::from_path_buf(dir).unwrap()
    }

    #[test]
    fn ordinary_run_reclaims_the_stable_name_with_no_warnings() {
        let dir = scratch("plain");
        let preferred = dir.join("package-stage");
        std::fs::create_dir_all(preferred.join("lib").as_std_path()).unwrap();

        let (stage, warnings) = prepare_package_stage(&preferred, false).unwrap();
        assert_eq!(stage, preferred, "an unlocked stage is reused in place");
        assert!(warnings.is_empty(), "no fallback, no warning: {warnings:?}");

        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    #[test]
    fn clean_stage_reports_what_it_swept() {
        // `--clean-stage` reclaims the stable name and, because a stale fallback
        // sibling was present, reports the sweep as an actionable note.
        let dir = scratch("clean");
        let preferred = dir.join("package-stage");
        std::fs::create_dir_all(preferred.as_std_path()).unwrap();
        let stale = dir.join("package-stage-0123456789abcdef");
        std::fs::create_dir_all(stale.as_std_path()).unwrap();

        let (stage, warnings) = prepare_package_stage(&preferred, true).unwrap();
        assert_eq!(stage, preferred);
        assert!(!stale.as_std_path().exists(), "stale fallback was swept");
        assert_eq!(warnings.len(), 1, "the sweep is reported");
        assert_eq!(warnings[0]["code"], "STAGE_CLEANED");
        assert_eq!(warnings[0]["swept"], 1);
        assert_eq!(warnings[0]["leftover"], 0);

        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }
}
