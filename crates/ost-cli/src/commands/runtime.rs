// SPDX-License-Identifier: Apache-2.0
//! `ost runtime` — pull / list / show runtimes (§14.2).
//!
//! `pull` writes a digest-bearing `runtime.json` under `~/.ost/runtimes/<id>`
//! from one of several backend **sources** (§ Phase 4b): `mock` materializes a
//! placeholder layout; `local` (`--from-usd`) adopts an existing OpenUSD install
//! in place; `build` (`--build <usd-src>`) builds OpenUSD from source into the
//! store via `build_usd.py`. The `artifact` source (fetch prebuilt) lands with
//! Phase 6.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Subcommand;

use camino::{Utf8Path, Utf8PathBuf};

use ost_core::host::Os;
use ost_core::paths::Store;
use ost_core::{tools, Error, Host, Result};
use ost_runtime::{
    python_minor, ExtensionRecord, RuntimeManifest, RuntimeSource, Validation, MANIFEST_FILE,
};

use crate::commands::resolve;
use crate::output::{self, Format};

/// Read an environment variable, treating empty as unset.
fn env_nonempty(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

#[derive(Debug, Subcommand)]
pub enum RuntimeCmd {
    /// Materialize a runtime into the local store.
    Pull {
        /// Platform calendar-year id, e.g. `cy2026`.
        platform: String,
        /// Profile to pull, e.g. `usd` or `lookdev`.
        #[arg(long, default_value = "core")]
        profile: String,
        /// Re-pull even if the runtime already exists.
        #[arg(long)]
        force: bool,
        /// Adopt an existing OpenUSD install at this path instead of
        /// materializing a mock layout (`local` source). Falls back to
        /// `OST_USD_ROOT` when unset.
        #[arg(long)]
        from_usd: Option<String>,
        /// Build OpenUSD from source into the store (`build` source), via the
        /// source tree's `build_scripts/build_usd.py`. Falls back to
        /// `OST_USD_SRC` when no path is given.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        build: Option<String>,
        /// Parallel build jobs for `--build` (passed to the builder as `-j`).
        #[arg(long)]
        jobs: Option<u32>,
        /// Extra argument forwarded to the builder (repeatable). With
        /// build_usd.py: e.g. `--build-arg --no-imaging`. With `--deps` (CMake):
        /// e.g. `--build-arg -DPXR_BUILD_IMAGING=OFF`. Hyphen values allowed.
        #[arg(long = "build-arg", allow_hyphen_values = true)]
        build_args: Vec<String>,
        /// Dependency prefix for a direct CMake build of `--build` (repeatable;
        /// joined into `CMAKE_PREFIX_PATH`). When given, OpenUSD is built with
        /// CMake against these deps instead of via build_usd.py. Falls back to
        /// `OST_USD_DEPS` (path-separator list).
        #[arg(long)]
        deps: Vec<String>,
    },
    /// List runtimes present in the local store.
    List,
    /// Show the manifest of a pulled runtime.
    Show {
        /// Platform calendar-year id, e.g. `cy2026`.
        platform: String,
        /// Profile, e.g. `usd`.
        #[arg(long, default_value = "core")]
        profile: String,
    },
    /// Validate a pulled runtime and record the outcome in its manifest.
    Validate {
        /// Platform calendar-year id, e.g. `cy2026`.
        platform: String,
        /// Profile, e.g. `usd`.
        #[arg(long, default_value = "core")]
        profile: String,
    },
    /// Explain how a profile resolves to capabilities and extensions.
    Explain {
        /// Platform calendar-year id, e.g. `cy2026`.
        platform: String,
        /// Profile, e.g. `lookdev`.
        #[arg(long, default_value = "core")]
        profile: String,
    },
}

pub fn run(cmd: RuntimeCmd, fmt: Format) -> Result<()> {
    match cmd {
        RuntimeCmd::Pull {
            platform,
            profile,
            force,
            from_usd,
            build,
            jobs,
            build_args,
            deps,
        } => pull(
            &platform,
            &profile,
            force,
            PullSource {
                from_usd,
                build,
                jobs,
                build_args,
                deps,
            },
            fmt,
        ),
        RuntimeCmd::List => list(fmt),
        RuntimeCmd::Show { platform, profile } => show(&platform, &profile, fmt),
        RuntimeCmd::Validate { platform, profile } => validate(&platform, &profile, fmt),
        RuntimeCmd::Explain { platform, profile } => explain(&platform, &profile, fmt),
    }
}

/// Subdirectories the local backend creates inside a runtime prefix.
fn layout_dirs(python_version: &str, has_usd: bool) -> Vec<String> {
    let mut dirs = vec![
        "bin".to_string(),
        "lib".to_string(),
        format!("lib/python{}/site-packages", python_minor(python_version)),
        "include".to_string(),
        "share/cmake".to_string(),
    ];
    if has_usd {
        dirs.push("plugin/usd".to_string());
    }
    dirs
}

/// How `pull` should obtain the runtime: mock (default), adopt, or build.
pub struct PullSource {
    /// `--from-usd <path>` (or `OST_USD_ROOT`): adopt an existing install.
    pub from_usd: Option<String>,
    /// `--build [<path>]` (or `OST_USD_SRC`): build from source. `Some("")`
    /// means the flag was given without a path (use the env fallback).
    pub build: Option<String>,
    pub jobs: Option<u32>,
    pub build_args: Vec<String>,
    /// `--deps <prefix>` (or `OST_USD_DEPS`): when non-empty, build OpenUSD with
    /// CMake directly against these dependency prefixes instead of build_usd.py.
    pub deps: Vec<String>,
}

fn pull(platform: &str, profile: &str, force: bool, src: PullSource, fmt: Format) -> Result<()> {
    let r = resolve(platform, profile)?;

    if r.pulled && !force {
        return Err(Error::Operation(format!(
            "runtime '{}' already pulled (use --force to re-pull)",
            r.runtime.id()
        )));
    }

    // Resolve the profile's capabilities to concrete extensions. This drives
    // both the prefix layout (USD plugins) and the recorded provenance, so
    // `pull` agrees with `runtime explain`.
    let catalog = ost_extension::load_all()?;
    let resolution = ost_extension::resolve(&catalog, &r.capabilities);
    let has_usd = resolution.extensions.iter().any(|e| e.id == "openusd");
    let extensions: Vec<ExtensionRecord> = resolution
        .extensions
        .iter()
        .map(|e| ExtensionRecord {
            id: e.id.clone(),
            version: e.version.clone(),
            features: e.features.iter().cloned().collect(),
        })
        .collect();

    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Choose the backend source. Precedence: build > adopt > mock.
    let adopt = src.from_usd.or_else(|| env_nonempty("OST_USD_ROOT"));
    let build_src = src.build.map(|p| {
        if p.is_empty() {
            env_nonempty("OST_USD_SRC").unwrap_or_default()
        } else {
            p
        }
    });

    // Dependency prefixes for a CMake-direct build (flag, else env list).
    let deps: Vec<String> = if !src.deps.is_empty() {
        src.deps.clone()
    } else {
        env_nonempty("OST_USD_DEPS")
            .map(|v| split_dep_prefixes(&v))
            .unwrap_or_default()
    };

    let manifest = if let Some(usd_src) = build_src {
        let opts = BuildOpts {
            jobs: src.jobs,
            extra: src.build_args,
            deps,
        };
        build_from_source(&r, &usd_src, &opts, extensions, created)?
    } else if let Some(usd_root) = adopt {
        adopt_local(&r, &usd_root, extensions, created)?
    } else {
        materialize_mock(&r, has_usd, extensions, created)?
    };

    let manifest_path = r.prefix.join(MANIFEST_FILE);
    let json = manifest
        .to_json()
        .map_err(|e| Error::parse(MANIFEST_FILE, anyhow::Error::new(e)))?;
    std::fs::write(manifest_path.as_std_path(), format!("{json}\n"))
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;

    if fmt.is_json() {
        output::json(&serde_json::json!({
            "pulled": true,
            "runtime": manifest.id,
            "prefix": r.prefix.to_string(),
            "digest": manifest.digest,
            "source": manifest.source.as_str(),
            "external_prefix": manifest.external_prefix,
            "layout": manifest.layout,
            "extensions": manifest.extensions,
        }));
        return Ok(());
    }

    println!(
        "{} runtime {} ({})",
        match manifest.source {
            RuntimeSource::Local => "Adopted",
            RuntimeSource::Build => "Built",
            _ => "Pulled",
        },
        manifest.id,
        manifest.source.as_str()
    );
    println!("  prefix:  {}", r.prefix);
    if let Some(ext) = &manifest.external_prefix {
        println!("  usd:     {ext}");
    }
    println!("  digest:  {}", manifest.digest);
    println!("  layout:  {}", manifest.layout.join(", "));
    if !manifest.extensions.is_empty() {
        let names: Vec<String> = manifest
            .extensions
            .iter()
            .map(|e| format!("{} {}", e.id, e.version))
            .collect();
        println!("  extensions: {}", names.join(", "));
    }
    println!("\nValidate with:");
    println!("  ost runtime validate {} --profile {}", platform, profile);
    Ok(())
}

/// Split an `OST_USD_DEPS` value into dependency prefixes using the platform's
/// PATH separator (`;` on Windows, `:` elsewhere). Using the OS separator —
/// rather than splitting on both — keeps Windows drive letters (`C:/deps`)
/// intact.
fn split_dep_prefixes(value: &str) -> Vec<String> {
    std::env::split_paths(value)
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// Materialize the mock prefix layout (no real OpenUSD): the original backend.
fn materialize_mock(
    r: &crate::commands::Resolved,
    has_usd: bool,
    extensions: Vec<ExtensionRecord>,
    created: u64,
) -> Result<RuntimeManifest> {
    let layout = layout_dirs(&r.python_version, has_usd);
    for sub in &layout {
        let dir = r.prefix.join(sub);
        std::fs::create_dir_all(dir.as_std_path()).map_err(|e| Error::io(dir.to_string(), e))?;
    }
    Ok(RuntimeManifest::build(
        &r.runtime,
        &r.python_version,
        r.capabilities.clone(),
        layout,
        extensions,
        created,
        RuntimeSource::Mock,
    ))
}

/// Adopt an existing OpenUSD install at `usd_root` in place (`local` source):
/// record a manifest in the store that points at the external prefix, without
/// copying or building. The real artifacts keep USD's own layout.
fn adopt_local(
    r: &crate::commands::Resolved,
    usd_root: &str,
    extensions: Vec<ExtensionRecord>,
    created: u64,
) -> Result<RuntimeManifest> {
    let root = Utf8PathBuf::from(usd_root);
    if !root.as_std_path().is_dir() {
        return Err(Error::Operation(format!(
            "--from-usd path '{root}' is not a directory"
        )));
    }

    if !looks_like_usd(&root) {
        return Err(Error::Operation(format!(
            "'{root}' does not look like an OpenUSD install (no plugin/usd or lib/**/pxr)"
        )));
    }

    // The store dir holds only the manifest (a pointer to the external root).
    std::fs::create_dir_all(r.prefix.as_std_path())
        .map_err(|e| Error::io(r.prefix.to_string(), e))?;

    let mut manifest = RuntimeManifest::build(
        &r.runtime,
        &r.python_version,
        r.capabilities.clone(),
        probe_usd_layout(&root),
        extensions,
        created,
        RuntimeSource::Local,
    );
    manifest.external_prefix = Some(root.to_string().replace('\\', "/"));
    Ok(manifest)
}

/// The USD-install subdirectories present under `root`. The `pxr` Python package
/// may live under `lib/python` or `lib/site-packages` depending on the build.
fn probe_usd_layout(root: &Utf8Path) -> Vec<String> {
    [
        "bin",
        "lib",
        "lib/python",
        "lib/site-packages",
        "plugin/usd",
        "include",
    ]
    .iter()
    .filter(|s| root.join(s).as_std_path().is_dir())
    .map(|s| s.to_string())
    .collect()
}

/// Whether `root` looks like an OpenUSD install (a strong marker is present).
fn looks_like_usd(root: &Utf8Path) -> bool {
    root.join("plugin/usd").as_std_path().is_dir()
        || ost_runtime::usd_python_dir(root)
            .join("pxr")
            .as_std_path()
            .is_dir()
}

/// The arguments to pass to `python` to run build_usd.py: the script, default
/// trims (kept lean; the user can re-enable via `--build-arg`), optional `-j`,
/// any forwarded args, then the install directory (build_usd.py's positional).
fn build_usd_args(
    script: &Utf8Path,
    install_dir: &Utf8Path,
    jobs: Option<u32>,
    extra: &[String],
) -> Vec<String> {
    let mut args = vec![script.to_string()];
    for trim in ["--no-examples", "--no-tutorials", "--no-docs", "--no-tests"] {
        args.push(trim.to_string());
    }
    if let Some(j) = jobs {
        args.push("-j".to_string());
        args.push(j.to_string());
    }
    args.extend(extra.iter().cloned());
    args.push(install_dir.to_string());
    args
}

/// Options for a `build` source pull.
pub struct BuildOpts {
    pub jobs: Option<u32>,
    pub extra: Vec<String>,
    /// Dependency prefixes; non-empty selects the CMake-direct path.
    pub deps: Vec<String>,
}

/// CMake configure arguments for a direct OpenUSD build: source, build dir,
/// generator, install prefix, the dependency `CMAKE_PREFIX_PATH`, Python, then
/// any forwarded `-D` flags. (Pure, for unit testing.)
fn cmake_configure_args(
    usd_src: &Utf8Path,
    build_dir: &Utf8Path,
    install: &Utf8Path,
    deps: &[String],
    python: &Utf8Path,
    ninja: Option<&str>,
    extra: &[String],
) -> Vec<String> {
    let fwd = |p: &Utf8Path| p.to_string().replace('\\', "/");
    let prefix_path = deps
        .iter()
        .map(|d| d.replace('\\', "/"))
        .collect::<Vec<_>>()
        .join(";");
    let mut args = vec![
        "-S".to_string(),
        fwd(usd_src),
        "-B".to_string(),
        fwd(build_dir),
        "-G".to_string(),
        "Ninja".to_string(),
        "-DCMAKE_BUILD_TYPE=Release".to_string(),
        format!("-DCMAKE_INSTALL_PREFIX={}", fwd(install)),
        format!("-DCMAKE_PREFIX_PATH={prefix_path}"),
        "-DPXR_ENABLE_PYTHON_SUPPORT=ON".to_string(),
        format!("-DPython3_EXECUTABLE={}", fwd(python)),
    ];
    if let Some(n) = ninja {
        args.push(format!("-DCMAKE_MAKE_PROGRAM={}", n.replace('\\', "/")));
    }
    args.extend(extra.iter().cloned());
    args
}

/// `cmake --build <dir> --target install` arguments.
fn cmake_build_args(build_dir: &Utf8Path, jobs: Option<u32>) -> Vec<String> {
    let mut args = vec![
        "--build".to_string(),
        build_dir.to_string().replace('\\', "/"),
        "--config".to_string(),
        "Release".to_string(),
        "--target".to_string(),
        "install".to_string(),
    ];
    if let Some(j) = jobs {
        args.push("--parallel".to_string());
        args.push(j.to_string());
    }
    args
}

/// The MSVC dev-environment delta to inject on Windows (empty elsewhere or when
/// `cl` is already on PATH), the same bootstrap `ost build` uses.
fn msvc_env() -> Vec<(String, String)> {
    if Host::detect().os != Os::Windows || tools::which("cl").is_some() {
        return Vec::new();
    }
    match ost_build::msvc::bootstrap() {
        Ok(Some(env)) => {
            println!(
                "==> msvc env   {} ({} vars)",
                env.vcvars.display(),
                env.vars.len()
            );
            env.vars
        }
        Ok(None) => {
            eprintln!("warning: MSVC not found; relying on the current environment");
            Vec::new()
        }
        Err(e) => {
            eprintln!("warning: could not load the MSVC environment: {e}");
            Vec::new()
        }
    }
}

/// Build OpenUSD from source into the store prefix (`build` source). Dispatches
/// to a direct CMake build when dependency prefixes are supplied, otherwise to
/// build_usd.py (which fetches+builds deps itself). Either way the artifacts land
/// in the store with USD's own layout, so re-pull is a cache hit.
fn build_from_source(
    r: &crate::commands::Resolved,
    usd_src: &str,
    opts: &BuildOpts,
    extensions: Vec<ExtensionRecord>,
    created: u64,
) -> Result<RuntimeManifest> {
    if usd_src.is_empty() {
        return Err(Error::Operation(
            "no OpenUSD source: pass `--build <path>` or set OST_USD_SRC".into(),
        ));
    }
    let src = Utf8PathBuf::from(usd_src);
    if !src.as_std_path().is_dir() {
        return Err(Error::Operation(format!(
            "--build source '{src}' is not a directory"
        )));
    }
    std::fs::create_dir_all(r.prefix.as_std_path())
        .map_err(|e| Error::io(r.prefix.to_string(), e))?;

    if opts.deps.is_empty() {
        build_with_script(r, &src, opts)?;
    } else {
        build_with_cmake(r, &src, opts)?;
    }

    if !looks_like_usd(&r.prefix) {
        return Err(Error::Operation(format!(
            "build finished but no OpenUSD install found under '{}'",
            r.prefix
        )));
    }
    let mut manifest = RuntimeManifest::build(
        &r.runtime,
        &r.python_version,
        r.capabilities.clone(),
        probe_usd_layout(&r.prefix),
        extensions,
        created,
        RuntimeSource::Build,
    );
    // A CMake-direct build links against external deps; record them so the
    // session env can expose their runtime libraries. build_usd.py is
    // self-contained (deps installed into the prefix), so this stays empty.
    manifest.runtime_deps = opts.deps.iter().map(|d| d.replace('\\', "/")).collect();
    Ok(manifest)
}

/// Drive the source tree's `build_scripts/build_usd.py` (handles dependencies).
fn build_with_script(
    r: &crate::commands::Resolved,
    src: &Utf8Path,
    opts: &BuildOpts,
) -> Result<()> {
    let script = src.join("build_scripts").join("build_usd.py");
    if !script.as_std_path().is_file() {
        return Err(Error::Operation(format!(
            "no build_scripts/build_usd.py under '{src}' (point --build at an OpenUSD checkout, \
             or pass --deps for a direct CMake build)"
        )));
    }
    let python = tools::which("python")
        .or_else(|| tools::which("python3"))
        .ok_or_else(|| Error::Operation("`python` not found — build_usd.py needs it".into()))?;
    let python = Utf8PathBuf::from_path_buf(python)
        .map_err(|_| Error::Operation("python path is not UTF-8".into()))?;

    let args = build_usd_args(&script, &r.prefix, opts.jobs, &opts.extra);
    println!(
        "==> building OpenUSD (build_usd.py) into {} (one-time, heavy)",
        r.prefix
    );
    println!("    {python} {}", args.join(" "));
    run_build_step(python.as_str(), &args, &msvc_env(), "build_usd.py")
}

/// Build OpenUSD directly with CMake against pre-provided dependency prefixes,
/// reusing the same compiler/Ninja bootstrap as `ost build`.
fn build_with_cmake(r: &crate::commands::Resolved, src: &Utf8Path, opts: &BuildOpts) -> Result<()> {
    for dep in &opts.deps {
        if !Utf8PathBuf::from(dep).as_std_path().is_dir() {
            return Err(Error::Operation(format!(
                "--deps prefix '{dep}' is not a directory"
            )));
        }
    }
    let cmake = tools::which("cmake")
        .ok_or_else(|| Error::Operation("`cmake` not found on PATH".into()))?;
    let cmake = Utf8PathBuf::from_path_buf(cmake)
        .map_err(|_| Error::Operation("cmake path is not UTF-8".into()))?;
    let python = tools::which("python")
        .or_else(|| tools::which("python3"))
        .ok_or_else(|| Error::Operation("`python` not found — USD needs it for bindings".into()))?;
    let python = Utf8PathBuf::from_path_buf(python)
        .map_err(|_| Error::Operation("python path is not UTF-8".into()))?;
    let ninja = tools::which("ninja").map(|p| p.display().to_string());

    // Keep the build tree out of the install prefix, under the store cache.
    let build_dir = Store::discover()
        .cache()
        .join("usd-build")
        .join(r.runtime.id());
    std::fs::create_dir_all(build_dir.as_std_path())
        .map_err(|e| Error::io(build_dir.to_string(), e))?;

    let env = msvc_env();
    let configure = cmake_configure_args(
        src,
        &build_dir,
        &r.prefix,
        &opts.deps,
        &python,
        ninja.as_deref(),
        &opts.extra,
    );
    let build = cmake_build_args(&build_dir, opts.jobs);

    println!(
        "==> building OpenUSD (cmake) into {} (one-time, heavy)",
        r.prefix
    );
    println!("    cmake {}", configure.join(" "));
    run_build_step(cmake.as_str(), &configure, &env, "cmake configure")?;
    println!("    cmake {}", build.join(" "));
    run_build_step(cmake.as_str(), &build, &env, "cmake build")
}

/// Run a build subprocess with the given extra environment, mapping failure to
/// an actionable error.
fn run_build_step(
    program: &str,
    args: &[String],
    extra_env: &[(String, String)],
    what: &str,
) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .envs(extra_env.iter().cloned())
        .status()
        .map_err(|e| Error::io(format!("run {what}"), e))?;
    if !status.success() {
        return Err(Error::Operation(format!(
            "{what} failed (exit {})",
            status.code().unwrap_or(-1)
        )));
    }
    Ok(())
}

fn list(fmt: Format) -> Result<()> {
    let store = Store::discover();
    let runtimes_dir = store.runtimes();

    let mut manifests: Vec<RuntimeManifest> = Vec::new();
    if runtimes_dir.as_std_path().is_dir() {
        let entries = std::fs::read_dir(runtimes_dir.as_std_path())
            .map_err(|e| Error::io(runtimes_dir.to_string(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| Error::io(runtimes_dir.to_string(), e))?;
            let manifest_path = entry.path().join(MANIFEST_FILE);
            if !manifest_path.is_file() {
                continue;
            }
            let src = std::fs::read_to_string(&manifest_path)
                .map_err(|e| Error::io(manifest_path.display().to_string(), e))?;
            if let Ok(m) = RuntimeManifest::from_json(&src) {
                manifests.push(m);
            }
        }
    }
    manifests.sort_by(|a, b| a.id.cmp(&b.id));

    if fmt.is_json() {
        let items: Vec<_> = manifests
            .iter()
            .map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "platform": m.platform,
                    "profile": m.profile,
                    "validation": m.validation,
                    "digest": m.digest,
                    "source": m.source.as_str(),
                })
            })
            .collect();
        output::json(&serde_json::json!({ "runtimes": items }));
        return Ok(());
    }

    if manifests.is_empty() {
        println!("No runtimes pulled. Try `ost runtime pull cy2026 --profile usd`.");
        return Ok(());
    }
    println!(
        "{:<48}  {:<9}  {:<8}  DIGEST",
        "RUNTIME", "VALIDATE", "SOURCE"
    );
    for m in &manifests {
        let validation = format!("{:?}", m.validation).to_lowercase();
        println!(
            "{:<48}  {:<9}  {:<8}  {}",
            m.id,
            validation,
            m.source.as_str(),
            short_digest(&m.digest)
        );
    }
    Ok(())
}

