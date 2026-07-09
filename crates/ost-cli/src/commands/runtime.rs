// SPDX-License-Identifier: Apache-2.0
//! `ost runtime` — pull / list / show runtimes (§14.2).
//!
//! `pull` writes a digest-bearing `runtime.json` under `~/.ost/runtimes/<id>`
//! from one of several backend **sources** (§ Phase 4b): `mock` materializes a
//! placeholder layout; `local` (`--from-usd`) adopts an existing OpenUSD install
//! in place; `build` (`--build <usd-src>`) builds OpenUSD from source into the
//! store via `build_usd.py`; `artifact` (`--from-artifact <digest>`)
//! materializes a prebuilt runtime from the local artifact registry (Phase 6).
//! `export` is the reverse edge: it packs a pulled real runtime into the
//! registry as a digest-addressed `openstrata.runtime` artifact.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Subcommand;

use camino::{Utf8Path, Utf8PathBuf};

use ost_artifact::{ArtifactKind, ArtifactSource, ArtifactStore};
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
        /// Materialize the runtime from a registry artifact (`artifact` source):
        /// a digest reference (`sha256:<hex>` or a unique hex prefix) of an
        /// `ost runtime export`ed artifact.
        #[arg(long, conflicts_with_all = ["from_usd", "build", "deps"])]
        from_artifact: Option<String>,
    },
    /// Export a pulled real runtime into the local artifact registry.
    Export {
        /// Platform calendar-year id, e.g. `cy2026`, or a full runtime id.
        platform: String,
        /// Profile, e.g. `usd`.
        #[arg(long, default_value = "core")]
        profile: String,
        /// Also keep the producer output (archive + manifest.json + SHA256SUMS)
        /// in this directory instead of a temporary staging dir.
        #[arg(long)]
        dist: Option<String>,
        /// Export only the SDK layout (include, lib, bin, plugin, cmake,
        /// libraries, resources, and CMake config), dropping the source/build
        /// tree of a runtime adopted from a full USD build. Much smaller archive
        /// and faster per-PR pull.
        #[arg(long)]
        slim: bool,
        /// zstd compression level (1–22). Lower is faster; the default (19)
        /// favors a small artifact, packed once and pulled many times.
        #[arg(long, default_value_t = ost_build::ZSTD_LEVEL)]
        level: i32,
        /// zstd worker threads for compression. Defaults to the host's
        /// available parallelism; `--jobs 1` forces the single-threaded encoder.
        #[arg(long)]
        jobs: Option<u32>,
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
    /// Re-adopt a `local` runtime from its recorded USD root, refreshing the
    /// manifest (real OpenUSD version, layout, digest) after install drift.
    Repair {
        /// Platform calendar-year id, e.g. `cy2026`, or a full runtime id.
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
            from_artifact,
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
                from_artifact,
            },
            fmt,
        ),
        RuntimeCmd::Export {
            platform,
            profile,
            dist,
            slim,
            level,
            jobs,
        } => export(
            &platform,
            &profile,
            dist.as_deref(),
            slim,
            ExportPack { level, jobs },
            fmt,
        ),
        RuntimeCmd::List => list(fmt),
        RuntimeCmd::Show { platform, profile } => show(&platform, &profile, fmt),
        RuntimeCmd::Validate { platform, profile } => validate(&platform, &profile, fmt),
        RuntimeCmd::Repair { platform, profile } => repair(&platform, &profile, fmt),
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
    /// `--from-artifact <digest>`: materialize from the local artifact registry.
    pub from_artifact: Option<String>,
}

