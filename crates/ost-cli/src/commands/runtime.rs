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

use std::fs::File;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Subcommand;

use camino::{Utf8Path, Utf8PathBuf};

use ost_artifact::{ArtifactKind, ArtifactSource, ArtifactStore};
use ost_core::host::Os;
use ost_core::paths::Store;
use ost_core::{digest, tools, Error, Host, Result};
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
        } => export(&platform, &profile, dist.as_deref(), fmt),
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
    //
    // Only correct (and note) it when the install is a genuinely *different*
    // release. The catalog default carries a certification-revision component
    // (`25.05.01`) that `pxr.h` doesn't expose, so adopting a real 25.05 install
    // would otherwise overwrite the richer `25.05.01` with the bare `25.05` and
    // print a "discrepancy" note for what is the same release.
    match detect_openusd_version(&root) {
        Some(real) => {
            if let Some(ext) = extensions.iter_mut().find(|e| e.id == "openusd") {
                if !same_openusd_release(&real, &ext.version) {
                    eprintln!(
                        "note: adopted OpenUSD reports version {real} (catalog default was {})",
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

    // Digest-pinned fetch: re-hash the stored archive before trusting it. A
    // store corrupted at rest must not materialize as a runtime.
    let archive = store.archive_path(&record);
    let mut f = File::open(archive.as_std_path()).map_err(|e| Error::io(archive.to_string(), e))?;
    let (actual, _) =
        digest::sha256_hex_reader(&mut f).map_err(|e| Error::io(archive.to_string(), e))?;
    if actual != record.digest {
        return Err(Error::coded(
            "ARTIFACT_DIGEST_MISMATCH",
            ost_core::Category::Validation,
            format!(
                "stored archive for {} hashes to {actual} — the local store is corrupted",
                record.short_digest()
            ),
        )
        .with_hint("re-import the artifact, then retry the pull"));
    }

    // Fresh materialization: never extract over a stale prefix.
    if r.prefix.as_std_path().exists() {
        std::fs::remove_dir_all(r.prefix.as_std_path())
            .map_err(|e| Error::io(r.prefix.to_string(), e))?;
    }
    std::fs::create_dir_all(r.prefix.as_std_path())
        .map_err(|e| Error::io(r.prefix.to_string(), e))?;

    let file = File::open(archive.as_std_path()).map_err(|e| Error::io(archive.to_string(), e))?;
    let decoder =
        zstd::stream::read::Decoder::new(file).map_err(|e| Error::io(archive.to_string(), e))?;
    let mut tar = tar::Archive::new(decoder);
    // `unpack` refuses entries that would escape the destination.
    tar.unpack(r.prefix.as_std_path())
        .map_err(|e| Error::io(r.prefix.to_string(), e))?;

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

/// `ost runtime export` — pack a pulled real runtime and register it in the
/// local artifact registry, addressed by digest.
fn export(platform: &str, profile: &str, dist: Option<&str>, fmt: Format) -> Result<()> {
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
    let files: Vec<Utf8PathBuf> = ost_build::stage_files(&effective)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::InvalidData {
                Error::validation(e.to_string())
            } else {
                Error::io(effective.to_string(), e)
            }
        })?
        .into_iter()
        .filter(|p| p != &effective.join(MANIFEST_FILE))
        .collect();
    if files.is_empty() {
        return Err(Error::validation(format!(
            "runtime '{}' has no files under '{effective}' — nothing to export",
            manifest.id
        )));
    }

    let store = Store::discover();
    let staging_default = store.cache().join("runtime-export").join(&manifest.id);
    let dist_dir = match dist {
        Some(d) => Utf8PathBuf::from(d),
        None => staging_default.clone(),
    };
    if dist_dir.as_std_path().exists() {
        std::fs::remove_dir_all(dist_dir.as_std_path())
            .map_err(|e| Error::io(dist_dir.to_string(), e))?;
    }
    std::fs::create_dir_all(dist_dir.as_std_path())
        .map_err(|e| Error::io(dist_dir.to_string(), e))?;

    let archive_name = format!("{}.tar.zst", manifest.id);
    let archive_path = dist_dir.join(&archive_name);
    let packed = ost_build::pack_dir(&effective, &archive_path, &files)
        .map_err(|e| Error::io(archive_path.to_string(), e))?;

    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let producer = runtime_artifact_manifest(&manifest, &archive_name, &packed, created);
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
             {recorded} (stale).\n      Re-pull to refresh: \
             ost runtime pull {platform} --profile {profile} --from-usd <usd-root> --force"
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
        report.checks.push(ost_runtime::Check {
            name: "openusd-version-drift",
            passed: false,
            detail: Some(format!(
                "manifest records OpenUSD {recorded}, but the install's pxr.h reports {real}; \
                 re-pull with `ost runtime pull {platform} --profile {profile} --from-usd <usd-root> --force`"
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
            "repair": format!(
                "ost runtime pull {platform} --profile {profile} --from-usd <usd-root> --force"
            ),
        }),
        None => serde_json::Value::Null,
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