fn show(platform: &str, profile: &str, fmt: Format) -> Result<()> {
    let r = resolve(platform, profile)?;
    let manifest_path = r.prefix.join(MANIFEST_FILE);
    if !manifest_path.as_std_path().is_file() {
        return Err(Error::Operation(format!(
            "runtime '{}' is not pulled (run `ost runtime pull {} --profile {}`)",
            r.runtime.id(),
            platform,
            profile
        )));
    }
    let src = std::fs::read_to_string(manifest_path.as_std_path())
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;
    let manifest = RuntimeManifest::from_json(&src)
        .map_err(|e| Error::parse(MANIFEST_FILE, anyhow::Error::new(e)))?;

    if fmt.is_json() {
        output::json(&serde_json::to_value(&manifest).expect("manifest serializes"));
        return Ok(());
    }

    println!("Runtime:    {}", manifest.id);
    println!("Platform:   {}", manifest.platform);
    println!("Profile:    {}", manifest.profile);
    println!("Variant:    {}", manifest.variant.slug());
    println!("Python:     {}", manifest.python);
    println!("Digest:     {}", manifest.digest);
    println!("Validation: {:?}", manifest.validation);
    println!("Source:     {}", manifest.source.as_str());
    println!("Prefix:     {}", r.prefix);
    if let Some(ext) = &manifest.external_prefix {
        println!("USD root:   {ext}");
    }
    if !manifest.runtime_deps.is_empty() {
        println!("Deps:       {}", manifest.runtime_deps.join(", "));
    }
    println!("Capabilities:");
    for cap in &manifest.capabilities {
        println!("  - {cap}");
    }
    if !manifest.extensions.is_empty() {
        println!("Extensions:");
        for ext in &manifest.extensions {
            if ext.features.is_empty() {
                println!("  - {} {}", ext.id, ext.version);
            } else {
                println!(
                    "  - {} {} [{}]",
                    ext.id,
                    ext.version,
                    ext.features.join(", ")
                );
            }
        }
    }
    Ok(())
}