fn pull(platform: &str, profile: &str, force: bool, src: PullSource, fmt: Format) -> Result<()> {
    let r = resolve(platform, profile)?;

    if r.pulled && !force {
        return Err(Error::usage(format!(
            "runtime '{}' already pulled (use --force to re-pull)",
            r.runtime.id()
        )));
    }

    // Resolve the profile's capabilities to concrete extensions. This drives
    // both the prefix layout (USD plugins) and the recorded provenance, so
    // `pull` agrees with `runtime explain`.
    let (has_usd, extensions) = resolve_extensions(&r)?;

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

    let manifest = if let Some(digest_ref) = &src.from_artifact {
        fetch_from_artifact(&r, digest_ref)?
    } else if let Some(usd_src) = build_src {
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
        output::success(&serde_json::json!({
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
            RuntimeSource::Artifact => "Fetched",
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

/// Resolve the profile's capabilities to concrete extensions (shared by `pull`
/// and `repair`, so both record the same provenance `runtime explain` shows).
fn resolve_extensions(r: &crate::commands::Resolved) -> Result<(bool, Vec<ExtensionRecord>)> {
    let catalog = ost_extension::load_all()?;
    let resolution = ost_extension::resolve(&catalog, &r.capabilities);
    let has_usd = resolution.extensions.iter().any(|e| e.id == "openusd");
    let extensions = resolution
        .extensions
        .iter()
        .map(|e| ExtensionRecord {
            id: e.id.clone(),
            version: e.version.clone(),
            features: e.features.iter().cloned().collect(),
        })
        .collect();
    Ok((has_usd, extensions))
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
    mut extensions: Vec<ExtensionRecord>,
    created: u64,
) -> Result<RuntimeManifest> {
    let root = Utf8PathBuf::from(usd_root);
    if !root.as_std_path().is_dir() {
        return Err(Error::usage(format!(
            "--from-usd path '{root}' is not a directory"
        )));
    }

    if !looks_like_usd(&root) {
        return Err(Error::usage(format!(
            "'{root}' does not look like an OpenUSD install (no plugin/usd or lib/**/pxr)"
        )));
    }

    // Record the *real* OpenUSD version read from the adopted install, not the
    // catalog's placeholder. Otherwise a 26.08 install is recorded as the
    // catalog default (25.05) and silently "satisfies" version ranges it should
    // fail — the gate ends up enforcing nothing.
    stamp_openusd_version(&mut extensions, &root, "adopted");

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

/// Correct the recorded `openusd` extension version to the real one read from
/// the install's `pxr.h`, when the two name genuinely different releases.
///
/// Both the adopt and `--build` paths use this: a freshly built or adopted tree
/// reports its true version in `include/pxr/pxr.h`, while `extensions` still
/// carries the catalog default (e.g. `25.05.01`). Stamping the real version
/// keeps the L1 range gate honest — otherwise a 26.x install silently satisfies
/// ranges it should fail.
///
/// Only corrects (and notes) a genuinely *different* release: the catalog
/// default carries a certification-revision component (`25.05.01`) that `pxr.h`
/// doesn't expose, so a real 25.05 install must not overwrite `25.05.01` with a
/// bare `25.05`. `context` labels the source in the note (`adopted` / `built`).
fn stamp_openusd_version(extensions: &mut [ExtensionRecord], root: &Utf8Path, context: &str) {
    match detect_openusd_version(root) {
        Some(real) => {
            if let Some(ext) = extensions.iter_mut().find(|e| e.id == "openusd") {
                if !same_openusd_release(&real, &ext.version) {
                    eprintln!(
                        "note: {context} OpenUSD reports version {real} (catalog default was {})",
                        ext.version
                    );
                    ext.version = real;
                }
            }
        }
        None => eprintln!(
            "warning: could not read the OpenUSD version from '{root}/include/pxr/pxr.h'; \
             recording the catalog default (the version gate may not reflect the real install)"
        ),
    }
}

/// Read the real OpenUSD version from an adopted install's `include/pxr/pxr.h`.
///
/// Returns the `<minor>.<patch>` form the catalog and version ranges use (e.g.
/// `26.08`): OpenUSD's `PXR_MAJOR_VERSION` is structurally 0, and a release like
/// `v26.08` is `PXR_MINOR_VERSION 26` + `PXR_PATCH_VERSION 8`. `None` if the
/// header is absent or unparseable (a header-less, binary-only install).
pub(crate) fn detect_openusd_version(root: &Utf8Path) -> Option<String> {
    let header = root.join("include/pxr/pxr.h");
    let src = std::fs::read_to_string(header.as_std_path()).ok()?;
    let field = |name: &str| -> Option<u32> {
        src.lines().find_map(|line| {
            let rest = line.trim().strip_prefix("#define")?.trim_start();
            let rest = rest.strip_prefix(name)?;
            // Require a token boundary so `PXR_VERSION` can't match a request for
            // `PXR_MINOR_VERSION` (or vice versa).
            if !rest.starts_with(char::is_whitespace) {
                return None;
            }
            rest.split_whitespace().next()?.parse::<u32>().ok()
        })
    };
    let minor = field("PXR_MINOR_VERSION")?;
    let patch = field("PXR_PATCH_VERSION")?;
    Some(format!("{minor}.{patch:02}"))
}

/// Do the `detected` `<minor>.<patch>` (from `pxr.h`) and the `catalog`'s
/// recorded version name the same OpenUSD release?
///
/// The catalog may carry an extra certification-revision component the header
/// doesn't expose (`25.05.01` for upstream `25.05`), so compare numerically over
/// only the leading components `pxr.h` provides. Returns `false` on either side
/// being unparseable, so a malformed catalog entry still gets corrected.
pub(crate) fn same_openusd_release(detected: &str, catalog: &str) -> bool {
    let nums = |s: &str| -> Option<Vec<u64>> {
        s.split('.').map(|p| p.trim().parse::<u64>().ok()).collect()
    };
    match (nums(detected), nums(catalog)) {
        (Some(d), Some(c)) => c.len() >= d.len() && d.iter().zip(&c).all(|(a, b)| a == b),
        _ => false,
    }
}

/// When the install's `pxr.h` names a different OpenUSD release than the manifest
/// records, return `(recorded, real)` so callers can flag the stale manifest.
///
/// The adopt/build step records the version from `pxr.h`, but a runtime recorded
/// before that derivation (or whose install changed underneath) keeps a stale
/// value — which makes the L1 range check pass for the wrong reason (dogfooding
/// reports #1–#5). `None` when there is no install header, no recorded `openusd`
/// version, or the two name the same release.
pub(crate) fn openusd_version_drift(
    manifest: &RuntimeManifest,
    artifact_prefix: &Utf8Path,
) -> Option<(String, String)> {
    let recorded = manifest
        .extensions
        .iter()
        .find(|e| e.id == "openusd")?
        .version
        .clone();
    let real = detect_openusd_version(artifact_prefix)?;
    (!same_openusd_release(&real, &recorded)).then_some((recorded, real))
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
        return Err(Error::usage(
            "no OpenUSD source: pass `--build <path>` or set OST_USD_SRC",
        ));
    }
    let src = Utf8PathBuf::from(usd_src);
    if !src.as_std_path().is_dir() {
        return Err(Error::usage(format!(
            "--build source '{src}' is not a directory"
        )));
    }
    emit_macos_build_notes(opts);
    // Warn now on missing build-interpreter deps rather than letting build_usd.py
    // abort deep in its run (report §Dogfood): a clean Python 3.13 lacks Jinja2
    // (schema tooling) and PySide6/PyOpenGL (usdview) that the profile implies.
    preflight_build_deps(&r.capabilities);
    std::fs::create_dir_all(r.prefix.as_std_path())
        .map_err(|e| Error::io(r.prefix.to_string(), e))?;

    if opts.deps.is_empty() {
        build_with_script(r, &src, opts)?;
    } else {
        build_with_cmake(r, &src, opts)?;
    }

    if !looks_like_usd(&r.prefix) {
        return Err(Error::validation(format!(
            "build finished but no OpenUSD install found under '{}'",
            r.prefix
        )));
    }
    // Stamp the version the freshly built tree actually reports (from its
    // `pxr.h`), not the catalog default — otherwise the manifest records e.g.
    // `25.05.01` for a `26.05` build, `runtime validate` fails with
    // `openusd-version-drift`, and `export` is hard-blocked with no non-destructive
    // recovery (report Finding A).
    let mut extensions = extensions;
    stamp_openusd_version(&mut extensions, &r.prefix, "built");

    // A from-source runtime that bundles `usdGenSchema` must also carry its
    // schema-gen Python deps (`jinja2` + `MarkupSafe`); `build_usd.py` needs
    // them only on the build host and never installs them into the tree, so a
    // published image would otherwise die with a bare `ModuleNotFoundError` in
    // `ost plugin build`'s schema-generate phase (report Finding D).
    provision_schema_gen_deps(r);
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

/// Provision the schema-gen Python deps into a freshly built runtime that
/// bundles `usdGenSchema` (report Finding D). Resolves an interpreter to run
/// `pip` and installs into the exact `lib/python` dir `ost` puts on
/// `PYTHONPATH`. Best-effort: a failure warns with the one-line manual fix
/// rather than discarding an otherwise-good (and expensive) build.
fn provision_schema_gen_deps(r: &crate::commands::Resolved) {
    if !ost_build::bundles_usdgenschema(&r.prefix) {
        return;
    }
    let python_lib_dir = ost_runtime::usd_python_dir(&r.prefix);
    let manual_fix = |argv: &str| {
        format!(
            "provision them manually with: {argv} -m pip install --target {python_lib_dir} {}",
            ost_build::SCHEMA_GEN_PACKAGES.join(" ")
        )
    };
    let Some(argv) = ost_build::resolve_run_python(&r.prefix, &r.python_version) else {
        eprintln!(
            "warning: this runtime bundles usdGenSchema but no Python interpreter was found to \
             provision its schema-gen deps ({}); {}",
            ost_build::SCHEMA_GEN_PACKAGES.join(" "),
            manual_fix("<python>")
        );
        return;
    };
    match ost_build::provision_schema_gen_deps(&r.prefix, &python_lib_dir, &argv) {
        Ok(ost_build::SchemaDepsOutcome::Installed(pkgs)) => {
            println!(
                "==> provisioned schema-gen deps into {python_lib_dir}: {}",
                pkgs.join(", ")
            );
        }
        Ok(_) => {}
        Err(e) => eprintln!(
            "warning: could not provision schema-gen deps ({e}); {}",
            manual_fix(&argv.join(" "))
        ),
    }
}

/// Re-derive a `build`-source runtime manifest from the tree already in the
/// store, without rebuilding (report Finding A). Re-probes the layout, re-reads
/// the real OpenUSD version from the built `pxr.h`, preserves the recorded
/// external dependency prefixes, and resets validation to pending. This is the
/// non-destructive recovery for a from-source runtime whose recorded version
/// drifted from the tree it built: the built bits are correct — only the
/// manifest's version field was stale — so `repair` corrects it in place instead
/// of forcing a `--from-usd` re-adopt that would throw away `build` provenance.
fn redetect_build(
    r: &crate::commands::Resolved,
    extensions: Vec<ExtensionRecord>,
    previous: &RuntimeManifest,
    created: u64,
) -> Result<RuntimeManifest> {
    if !looks_like_usd(&r.prefix) {
        return Err(Error::coded(
            "REPAIR_NO_BUILD_TREE",
            ost_core::Category::Precondition,
            format!(
                "runtime '{}' is `build`-sourced but no OpenUSD install is present under \
                 '{}'; rebuild it with `--build <usd-src> --force`",
                previous.id, r.prefix
            ),
        ));
    }
    let mut extensions = extensions;
    stamp_openusd_version(&mut extensions, &r.prefix, "built");
    let mut manifest = RuntimeManifest::build(
        &r.runtime,
        &r.python_version,
        r.capabilities.clone(),
        probe_usd_layout(&r.prefix),
        extensions,
        created,
        RuntimeSource::Build,
    );
    // A CMake-direct build linked against external dep prefixes; carry them
    // forward so the session env still exposes their runtime libraries.
    manifest.runtime_deps = previous.runtime_deps.clone();
    Ok(manifest)
}

/// Materialize a runtime from a registry artifact (`artifact` source): resolve
/// the digest, re-verify the archive bytes, extract into the store prefix, and
/// restore the runtime manifest that traveled in the artifact's provenance.
fn fetch_from_artifact(r: &crate::commands::Resolved, digest_ref: &str) -> Result<RuntimeManifest> {
    let store = ArtifactStore::discover();
    let record = store.resolve(digest_ref)?;
    if record.kind != ArtifactKind::Runtime {
        return Err(Error::coded(
            "ARTIFACT_KIND_MISMATCH",
            ost_core::Category::Validation,
            format!(
                "artifact {} is a {} ('{}'), not a runtime",
                record.short_digest(),
                record.kind.as_str(),
                record.name
            ),
        )
        .with_hint("list runtime artifacts with `ost artifact list --kind runtime`"));
    }

    // The runtime manifest travels in the producer manifest (not in the
    // archive), so the archive stays a pure USD tree and the manifest can be
    // rewritten for the new materialization without unpacking first.
    let producer = store.producer_manifest(&record)?;
    let embedded = producer
        .get("provenance")
        .and_then(|p| p.get("runtime_manifest"))
        .ok_or_else(|| {
            Error::InvalidManifest(
                "runtime artifact carries no provenance.runtime_manifest".to_string(),
            )
        })?;
    let mut manifest: RuntimeManifest = serde_json::from_value(embedded.clone())
        .map_err(|e| Error::parse("runtime_manifest in artifact", anyhow::Error::new(e)))?;

    let requested = r.runtime.id();
    if manifest.id != requested {
        return Err(Error::coded(
            "ARTIFACT_RUNTIME_MISMATCH",
            ost_core::Category::Validation,
            format!(
                "artifact {} contains runtime '{}', but '{requested}' was requested",
                record.short_digest(),
                manifest.id
            ),
        )
        .with_hint("check `ost artifact show <digest>` for the artifact's target/profile"));
    }

    // Fresh materialization: never extract over a stale prefix. The extract
    // itself is digest-pinned — the store re-hashes the archive before
    // trusting it, so a store corrupted at rest cannot become a runtime.
    if r.prefix.as_std_path().exists() {
        std::fs::remove_dir_all(r.prefix.as_std_path())
            .map_err(|e| Error::io(r.prefix.to_string(), e))?;
    }
    store.extract(&record.digest, &r.prefix)?;

    if !looks_like_usd(&r.prefix) {
        return Err(Error::validation(format!(
            "artifact {} extracted, but no OpenUSD install found under '{}'",
            record.short_digest(),
            r.prefix
        )));
    }

    // The runtime now lives in the store: it is `artifact`-sourced, its files
    // are local (no external root), and it points back at the registry entry.
    // The canonical digest is unchanged — source fields are provenance, not
    // identity.
    manifest.source = RuntimeSource::Artifact;
    manifest.external_prefix = None;
    manifest.artifact_digest = Some(record.digest.clone());
    Ok(manifest)
}

/// The gates a runtime must pass to be exported as a registry artifact.
///
/// Pure over the manifest, so the refusals are unit-testable: a `mock` runtime
/// has no real artifacts to ship; external `runtime_deps` would not travel with
/// the archive (the extracted runtime could not load them); and an unvalidated
/// runtime must not become a digest CI pins (quality bar: every published
/// artifact includes provenance and validation).
fn check_exportable(manifest: &RuntimeManifest) -> Result<()> {
    if !manifest.source.is_real() {
        return Err(Error::coded(
            "EXPORT_REAL_RUNTIME_REQUIRED",
            ost_core::Category::Precondition,
            format!(
                "runtime '{}' is a mock layout — there are no real artifacts to export",
                manifest.id
            ),
        )
        .with_hint("pull a real runtime first: `--from-usd <usd-root>` or `--build <usd-src>`"));
    }
    if !manifest.runtime_deps.is_empty() {
        return Err(Error::coded(
            "EXPORT_DEPS_NOT_PORTABLE",
            ost_core::Category::Validation,
            format!(
                "runtime '{}' links against external dependency prefixes ({}) that would \
                 not travel with the artifact",
                manifest.id,
                manifest.runtime_deps.join(", ")
            ),
        )
        .with_hint(
            "export a self-contained runtime: build via build_usd.py (no --deps), \
             which installs dependencies into the prefix",
        ));
    }
    if manifest.validation != Validation::Passed {
        return Err(Error::coded(
            "EXPORT_VALIDATION_REQUIRED",
            ost_core::Category::Validation,
            format!(
                "runtime '{}' has not passed validation (status: {})",
                manifest.id,
                manifest.validation.as_str()
            ),
        )
        .with_hint("run `ost runtime validate <platform> --profile <profile>` first"));
    }
    Ok(())
}

/// The producer `manifest.json` for a runtime artifact (`openstrata.runtime`).
///
/// Mirrors the package/plugin producer manifests (same top-level identity +
/// `files[]`), with the full runtime manifest embedded under
/// `provenance.runtime_manifest` so a fetch can restore `runtime.json` without
/// the archive carrying store-specific state. `licenses` stays empty until
/// runtime content attribution lands (see the roadmap's licensing section).
fn runtime_artifact_manifest(
    manifest: &RuntimeManifest,
    archive_name: &str,
    packed: &ost_build::PackResult,
    created: u64,
) -> serde_json::Value {
    let version = manifest
        .extensions
        .iter()
        .find(|e| e.id == "openusd")
        .map(|e| e.version.clone())
        .unwrap_or_else(|| manifest.platform.clone());
    let files: Vec<_> = packed
        .files
        .iter()
        .map(|f| serde_json::json!({ "path": f.path, "sha256": f.sha256, "size": f.size }))
        .collect();
    serde_json::json!({
        "schema": 1,
        "kind": ost_artifact::RUNTIME_KIND,
        "name": manifest.id,
        "version": version,
        "target": manifest.variant.slug(),
        "licenses": [],
        "archive": archive_name,
        "archive_digest": packed.archive_digest,
        "archive_size": packed.archive_size,
        "total_size": packed.total_size,
        "created_unix": created,
        "provenance": {
            "platform": manifest.platform,
            "profile": manifest.profile,
            "validation": manifest.validation.as_str(),
            "runtime": {
                "id": manifest.id,
                "digest": manifest.digest,
                "source": manifest.source.as_str(),
                "validation": manifest.validation.as_str(),
            },
            "runtime_manifest": serde_json::to_value(manifest).unwrap_or_default(),
        },
        "files": files,
    })
}

/// Compression knobs for `ost runtime export` (`--level` / `--jobs`).
struct ExportPack {
    level: i32,
    jobs: Option<u32>,
}

/// The zstd worker count to pack with: the requested `--jobs`, else the host's
/// available parallelism (falling back to single-threaded). Multithreading is
/// the default here because a full adopted runtime is ~14 GB and packs for
/// tens of minutes single-threaded (report #10).
fn default_pack_workers() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1)
}

/// Render a byte count as a compact human-readable size for progress output.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// `ost runtime export` — pack a pulled real runtime and register it in the
/// local artifact registry, addressed by digest.
fn export(
    platform: &str,
    profile: &str,
    dist: Option<&str>,
    slim: bool,
    pack: ExportPack,
    fmt: Format,
) -> Result<()> {
    let (platform, profile) = platform_profile(platform, profile);
    let r = resolve(&platform, &profile)?;
    let manifest_path = r.prefix.join(MANIFEST_FILE);
    if !manifest_path.as_std_path().is_file() {
        return Err(Error::coded(
            "RUNTIME_NOT_FOUND",
            ost_core::Category::Precondition,
            format!(
                "runtime '{}' is not pulled (run `ost runtime pull {platform} --profile {profile}`)",
                r.runtime.id()
            ),
        ));
    }
    let src = std::fs::read_to_string(manifest_path.as_std_path())
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;
    let manifest = RuntimeManifest::from_json(&src)
        .map_err(|e| Error::parse(MANIFEST_FILE, anyhow::Error::new(e)))?;

    check_exportable(&manifest)?;

    // Pack the runtime's real artifacts: the effective prefix (external root
    // for an adopted runtime), minus the store's own runtime.json — the
    // manifest travels in the producer manifest instead, so the archive is a
    // pure USD tree.
    let effective = Utf8PathBuf::from(manifest.effective_prefix(&r.prefix));
    let map_stage_error = |e: std::io::Error| {
        if e.kind() == std::io::ErrorKind::InvalidData {
            Error::validation(e.to_string())
        } else {
            Error::io(effective.to_string(), e)
        }
    };
    // A slim export keeps only the SDK layout, dropping the source/build tree an
    // adopted build-tree runtime carries. It prunes excluded top-level entries
    // before walking them, so a build tree symlink or socket cannot veto an SDK
    // artifact that would never include it.
    let (files, excluded_dirs): (Vec<Utf8PathBuf>, Vec<String>) = if slim {
        let sdk = ost_build::sdk_stage_files(&effective).map_err(map_stage_error)?;
        let files = sdk
            .files
            .into_iter()
            .filter(|p| p != &effective.join(MANIFEST_FILE))
            .collect();
        let excluded = sdk
            .excluded_top_level
            .into_iter()
            .filter(|p| p != MANIFEST_FILE)
            .collect();
        (files, excluded)
    } else {
        let files: Vec<Utf8PathBuf> = ost_build::stage_files(&effective)
            .map_err(map_stage_error)?
            .into_iter()
            .filter(|p| p != &effective.join(MANIFEST_FILE))
            .collect();
        (files, Vec::new())
    };
    if files.is_empty() {
        let message = if slim {
            format!(
                "runtime '{}' has no SDK-layout files under '{effective}' — nothing to export \
                 (is this an OpenUSD install/build prefix?)",
                manifest.id
            )
        } else {
            format!(
                "runtime '{}' has no files under '{effective}' — nothing to export",
                manifest.id
            )
        };
        return Err(Error::validation(message));
    }
    if slim && !fmt.is_json() {
        println!(
            "Slim export: keeping {} files (SDK layout); dropping top-level: {}",
            files.len(),
            if excluded_dirs.is_empty() {
                "nothing".to_string()
            } else {
                excluded_dirs.join(", ")
            }
        );
    }
    let store = Store::discover();
    let staging_default = store.cache().join("runtime-export").join(&manifest.id);
    let dist_dir = if let Some(d) = dist {
        let dir = Utf8PathBuf::from(d);
        if dir.as_std_path().exists() {
            if !dir.as_std_path().is_dir() {
                return Err(Error::usage(format!(
                    "--dist path '{dir}' exists but is not a directory"
                )));
            }
            let mut entries =
                std::fs::read_dir(dir.as_std_path()).map_err(|e| Error::io(dir.to_string(), e))?;
            if let Some(entry) = entries.next() {
                entry.map_err(|e| Error::io(dir.to_string(), e))?;
                return Err(Error::usage(format!(
                    "refusing to write runtime export into non-empty --dist directory '{dir}'"
                )));
            }
        } else {
            std::fs::create_dir_all(dir.as_std_path())
                .map_err(|e| Error::io(dir.to_string(), e))?;
        }
        dir
    } else {
        if staging_default.as_std_path().exists() {
            std::fs::remove_dir_all(staging_default.as_std_path())
                .map_err(|e| Error::io(staging_default.to_string(), e))?;
        }
        std::fs::create_dir_all(staging_default.as_std_path())
            .map_err(|e| Error::io(staging_default.to_string(), e))?;
        staging_default.clone()
    };

    let archive_name = format!("{}.tar.zst", manifest.id);
    let archive_path = dist_dir.join(&archive_name);

    let workers = pack.jobs.unwrap_or_else(default_pack_workers);
    let opts = ost_build::PackOptions {
        level: pack.level,
        workers,
    };
    // Progress to stderr (throttled, in-place) so a long single- or
    // multi-threaded pack shows liveness; suppressed in JSON mode so the only
    // stdout content stays the success object.
    let show_progress = !fmt.is_json();
    if show_progress {
        println!(
            "Packing {} file{} from {effective} (zstd level {}, {} worker{})",
            files.len(),
            if files.len() == 1 { "" } else { "s" },
            opts.level,
            workers,
            if workers == 1 { "" } else { "s" },
        );
    }
    let start = std::time::Instant::now();
    let mut last = start;
    let mut progress = |p: ost_build::PackProgress| {
        if !show_progress {
            return;
        }
        let now = std::time::Instant::now();
        // ~4 Hz, but always render the final file so the line ends complete.
        if p.files_done == p.files_total
            || now.duration_since(last) >= std::time::Duration::from_millis(250)
        {
            last = now;
            eprint!(
                "\r  {}/{} files, {} in {}s   ",
                p.files_done,
                p.files_total,
                human_bytes(p.bytes_done),
                start.elapsed().as_secs(),
            );
            let _ = std::io::Write::flush(&mut std::io::stderr());
        }
    };
    let packed = ost_build::pack_dir_with(&effective, &archive_path, &files, opts, &mut progress)
        .map_err(|e| Error::io(archive_path.to_string(), e))?;
    if show_progress {
        eprintln!(); // terminate the in-place progress line
    }

    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut producer = runtime_artifact_manifest(&manifest, &archive_name, &packed, created);
    // Record which layout was shipped so a fetch/inspection can tell a slim SDK
    // artifact from a full one (they are distinct digests of the same runtime).
    if let Some(obj) = producer.as_object_mut() {
        obj.insert(
            "layout_profile".into(),
            serde_json::json!(if slim { "sdk" } else { "full" }),
        );
    }
    let producer_json = serde_json::to_string_pretty(&producer)
        .map_err(|e| Error::parse("runtime artifact manifest", anyhow::Error::new(e)))?;
    let producer_path = dist_dir.join("manifest.json");
    std::fs::write(producer_path.as_std_path(), format!("{producer_json}\n"))
        .map_err(|e| Error::io(producer_path.to_string(), e))?;
    let bare = packed
        .archive_digest
        .strip_prefix("sha256:")
        .unwrap_or(&packed.archive_digest);
    let sums = dist_dir.join("SHA256SUMS");
    std::fs::write(sums.as_std_path(), format!("{bare}  {archive_name}\n"))
        .map_err(|e| Error::io(sums.to_string(), e))?;

    // Register in the registry. Export enforced the gates above, so the entry
    // is `published` — the trusted tier CI pins.
    let registry = ArtifactStore::discover();
    let out = registry.import(&dist_dir, ArtifactSource::Published)?;

    // The registry holds the canonical copy; drop the temporary staging unless
    // the user asked to keep a dist dir.
    if dist.is_none() {
        let _ = std::fs::remove_dir_all(staging_default.as_std_path());
    }

    if fmt.is_json() {
        output::success(&serde_json::json!({
            "exported": true,
            "already_present": out.already_present,
            "runtime": manifest.id,
            "digest": out.record.digest,
            "archive_size": out.record.archive_size,
            "files": out.record.file_count,
            "dist": dist.map(|d| d.to_string()),
            "layout_profile": if slim { "sdk" } else { "full" },
            "excluded_top_level": excluded_dirs,
        }));
        return Ok(());
    }
    if out.already_present {
        println!(
            "Already in the registry: {} is stored as {}",
            manifest.id,
            out.record.short_digest()
        );
    } else {
        println!("Exported runtime {}", manifest.id);
    }
    println!("  digest: {}", out.record.digest);
    println!(
        "  fetch anywhere with: ost runtime pull {platform} --profile {profile} --from-artifact {}",
        out.record.digest
    );
    Ok(())
}

/// The Python packages `build_usd.py` needs on the *build host* for a given
/// profile's capabilities, as `(import_name, pip_name)` pairs. usdGenSchema
/// needs Jinja2; a Hydra/usdview build needs PySide6 + PyOpenGL; a Qt profile
/// needs PySide6. Pure, so the mapping is unit-testable.
fn build_dep_requirements(capabilities: &[String]) -> Vec<(&'static str, &'static str)> {
    let has = |c: &str| capabilities.iter().any(|x| x == c);
    let wants_usd = capabilities.iter().any(|c| c.starts_with("usd-"));
    let wants_view = has("hydra-preview");
    let wants_qt = has("qt-ui") || wants_view;

    let mut needed: Vec<(&str, &str)> = Vec::new();
    if wants_usd {
        needed.push(("jinja2", "Jinja2"));
    }
    if wants_qt {
        needed.push(("PySide6", "PySide6"));
    }
    if wants_view {
        needed.push(("OpenGL", "PyOpenGL"));
    }
    needed
}

/// Probe the host interpreter for the build-time Python deps the profile implies
/// and warn (never fail) on the missing ones before `build_usd.py` runs. Best
/// effort: if no interpreter or the probe itself fails, stay silent — the build
/// step surfaces the real error, and a preflight must not cry wolf.
fn preflight_build_deps(capabilities: &[String]) {
    let needed = build_dep_requirements(capabilities);
    if needed.is_empty() {
        return;
    }
    let Some(python) = tools::which("python").or_else(|| tools::which("python3")) else {
        return;
    };
    let imports: Vec<&str> = needed.iter().map(|(i, _)| *i).collect();
    let list = imports
        .iter()
        .map(|m| format!("'{m}'"))
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "import importlib.util as u;print(','.join(m for m in [{list}] if u.find_spec(m) is None))"
    );
    let out = std::process::Command::new(&python)
        .arg("-c")
        .arg(&script)
        .output();
    let missing: Vec<String> = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .trim()
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => return,
    };
    if missing.is_empty() {
        return;
    }
    let pip: Vec<&str> = needed
        .iter()
        .filter(|(i, _)| missing.iter().any(|m| m == i))
        .map(|(_, p)| *p)
        .collect();
    eprintln!(
        "warning: build_usd.py needs Python packages not importable by {}: {}",
        python.display(),
        missing.join(", ")
    );
    eprintln!(
        "  install them first: {} -m pip install {}",
        python.display(),
        pip.join(" ")
    );
    eprintln!("  (schema generation needs Jinja2; usdview needs PySide6 + PyOpenGL)");
}

fn emit_macos_build_notes(opts: &BuildOpts) {
    if Host::detect().os != Os::Macos {
        return;
    }

    eprintln!(
        "note: macOS OpenUSD source builds may need full Xcode for upstream codesign; \
         Command-Line-Tools-only hosts can require an ad-hoc codesign fallback"
    );
    eprintln!(
        "note: with CMake 4 and bundled dependencies, retry with \
         `--build-arg -DCMAKE_POLICY_VERSION_MINIMUM=3.5` if configure fails"
    );
    if opts.deps.is_empty() {
        eprintln!(
            "note: usdview builds need Python UI packages such as PySide6, PyOpenGL, \
             and Jinja2 available to the build"
        );
    }
}

/// Drive the source tree's `build_scripts/build_usd.py` (handles dependencies).
fn build_with_script(
    r: &crate::commands::Resolved,
    src: &Utf8Path,
    opts: &BuildOpts,
) -> Result<()> {
    let script = src.join("build_scripts").join("build_usd.py");
    if !script.as_std_path().is_file() {
        return Err(Error::usage(format!(
            "no build_scripts/build_usd.py under '{src}' (point --build at an OpenUSD checkout, \
             or pass --deps for a direct CMake build)"
        )));
    }
    let python = tools::which("python")
        .or_else(|| tools::which("python3"))
        .ok_or_else(|| {
            Error::coded(
                "REQUIRED_TOOL_MISSING",
                ost_core::Category::Precondition,
                "`python` not found — build_usd.py needs it",
            )
        })?;
    let python = Utf8PathBuf::from_path_buf(python).map_err(|_| {
        Error::coded(
            "INTERNAL_ERROR",
            ost_core::Category::Internal,
            "python path is not UTF-8",
        )
    })?;

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
            return Err(Error::usage(format!(
                "--deps prefix '{dep}' is not a directory"
            )));
        }
    }
    let cmake = tools::which("cmake").ok_or_else(|| {
        Error::coded(
            "REQUIRED_TOOL_MISSING",
            ost_core::Category::Precondition,
            "`cmake` not found on PATH",
        )
    })?;
    let cmake = Utf8PathBuf::from_path_buf(cmake).map_err(|_| {
        Error::coded(
            "INTERNAL_ERROR",
            ost_core::Category::Internal,
            "cmake path is not UTF-8",
        )
    })?;
    let python = tools::which("python")
        .or_else(|| tools::which("python3"))
        .ok_or_else(|| {
            Error::coded(
                "REQUIRED_TOOL_MISSING",
                ost_core::Category::Precondition,
                "`python` not found — USD needs it for bindings",
            )
        })?;
    let python = Utf8PathBuf::from_path_buf(python).map_err(|_| {
        Error::coded(
            "INTERNAL_ERROR",
            ost_core::Category::Internal,
            "python path is not UTF-8",
        )
    })?;
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
        return Err(Error::coded(
            "EXTERNAL_TOOL_FAILED",
            ost_core::Category::ExternalTool,
            format!("{what} failed (exit {})", status.code().unwrap_or(-1)),
        ));
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
        output::success(&serde_json::json!({ "runtimes": items }));
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

/// Resolve the `(platform, profile)` pair a `show`/`validate` invocation refers
/// to, accepting either form the rest of the CLI prints:
/// - `<platform> [--profile <profile>]` (the documented form), or
/// - the full runtime id `ost runtime list` prints, e.g.
///   `openstrata-cy2026-windows-x86_64-py313-usd`.
///
/// When the positional arg is a full id its embedded platform/profile win (the
/// id is self-contained, so a stray `--profile` flag is ignored).
fn platform_profile(positional: &str, profile_flag: &str) -> (String, String) {
    split_runtime_id(positional)
        .unwrap_or_else(|| (positional.to_string(), profile_flag.to_string()))
}

/// Split a full runtime id into `(platform, profile)`. The id is
/// `openstrata-<platform>-<os>-<arch>-py<ver>-<profile>`; the variant slug is
/// always exactly three `-`-separated tokens (`<os>-<arch>-py<ver>`), so the
/// platform is the first token and the profile is everything after the variant
/// (which keeps a hyphenated profile like `lookdev-ai` intact). `None` for
/// anything that is not a runtime id.
fn split_runtime_id(id: &str) -> Option<(String, String)> {
    let rest = id.strip_prefix("openstrata-")?;
    let parts: Vec<&str> = rest.split('-').collect();
    if parts.len() < 5 {
        return None;
    }
    Some((parts[0].to_string(), parts[4..].join("-")))
}

fn show(platform: &str, profile: &str, fmt: Format) -> Result<()> {
    let (platform, profile) = platform_profile(platform, profile);
    let r = resolve(&platform, &profile)?;
    let manifest_path = r.prefix.join(MANIFEST_FILE);
    if !manifest_path.as_std_path().is_file() {
        return Err(Error::coded(
            "RUNTIME_NOT_FOUND",
            ost_core::Category::Precondition,
            format!(
                "runtime '{}' is not pulled (run `ost runtime pull {} --profile {}`)",
                r.runtime.id(),
                platform,
                profile
            ),
        ));
    }
    let src = std::fs::read_to_string(manifest_path.as_std_path())
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;
    let manifest = RuntimeManifest::from_json(&src)
        .map_err(|e| Error::parse(MANIFEST_FILE, anyhow::Error::new(e)))?;

    if fmt.is_json() {
        let mut body = serde_json::to_value(&manifest).expect("manifest serializes");
        if let Some(obj) = body.as_object_mut() {
            obj.insert(
                "openusd_version_drift".into(),
                openusd_version_drift_json(&manifest, &r.artifact_prefix, &platform, &profile),
            );
        }
        output::success(&body);
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
    if let Some(ad) = &manifest.artifact_digest {
        println!("Artifact:   {ad}");
    }
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
    // Flag a recorded OpenUSD version that disagrees with the install's pxr.h.
    if let Some((recorded, real)) = openusd_version_drift(&manifest, &r.artifact_prefix) {
        println!(
            "\nNote: the install's pxr.h reports OpenUSD {real}, but the manifest records \
             {recorded} (stale).\n      Fix with: {}",
            drift_repair_command(&manifest, &platform, &profile)
        );
    }
    Ok(())
}

fn validate(platform: &str, profile: &str, fmt: Format) -> Result<()> {
    let (platform, profile) = platform_profile(platform, profile);
    let r = resolve(&platform, &profile)?;
    let manifest_path = r.prefix.join(MANIFEST_FILE);
    if !manifest_path.as_std_path().is_file() {
        return Err(Error::coded(
            "RUNTIME_NOT_FOUND",
            ost_core::Category::Precondition,
            format!(
                "runtime '{}' is not pulled (run `ost runtime pull {} --profile {}`)",
                r.runtime.id(),
                platform,
                profile
            ),
        ));
    }
    let src = std::fs::read_to_string(manifest_path.as_std_path())
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;
    let mut manifest = RuntimeManifest::from_json(&src)
        .map_err(|e| Error::parse(MANIFEST_FILE, anyhow::Error::new(e)))?;

    // Validate against the effective artifact prefix (the external USD root for
    // an adopted runtime; the store prefix otherwise).
    let mut report = ost_runtime::validate(&r.artifact_prefix, &manifest);
    if let Some((recorded, real)) = openusd_version_drift(&manifest, &r.artifact_prefix) {
        let fix = drift_repair_command(&manifest, &platform, &profile);
        report.checks.push(ost_runtime::Check {
            name: "openusd-version-drift",
            passed: false,
            detail: Some(format!(
                "manifest records OpenUSD {recorded}, but the install's pxr.h reports {real}; \
                 fix with `{fix}`"
            )),
        });
    }
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
        output::report(
            passed,
            &serde_json::json!({
                "runtime": manifest.id,
                "validation": if passed { "passed" } else { "failed" },
                "checks": checks,
            }),
        );
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

fn openusd_version_drift_json(
    manifest: &RuntimeManifest,
    artifact_prefix: &Utf8Path,
    platform: &str,
    profile: &str,
) -> serde_json::Value {
    match openusd_version_drift(manifest, artifact_prefix) {
        Some((recorded, detected)) => serde_json::json!({
            "recorded": recorded,
            "detected": detected,
            "repair": drift_repair_command(manifest, platform, profile),
        }),
        None => serde_json::Value::Null,
    }
}

/// The exact, copy-paste command that repairs a drifted runtime manifest
/// (dogfooding #7: never make the user reconstruct flags or paths).
fn drift_repair_command(manifest: &RuntimeManifest, platform: &str, profile: &str) -> String {
    match (manifest.source, &manifest.external_prefix) {
        // An adopted runtime records its USD root: one command, no blanks.
        (RuntimeSource::Local, Some(_)) => {
            format!("ost runtime repair {platform} --profile {profile}")
        }
        // A build runtime is re-detected in place from its store tree (no rebuild):
        // `repair` re-reads the built pxr.h and restamps the version. Rebuilding
        // (`--build … --force`) would only reproduce the same drifted manifest.
        (RuntimeSource::Build, _) => {
            format!("ost runtime repair {platform} --profile {profile}")
        }
        // An artifact runtime is re-materialized from its pinned digest.
        (RuntimeSource::Artifact, _) => format!(
            "ost runtime pull {platform} --profile {profile} --from-artifact {} --force",
            manifest.artifact_digest.as_deref().unwrap_or("<digest>")
        ),
        _ => {
            format!("ost runtime pull {platform} --profile {profile} --from-usd <usd-root> --force")
        }
    }
}

/// `ost runtime repair` — re-adopt a `local` runtime from its recorded USD
/// root, refreshing the recorded OpenUSD version, layout, and digest in one
/// step (the drift fix `runtime show`/`validate` point at).
fn repair(platform: &str, profile: &str, fmt: Format) -> Result<()> {
    let (platform, profile) = platform_profile(platform, profile);
    let r = resolve(&platform, &profile)?;
    let manifest_path = r.prefix.join(MANIFEST_FILE);
    if !manifest_path.as_std_path().is_file() {
        return Err(Error::coded(
            "RUNTIME_NOT_FOUND",
            ost_core::Category::Precondition,
            format!(
                "runtime '{}' is not pulled (run `ost runtime pull {platform} --profile {profile}`)",
                r.runtime.id()
            ),
        ));
    }
    let src = std::fs::read_to_string(manifest_path.as_std_path())
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;
    let manifest = RuntimeManifest::from_json(&src)
        .map_err(|e| Error::parse(MANIFEST_FILE, anyhow::Error::new(e)))?;

    // repair re-derives the manifest from the tree the runtime already points at,
    // without discarding provenance. Two non-destructive recoveries:
    //   - `local`: re-adopt from the recorded external USD root.
    //   - `build`: re-detect from the built tree in the store (report Finding A) —
    //     the built bits are correct, only the recorded version drifted.
    // Anything else (mock, artifact) has no in-place re-derivation and is pointed
    // at its own refresh command.
    if !matches!(
        (manifest.source, &manifest.external_prefix),
        (RuntimeSource::Local, Some(_)) | (RuntimeSource::Build, _)
    ) {
        return Err(Error::coded(
            "REPAIR_UNSUPPORTED_SOURCE",
            ost_core::Category::Precondition,
            format!(
                "repair re-derives a `local` or `build` runtime in place; \
                 runtime '{}' has source '{}'",
                manifest.id,
                manifest.source.as_str()
            ),
        )
        .with_hint(format!(
            "refresh it with: {}",
            drift_repair_command(&manifest, &platform, &profile)
        )));
    }

    let recorded_before = manifest
        .extensions
        .iter()
        .find(|e| e.id == "openusd")
        .map(|e| e.version.clone());

    let (_has_usd, extensions) = resolve_extensions(&r)?;
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Re-derive deliberately: re-probes the layout, re-reads pxr.h, and resets
    // validation to pending — a repaired manifest still has to prove itself.
    let (repaired, readopted_from) = match (manifest.source, &manifest.external_prefix) {
        (RuntimeSource::Local, Some(root)) => {
            let root = root.clone();
            (adopt_local(&r, &root, extensions, created)?, Some(root))
        }
        _ => (redetect_build(&r, extensions, &manifest, created)?, None),
    };
    let json = repaired
        .to_json()
        .map_err(|e| Error::parse(MANIFEST_FILE, anyhow::Error::new(e)))?;
    std::fs::write(manifest_path.as_std_path(), format!("{json}\n"))
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;

    let recorded_after = repaired
        .extensions
        .iter()
        .find(|e| e.id == "openusd")
        .map(|e| e.version.clone());

    if fmt.is_json() {
        output::success(&serde_json::json!({
            "repaired": true,
            "runtime": repaired.id,
            "source": repaired.source.as_str(),
            "usd_root": readopted_from,
            "openusd_before": recorded_before,
            "openusd_after": recorded_after,
            "digest": repaired.digest,
            "validation": repaired.validation.as_str(),
        }));
        return Ok(());
    }
    match &readopted_from {
        Some(root) => println!("Repaired runtime {} (re-adopted {root})", repaired.id),
        None => println!(
            "Repaired runtime {} (re-detected the built tree in the store)",
            repaired.id
        ),
    }
    match (&recorded_before, &recorded_after) {
        (Some(b), Some(a)) if b != a => println!("  openusd: {b} -> {a}"),
        (_, Some(a)) => println!("  openusd: {a} (unchanged)"),
        _ => {}
    }
    println!("  digest:  {}", repaired.digest);
    println!("\nRe-validate with:");
    println!("  ost runtime validate {platform} --profile {profile}");
    Ok(())
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
        output::success(&serde_json::json!({
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
    use ost_core::host::{Arch, Os};
    use ost_runtime::Runtime;

    /// A manifest shaped like a self-contained, validated `build` runtime.
    fn exportable_manifest() -> RuntimeManifest {
        let host = ost_core::Host {
            os: Os::Linux,
            arch: Arch::X86_64,
        };
        let rt = Runtime::resolve("cy2026", "usd", &host, "3.13.x");
        let mut m = RuntimeManifest::build(
            &rt,
            "3.13.x",
            vec!["usd-stage-read".into()],
            vec!["bin".into(), "lib".into()],
            vec![ExtensionRecord {
                id: "openusd".into(),
                version: "26.08".into(),
                features: vec!["core".into()],
            }],
            1_750_000_000,
            RuntimeSource::Build,
        );
        m.set_validation(Validation::Passed);
        m
    }

    #[test]
    fn export_gates_refuse_mock_deps_and_unvalidated() {
        assert!(check_exportable(&exportable_manifest()).is_ok());

        let mut mock = exportable_manifest();
        mock.source = RuntimeSource::Mock;
        let err = check_exportable(&mock).unwrap_err();
        assert_eq!(err.code(), "EXPORT_REAL_RUNTIME_REQUIRED");

        let mut deps = exportable_manifest();
        deps.runtime_deps = vec!["/deps/tbb".into()];
        let err = check_exportable(&deps).unwrap_err();
        assert_eq!(err.code(), "EXPORT_DEPS_NOT_PORTABLE");

        let mut pending = exportable_manifest();
        pending.set_validation(Validation::Pending);
        let err = check_exportable(&pending).unwrap_err();
        assert_eq!(err.code(), "EXPORT_VALIDATION_REQUIRED");
    }

    #[test]
    fn runtime_artifact_manifest_embeds_identity_and_provenance() {
        let m = exportable_manifest();
        let packed = ost_build::PackResult {
            files: vec![],
            archive_digest: format!("sha256:{}", "ab".repeat(32)),
            total_size: 10,
            archive_size: 5,
        };
        let producer = runtime_artifact_manifest(&m, "rt.tar.zst", &packed, 1_760_000_000);

        assert_eq!(producer["kind"], "openstrata.runtime");
        assert_eq!(producer["name"], m.id);
        // Version prefers the openusd extension's real version.
        assert_eq!(producer["version"], "26.08");
        assert_eq!(producer["provenance"]["runtime"]["digest"], m.digest);
        assert_eq!(producer["provenance"]["runtime"]["validation"], "passed");
        // The embedded manifest restores byte-equal on fetch.
        let embedded: RuntimeManifest =
            serde_json::from_value(producer["provenance"]["runtime_manifest"].clone()).unwrap();
        assert_eq!(embedded, m);

        // It derives a valid registry record of kind `runtime`.
        let record = ost_artifact::ArtifactRecord::from_producer_manifest(
            &producer,
            ArtifactSource::Published,
            1_760_000_000,
            "ost test",
        )
        .unwrap();
        assert_eq!(record.kind, ArtifactKind::Runtime);
        assert_eq!(record.name, m.id);
        assert_eq!(record.validation, "passed");
        assert_eq!(record.runtime_digest.as_deref(), Some(m.digest.as_str()));
    }

    #[test]
    fn drift_repair_command_is_copy_paste_exact_per_source() {
        // Adopted local: the one-step repair command, no blanks to fill.
        let mut local = exportable_manifest();
        local.source = RuntimeSource::Local;
        local.external_prefix = Some("/opt/usd".into());
        assert_eq!(
            drift_repair_command(&local, "cy2026", "usd"),
            "ost runtime repair cy2026 --profile usd"
        );

        // Artifact: re-materialize from the exact pinned digest.
        let mut artifact = exportable_manifest();
        artifact.source = RuntimeSource::Artifact;
        artifact.artifact_digest = Some(format!("sha256:{}", "ab".repeat(32)));
        let cmd = drift_repair_command(&artifact, "cy2026", "usd");
        assert!(cmd.contains("--from-artifact sha256:"), "{cmd}");
        assert!(cmd.ends_with("--force"));

        // Build: re-detect the built tree in place (no rebuild, no blanks) — a
        // rebuild would only reproduce the same drifted manifest (Finding A).
        let build = exportable_manifest();
        assert_eq!(
            drift_repair_command(&build, "cy2026", "usd"),
            "ost runtime repair cy2026 --profile usd"
        );
    }

    #[test]
    fn full_runtime_id_splits_into_platform_and_profile() {
        assert_eq!(
            split_runtime_id("openstrata-cy2026-windows-x86_64-py313-usd"),
            Some(("cy2026".to_string(), "usd".to_string()))
        );
        // A hyphenated profile survives (everything after the 3-token variant).
        assert_eq!(
            split_runtime_id("openstrata-cy2026-linux-x86_64-py311-lookdev-ai"),
            Some(("cy2026".to_string(), "lookdev-ai".to_string()))
        );
    }

    #[test]
    fn detects_real_openusd_version_from_header() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut root = Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
        root.push(format!("ost-pxrh-{}-{nanos}", std::process::id()));
        let pxr_dir = root.join("include/pxr");
        std::fs::create_dir_all(pxr_dir.as_std_path()).unwrap();
        std::fs::write(
            pxr_dir.join("pxr.h").as_std_path(),
            "#define PXR_MAJOR_VERSION 0\n\
             #define PXR_MINOR_VERSION 26\n\
             #define PXR_PATCH_VERSION 8\n\
             #define PXR_VERSION 2608\n",
        )
        .unwrap();

        assert_eq!(detect_openusd_version(&root), Some("26.08".to_string()));
        // Missing header → no guess.
        std::fs::remove_dir_all(root.as_std_path()).ok();
        assert_eq!(detect_openusd_version(&root), None);
    }

    #[test]
    fn same_release_ignores_catalog_certification_suffix() {
        // The detected `<minor>.<patch>` matches the catalog default's leading
        // components → same release; the `.01` certification revision is kept and
        // no "discrepancy" note fires.
        assert!(same_openusd_release("25.05", "25.05.01"));
        // A genuinely different install is corrected (and noted).
        assert!(!same_openusd_release("26.08", "25.05.01"));
        assert!(!same_openusd_release("25.06", "25.05.01"));
        // Equal-length exact match still holds; unparseable input is treated as a
        // mismatch so a malformed catalog entry gets overwritten.
        assert!(same_openusd_release("25.05", "25.05"));
        assert!(!same_openusd_release("25.05", "twentyfive"));
    }

    #[test]
    fn openusd_version_drift_reports_stale_manifest() {
        use ost_core::host::Arch;

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut root = Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
        root.push(format!("ost-pxrd-{}-{nanos}", std::process::id()));
        let pxr_dir = root.join("include/pxr");
        std::fs::create_dir_all(pxr_dir.as_std_path()).unwrap();
        std::fs::write(
            pxr_dir.join("pxr.h").as_std_path(),
            "#define PXR_MAJOR_VERSION 0\n\
             #define PXR_MINOR_VERSION 26\n\
             #define PXR_PATCH_VERSION 8\n",
        )
        .unwrap();

        let host = Host {
            os: Os::Linux,
            arch: Arch::X86_64,
        };
        let rt = ost_runtime::Runtime::resolve("cy2026", "usd", &host, "3.13.x");
        let manifest = RuntimeManifest::build(
            &rt,
            "3.13.x",
            vec!["usd-stage-read".into()],
            vec![],
            vec![ExtensionRecord {
                id: "openusd".into(),
                version: "25.05.01".into(),
                features: vec!["core".into()],
            }],
            1_700_000_000,
            RuntimeSource::Local,
        );

        assert_eq!(
            openusd_version_drift(&manifest, &root),
            Some(("25.05.01".to_string(), "26.08".to_string()))
        );
        let json = openusd_version_drift_json(&manifest, &root, "cy2026", "usd");
        assert_eq!(json["recorded"], "25.05.01");
        assert_eq!(json["detected"], "26.08");
        assert!(json["repair"]
            .as_str()
            .unwrap()
            .contains("ost runtime pull cy2026 --profile usd"));

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn build_dep_requirements_track_the_profile_capabilities() {
        // A minimal core profile implies no USD build deps.
        assert!(build_dep_requirements(&["python-tooling".into(), "image-io".into()]).is_empty());

        // A USD profile needs Jinja2 for schema generation, nothing UI.
        let usd = build_dep_requirements(&["usd-stage-read".into(), "usd-shading".into()]);
        assert_eq!(usd, vec![("jinja2", "Jinja2")]);

        // A dev profile with qt-ui needs PySide6 but not PyOpenGL.
        let dev = build_dep_requirements(&["qt-ui".into(), "cmake-build".into()]);
        assert_eq!(dev, vec![("PySide6", "PySide6")]);

        // A lookdev profile (hydra-preview) needs all three.
        let lookdev = build_dep_requirements(&[
            "usd-stage-read".into(),
            "usd-materialx".into(),
            "hydra-preview".into(),
        ]);
        assert_eq!(
            lookdev,
            vec![
                ("jinja2", "Jinja2"),
                ("PySide6", "PySide6"),
                ("OpenGL", "PyOpenGL")
            ]
        );
    }

    #[test]
    fn stamp_corrects_catalog_default_to_built_version() {
        // A freshly built tree reports its real version in pxr.h; stamping must
        // overwrite the catalog default so the L1 gate reflects the real build
        // (report Finding A: the `--build` path used to record the default).
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut root = Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
        root.push(format!("ost-stamp-{}-{nanos}", std::process::id()));
        let pxr_dir = root.join("include/pxr");
        std::fs::create_dir_all(pxr_dir.as_std_path()).unwrap();
        std::fs::write(
            pxr_dir.join("pxr.h").as_std_path(),
            "#define PXR_MINOR_VERSION 26\n#define PXR_PATCH_VERSION 5\n",
        )
        .unwrap();

        let mut exts = vec![ExtensionRecord {
            id: "openusd".into(),
            version: "25.05.01".into(),
            features: vec!["core".into()],
        }];
        stamp_openusd_version(&mut exts, &root, "built");
        assert_eq!(exts[0].version, "26.05");

        // Same release (bare 26.05 vs 26.05) is left untouched — no spurious note.
        stamp_openusd_version(&mut exts, &root, "built");
        assert_eq!(exts[0].version, "26.05");

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn non_ids_are_not_split() {
        assert_eq!(split_runtime_id("cy2026"), None);
        assert_eq!(split_runtime_id("openstrata-cy2026"), None);
        // The bare-platform form falls through to the --profile flag.
        assert_eq!(
            platform_profile("cy2026", "usd"),
            ("cy2026".to_string(), "usd".to_string())
        );
        // A full id ignores the (contradictory) flag.
        assert_eq!(
            platform_profile("openstrata-cy2026-windows-x86_64-py313-usd", "core"),
            ("cy2026".to_string(), "usd".to_string())
        );
    }

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