fn validate(platform: &str, profile: &str, fmt: Format) -> Result<()> {
    let r = resolve(platform, profile)?;
    let manifest_path = r.prefix.join(MANIFEST_FILE);
    if !manifest_path.as_std_path().is_file() {
        return Err(Error::Operation(format!(
            "runtime '{}' is not pulled (run `ost runtime pull {} --profile {}`)",
            r.runtime.id(),
            platform,
            profile
        )));
    }
    let src = std::fs::read_to_string(manifest_path.as_std_path())
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;
    let mut manifest = RuntimeManifest::from_json(&src)
        .map_err(|e| Error::parse(MANIFEST_FILE, anyhow::Error::new(e)))?;

    // Validate against the effective artifact prefix (the external USD root for
    // an adopted runtime; the store prefix otherwise).
    let report = ost_runtime::validate(&r.artifact_prefix, &manifest);
    let passed = report.passed();

    // Record the outcome back into the manifest (digest is unaffected).
    manifest.set_validation(if passed {
        Validation::Passed
    } else {
        Validation::Failed
    });
    let json = manifest
        .to_json()
        .map_err(|e| Error::parse(MANIFEST_FILE, anyhow::Error::new(e)))?;
    std::fs::write(manifest_path.as_std_path(), format!("{json}\n"))
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;

    if fmt.is_json() {
        let checks: Vec<_> = report
            .checks
            .iter()
            .map(|c| {
                serde_json::json!({
                    "name": c.name,
                    "passed": c.passed,
                    "detail": c.detail,
                })
            })
            .collect();
        output::json(&serde_json::json!({
            "runtime": manifest.id,
            "validation": if passed { "passed" } else { "failed" },
            "checks": checks,
        }));
    } else {
        println!("Validating {}", manifest.id);
        for c in &report.checks {
            let mark = if c.passed { "ok  " } else { "FAIL" };
            match &c.detail {
                Some(d) => println!("  [{mark}] {} — {d}", c.name),
                None => println!("  [{mark}] {}", c.name),
            }
        }
        println!(
            "\n{}",
            if passed {
                "Result: passed"
            } else {
                "Result: FAILED"
            }
        );
    }

    // The report above is this command's own output (human or JSON envelope),
    // so on failure exit with the validation category code (§14.4) directly
    // rather than returning an Err that would render a second document.
    if passed {
        Ok(())
    } else {
        std::process::exit(ost_core::Category::Validation.exit_code() as i32);
    }
}

fn explain(platform: &str, profile: &str, fmt: Format) -> Result<()> {
    let r = resolve(platform, profile)?;
    let catalog = ost_extension::load_all()?;
    let resolution = ost_extension::resolve(&catalog, &r.capabilities);

    if fmt.is_json() {
        let caps: Vec<_> = resolution
            .edges
            .iter()
            .map(|e| {
                serde_json::json!({
                    "capability": e.capability,
                    "provider": e.extension,
                    "feature": e.feature,
                })
            })
            .collect();
        let exts: Vec<_> = resolution
            .extensions
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "version": e.version,
                    "features": e.features,
                    "packages": e.packages,
                    "allowed_range": e.allowed_range,
                    "certified": e.certified.as_ref().map(|c| serde_json::json!({
                        "version": c.version,
                        "features": c.features,
                        "validation": c.validation,
                    })),
                    "uncertified": e.uncertified,
                })
            })
            .collect();
        output::json(&serde_json::json!({
            "runtime": r.runtime.id(),
            "platform": platform,
            "profile": profile,
            "capabilities": caps,
            "extensions": exts,
            "runtime_provided": resolution.runtime_provided,
        }));
        return Ok(());
    }

    println!("Runtime {}", r.runtime.id());
    println!("  platform: {platform}   profile: {profile}");

    println!("\nCapabilities:");
    let width = resolution
        .edges
        .iter()
        .map(|e| e.capability.len())
        .max()
        .unwrap_or(0);
    for edge in &resolution.edges {
        let provider = match (&edge.extension, &edge.feature) {
            (Some(ext), Some(feature)) => format!("{ext} [{feature}]"),
            (Some(ext), None) => ext.clone(),
            (None, _) => "runtime".to_string(),
        };
        println!("  {:<width$}  {provider}", edge.capability);
    }

    if resolution.extensions.is_empty() {
        println!("\nExtensions: (none — base runtime only)");
    } else {
        println!("\nExtensions:");
        for ext in &resolution.extensions {
            println!("  {} {}", ext.id, ext.version);
            if !ext.features.is_empty() {
                let feats: Vec<_> = ext.features.iter().cloned().collect();
                println!("    features:  {}", feats.join(", "));
            }
            if !ext.packages.is_empty() {
                let pkgs: Vec<_> = ext.packages.iter().cloned().collect();
                println!("    packages:  {}", pkgs.join(", "));
            }
            if let Some(c) = &ext.certified {
                let val = c.validation.as_deref().unwrap_or("unvalidated");
                if c.features.is_empty() {
                    println!("    certified: {} ({val})", c.version);
                } else {
                    println!(
                        "    certified: {} [{}] ({val})",
                        c.version,
                        c.features.join(", ")
                    );
                }
            } else if ext.uncertified {
                let feats: Vec<_> = ext.features.iter().cloned().collect();
                println!(
                    "    certified: NONE — no certified build covers [{}] (UNCERTIFIED)",
                    feats.join(", ")
                );
            }
            if let Some(range) = &ext.allowed_range {
                println!("    range:     {range}");
            }
        }
    }
    Ok(())
}

fn short_digest(digest: &str) -> String {
    // `sha256:abcd...` -> `sha256:abcd1234`
    match digest.split_once(':') {
        Some((algo, hex)) => format!("{algo}:{}", &hex[..hex.len().min(12)]),
        None => digest.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_usd_args_put_install_dir_last_and_forward_extras() {
        let script = Utf8PathBuf::from("/src/build_scripts/build_usd.py");
        let prefix = Utf8PathBuf::from("/store/rt");
        let args = build_usd_args(
            &script,
            &prefix,
            Some(8),
            &["--no-imaging".to_string(), "--no-usdview".to_string()],
        );
        // Script first, install dir last (build_usd.py's positional).
        assert_eq!(args.first().unwrap(), "/src/build_scripts/build_usd.py");
        assert_eq!(args.last().unwrap(), "/store/rt");
        // Default trims, parallelism, and forwarded extras are all present.
        assert!(args.iter().any(|a| a == "--no-tests"));
        assert!(args.windows(2).any(|w| w == ["-j", "8"]));
        assert!(args.iter().any(|a| a == "--no-imaging"));
    }

    #[test]
    fn cmake_configure_args_set_prefix_path_and_forward_defines() {
        let args = cmake_configure_args(
            &Utf8PathBuf::from("/src/OpenUSD"),
            &Utf8PathBuf::from("/cache/build"),
            &Utf8PathBuf::from("/store/rt"),
            &["/deps/a".to_string(), "/deps/b".to_string()],
            &Utf8PathBuf::from("/usr/bin/python"),
            Some("/tools/ninja"),
            &["-DPXR_BUILD_IMAGING=OFF".to_string()],
        );
        assert!(args.windows(2).any(|w| w == ["-S", "/src/OpenUSD"]));
        assert!(args.iter().any(|a| a == "-DCMAKE_INSTALL_PREFIX=/store/rt"));
        // Multiple dep prefixes are joined with ';' into CMAKE_PREFIX_PATH.
        assert!(args
            .iter()
            .any(|a| a == "-DCMAKE_PREFIX_PATH=/deps/a;/deps/b"));
        assert!(args
            .iter()
            .any(|a| a == "-DCMAKE_MAKE_PROGRAM=/tools/ninja"));
        assert!(args.iter().any(|a| a == "-DPXR_BUILD_IMAGING=OFF"));
    }

    #[test]
    fn cmake_build_args_install_target_with_parallelism() {
        let args = cmake_build_args(&Utf8PathBuf::from("/cache/build"), Some(4));
        assert!(args.windows(2).any(|w| w == ["--target", "install"]));
        assert!(args.windows(2).any(|w| w == ["--parallel", "4"]));
    }

    #[test]
    fn dep_prefixes_split_on_the_os_path_separator() {
        // Empty entries are dropped.
        assert!(split_dep_prefixes("").is_empty());

        // Splitting uses the platform separator, so Windows drive letters in an
        // absolute path survive intact rather than being torn at the colon.
        #[cfg(windows)]
        {
            let deps = split_dep_prefixes(r"C:\deps\a;D:\deps\b");
            assert_eq!(
                deps,
                vec![r"C:\deps\a".to_string(), r"D:\deps\b".to_string()]
            );
        }
        #[cfg(not(windows))]
        {
            let deps = split_dep_prefixes("/deps/a:/deps/b");
            assert_eq!(deps, vec!["/deps/a".to_string(), "/deps/b".to_string()]);
        }
    }
}
