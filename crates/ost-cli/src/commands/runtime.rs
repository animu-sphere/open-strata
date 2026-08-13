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

use std::collections::BTreeSet;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::Subcommand;

use camino::{Utf8Path, Utf8PathBuf};

use ost_artifact::{ArtifactKind, ArtifactSource, ArtifactStore};
use ost_build::GlibcVersion;
use ost_core::host::Os;
use ost_core::paths::Store;
use ost_core::variant::Abi;
use ost_core::{tools, Error, Host, Result, Variant};
use ost_platform::version_satisfies_constraint;
use ost_runtime::{
    python_minor, ExtensionRecord, HostPackageManager, HostRequirement, OpenUsdBuilder,
    OpenUsdVariantId, ResolvedDependencyIdentity, ResolvedOpenUsdCompatibility,
    ResolvedSourceIdentity, RuntimeManifest, RuntimeSource, Validation, MANIFEST_FILE,
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
        /// build_usd.py: e.g. `--build-arg --examples`. With `--deps` (CMake):
        /// e.g. `--build-arg -DPXR_BUILD_TESTS=OFF`. Hyphen values allowed.
        #[arg(long = "build-arg", allow_hyphen_values = true)]
        build_args: Vec<String>,
        /// Constrained OpenUSD build shape. Defaults to `standard` for managed
        /// source builds. The selected CY cell supplies deterministic builder
        /// arguments and becomes part of runtime identity.
        #[arg(long = "openusd-variant", value_parser = parse_openusd_variant)]
        openusd_variant: Option<OpenUsdVariantId>,
        /// macOS SDK to build `--build` against: a full path, or a version like
        /// `15.2` resolved with `xcrun --sdk macosx<version> --show-sdk-path`.
        /// Sets `CMAKE_OSX_SYSROOT` for the whole build.
        #[arg(long)]
        sdk: Option<String>,
        /// macOS deployment target for `--build` (`CMAKE_OSX_DEPLOYMENT_TARGET`),
        /// e.g. `14.5` — the oldest macOS the produced runtime must load on.
        #[arg(long)]
        deployment_target: Option<String>,
        /// Dependency prefix for a direct CMake build of `--build` (repeatable;
        /// joined into `CMAKE_PREFIX_PATH`). When given, OpenUSD is built with
        /// CMake against these deps instead of via build_usd.py. Falls back to
        /// `OST_USD_DEPS` (path-separator list).
        #[arg(long)]
        deps: Vec<String>,
        /// Native package the produced runtime leaves to its consuming host,
        /// written as `apt:PACKAGE` (Linux) or `brew:FORMULA` (macOS).
        /// Repeatable and recorded as compatibility identity.
        #[arg(
            long = "host-package",
            value_name = "MANAGER:PACKAGE",
            value_parser = parse_host_requirement,
            conflicts_with = "from_artifact"
        )]
        host_requirements: Vec<HostRequirement>,
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
        /// libraries, resources, share, and CMake config), dropping the
        /// source/build tree of a runtime adopted from a full USD build. Much
        /// smaller archive and faster per-PR pull.
        #[arg(long)]
        slim: bool,
        /// zstd compression level (1–22). Lower is faster; the default (19)
        /// favors a small artifact, packed once and pulled many times.
        #[arg(long, default_value_t = ost_build::ZSTD_LEVEL)]
        level: i32,
        /// zstd worker threads for compression. Defaults to the host's
        /// available parallelism, or the byte-stable single-threaded encoder
        /// when SOURCE_DATE_EPOCH is set; `--jobs 0` also forces it explicitly.
        #[arg(long)]
        jobs: Option<u32>,
        /// JSON file describing what produced this artifact, so a producer that
        /// is not GitHub Actions can still emit provenance. Requires a non-empty
        /// `source.repository`, `source.revision`, `builder.id`, and a populated
        /// `builder.identity` object. Optional `dependencies` entries require an
        /// exact name, version, and source repository/revision. A managed
        /// `build_usd.py` runtime inserts its automatically captured closure when
        /// omitted and rejects conflicting source/dependency claims. A managed
        /// build from a non-Git source requires this file to identify its source.
        /// Captured entries also carry a canonical `archive_digest`.
        #[arg(long)]
        build_metadata: Option<Utf8PathBuf>,
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
            openusd_variant,
            sdk,
            deployment_target,
            deps,
            host_requirements,
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
                openusd_variant,
                sdk,
                deployment_target,
                deps,
                host_requirements,
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
            build_metadata,
        } => export(
            &platform,
            &profile,
            dist.as_deref(),
            slim,
            ExportPack { level, jobs },
            build_metadata.as_deref(),
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

/// Parse a declared native package without ever passing user input through a
/// shell. Keeping the accepted alphabet aligned with CI's `host_packages`
/// contract makes the runtime declaration directly renderable there.
fn parse_host_requirement(value: &str) -> std::result::Result<HostRequirement, String> {
    let (manager, name) = value.split_once(':').ok_or_else(|| {
        format!("invalid host package '{value}' (expected apt:PACKAGE or brew:FORMULA)")
    })?;
    let manager = match manager {
        "apt" => HostPackageManager::Apt,
        "brew" => HostPackageManager::Brew,
        other => {
            return Err(format!(
                "unknown host package manager '{other}' (expected apt or brew)"
            ))
        }
    };
    if !manager.accepts_name(name) {
        return Err(format!(
            "invalid {} package name '{name}' (expected one package-manager argument, no flags or shell metacharacters)",
            manager.as_str()
        ));
    }
    Ok(HostRequirement {
        manager,
        name: name.to_string(),
    })
}

fn validate_host_requirement_targets(requirements: &[HostRequirement], os: Os) -> Result<()> {
    for requirement in requirements {
        let valid = matches!(
            (os, requirement.manager),
            (Os::Linux, HostPackageManager::Apt) | (Os::Macos, HostPackageManager::Brew)
        );
        if !valid {
            return Err(Error::usage(format!(
                "host package '{}:{}' does not apply to the runtime target '{}'",
                requirement.manager.as_str(),
                requirement.name,
                os.as_str()
            ))
            .with_hint(match os {
                Os::Linux => "declare Linux packages as --host-package apt:PACKAGE",
                Os::Macos => "declare macOS formulae as --host-package brew:FORMULA",
                Os::Windows => {
                    "Windows runtime host dependencies must be provisioned on the runner image; no package manager is assumed"
                }
            }));
        }
    }
    Ok(())
}

fn validate_embedded_host_requirement_targets(
    requirements: &[HostRequirement],
    os: Os,
) -> Result<()> {
    validate_host_requirement_targets(requirements, os).map_err(|error| {
        Error::InvalidManifest(format!(
            "runtime host_requirements do not match target '{}': {error}",
            os.as_str()
        ))
    })
}

fn print_host_requirements(requirements: &[HostRequirement], prefix: &str) {
    if requirements.is_empty() {
        return;
    }
    let rendered = requirements
        .iter()
        .map(|requirement| format!("{}:{}", requirement.manager.as_str(), requirement.name))
        .collect::<Vec<_>>()
        .join(", ");
    println!("{prefix}{rendered}");
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
    pub openusd_variant: Option<OpenUsdVariantId>,
    /// `--sdk <path|version>`: the macOS SDK to build against.
    pub sdk: Option<String>,
    /// `--deployment-target <version>`: the oldest macOS the build must load on.
    pub deployment_target: Option<String>,
    /// `--deps <prefix>` (or `OST_USD_DEPS`): when non-empty, build OpenUSD with
    /// CMake directly against these dependency prefixes instead of build_usd.py.
    pub deps: Vec<String>,
    /// Host-native packages intentionally excluded from a produced runtime.
    pub host_requirements: Vec<HostRequirement>,
    /// `--from-artifact <digest>`: materialize from the local artifact registry.
    pub from_artifact: Option<String>,
}

fn pull(platform: &str, profile: &str, force: bool, src: PullSource, fmt: Format) -> Result<()> {
    let r = resolve(platform, profile)?;

    validate_host_requirement_targets(&src.host_requirements, r.runtime.variant.os)?;

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

    if src.openusd_variant.is_some() && build_src.is_none() {
        return Err(
            Error::usage("--openusd-variant applies only to `runtime pull --build`")
                .with_hint("add --build <OPENUSD_SOURCE>, or remove --openusd-variant"),
        );
    }

    // Dependency prefixes for a CMake-direct build (flag, else env list).
    let deps: Vec<String> = if !src.deps.is_empty() {
        src.deps.clone()
    } else {
        env_nonempty("OST_USD_DEPS")
            .map(|v| split_dep_prefixes(&v))
            .unwrap_or_default()
    };

    let mut selected_openusd: Option<ResolvedOpenUsdCompatibility> = None;
    let mut manifest = if let Some(digest_ref) = &src.from_artifact {
        fetch_from_artifact(&r, digest_ref)?
    } else if let Some(usd_src) = build_src {
        let usd_src = validate_openusd_source(&usd_src)?;
        let platform_manifest = ost_platform::load_one(platform)?;
        let builder = if deps.is_empty() {
            OpenUsdBuilder::BuildUsd
        } else {
            OpenUsdBuilder::Cmake
        };
        let mut selection = resolve_openusd_build(
            &platform_manifest,
            r.runtime.variant.os,
            r.runtime.variant.arch,
            src.openusd_variant,
            builder,
            src.build_args,
        )?;
        let prepared = selection
            .compatibility
            .take()
            .map(|compatibility| prepare_openusd_build(compatibility, builder, &mut selection.args))
            .transpose()?;
        if force {
            reset_managed_runtime_build(&r)?;
        }
        let opts = BuildOpts {
            jobs: src.jobs,
            extra: selection.args,
            python: prepared.as_ref().map(|value| value.python.clone()),
            build_env: prepared
                .as_ref()
                .map(|value| value.build_env.clone())
                .unwrap_or_default(),
            macos: MacosBuildOpts {
                sdk: src.sdk,
                deployment_target: src.deployment_target,
            },
            deps,
        };
        let manifest = build_from_source(&r, &usd_src, &opts, extensions, created)?;
        selected_openusd = prepared
            .map(|prepared| {
                finalize_openusd_build(prepared.compatibility, builder, &r.prefix, &opts.deps)
            })
            .transpose()?;
        manifest
    } else if let Some(usd_root) = adopt {
        adopt_local(&r, &usd_root, extensions, created)?
    } else {
        materialize_mock(&r, has_usd, extensions, created)?
    };
    if manifest.source != RuntimeSource::Artifact {
        manifest.set_host_requirements(src.host_requirements);
    }
    if selected_openusd.is_some() {
        manifest.set_openusd_compatibility(selected_openusd)?;
    }

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
            "host_requirements": manifest.host_requirements,
            "openusd_compatibility": manifest.openusd_compatibility,
            "build_source": manifest.build_source,
            "build_dependencies": manifest.build_dependencies,
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
    print_host_requirements(&manifest.host_requirements, "  host:    ");
    print_openusd_compatibility(&manifest, "  ");
    print_build_identities(&manifest, "  ");
    println!("\nValidate with:");
    println!("  ost runtime validate {} --profile {}", platform, profile);
    Ok(())
}

fn parse_openusd_variant(value: &str) -> std::result::Result<OpenUsdVariantId, String> {
    match value {
        "headless" => Ok(OpenUsdVariantId::Headless),
        "standard" => Ok(OpenUsdVariantId::Standard),
        "vulkan" => Ok(OpenUsdVariantId::Vulkan),
        _ => Err(format!(
            "unknown OpenUSD variant '{value}' (expected headless, standard, or vulkan)"
        )),
    }
}

#[derive(Debug)]
struct OpenUsdBuildSelection {
    compatibility: Option<ResolvedOpenUsdCompatibility>,
    args: Vec<String>,
}

/// Select a declared cell without regressing legacy builds on targets that do
/// not yet have an approved matrix cell. An explicit `--openusd-variant` is a
/// compatibility claim and therefore fails when absent; an implicit standard
/// selection falls back to the legacy unclassified build on those targets.
fn resolve_openusd_build(
    platform: &ost_platform::Platform,
    os: Os,
    arch: ost_core::host::Arch,
    requested: Option<OpenUsdVariantId>,
    builder: OpenUsdBuilder,
    user_args: Vec<String>,
) -> Result<OpenUsdBuildSelection> {
    let variant_id = requested.unwrap_or(OpenUsdVariantId::Standard);
    let Some((compatibility, variant)) = platform.resolve_openusd(os, arch, variant_id) else {
        if requested.is_none() {
            return Ok(OpenUsdBuildSelection {
                compatibility: None,
                args: user_args,
            });
        }
        return Err(Error::coded(
            "OPENUSD_BUILD_CELL_UNSUPPORTED",
            ost_core::Category::Configuration,
            format!(
                "platform '{}' declares no OpenUSD '{}' cell for {}-{}",
                platform.id,
                variant_id.as_str(),
                os.as_str(),
                arch.as_str()
            ),
        )
        .with_hint(
            "select a declared platform/architecture cell or add it to the platform manifest",
        ));
    };

    if !variant.builders.contains(&builder) {
        return Err(Error::coded(
            "OPENUSD_VARIANT_BUILDER_UNSUPPORTED",
            ost_core::Category::Configuration,
            format!(
                "OpenUSD '{}' for '{}' does not support the {} builder",
                variant_id.as_str(),
                platform.id,
                match builder {
                    OpenUsdBuilder::BuildUsd => "build_usd.py",
                    OpenUsdBuilder::Cmake => "CMake-direct",
                }
            ),
        )
        .with_hint(match builder {
            OpenUsdBuilder::BuildUsd => {
                "supply --deps to select the declared CMake-direct build cell"
            }
            OpenUsdBuilder::Cmake => {
                "remove --deps to select build_usd.py, or declare CMake support in the CY cell"
            }
        }));
    }

    let mut args = match builder {
        OpenUsdBuilder::BuildUsd => variant.build_usd_args.clone(),
        OpenUsdBuilder::Cmake => variant.cmake_args.clone(),
    };
    if builder == OpenUsdBuilder::BuildUsd && !variant.cmake_args.is_empty() {
        args.push(build_usd_cmake_arg(&variant.cmake_args));
    }

    let component_conflict = if builder == OpenUsdBuilder::BuildUsd {
        args.iter().find_map(|declared| {
            option_name(declared).and_then(|name| {
                let component = name.strip_prefix("no-").unwrap_or(name);
                user_args
                    .iter()
                    .find(|candidate| names_component(candidate, component))
                    .map(|candidate| (component.to_string(), candidate.clone()))
            })
        })
    } else {
        None
    };
    if let Some((dimension, supplied)) = component_conflict {
        return Err(Error::coded(
            "OPENUSD_VARIANT_OVERRIDE",
            ost_core::Category::Configuration,
            format!(
                "--build-arg '{supplied}' overrides compatibility dimension '{dimension}' from OpenUSD variant '{}'",
                variant_id.as_str()
            ),
        )
        .with_hint("remove the conflicting --build-arg or select the variant that declares the required capability"));
    }

    let mut protected: BTreeSet<String> = variant
        .cmake_args
        .iter()
        .flat_map(|arg| cmake_definition_keys(arg))
        .collect();
    protected.extend(
        [
            "CMAKE_C_COMPILER",
            "CMAKE_CXX_COMPILER",
            "CMAKE_CXX_STANDARD",
            "CMAKE_PREFIX_PATH",
            "Python_EXECUTABLE",
            "Python3_EXECUTABLE",
            "TBB_DIR",
            "TBB_ROOT",
            "TBB_ROOT_DIR",
        ]
        .into_iter()
        .map(str::to_string),
    );
    if let Some((dimension, supplied)) = user_args.iter().find_map(|arg| {
        cmake_definition_keys(arg)
            .into_iter()
            .find(|key| protected.contains(key))
            .map(|key| (key, arg.clone()))
    }) {
        return Err(Error::coded(
            "OPENUSD_VARIANT_OVERRIDE",
            ost_core::Category::Configuration,
            format!(
                "--build-arg '{supplied}' overrides compatibility dimension '{dimension}' from OpenUSD variant '{}'",
                variant_id.as_str()
            ),
        )
        .with_hint("remove the conflicting --build-arg or select the variant that declares the required capability"));
    }
    args.extend(user_args);
    Ok(OpenUsdBuildSelection {
        compatibility: Some(compatibility),
        args,
    })
}

fn cmake_definition_key(arg: &str) -> Option<&str> {
    arg.strip_prefix("-D")?
        .split(['=', ':'])
        .next()
        .filter(|key| !key.is_empty())
}

fn cmake_definition_keys(arg: &str) -> Vec<String> {
    arg.split(|character: char| character == ',' || character.is_whitespace() || character == '"')
        .filter_map(|token| token.find("-D").map(|offset| &token[offset..]))
        .filter_map(cmake_definition_key)
        .map(str::to_string)
        .collect()
}

fn build_usd_cmake_arg(args: &[String]) -> String {
    format!("--build-args=USD,{}", args.join(" "))
}

struct PreparedOpenUsdBuild {
    compatibility: ResolvedOpenUsdCompatibility,
    python: Utf8PathBuf,
    build_env: Vec<(String, String)>,
}

fn prepare_openusd_build(
    mut compatibility: ResolvedOpenUsdCompatibility,
    builder: OpenUsdBuilder,
    args: &mut Vec<String>,
) -> Result<PreparedOpenUsdBuild> {
    let (python, python_version) =
        find_python_for_constraint(&compatibility.python.version_constraint)?;
    compatibility.python.version = Some(python_version);

    if compatibility.toolchain.family != "gcc" || compatibility.os != Os::Linux {
        return Err(Error::coded(
            "OPENUSD_TOOLCHAIN_UNSUPPORTED",
            ost_core::Category::Configuration,
            format!(
                "managed compatibility verification does not support {} on {}",
                compatibility.toolchain.family,
                compatibility.os.as_str()
            ),
        )
        .with_hint(
            "remove the explicit variant or add a verified toolchain adapter for this cell",
        ));
    }
    let (cc, cxx, compiler_version) =
        find_gcc_toolchain(&compatibility.toolchain.version_constraint)?;
    compatibility.toolchain.version = Some(compiler_version);

    let pins = vec![
        format!("-DCMAKE_C_COMPILER={}", cc.as_str().replace('\\', "/")),
        format!("-DCMAKE_CXX_COMPILER={}", cxx.as_str().replace('\\', "/")),
        format!(
            "-DCMAKE_CXX_STANDARD={}",
            compatibility.toolchain.cxx_standard
        ),
        format!(
            "-DPython3_EXECUTABLE={}",
            python.as_str().replace('\\', "/")
        ),
        format!("-DPython_EXECUTABLE={}", python.as_str().replace('\\', "/")),
    ];
    match builder {
        OpenUsdBuilder::BuildUsd => {
            if let Some(passthrough) = args
                .iter_mut()
                .find(|arg| arg.starts_with("--build-args=USD,"))
            {
                passthrough.push(' ');
                passthrough.push_str(&pins.join(" "));
            } else {
                args.push(build_usd_cmake_arg(&pins));
            }
        }
        OpenUsdBuilder::Cmake => args.extend(pins),
    }

    Ok(PreparedOpenUsdBuild {
        compatibility,
        python,
        build_env: vec![
            ("CC".into(), cc.to_string()),
            ("CXX".into(), cxx.to_string()),
        ],
    })
}

fn finalize_openusd_build(
    mut compatibility: ResolvedOpenUsdCompatibility,
    builder: OpenUsdBuilder,
    prefix: &Utf8Path,
    deps: &[String],
) -> Result<ResolvedOpenUsdCompatibility> {
    let roots: Vec<Utf8PathBuf> = match builder {
        OpenUsdBuilder::BuildUsd => vec![prefix.to_path_buf()],
        OpenUsdBuilder::Cmake => deps.iter().map(Utf8PathBuf::from).collect(),
    };
    let tbb_version = tbb_version_from_roots(&roots).ok_or_else(|| {
        Error::coded(
            "OPENUSD_PROVIDER_VERSION_UNKNOWN",
            ost_core::Category::Validation,
            format!(
                "could not observe oneTBB version from {} provider roots",
                compatibility.tbb.provider
            ),
        )
        .with_hint("provide a oneTBB install carrying include/oneapi/tbb/version.h or include/tbb/version.h")
    })?;
    ensure_version_matches(
        "oneTBB",
        &tbb_version,
        &compatibility.tbb.version_constraint,
    )?;
    compatibility.tbb.version = Some(tbb_version);

    if compatibility.toolchain.runtime.family != "glibc" {
        return Err(Error::coded(
            "OPENUSD_RUNTIME_BOUNDARY_UNSUPPORTED",
            ost_core::Category::Configuration,
            format!(
                "managed compatibility verification does not support runtime boundary '{}'",
                compatibility.toolchain.runtime.family
            ),
        ));
    }
    let files =
        ost_build::stage_files(prefix).map_err(|error| Error::io(prefix.to_string(), error))?;
    let glibc = ost_build::max_glibc_floor(files.iter().map(Utf8PathBuf::as_path))
        .map_err(|error| Error::io(prefix.to_string(), error))?
        .ok_or_else(|| {
            Error::coded(
                "OPENUSD_RUNTIME_BOUNDARY_UNKNOWN",
                ost_core::Category::Validation,
                format!("no glibc symbol floor could be measured under '{prefix}'"),
            )
        })?
        .to_string();
    ensure_version_matches(
        "glibc",
        &glibc,
        &compatibility.toolchain.runtime.version_constraint,
    )?;
    compatibility.toolchain.runtime.version = Some(glibc);

    if !compatibility.is_verified() {
        return Err(Error::coded(
            "OPENUSD_COMPATIBILITY_UNVERIFIED",
            ost_core::Category::Validation,
            "OpenUSD build completed without exact compatibility provider versions",
        ));
    }
    Ok(compatibility)
}

fn find_python_for_constraint(constraint: &str) -> Result<(Utf8PathBuf, String)> {
    let preferred = numeric_prefix(constraint, 2).map(|version| format!("python{version}"));
    let mut candidates: Vec<String> = preferred.into_iter().collect();
    candidates.extend(["python3".into(), "python".into()]);
    candidates.dedup();
    let mut observed = Vec::new();
    for candidate in candidates {
        let Some(path) = tools::which(&candidate) else {
            continue;
        };
        let path = utf8_tool_path(path, &candidate)?;
        let version = command_stdout(
            &path,
            &["-c", "import platform; print(platform.python_version())"],
            "probe Python version",
        )?;
        if version_satisfies_constraint(&version, constraint) {
            return Ok((path, version));
        }
        observed.push(format!("{}={version}", path.as_str()));
    }
    Err(provider_version_mismatch(
        "Python",
        constraint,
        &observed.join(", "),
    ))
}

fn find_gcc_toolchain(constraint: &str) -> Result<(Utf8PathBuf, Utf8PathBuf, String)> {
    let major = numeric_prefix(constraint, 1);
    let mut candidates = Vec::new();
    if let Some(major) = major {
        candidates.push((format!("gcc-{major}"), format!("g++-{major}")));
    }
    candidates.push(("gcc".into(), "g++".into()));
    candidates.dedup();
    let mut observed = Vec::new();
    for (cc_name, cxx_name) in candidates {
        let (Some(cc), Some(cxx)) = (tools::which(&cc_name), tools::which(&cxx_name)) else {
            continue;
        };
        let cc = utf8_tool_path(cc, &cc_name)?;
        let cxx = utf8_tool_path(cxx, &cxx_name)?;
        let cc_version = command_stdout(
            &cc,
            &["-dumpfullversion", "-dumpversion"],
            "probe GCC version",
        )?;
        let cxx_version = command_stdout(
            &cxx,
            &["-dumpfullversion", "-dumpversion"],
            "probe G++ version",
        )?;
        if cc_version == cxx_version && version_satisfies_constraint(&cc_version, constraint) {
            return Ok((cc, cxx, cc_version));
        }
        observed.push(format!("{cc_name}={cc_version}/{cxx_name}={cxx_version}"));
    }
    Err(provider_version_mismatch(
        "GCC",
        constraint,
        &observed.join(", "),
    ))
}

fn utf8_tool_path(path: std::path::PathBuf, label: &str) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(path).map_err(|_| {
        Error::coded(
            "INTERNAL_ERROR",
            ost_core::Category::Internal,
            format!("{label} path is not UTF-8"),
        )
    })
}

fn command_stdout(program: &Utf8Path, args: &[&str], label: &str) -> Result<String> {
    let output = Command::new(program.as_str())
        .args(args)
        .output()
        .map_err(|error| Error::io(label, error))?;
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || value.is_empty() {
        return Err(Error::coded(
            "OPENUSD_PROVIDER_PROBE_FAILED",
            ost_core::Category::Precondition,
            format!("{label} failed for '{program}'"),
        ));
    }
    Ok(value)
}

fn provider_version_mismatch(provider: &str, required: &str, observed: &str) -> Error {
    Error::coded(
        "OPENUSD_PROVIDER_VERSION_MISMATCH",
        ost_core::Category::Precondition,
        format!(
            "{provider} does not satisfy CY constraint '{required}'{}",
            if observed.is_empty() {
                " (no matching executable found)".to_string()
            } else {
                format!(" (observed {observed})")
            }
        ),
    )
    .with_hint(format!(
        "install {provider} {required} and make its executable visible on PATH"
    ))
}

fn ensure_version_matches(provider: &str, observed: &str, constraint: &str) -> Result<()> {
    if version_satisfies_constraint(observed, constraint) {
        return Ok(());
    }
    Err(provider_version_mismatch(provider, constraint, observed))
}

fn numeric_prefix(version: &str, count: usize) -> Option<String> {
    let parts: Vec<&str> = version
        .split('.')
        .take(count)
        .filter(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
        .collect();
    (parts.len() == count).then(|| parts.join("."))
}

fn tbb_version_from_roots(roots: &[Utf8PathBuf]) -> Option<String> {
    for root in roots {
        for relative in ["include/oneapi/tbb/version.h", "include/tbb/version.h"] {
            let path = root.join(relative);
            let Ok(source) = std::fs::read_to_string(path.as_std_path()) else {
                continue;
            };
            let (Some(major), Some(minor)) = (
                cpp_define(&source, "TBB_VERSION_MAJOR"),
                cpp_define(&source, "TBB_VERSION_MINOR"),
            ) else {
                continue;
            };
            let patch = cpp_define(&source, "TBB_VERSION_PATCH").unwrap_or("0");
            return Some(format!("{major}.{minor}.{patch}"));
        }
    }
    None
}

fn cpp_define<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    source.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        (parts.next() == Some("#define") && parts.next() == Some(name))
            .then(|| parts.next())
            .flatten()
    })
}

fn reset_managed_runtime_build(r: &crate::commands::Resolved) -> Result<()> {
    let store = Store::discover();
    let targets = [
        r.prefix.clone(),
        store.cache().join("usd-build").join(r.runtime.id()),
    ];
    for target in targets {
        reset_managed_path(&store.root, &target)?;
    }
    Ok(())
}

fn reset_managed_path(store_root: &Utf8Path, target: &Utf8Path) -> Result<()> {
    if !target.as_std_path().exists() {
        return Ok(());
    }
    // `Path::starts_with` is lexical: `store/cache/../../../outside` still has
    // `store` as a prefix. Resolve both existing paths before a recursive
    // delete so catalog-controlled runtime ids cannot escape the store through
    // parent components (or through a directory link).
    let canonical_root = std::fs::canonicalize(store_root.as_std_path())
        .map_err(|error| Error::io(store_root.to_string(), error))?;
    let canonical_target = std::fs::canonicalize(target.as_std_path())
        .map_err(|error| Error::io(target.to_string(), error))?;
    if canonical_target == canonical_root || !canonical_target.starts_with(&canonical_root) {
        return Err(Error::coded(
            "INTERNAL_ERROR",
            ost_core::Category::Internal,
            format!("refusing to reset managed build path outside '{store_root}': '{target}'"),
        ));
    }
    std::fs::remove_dir_all(target.as_std_path())
        .map_err(|error| Error::io(target.to_string(), error))?;
    Ok(())
}

fn print_openusd_compatibility(manifest: &RuntimeManifest, prefix: &str) {
    let Some(selected) = &manifest.openusd_compatibility else {
        return;
    };
    println!(
        "{prefix}OpenUSD: {} ({} {} / C++{}, {} {} via {}, {} {} via {})",
        selected.variant.as_str(),
        selected.toolchain.family,
        selected
            .toolchain
            .version
            .as_deref()
            .unwrap_or("<unverified>"),
        selected.toolchain.cxx_standard,
        selected.python.family,
        selected.python.version.as_deref().unwrap_or("<unverified>"),
        selected.python.provider,
        selected.tbb.family,
        selected.tbb.version.as_deref().unwrap_or("<unverified>"),
        selected.tbb.provider,
    );
    println!("{prefix}graphics: {}", selected.capabilities.join(", "));
}

fn print_build_identities(manifest: &RuntimeManifest, prefix: &str) {
    if let Some(source) = &manifest.build_source {
        println!(
            "{prefix}source:   {}@{}",
            source.repository, source.revision
        );
    }
    if !manifest.build_dependencies.is_empty() {
        println!("{prefix}build dependencies:");
        for dependency in &manifest.build_dependencies {
            let archive = dependency
                .archive_digest
                .as_deref()
                .map(|digest| format!(", archive {digest}"))
                .unwrap_or_default();
            println!(
                "{prefix}  - {} {} ({}@{}{archive})",
                dependency.name,
                dependency.version,
                dependency.source.repository,
                dependency.source.revision
            );
        }
    }
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

    // An adopted tree bundling `usdGenSchema` needs the same schema-gen Python
    // deps a `--build` pull provisions. Publishing a runtime built by driving
    // `build_usd.py` directly is a supported (and, until the component-flag fix
    // above, sometimes required) path, and it should not silently ship a
    // usdGenSchema that dies on `ModuleNotFoundError: jinja2` in the consumer's
    // schema-generate phase (reports 29 §5, 30 §4).
    //
    // This is the one write `--from-usd` makes into the tree it adopts, and that
    // tree is the caller's — possibly a shared or system install. Say so before
    // writing rather than reporting it afterwards. The provisioning itself is
    // idempotent: an install that already imports the deps is left untouched.
    if ost_build::bundles_usdgenschema(&root) {
        println!(
            "==> {root} bundles usdGenSchema; ensuring its schema-gen deps ({}) are present \
             in {} — the adopted tree is written to only if they are missing",
            ost_build::SCHEMA_GEN_PACKAGES.join(" "),
            ost_runtime::usd_python_dir_for(&root, Some(&r.python_version))
        );
    }
    provision_schema_gen_deps(&root, &r.python_version);

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

/// The USD-install subdirectories present under `root`.
///
/// The `pxr` Python package location is build-dependent — `lib/python` up to
/// OpenUSD 26.05, `lib/python<X.Y>/site-packages` or `Lib/site-packages` from
/// 26.08 — so the resolved directory is appended from
/// [`ost_runtime::usd_python_dir`] rather than enumerated here. Recording the
/// concrete layout is what lets a consumer tell which convention this artifact
/// shipped (report 29 §3).
fn probe_usd_layout(root: &Utf8Path) -> Vec<String> {
    let mut layout: Vec<String> = ["bin", "lib", "plugin/usd", "include"]
        .iter()
        .filter(|s| root.join(s).as_std_path().is_dir())
        .map(|s| s.to_string())
        .collect();
    let python_dir = ost_runtime::usd_python_dir(root);
    if python_dir.join("pxr").as_std_path().is_dir() {
        if let Ok(rel) = python_dir.strip_prefix(root) {
            layout.push(rel.to_string().replace('\\', "/"));
        }
    }
    layout
}

/// Whether `root` looks like an OpenUSD install (a strong marker is present).
fn looks_like_usd(root: &Utf8Path) -> bool {
    root.join("plugin/usd").as_std_path().is_dir()
        || ost_runtime::usd_python_dir(root)
            .join("pxr")
            .as_std_path()
            .is_dir()
}

/// The `build_usd.py` components OpenStrata turns off by default, to keep a
/// published runtime lean.
///
/// Each is one half of an `argparse` mutually exclusive group, so these are
/// **defaults, not constraints**: a caller who names either half through
/// `--build-arg` owns the decision and ost's half is dropped. Appending them
/// unconditionally produced `--no-examples --examples`, which `argparse` rejects
/// as a pair rather than letting the later flag win — and `--examples` is where
/// OpenUSD 26.08 ships its OpenExec/ExecIr reference material, so the forced-off
/// set was blocking the exact build three platforms needed (dogfooding reports
/// 29 §1, 30 §2).
const DEFAULT_COMPONENT_TRIMS: &[&str] = &["examples", "tutorials", "docs", "tests"];

/// The option name in `arg`, stripped of `--` and of any `=value` suffix
/// `argparse` also accepts. `None` for a positional or a short flag.
fn option_name(arg: &str) -> Option<&str> {
    arg.strip_prefix("--")
        .map(|rest| rest.split('=').next().unwrap_or(rest))
}

/// Whether `arg` names `component` in either half of a `--x` / `--no-x` pair.
fn names_component(arg: &str, component: &str) -> bool {
    option_name(arg).is_some_and(|n| n.strip_prefix("no-").unwrap_or(n) == component)
}

/// The `--x` / `--no-x` pairs where the argv names **both** halves.
///
/// `build_usd.py` puts its component toggles in `argparse` mutually exclusive
/// groups, which error on the pair. The check is structural rather than a
/// hardcoded catalog of build_usd.py flags, so it stays correct as upstream adds
/// components — and it lets ost refuse in one sentence instead of spending a
/// process spawn to surface a two-line usage dump (report 29 §1, secondary ask).
fn conflicting_component_flags(args: &[String]) -> Vec<String> {
    let mut conflicts: Vec<String> = Vec::new();
    for arg in args {
        let Some(component) = option_name(arg).and_then(|n| n.strip_prefix("no-")) else {
            continue;
        };
        if args.iter().any(|a| option_name(a) == Some(component))
            && !conflicts.iter().any(|c| c == component)
        {
            conflicts.push(component.to_string());
        }
    }
    conflicts
}

/// The arguments to pass to `python` to run build_usd.py: the script, the
/// component defaults the caller did not override, optional `-j`, any forwarded
/// args, then the install directory (build_usd.py's positional).
///
/// See [`DEFAULT_COMPONENT_TRIMS`] for why a forwarded `--examples` suppresses
/// ost's `--no-examples` instead of colliding with it.
fn build_usd_args(
    script: &Utf8Path,
    install_dir: &Utf8Path,
    jobs: Option<u32>,
    extra: &[String],
) -> Vec<String> {
    let mut args = vec![script.to_string()];
    for component in DEFAULT_COMPONENT_TRIMS {
        if extra.iter().any(|a| names_component(a, component)) {
            continue;
        }
        args.push(format!("--no-{component}"));
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
    /// Exact interpreter verified against the selected CY cell.
    pub python: Option<Utf8PathBuf>,
    /// Compatibility-critical compiler environment pinned by the CY cell.
    pub build_env: Vec<(String, String)>,
    /// macOS SDK and deployment floor for this build.
    pub macos: MacosBuildOpts,
    /// Dependency prefixes; non-empty selects the CMake-direct path.
    pub deps: Vec<String>,
}

/// The two macOS knobs a 26.08 build needs and could previously only reach by
/// smuggling them through `--build-arg --cmake-build-args=...`, where they
/// competed with the `CMAKE_POLICY_VERSION_MINIMUM` the same host also needs.
///
/// OpenUSD 26.08 cannot be compiled against the macOS 14.5 SDK at C++17 — libc++
/// only routes `allocate_shared` through the allocator under C++20 there, so
/// `HD_DECLARE_DATASOURCE`'s friendship never applies — and installing a newer
/// clang does not get you a newer SDK, because `xcrun` selects the SDK matching
/// the running OS. Two full builds failed at 73% before that was understood
/// (report 30 §2); one `xcrun --show-sdk-path` before the spawn turns it into a
/// sentence.
#[derive(Debug, Default, Clone)]
pub struct MacosBuildOpts {
    /// A full SDK path, or a version like `15.2` resolved through `xcrun`.
    pub sdk: Option<String>,
    /// `CMAKE_OSX_DEPLOYMENT_TARGET`, e.g. `14.5`.
    pub deployment_target: Option<String>,
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
    src: &Utf8Path,
    opts: &BuildOpts,
    extensions: Vec<ExtensionRecord>,
    created: u64,
) -> Result<RuntimeManifest> {
    emit_macos_build_notes(opts);
    // Resolve and validate source identity before a multi-hour build. A dirty
    // checkout cannot be represented by HEAD and must never produce an artifact
    // that claims the clean commit's identity.
    let build_source = git_source_identity(src)?;
    // Warn now on missing build-interpreter deps rather than letting build_usd.py
    // abort deep in its run (report §Dogfood): a clean Python 3.13 lacks Jinja2
    // (schema tooling) and PySide6/PyOpenGL (usdview) that the profile implies.
    preflight_build_deps(&r.capabilities, opts.python.as_deref());
    std::fs::create_dir_all(r.prefix.as_std_path())
        .map_err(|e| Error::io(r.prefix.to_string(), e))?;

    let build_dependencies = if opts.deps.is_empty() {
        build_with_script(r, src, opts)?
    } else {
        build_with_cmake(r, src, opts)?;
        Vec::new()
    };

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
    provision_schema_gen_deps(&r.prefix, &r.python_version);
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
    manifest.set_build_identities(build_source, build_dependencies)?;
    Ok(manifest)
}

/// Capture a source checkout when it is Git-backed. Source archives remain a
/// supported input and can still supply their exact identity at export through
/// `--build-metadata`; an ambient repository must never be substituted for it.
fn git_source_identity(src: &Utf8Path) -> Result<Option<ResolvedSourceIdentity>> {
    let Some(git) = tools::which("git") else {
        return Ok(None);
    };
    let inside = Command::new(&git)
        .args(["-C", src.as_str(), "rev-parse", "--is-inside-work-tree"])
        .output()
        .map_err(|error| Error::io("inspect OpenUSD source repository", error))?;
    if !inside.status.success() || String::from_utf8_lossy(&inside.stdout).trim() != "true" {
        return Ok(None);
    }
    let status = Command::new(&git)
        .args([
            "-C",
            src.as_str(),
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
        ])
        .output()
        .map_err(|error| Error::io("inspect OpenUSD source worktree", error))?;
    if !status.status.success() {
        return Err(Error::coded(
            "OPENUSD_SOURCE_IDENTITY_FAILED",
            ost_core::Category::ExternalTool,
            "git could not inspect the OpenUSD source worktree",
        ));
    }
    if !status.stdout.is_empty() {
        return Err(Error::coded(
            "OPENUSD_SOURCE_DIRTY",
            ost_core::Category::Precondition,
            "the OpenUSD source checkout has tracked or untracked changes, so HEAD is not its exact identity",
        )
        .with_hint("commit, stash, or remove the source-tree changes before building"));
    }
    let revision = Command::new(&git)
        .args(["-C", src.as_str(), "rev-parse", "HEAD"])
        .output()
        .map_err(|error| Error::io("read OpenUSD source revision", error))?;
    if !revision.status.success() {
        return Ok(None);
    }
    let repository = Command::new(&git)
        .args(["-C", src.as_str(), "remote", "get-url", "origin"])
        .output()
        .map_err(|error| Error::io("read OpenUSD source origin", error))?;
    if !repository.status.success() {
        return Ok(None);
    }
    let revision = String::from_utf8_lossy(&revision.stdout).trim().to_string();
    let repository = normalize_git_repository(String::from_utf8_lossy(&repository.stdout).trim());
    let identity = ResolvedSourceIdentity {
        repository,
        revision,
    };
    Ok(identity.is_verified().then_some(identity))
}

fn normalize_git_repository(value: &str) -> String {
    let value = value.trim();
    if let Some(rest) = value.strip_prefix("git@github.com:") {
        return sanitize_source_url(&format!("https://github.com/{rest}"));
    }
    if let Some(rest) = value.strip_prefix("ssh://git@github.com/") {
        return sanitize_source_url(&format!("https://github.com/{rest}"));
    }
    sanitize_source_url(value)
}

/// Keep stable source location components while preventing credentials or
/// expiring query parameters from entering runtime manifests and provenance.
fn sanitize_source_url(value: &str) -> String {
    let value = value
        .split(['?', '#'])
        .next()
        .unwrap_or(value)
        .trim_end_matches('/');
    let sanitized = if let Some((scheme, rest)) = value.split_once("://") {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let authority = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        if path.is_empty() {
            format!("{scheme}://{authority}")
        } else {
            format!("{scheme}://{authority}/{path}")
        }
    } else if let Some((_, remote)) = value.split_once('@') {
        if let Some((host, path)) = remote.split_once(':') {
            format!("https://{host}/{path}")
        } else {
            remote.to_string()
        }
    } else {
        value.to_string()
    };
    sanitized.trim_end_matches(".git").to_string()
}

fn validate_openusd_source(usd_src: &str) -> Result<Utf8PathBuf> {
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
    Ok(src)
}

/// Provision the schema-gen Python deps into a runtime that bundles
/// `usdGenSchema` (report Finding D). Resolves an interpreter to run `pip` and
/// installs into the exact Python directory `ost` puts on `PYTHONPATH`.
/// Best-effort: a failure warns with the one-line manual fix rather than
/// discarding an otherwise-good (and expensive) build.
///
/// `prefix` is the install being provisioned, which is the store prefix for a
/// `--build` pull and the external USD root for a `--from-usd` adopt. The adopt
/// path used to skip this entirely, so every runtime published by driving
/// `build_usd.py` directly still needed a manual `pip install jinja2` — three
/// times across three platforms (reports 29 §5, 30 §4).
fn provision_schema_gen_deps(prefix: &Utf8Path, python_version: &str) {
    if !ost_build::bundles_usdgenschema(prefix) {
        return;
    }
    // Version-aware: this is a `pip install --target`, and a 26.08 tree carrying
    // more than one `lib/python<X.Y>/site-packages` would otherwise be picked by
    // sort order — installing the deps into an ABI the runtime never loads.
    let python_lib_dir = ost_runtime::usd_python_dir_for(prefix, Some(python_version));
    let manual_fix = |argv: &str| {
        format!(
            "provision them manually with: {argv} -m pip install --target {python_lib_dir} {}",
            ost_build::SCHEMA_GEN_PACKAGES.join(" ")
        )
    };
    let Some(argv) = ost_build::resolve_run_python(prefix, python_version) else {
        eprintln!(
            "warning: this runtime bundles usdGenSchema but no Python interpreter was found to \
             provision its schema-gen deps ({}); {}",
            ost_build::SCHEMA_GEN_PACKAGES.join(" "),
            manual_fix("<python>")
        );
        return;
    };
    match ost_build::provision_schema_gen_deps(prefix, &python_lib_dir, &argv) {
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
    manifest.set_openusd_compatibility(previous.openusd_compatibility.clone())?;
    manifest.set_build_identities(
        previous.build_source.clone(),
        previous.build_dependencies.clone(),
    )?;
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
    validate_embedded_host_requirement_targets(&manifest.host_requirements, manifest.variant.os)?;

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

    ensure_host_requirements(&record, &manifest.host_requirements)?;

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

/// Validate the host contract embedded in a stored runtime artifact. This is
/// shared by low-level `artifact pull` and `runtime pull --from-artifact`, so a
/// downloaded artifact and a locally handed-off artifact fail with the same
/// pre-launch diagnostic.
pub(crate) fn check_artifact_host_requirements(
    store: &ArtifactStore,
    record: &ost_artifact::ArtifactRecord,
) -> Result<Vec<HostRequirement>> {
    let requirements = artifact_host_requirements(store, record)?;
    ensure_host_requirements(record, &requirements)?;
    Ok(requirements)
}

pub(crate) fn artifact_host_requirements(
    store: &ArtifactStore,
    record: &ost_artifact::ArtifactRecord,
) -> Result<Vec<HostRequirement>> {
    if record.kind != ArtifactKind::Runtime {
        return Ok(Vec::new());
    }
    let producer = store.producer_manifest(record)?;
    let embedded = producer
        .get("provenance")
        .and_then(|value| value.get("runtime_manifest"))
        .ok_or_else(|| {
            Error::InvalidManifest(
                "runtime artifact carries no provenance.runtime_manifest".to_string(),
            )
        })?;
    let manifest: RuntimeManifest = serde_json::from_value(embedded.clone())
        .map_err(|error| Error::parse("runtime_manifest in artifact", anyhow::Error::new(error)))?;
    validate_embedded_host_requirement_targets(&manifest.host_requirements, manifest.variant.os)?;
    Ok(manifest.host_requirements)
}

fn ensure_host_requirements(
    record: &ost_artifact::ArtifactRecord,
    requirements: &[HostRequirement],
) -> Result<()> {
    let missing: Vec<&HostRequirement> = requirements
        .iter()
        .filter(|requirement| !host_requirement_is_satisfied(requirement))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }

    let selected = format!("{}:{}", record.name, record.short_digest());
    let missing_names = missing
        .iter()
        .map(|requirement| format!("{}:{}", requirement.manager.as_str(), requirement.name))
        .collect::<Vec<_>>()
        .join(", ");
    let apt = missing
        .iter()
        .filter(|requirement| requirement.manager == HostPackageManager::Apt)
        .map(|requirement| requirement.name.as_str())
        .collect::<Vec<_>>();
    let brew = missing
        .iter()
        .filter(|requirement| requirement.manager == HostPackageManager::Brew)
        .map(|requirement| requirement.name.as_str())
        .collect::<Vec<_>>();
    let mut actions = Vec::new();
    if !apt.is_empty() {
        actions.push(format!(
            "install with `sudo apt-get update && sudo apt-get install --no-install-recommends {}`",
            apt.join(" ")
        ));
    }
    if !brew.is_empty() {
        actions.push(format!("install with `brew install {}`", brew.join(" ")));
    }
    Err(Error::coded(
        "ARTIFACT_HOST_REQUIREMENT_MISSING",
        ost_core::Category::Precondition,
        format!(
            "selected runtime artifact '{selected}' requires missing host package(s): {missing_names}"
        ),
    )
    .with_hint(actions.join("; ")))
}

fn host_requirement_is_satisfied(requirement: &HostRequirement) -> bool {
    match requirement.manager {
        HostPackageManager::Apt => Command::new("dpkg-query")
            .args([
                "--show",
                "--showformat=${db:Status-Abbrev}",
                &requirement.name,
            ])
            .output()
            .is_ok_and(|output| {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).starts_with("ii ")
            }),
        HostPackageManager::Brew => Command::new("brew")
            .args(["list", "--versions", &requirement.name])
            .output()
            .is_ok_and(|output| output.status.success() && !output.stdout.is_empty()),
    }
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

/// Build the same validation report `ost runtime validate` would show for this
/// materialized runtime, including CLI-owned drift checks layered over the core
/// runtime checks.
fn current_validation_report(
    manifest: &RuntimeManifest,
    artifact_prefix: &Utf8Path,
    platform: &str,
    profile: &str,
) -> ost_runtime::ValidationReport {
    let mut report = ost_runtime::validate(artifact_prefix, manifest);
    if let Some((recorded, real)) = openusd_version_drift(manifest, artifact_prefix) {
        let fix = drift_repair_command(manifest, platform, profile);
        report.checks.push(ost_runtime::Check {
            name: "openusd-version-drift",
            passed: false,
            skipped: false,
            detail: Some(format!(
                "manifest records OpenUSD {recorded}, but the install's pxr.h reports {real}; \
                 fix with `{fix}`"
            )),
        });
    }
    if let Some(check) = consumer_configure_check(artifact_prefix, manifest) {
        report.checks.push(check);
    }
    report
}

/// Configure a trivial `find_package(pxr)` consumer against the materialized
/// runtime, in a scratch directory.
///
/// OpenStrata measured what a runtime *is* far more carefully than what a
/// runtime *needs*: a published 26.08 Linux runtime passed `runtime validate`
/// 7/7 on a machine where consuming it was impossible, because its bundled
/// MaterialX exports an unconditional `find_dependency(X11 REQUIRED)` and
/// `pxrConfig.cmake` chains into it — four CI lanes went red in under twenty
/// seconds (report 31 §4). Every consumer starts with this configure, so this is
/// the cheapest possible reproduction of the thing that actually broke, run on
/// the producer's machine before the digest ever reaches a matrix.
///
/// It never fails for its own missing preconditions: no CMake, or a runtime
/// carrying no `pxrConfig.cmake` to find, is a skip that names what was missing.
fn consumer_configure_check(
    prefix: &Utf8Path,
    manifest: &RuntimeManifest,
) -> Option<ost_runtime::Check> {
    const NAME: &str = "consumer-configure";
    if !manifest.source.is_real() {
        return None;
    }
    // Nothing to find: a profile that ships no CMake package is not a consumer
    // configure failure, and reporting one would be noise on every core runtime.
    let has_config = ["lib/cmake/pxr/pxrConfig.cmake", "pxrConfig.cmake"]
        .iter()
        .any(|relative| prefix.join(relative).as_std_path().is_file());
    if !has_config {
        return Some(ost_runtime::Check::skip(
            NAME,
            format!("no pxrConfig.cmake under {prefix}"),
        ));
    }
    let Some(cmake) = ost_core::tools::which("cmake") else {
        return Some(ost_runtime::Check::skip(
            NAME,
            "cmake is not on PATH, so a consumer configure cannot be attempted",
        ));
    };

    let scratch = match scratch_dir("consumer-configure") {
        Ok(scratch) => scratch,
        Err(error) => return Some(ost_runtime::Check::skip(NAME, error.to_string())),
    };

    // The same three Development variables `plugin build`'s toolchain pins. An
    // adopted runtime bakes the export machine's interpreter paths into
    // `pxrConfig.cmake`, so without these the check would report a Python that
    // exists on no consumer's machine instead of the runtime's real state.
    let mut args = vec![
        format!("-Dpxr_ROOT={prefix}"),
        // The consumer contract `ost plugin build` pins for the caller. Without
        // it an adopted runtime's baked Python paths decide the outcome, and the
        // check would report the host's interpreter rather than the runtime.
        "-DCMAKE_POLICY_VERSION_MINIMUM=3.5".to_string(),
    ];
    if let Some(python) = ost_build::resolve_for_runtime(prefix, &manifest.python) {
        for name in ["Python3", "Python"] {
            args.push(format!("-D{name}_EXECUTABLE={}", python.executable));
            args.push(format!("-D{name}_LIBRARY={}", python.library));
            args.push(format!("-D{name}_INCLUDE_DIR={}", python.include_dir));
        }
    }

    match scratch_configure(
        &cmake,
        &scratch.path,
        "consumer",
        "find_package(pxr REQUIRED)\n",
        &args,
    ) {
        ConfigureOutcome::Passed => Some(ost_runtime::Check {
            name: NAME,
            passed: true,
            skipped: false,
            detail: Some(format!("find_package(pxr) configures against {prefix}")),
        }),
        ConfigureOutcome::NoAnswer(detail) => Some(ost_runtime::Check::skip(NAME, detail)),
        ConfigureOutcome::Failed(tail) => {
            // A failed configure is only evidence about the *runtime* if this
            // host can configure at all. `project(LANGUAGES CXX)` compiles a
            // probe, so no compiler — or one that cannot link — fails the same
            // check for a reason the artifact has nothing to do with. Ask the
            // toolchain the question on its own before blaming the runtime;
            // paid only on the failure path, so a passing host runs one
            // configure as before.
            match scratch_configure(&cmake, &scratch.path, "toolchain", "", &[]) {
                ConfigureOutcome::Passed => Some(ost_runtime::Check {
                    name: NAME,
                    passed: false,
                    skipped: false,
                    detail: Some(format!("find_package(pxr) failed against {prefix}: {tail}")),
                }),
                ConfigureOutcome::Failed(probe) | ConfigureOutcome::NoAnswer(probe) => {
                    Some(ost_runtime::Check::skip(
                        NAME,
                        format!(
                            "this host cannot configure a trivial CXX project, so a consumer \
                             configure proves nothing about the runtime: {probe}"
                        ),
                    ))
                }
            }
        }
    }
}

/// How long one scratch configure may run before it is treated as no answer.
///
/// A compiler-ABI try-compile against a broken toolchain is one of the ways a
/// configure hangs rather than fails, and a validation check that never returns
/// is worse than one that reports it could not tell.
const CONSUMER_CONFIGURE_TIMEOUT: Duration = Duration::from_secs(180);

/// What one scratch configure concluded.
enum ConfigureOutcome {
    Passed,
    /// Configure ran and failed; carries the tail of its output.
    Failed(String),
    /// Configure could not be run to a conclusion here (no spawn, timeout, I/O).
    NoAnswer(String),
}

/// Configure a one-file CMake project under `scratch/<name>`, capturing output
/// to a file so a large failure cannot deadlock on a full pipe, and bounding it
/// with [`CONSUMER_CONFIGURE_TIMEOUT`].
fn scratch_configure(
    cmake: &std::path::Path,
    scratch: &Utf8Path,
    name: &str,
    body: &str,
    args: &[String],
) -> ConfigureOutcome {
    let root = scratch.join(name);
    let log_path = scratch.join(format!("{name}.log"));
    let project = format!(
        "cmake_minimum_required(VERSION 3.21)\n\
         project(ost_{name}_configure LANGUAGES CXX)\n{body}"
    );
    if let Err(error) = std::fs::create_dir_all(root.as_std_path())
        .and_then(|()| std::fs::write(root.join("CMakeLists.txt").as_std_path(), project))
    {
        return ConfigureOutcome::NoAnswer(format!("{root}: {error}"));
    }
    let log = match std::fs::File::create(log_path.as_std_path()) {
        Ok(log) => log,
        Err(error) => return ConfigureOutcome::NoAnswer(format!("{log_path}: {error}")),
    };
    let stderr_log = match log.try_clone() {
        Ok(clone) => clone,
        Err(error) => return ConfigureOutcome::NoAnswer(format!("{log_path}: {error}")),
    };

    let mut child = match Command::new(cmake)
        .arg("-S")
        .arg(root.as_std_path())
        .arg("-B")
        .arg(root.join("build").as_std_path())
        .args(args)
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(stderr_log))
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return ConfigureOutcome::NoAnswer(format!(
                "could not run {}: {error}",
                cmake.display()
            ))
        }
    };

    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if started.elapsed() >= CONSUMER_CONFIGURE_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return ConfigureOutcome::NoAnswer(format!(
                        "configure did not finish within {}s: {}",
                        CONSUMER_CONFIGURE_TIMEOUT.as_secs(),
                        configure_failure_tail(&read_lossy(&log_path))
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                return ConfigureOutcome::NoAnswer(format!("wait {}: {error}", cmake.display()))
            }
        }
    };
    if status.success() {
        ConfigureOutcome::Passed
    } else {
        ConfigureOutcome::Failed(configure_failure_tail(&read_lossy(&log_path)))
    }
}

fn read_lossy(path: &Utf8Path) -> String {
    std::fs::read(path.as_std_path())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

/// The last few meaningful lines of a failed configure, for a one-line detail.
fn configure_failure_tail(output: &str) -> String {
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(6);
    if lines.is_empty() {
        return "<no output>".into();
    }
    lines[start..].join(" / ")
}

/// A scratch directory removed when the returned guard drops.
struct ScratchDir {
    path: Utf8PathBuf,
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path.as_std_path());
    }
}

fn scratch_dir(label: &str) -> Result<ScratchDir> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let base = Utf8PathBuf::from_path_buf(std::env::temp_dir())
        .map_err(|path| Error::config(format!("temp dir is not UTF-8: {}", path.display())))?;
    let path = base.join(format!("ost-{label}-{}-{nonce}", std::process::id()));
    // `create_dir`, not `create_dir_all`: the temp directory is world-writable on
    // Unix, and this must fail rather than adopt a path someone else placed there
    // — including a symlink pointing somewhere we would then write into and
    // recursively remove on drop.
    std::fs::create_dir(path.as_std_path()).map_err(|e| Error::io(path.to_string(), e))?;
    Ok(ScratchDir { path })
}

fn failed_validation_summary(report: &ost_runtime::ValidationReport) -> String {
    report
        .checks
        .iter()
        .filter(|c| !c.passed)
        .map(|c| match &c.detail {
            Some(detail) => format!("{} ({detail})", c.name),
            None => c.name.to_string(),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Export trusts the persisted `validation: passed` status only as evidence
/// that the user validated deliberately; before packing, re-run the current
/// checks so an older manifest cannot bypass checks added after it was stamped.
fn check_current_export_validation(
    manifest: &RuntimeManifest,
    artifact_prefix: &Utf8Path,
    platform: &str,
    profile: &str,
) -> Result<()> {
    let report = current_validation_report(manifest, artifact_prefix, platform, profile);
    if report.passed() {
        return Ok(());
    }
    Err(Error::coded(
        "EXPORT_VALIDATION_REQUIRED",
        ost_core::Category::Validation,
        format!(
            "runtime '{}' no longer passes validation: {}",
            manifest.id,
            failed_validation_summary(&report)
        ),
    )
    .with_hint(format!(
        "run `ost runtime validate {platform} --profile {profile}` and repair the reported checks"
    )))
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
    // A symlink records its in-tree target so a verifier can distinguish it from a
    // regular file whose bytes happen to equal that target.
    let files: Vec<_> = packed.files.iter().map(|f| f.manifest_json()).collect();
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
        // The producing tool names itself here so the registry can
        // record the artifact's origin instead of whoever imported it.
        "producer": format!("ost {}", env!("CARGO_PKG_VERSION")),
        "host_requirements": manifest.host_requirements,
        "openusd_compatibility": manifest.openusd_compatibility,
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

/// The variant a Linux/glibc runtime should be labeled with, given the glibc
/// floor measured from its ELF binaries. Returns `None` when there is nothing to
/// correct — a non-glibc variant (Windows/macOS), or a runtime whose floor could
/// not be measured (no ELF glibc references at all). The measured floor is
/// authoritative even when it is *lower* than the recorded nominal: the artifact
/// label must describe the symbol versions the binaries actually impose, so that
/// `--require-target` cannot pass on a runtime the host cannot load (ask #7).
fn variant_for_measured_floor(variant: &Variant, floor: Option<GlibcVersion>) -> Option<Variant> {
    match (&variant.abi, floor) {
        (Abi::Glibc { .. }, Some(floor)) => {
            let mut corrected = variant.clone();
            corrected.abi = Abi::Glibc {
                version: floor.token(),
            };
            Some(corrected)
        }
        _ => None,
    }
}

/// The variant a measured macOS deployment floor implies.
///
/// `Abi::Native` on macOS asserts nothing, so a measured floor replaces it: a
/// runtime whose binaries require macOS 14.5 is labeled `macos145` and a
/// consumer's `--require-target` can finally mean something. A re-measured
/// runtime that already carries a floor is relabeled the same way, so a rebuild
/// that raises the floor cannot keep an old, lower label.
fn variant_for_macos_floor(variant: &Variant, floor: ost_build::MacosFloor) -> Option<Variant> {
    let target = floor.deployment_target?;
    if variant.os != ost_core::host::Os::Macos {
        return None;
    }
    let mut corrected = variant.clone();
    corrected.abi = Abi::Macos {
        version: target.token(),
    };
    (corrected.abi != variant.abi).then_some(corrected)
}

/// Compression knobs for `ost runtime export` (`--level` / `--jobs`).
struct ExportPack {
    level: i32,
    jobs: Option<u32>,
}

/// The zstd worker count to pack with: a reproducible build uses the stable
/// single-threaded encoder (`0`); an ordinary export uses the host's available
/// parallelism. Multithreading remains the fast default because a full adopted
/// runtime is ~14 GB and packs for tens of minutes single-threaded (report #10).
fn default_pack_workers(reproducible: bool) -> u32 {
    if reproducible {
        return 0;
    }
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
    build_metadata: Option<&Utf8Path>,
    fmt: Format,
) -> Result<()> {
    // Read and validate before packing: a malformed metadata file should fail
    // in the first second, not after compressing a multi-gigabyte runtime.
    let explicit_build_metadata = build_metadata.is_some();
    let mut build_metadata = build_metadata
        .map(|path| {
            let source = std::fs::read_to_string(path.as_std_path())
                .map_err(|error| Error::io(path.to_string(), error))?;
            ost_artifact::parse_build_metadata(&source)
        })
        .transpose()?;
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
    build_metadata = artifact_build_metadata(&manifest, build_metadata, explicit_build_metadata)?;

    check_exportable(&manifest)?;

    // Pack the runtime's real artifacts: the effective prefix (external root
    // for an adopted runtime), minus the store's own runtime.json — the
    // manifest travels in the producer manifest instead, so the archive is a
    // pure USD tree.
    let effective = Utf8PathBuf::from(manifest.effective_prefix(&r.prefix));
    check_current_export_validation(&manifest, &effective, &platform, &profile)?;

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
    // A slim export drops whole top-level trees. If the SDK's own CMake package
    // config points into one of them, `find_package` succeeds against the
    // shipped artifact and then resolves a path that is not there — a failure
    // that surfaces at the consumer's configure step, far from this command.
    let dropped_references = if slim {
        referenced_excluded_dirs(&files, &excluded_dirs)
    } else {
        Vec::new()
    };
    for (directory, config) in &dropped_references {
        eprintln!(
            "warning: --slim drops '{directory}/', but the exported CMake config '{config}' \
             refers to it; consumers resolving that path will not find it in this artifact"
        );
    }

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

    // zstd's multithreaded frame bytes depend on the worker count. A
    // SOURCE_DATE_EPOCH build must therefore avoid the host-dependent
    // available_parallelism default as well as pinning tar mtimes.
    let reproducible = ost_build::source_date_epoch_opt().is_some();
    let workers = pack
        .jobs
        .unwrap_or_else(|| default_pack_workers(reproducible));
    let opts = ost_build::PackOptions {
        level: pack.level,
        workers,
        mtime: ost_build::source_date_epoch(),
        // Host requirements live outside the runtime tree but are compatibility
        // identity. Salt the tar stream with the canonical runtime digest so a
        // metadata-only contract change receives a new immutable artifact pin.
        identity_digest: Some(manifest.digest.clone()),
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

    // Pin to SOURCE_DATE_EPOCH when set so the manifest reproduces alongside the
    // archive; otherwise stamp wall-clock provenance.
    let created = ost_build::source_date_epoch_opt().unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    });
    let mut producer = runtime_artifact_manifest(&manifest, &archive_name, &packed, created);
    producer[ost_artifact::OPENUSD_SELECTOR_SCHEMA_FIELD] =
        serde_json::json!(ost_artifact::OPENUSD_SELECTOR_SCHEMA);

    // Measure the real glibc floor from the packed ELF binaries and label the
    // artifact with it, overriding a fabricated/defaulted `glibc228` nominal. The
    // `target` a support line pins with `--require-target` must describe the
    // symbol versions the binaries actually impose, or a runtime built on a newer
    // glibc silently "passes" its ABI check and then fails to load on the runner
    // (v0.11.0 report, ask #7). The embedded build provenance is left faithful:
    // `glibc_floor` records both the measured floor and the recorded nominal so
    // the drift is visible.
    let glibc_floor = if matches!(manifest.variant.abi, Abi::Glibc { .. }) {
        ost_build::max_glibc_floor(files.iter().map(Utf8PathBuf::as_path))
            .map_err(|e| Error::io(effective.to_string(), e))?
    } else {
        None
    };
    let measured_target = variant_for_measured_floor(&manifest.variant, glibc_floor)
        .map(|corrected| corrected.slug());
    if let (Some(floor), Some(target)) = (glibc_floor, &measured_target) {
        if let Some(obj) = producer.as_object_mut() {
            obj.insert("target".into(), serde_json::json!(target));
            obj.insert(
                "glibc_floor".into(),
                serde_json::json!({
                    "measured": floor.to_string(),
                    "recorded": manifest.variant.abi.describe(),
                    "source": "elf-symbol-versions",
                }),
            );
        }
        if !fmt.is_json() && target != &manifest.variant.slug() {
            println!(
                "Measured glibc floor {floor} (recorded {}); labeling the artifact target {target}",
                manifest.variant.abi.describe(),
            );
        }
    }

    // macOS gets the same treatment one platform over: Linux measures its glibc
    // floor into the target string and Windows carries `msvc143`, while macOS
    // carried `"abi": "native"` — which asserts nothing, so `--require-target
    // macos-arm64-py313` passed for an artifact the host could not load (report
    // 30 §1). The deployment target and SDK are read out of the binaries' own
    // load commands, so the measurement is the artifact's, not the builder's
    // claim about it.
    let macos_floor = if manifest.variant.os == ost_core::host::Os::Macos {
        ost_build::max_macos_floor(files.iter().map(Utf8PathBuf::as_path))
            .map_err(|e| Error::io(effective.to_string(), e))?
    } else {
        ost_build::MacosFloor::default()
    };
    if !macos_floor.is_empty() {
        let corrected = variant_for_macos_floor(&manifest.variant, macos_floor);
        if let Some(obj) = producer.as_object_mut() {
            if let Some(corrected) = &corrected {
                obj.insert("target".into(), serde_json::json!(corrected.slug()));
            }
            obj.insert(
                "macos_floor".into(),
                serde_json::json!({
                    "deployment_target": macos_floor.deployment_target.map(|v| v.to_string()),
                    "sdk": macos_floor.sdk.map(|v| v.to_string()),
                    "recorded": manifest.variant.abi.describe(),
                    "source": "mach-o-load-commands",
                }),
            );
        }
        if !fmt.is_json() {
            let sdk = macos_floor
                .sdk
                .map(|sdk| format!(", built against the {sdk} SDK"))
                .unwrap_or_default();
            match &corrected {
                Some(corrected) => println!(
                    "Measured macOS deployment target {}{sdk}; labeling the artifact target {}",
                    macos_floor
                        .deployment_target
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "unknown".into()),
                    corrected.slug()
                ),
                None => println!(
                    "Measured macOS deployment target {}{sdk}",
                    macos_floor
                        .deployment_target
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "unknown".into())
                ),
            }
        }
    }

    // Record which layout was shipped so a fetch/inspection can tell a slim SDK
    // artifact from a full one (they are distinct digests of the same runtime).
    if let Some(obj) = producer.as_object_mut() {
        obj.insert(
            "layout_profile".into(),
            serde_json::json!(if slim { "sdk" } else { "full" }),
        );
    }
    // Explicit metadata is set before evidence is generated, which is also what
    // makes it win: `generate_evidence` only falls back to the ambient GitHub
    // Actions environment when the manifest carries no `build` of its own.
    if let Some(build) = build_metadata {
        producer["build"] = build;
    }
    let evidence = ost_artifact::generate_evidence(&dist_dir, &mut producer)?;
    let producer_json = serde_json::to_string_pretty(&producer)
        .map_err(|e| Error::parse("runtime artifact manifest", anyhow::Error::new(e)))?;
    let producer_path = dist_dir.join("manifest.json");
    std::fs::write(producer_path.as_std_path(), format!("{producer_json}\n"))
        .map_err(|e| Error::io(producer_path.to_string(), e))?;
    let bare = packed
        .archive_digest
        .strip_prefix("sha256:")
        .unwrap_or(&packed.archive_digest);
    let mut checksum_lines = vec![format!("{bare}  {archive_name}")];
    checksum_lines.extend(evidence.iter().map(|layer| {
        format!(
            "{}  {}",
            layer
                .digest
                .strip_prefix("sha256:")
                .unwrap_or(&layer.digest),
            layer.path
        )
    }));
    let sums = dist_dir.join("SHA256SUMS");
    std::fs::write(
        sums.as_std_path(),
        format!("{}\n", checksum_lines.join("\n")),
    )
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
            "target": out.record.target,
            "glibc_floor": glibc_floor.map(|f| f.to_string()),
            "archive_size": out.record.archive_size,
            "files": out.record.file_count,
            "dist": dist.map(|d| d.to_string()),
            "layout_profile": if slim { "sdk" } else { "full" },
            "host_requirements": manifest.host_requirements,
            "excluded_top_level": excluded_dirs,
            // Excluded trees the shipped CMake config still points at.
            "dropped_referenced_layout": dropped_references
                .iter()
                .map(|(directory, config)| serde_json::json!({
                    "directory": directory,
                    "referenced_by": config,
                }))
                .collect::<Vec<_>>(),
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

/// Combine builder identity with the exact identities observed at managed
/// runtime build time. Explicit source/dependency claims are checked instead of
/// silently overwritten; ambient GitHub metadata contributes the workflow but
/// its repository is replaced by the OpenUSD checkout that produced the bytes.
fn artifact_build_metadata(
    manifest: &RuntimeManifest,
    supplied: Option<serde_json::Value>,
    supplied_explicitly: bool,
) -> Result<Option<serde_json::Value>> {
    if manifest.build_source.is_none() && manifest.build_dependencies.is_empty() {
        return Ok(supplied);
    }
    if manifest.build_source.is_none()
        && !manifest.build_dependencies.is_empty()
        && !supplied_explicitly
    {
        return Err(Error::coded(
            "BUILD_METADATA_SOURCE_REQUIRED",
            ost_core::Category::Precondition,
            "a managed runtime built from a non-Git source needs explicit source identity before export",
        )
        .with_hint(
            "pass --build-metadata with the source repository/revision and builder identity; omit dependencies to reuse the captured closure",
        ));
    }

    let mut build = supplied
        .or_else(ost_artifact::github_build_metadata)
        .or_else(|| {
            manifest.build_source.as_ref().map(|source| {
                serde_json::json!({
                    "source": source,
                    "builder": {
                        "id": "https://openstrata.dev/builders/ost/runtime-pull",
                        "identity": {
                            "tool": "ost",
                            "version": env!("CARGO_PKG_VERSION"),
                            "runtime": manifest.id,
                            "runtime_digest": manifest.digest,
                        }
                    }
                })
            })
        });
    let Some(build) = build.as_mut() else {
        return Ok(None);
    };

    if let Some(source) = &manifest.build_source {
        if supplied_explicitly {
            let declared: ResolvedSourceIdentity = serde_json::from_value(build["source"].clone())
                .map_err(|error| {
                    Error::coded(
                        "BUILD_METADATA_SOURCE_MISMATCH",
                        ost_core::Category::Validation,
                        format!("build metadata source cannot be compared with the managed source: {error}"),
                    )
                })?;
            if &declared != source {
                return Err(Error::coded(
                    "BUILD_METADATA_SOURCE_MISMATCH",
                    ost_core::Category::Validation,
                    format!(
                        "build metadata source {}@{} differs from managed source {}@{}",
                        declared.repository, declared.revision, source.repository, source.revision
                    ),
                )
                .with_hint(
                    "export with metadata for the exact checkout used by `runtime pull --build`",
                ));
            }
        }
        build["source"] = serde_json::to_value(source)
            .map_err(|error| Error::parse("managed build source", anyhow::Error::new(error)))?;
    }

    if !manifest.build_dependencies.is_empty() {
        if let Some(declared) = build.get("dependencies") {
            let mut declared: Vec<ResolvedDependencyIdentity> =
                serde_json::from_value(declared.clone()).map_err(|error| {
                    Error::coded(
                        "BUILD_METADATA_DEPENDENCY_MISMATCH",
                        ost_core::Category::Validation,
                        format!("build metadata dependencies cannot be compared with the managed closure: {error}"),
                    )
                })?;
            declared.sort_by(|left, right| left.name.cmp(&right.name));
            if declared != manifest.build_dependencies {
                return Err(Error::coded(
                    "BUILD_METADATA_DEPENDENCY_MISMATCH",
                    ost_core::Category::Validation,
                    "build metadata dependencies differ from the closure captured from build_usd.py",
                )
                .with_hint("omit metadata dependencies to use the automatically captured closure"));
            }
        }
        build["dependencies"] =
            serde_json::to_value(&manifest.build_dependencies).map_err(|error| {
                Error::parse("managed build dependencies", anyhow::Error::new(error))
            })?;
    }
    Ok(Some(build.clone()))
}

/// Excluded top-level directories that the exported CMake package configs still
/// refer to, as `(directory, config file name)`.
///
/// Only the `.cmake` files actually being shipped are read: those are what a
/// consumer's `find_package` will evaluate, and a reference from a file that is
/// not in the artifact cannot break anyone.
///
/// The match is on a path *segment* (`/build/`), not a bare substring, so a
/// config mentioning `rebuild_flags` is not read as pointing at `build/`.
fn referenced_excluded_dirs(files: &[Utf8PathBuf], excluded: &[String]) -> Vec<(String, String)> {
    if excluded.is_empty() {
        return Vec::new();
    }
    let mut found: BTreeSet<(String, String)> = BTreeSet::new();
    for file in files {
        if !file
            .extension()
            .unwrap_or_default()
            .eq_ignore_ascii_case("cmake")
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(file.as_std_path()) else {
            continue;
        };
        let text = text.replace('\\', "/");
        let name = file
            .file_name()
            .map(str::to_string)
            .unwrap_or_else(|| file.to_string());
        for directory in excluded {
            // A trailing delimiter keeps `/build/` from matching `/buildinfo/`.
            let segment = format!("/{directory}/");
            if text.contains(&segment) {
                found.insert((directory.clone(), name.clone()));
            }
        }
    }
    found.into_iter().collect()
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
fn preflight_build_deps(capabilities: &[String], selected_python: Option<&Utf8Path>) {
    let needed = build_dep_requirements(capabilities);
    if needed.is_empty() {
        return;
    }
    let python = selected_python
        .map(|path| path.as_std_path().to_path_buf())
        .or_else(|| tools::which("python"))
        .or_else(|| tools::which("python3"));
    let Some(python) = python else { return };
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
    for line in missing_dep_warning(&python.to_string_lossy(), &needed, &missing) {
        eprintln!("{line}");
    }
}

/// Format the missing-build-dep warning (the `import_name → pip_name` mapping is
/// what turns a bare import miss into an actionable `pip install`). Pure so the
/// warning — and its fix line — is testable without a clean interpreter (report
/// ask #6): returns the lines to print, or empty when nothing is missing.
fn missing_dep_warning(
    python: &str,
    needed: &[(&'static str, &'static str)],
    missing: &[String],
) -> Vec<String> {
    if missing.is_empty() {
        return Vec::new();
    }
    let pip: Vec<&str> = needed
        .iter()
        .filter(|(i, _)| missing.iter().any(|m| m == i))
        .map(|(_, p)| *p)
        .collect();
    vec![
        format!(
            "warning: build_usd.py needs Python packages not importable by {python}: {}",
            missing.join(", ")
        ),
        format!(
            "  install them first: {python} -m pip install {}",
            pip.join(" ")
        ),
        "  (schema generation needs Jinja2; usdview needs PySide6 + PyOpenGL)".to_string(),
    ]
}

/// Resolve `--sdk` / `--deployment-target` into the environment every build
/// tool in the tree reads.
///
/// `SDKROOT` and `MACOSX_DEPLOYMENT_TARGET` are what initialize
/// `CMAKE_OSX_SYSROOT` and `CMAKE_OSX_DEPLOYMENT_TARGET`, and — unlike a `-D`
/// on one configure — they reach every dependency `build_usd.py` builds on the
/// way to OpenUSD, which is exactly the scope the problem has.
///
/// A version is resolved with `xcrun` *before* the spawn, so an SDK that is not
/// installed is one sentence now rather than a build that fails at 73%.
fn macos_build_env(opts: &MacosBuildOpts) -> Result<Vec<(String, String)>> {
    let mut env = Vec::new();
    if opts.sdk.is_none() && opts.deployment_target.is_none() {
        return Ok(env);
    }
    if Host::detect().os != Os::Macos {
        return Err(Error::usage(
            "--sdk and --deployment-target are macOS build knobs and this host is not macOS",
        )
        .with_hint("drop the flag, or run the build on a macOS host"));
    }
    if let Some(sdk) = &opts.sdk {
        let path = if Utf8PathBuf::from(sdk).as_std_path().is_dir() {
            sdk.clone()
        } else {
            resolve_macos_sdk(sdk)?
        };
        env.push(("SDKROOT".to_string(), path));
    }
    if let Some(target) = &opts.deployment_target {
        env.push(("MACOSX_DEPLOYMENT_TARGET".to_string(), target.clone()));
    }
    Ok(env)
}

/// The explicit `-D` form of the macOS build environment, for the CMake-direct
/// path. The environment initializes these variables anyway; naming them on the
/// command line puts them in the printed configure line and in `CMakeCache.txt`,
/// where the build that produced a runtime can be read back.
fn macos_cmake_args(env: &[(String, String)]) -> Vec<String> {
    env.iter()
        .filter_map(|(key, value)| match key.as_str() {
            "SDKROOT" => Some(format!("-DCMAKE_OSX_SYSROOT={value}")),
            "MACOSX_DEPLOYMENT_TARGET" => Some(format!("-DCMAKE_OSX_DEPLOYMENT_TARGET={value}")),
            _ => None,
        })
        .collect()
}

/// Ask `xcrun` for the path of a named macOS SDK version.
fn resolve_macos_sdk(version: &str) -> Result<String> {
    let sdk = format!("macosx{version}");
    let output = Command::new("xcrun")
        .args(["--sdk", &sdk, "--show-sdk-path"])
        .output()
        .map_err(|error| Error::io("run xcrun", error))?;
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || path.is_empty() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Error::coded(
            "REQUIRED_TOOL_MISSING",
            ost_core::Category::Precondition,
            format!(
                "no macOS SDK '{sdk}' on this host{}",
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            ),
        )
        .with_hint(
            "install the SDK (a newer Xcode), or pass --sdk with the full path to one; \
             `xcrun --show-sdk-path` prints the default, which matches the running OS",
        ));
    }
    Ok(path)
}

fn emit_macos_build_notes(opts: &BuildOpts) {
    if Host::detect().os != Os::Macos {
        return;
    }
    if opts.macos.sdk.is_none() {
        eprintln!(
            "note: OpenUSD 26.08 needs the macOS 15.2 SDK at C++17 (libc++ routes \
             allocate_shared through the allocator only under C++20 on 14.5); select one \
             with `--sdk 15.2` rather than relying on the SDK matching the running OS"
        );
    }
    if opts.macos.deployment_target.is_none() {
        eprintln!(
            "note: no --deployment-target: the produced runtime's macOS floor will be \
             whatever the toolchain defaults to, and is measured into its target string at \
             `runtime export`"
        );
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

#[derive(Debug, serde::Deserialize)]
struct BuildUsdDownload {
    dependency: String,
    url: String,
    archive_sha256: String,
}

// Execute the upstream script from its original path and replace only its
// DownloadURL binding after dependency selection, immediately before installs
// begin. Installer functions resolve that global at call time, so this records
// the URL that actually won platform/option/fallback selection without editing
// the checkout or maintaining a second dependency catalog in Rust.
const BUILD_USD_CAPTURE_RUNNER: &str = r#"
import hashlib as _ost_hashlib
import json as _ost_json
import os as _ost_os
import sys as _ost_sys

_ost_script = _ost_sys.argv[1]
_ost_sys.argv = _ost_sys.argv[1:]
_ost_script_dir = _ost_os.path.dirname(_ost_os.path.abspath(_ost_script))
if _ost_script_dir not in _ost_sys.path:
    _ost_sys.path.insert(0, _ost_script_dir)
with open(_ost_script, "r", encoding="utf-8") as _ost_file:
    _ost_source = _ost_file.read()
_ost_marker = "try:\n    # Download and install 3rd-Party dependencies"
_ost_hook = r'''
import hashlib as _ost_hashlib
import json as _ost_json
import os as _ost_os
_ost_original_download_url = DownloadURL
def DownloadURL(*args, **kwargs):
    result = _ost_original_download_url(*args, **kwargs)
    url = args[0]
    context = args[1]
    dep = globals().get("dep")
    name = getattr(dep, "name", "")
    if name and name != "USD":
        destination = kwargs.get("destFileName")
        if destination is None and len(args) > 5:
            destination = args[5]
        if destination is None:
            destination = url.split("/")[-1]
        archive = _ost_os.path.join(context.srcDir, destination)
        digest = _ost_hashlib.sha256()
        with open(archive, "rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(block)
        with open(_ost_os.environ["OST_BUILD_USD_DEPENDENCY_CAPTURE"], "a", encoding="utf-8") as capture:
            capture.write(_ost_json.dumps({
                "dependency": name,
                "url": url,
                "archive_sha256": digest.hexdigest(),
            }, sort_keys=True) + "\n")
    return result
'''
if _ost_source.count(_ost_marker) != 1:
    raise RuntimeError("build_usd.py install marker changed; dependency capture cannot run safely")
_ost_source = _ost_source.replace(_ost_marker, _ost_hook + "\n" + _ost_marker, 1)
exec(compile(_ost_source, _ost_script, "exec"), {"__file__": _ost_script, "__name__": "__main__"})
"#;

const BUILD_USD_INSTALL_MARKER: &str = "try:\n    # Download and install 3rd-Party dependencies";

/// Drive the source tree's `build_scripts/build_usd.py` and return the exact
/// source archives its selected dependency installers consumed.
fn build_with_script(
    r: &crate::commands::Resolved,
    src: &Utf8Path,
    opts: &BuildOpts,
) -> Result<Vec<ResolvedDependencyIdentity>> {
    let script = src.join("build_scripts").join("build_usd.py");
    if !script.as_std_path().is_file() {
        return Err(Error::usage(format!(
            "no build_scripts/build_usd.py under '{src}' (point --build at an OpenUSD checkout, \
             or pass --deps for a direct CMake build)"
        )));
    }
    let script_source = std::fs::read_to_string(script.as_std_path())
        .map_err(|error| Error::io(script.to_string(), error))?;
    if script_source.matches(BUILD_USD_INSTALL_MARKER).count() != 1 {
        return Err(Error::coded(
            "OPENUSD_DEPENDENCY_CAPTURE_UNSUPPORTED",
            ost_core::Category::Configuration,
            "build_usd.py install structure changed; exact dependency capture cannot be attached safely",
        )
        .with_hint(
            "use a supported OpenUSD release or update OST's build_usd.py capture adapter",
        ));
    }
    let python = opts
        .python
        .clone()
        .or_else(|| {
            tools::which("python")
                .or_else(|| tools::which("python3"))
                .and_then(|path| Utf8PathBuf::from_path_buf(path).ok())
        })
        .ok_or_else(|| {
            Error::coded(
                "REQUIRED_TOOL_MISSING",
                ost_core::Category::Precondition,
                "`python` not found — build_usd.py needs it",
            )
        })?;

    let script_args = build_usd_args(&script, &r.prefix, opts.jobs, &opts.extra);
    // build_usd.py's component toggles are argparse mutually exclusive groups:
    // naming both halves is a hard error there. ost knows the exact argv it is
    // about to run, so refuse now rather than pay a process spawn to surface a
    // two-line usage dump (report 29 §1).
    let conflicts = conflicting_component_flags(&script_args);
    if !conflicts.is_empty() {
        let pairs = conflicts
            .iter()
            .map(|c| format!("`--{c}` and `--no-{c}`"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(Error::usage(format!(
            "build_usd.py refuses both halves of a component toggle: {pairs}"
        ))
        .with_hint("pass only the half you want through `--build-arg`"));
    }
    println!(
        "==> building OpenUSD (build_usd.py) into {} (one-time, heavy)",
        r.prefix
    );
    println!("    {python} {}", script_args.join(" "));
    // Preserve partial capture across a failed build. build_usd.py skips deps it
    // already installed on retry, so discarding this ledger would make the next
    // successful manifest record only the remaining suffix of the closure. The
    // key isolates retries by exact script contents and argv.
    let capture_key = ost_core::digest::sha256_hex(
        serde_json::to_string(&(&script_source, &script_args))
            .expect("build_usd capture identity serializes")
            .as_bytes(),
    );
    let capture_path = r.prefix.join(format!(
        ".ost-build-usd-dependencies-{}.jsonl",
        capture_key.trim_start_matches("sha256:")
    ));
    let mut args = vec!["-c".to_string(), BUILD_USD_CAPTURE_RUNNER.to_string()];
    args.extend(script_args);
    let mut env = msvc_env();
    env.extend(opts.build_env.iter().cloned());
    env.extend(macos_build_env(&opts.macos)?);
    env.push((
        "OST_BUILD_USD_DEPENDENCY_CAPTURE".to_string(),
        capture_path.to_string(),
    ));
    let build_result = run_build_step(python.as_str(), &args, &env, "build_usd.py");
    let capture_result = read_build_usd_dependencies(&capture_path);
    build_result?;
    let dependencies = capture_result?;
    if let Err(error) = std::fs::remove_file(capture_path.as_std_path()) {
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(Error::io(capture_path.to_string(), error));
        }
    }
    Ok(dependencies)
}

fn read_build_usd_dependencies(path: &Utf8Path) -> Result<Vec<ResolvedDependencyIdentity>> {
    let source = match std::fs::read_to_string(path.as_std_path()) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(Error::io(path.to_string(), error)),
    };
    let mut dependencies = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let download: BuildUsdDownload = serde_json::from_str(line).map_err(|error| {
            Error::parse(
                format!("{} line {}", path, index + 1),
                anyhow::Error::new(error),
            )
        })?;
        dependencies.push(dependency_identity_from_download(download)?);
    }
    dependencies.sort();
    for pair in dependencies.windows(2) {
        if pair[0].name == pair[1].name && pair[0] != pair[1] {
            return Err(Error::coded(
                "OPENUSD_DEPENDENCY_CAPTURE_INVALID",
                ost_core::Category::Validation,
                "build_usd.py downloaded conflicting source archives for the same dependency",
            ));
        }
    }
    dependencies.dedup();
    Ok(dependencies)
}

fn dependency_identity_from_download(
    download: BuildUsdDownload,
) -> Result<ResolvedDependencyIdentity> {
    let sanitized_url = sanitize_source_url(&download.url);
    let (repository, revision) = source_identity_from_download_url(
        &download.dependency,
        &sanitized_url,
        &download.archive_sha256,
    );
    let version = dependency_version(&revision, &sanitized_url, &download.archive_sha256);
    let identity = ResolvedDependencyIdentity {
        name: download.dependency,
        version,
        source: ResolvedSourceIdentity {
            repository,
            revision,
        },
        archive_digest: Some(format!("sha256:{}", download.archive_sha256)),
    };
    if !identity.is_verified() {
        return Err(Error::coded(
            "OPENUSD_DEPENDENCY_CAPTURE_INVALID",
            ost_core::Category::Validation,
            format!(
                "build_usd.py dependency '{}' has an incomplete or invalid source identity",
                identity.name
            ),
        ));
    }
    Ok(identity)
}

fn source_identity_from_download_url(
    dependency: &str,
    url: &str,
    archive_sha256: &str,
) -> (String, String) {
    let url = sanitize_source_url(url);
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        let mut parts = rest.split('/');
        if let (Some(owner), Some(repository)) = (parts.next(), parts.next()) {
            let repository = format!("https://github.com/{owner}/{repository}");
            let tail = parts.collect::<Vec<_>>();
            if let Some(index) = tail.iter().position(|part| *part == "archive") {
                let revision = tail[index + 1..]
                    .join("/")
                    .trim_end_matches(".zip")
                    .trim_start_matches("refs/tags/")
                    .to_string();
                if !revision.is_empty() {
                    return (repository, revision);
                }
            }
            if tail.first() == Some(&"releases") && tail.get(1) == Some(&"download") {
                if let Some(revision) = tail.get(2) {
                    return (repository, (*revision).to_string());
                }
            }
        }
    }
    if let Some(rest) = url.strip_prefix("https://gitlab.com/") {
        let parts = rest.split('/').collect::<Vec<_>>();
        if parts.len() >= 6 && parts[2] == "-" && parts[3] == "archive" {
            return (
                format!("https://gitlab.com/{}/{}", parts[0], parts[1]),
                parts[4].to_string(),
            );
        }
    }
    if dependency.eq_ignore_ascii_case("boost") {
        let version = url
            .split('/')
            .find(|part| {
                part.chars()
                    .next()
                    .is_some_and(|value| value.is_ascii_digit())
                    && part.contains('.')
            })
            .unwrap_or(archive_sha256);
        return (
            "https://github.com/boostorg/boost".to_string(),
            format!("boost-{version}"),
        );
    }
    (url, format!("sha256:{archive_sha256}"))
}

fn dependency_version(revision: &str, url: &str, archive_sha256: &str) -> String {
    let candidate = revision
        .strip_prefix("refs/tags/")
        .unwrap_or(revision)
        .strip_prefix("boost-")
        .unwrap_or(revision)
        .trim_start_matches('v');
    if candidate.chars().any(|value| value.is_ascii_digit())
        && candidate.chars().any(|value| value == '.' || value == '_')
    {
        return candidate.replace('_', ".");
    }
    let filename = url.rsplit('/').next().unwrap_or_default();
    let filename = filename.trim_end_matches(".zip");
    let version = filename.split(['-', '_']).find(|part| {
        part.chars()
            .next()
            .is_some_and(|value| value.is_ascii_digit())
    });
    version
        .filter(|value| !value.is_empty())
        .unwrap_or(archive_sha256)
        .to_string()
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
    let python = opts
        .python
        .clone()
        .or_else(|| {
            tools::which("python")
                .or_else(|| tools::which("python3"))
                .and_then(|path| Utf8PathBuf::from_path_buf(path).ok())
        })
        .ok_or_else(|| {
            Error::coded(
                "REQUIRED_TOOL_MISSING",
                ost_core::Category::Precondition,
                "`python` not found — USD needs it for bindings",
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

    let mut env = msvc_env();
    env.extend(opts.build_env.iter().cloned());
    let macos = macos_build_env(&opts.macos)?;
    env.extend(macos.iter().cloned());
    let configure = cmake_configure_args(
        src,
        &build_dir,
        &r.prefix,
        &opts.deps,
        &python,
        ninja.as_deref(),
        &macos_cmake_args(&macos)
            .into_iter()
            .chain(opts.extra.iter().cloned())
            .collect::<Vec<_>>(),
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
                    "openusd_compatibility": m.openusd_compatibility,
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
    print_host_requirements(&manifest.host_requirements, "Host needs: ");
    print_openusd_compatibility(&manifest, "");
    print_build_identities(&manifest, "");
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
    let report = current_validation_report(&manifest, &r.artifact_prefix, &platform, &profile);
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
                    "skipped": c.skipped,
                    "status": c.status(),
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
            let mark = match (c.passed, c.skipped) {
                (_, true) => "skip",
                (true, false) => "ok  ",
                (false, false) => "FAIL",
            };
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
    let (mut repaired, readopted_from) = match (manifest.source, &manifest.external_prefix) {
        (RuntimeSource::Local, Some(root)) => {
            let root = root.clone();
            (adopt_local(&r, &root, extensions, created)?, Some(root))
        }
        _ => (redetect_build(&r, extensions, &manifest, created)?, None),
    };
    repaired.set_host_requirements(manifest.host_requirements.clone());
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

    /// A unique scratch directory for tests that need real files on disk.
    fn temp_dir(tag: &str) -> Utf8PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("ost-rt-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Utf8PathBuf::from_path_buf(dir).unwrap()
    }

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

    fn verified_compatibility(variant: OpenUsdVariantId) -> ResolvedOpenUsdCompatibility {
        let platform = ost_platform::load_one("cy2026").unwrap();
        let (mut compatibility, _) = platform
            .resolve_openusd(Os::Linux, Arch::X86_64, variant)
            .unwrap();
        compatibility.toolchain.version = Some("14.2.0".into());
        compatibility.toolchain.runtime.version = Some("2.28".into());
        compatibility.python.version = Some("3.13.7".into());
        compatibility.tbb.version = Some("2022.1.0".into());
        compatibility
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
    fn host_package_declarations_are_bounded_and_targeted() {
        let apt = parse_host_requirement("apt:libx11-dev").unwrap();
        assert_eq!(apt.manager, HostPackageManager::Apt);
        assert_eq!(apt.name, "libx11-dev");
        assert!(parse_host_requirement("apt:--option").is_err());
        assert!(parse_host_requirement("apt:libx11-dev;id").is_err());
        assert!(parse_host_requirement("unknown:thing").is_err());

        assert!(validate_host_requirement_targets(std::slice::from_ref(&apt), Os::Linux).is_ok());
        assert!(validate_host_requirement_targets(&[apt], Os::Macos).is_err());
        let brew = parse_host_requirement("brew:openimageio").unwrap();
        assert!(validate_host_requirement_targets(&[brew], Os::Macos).is_ok());
        assert!(parse_host_requirement("brew:python@3.13").is_ok());
    }

    #[test]
    fn declared_openusd_variants_select_deterministic_builder_args() {
        let platform = ost_platform::load_one("cy2026").unwrap();
        let headless = resolve_openusd_build(
            &platform,
            Os::Linux,
            Arch::X86_64,
            Some(OpenUsdVariantId::Headless),
            OpenUsdBuilder::BuildUsd,
            vec!["--no-tests".into()],
        )
        .unwrap();
        assert!(headless.args.iter().any(|arg| arg == "--no-imaging"));
        assert!(headless.args.iter().any(|arg| arg == "--no-tests"));
        let compatibility = headless.compatibility.unwrap();
        assert_eq!(compatibility.variant, OpenUsdVariantId::Headless);
        assert_eq!(compatibility.python.provider, "platform");
        assert_eq!(compatibility.tbb.provider, "platform");

        let vulkan = resolve_openusd_build(
            &platform,
            Os::Linux,
            Arch::X86_64,
            Some(OpenUsdVariantId::Vulkan),
            OpenUsdBuilder::Cmake,
            Vec::new(),
        )
        .unwrap();
        assert!(vulkan
            .args
            .iter()
            .any(|arg| arg == "-DPXR_ENABLE_VULKAN_SUPPORT=ON"));
    }

    #[test]
    fn variant_supports_self_contained_vulkan_and_refuses_compatibility_overrides() {
        let platform = ost_platform::load_one("cy2026").unwrap();
        let vulkan = resolve_openusd_build(
            &platform,
            Os::Linux,
            Arch::X86_64,
            Some(OpenUsdVariantId::Vulkan),
            OpenUsdBuilder::BuildUsd,
            Vec::new(),
        )
        .unwrap();
        assert!(vulkan.args.iter().any(|arg| {
            arg.starts_with("--build-args=USD,") && arg.contains("-DPXR_ENABLE_VULKAN_SUPPORT=ON")
        }));

        for (builder, arg) in [
            (OpenUsdBuilder::Cmake, "-DPXR_BUILD_IMAGING=OFF"),
            (
                OpenUsdBuilder::Cmake,
                "-DCMAKE_PREFIX_PATH=/unverified/deps",
            ),
            (OpenUsdBuilder::Cmake, "-DTBB_DIR=/unverified/tbb"),
            (
                OpenUsdBuilder::BuildUsd,
                "--build-args=USD,-DPXR_BUILD_IMAGING=OFF",
            ),
            (
                OpenUsdBuilder::BuildUsd,
                "--cmake-build-args=USD,-DPXR_BUILD_IMAGING=OFF",
            ),
        ] {
            let overridden = resolve_openusd_build(
                &platform,
                Os::Linux,
                Arch::X86_64,
                Some(OpenUsdVariantId::Standard),
                builder,
                vec![arg.into()],
            )
            .unwrap_err();
            assert_eq!(overridden.code(), "OPENUSD_VARIANT_OVERRIDE", "{arg}");
        }
    }

    #[test]
    fn implicit_variant_preserves_legacy_targets_without_approved_cell() {
        let platform = ost_platform::load_one("cy2026").unwrap();
        let legacy = resolve_openusd_build(
            &platform,
            Os::Windows,
            Arch::X86_64,
            None,
            OpenUsdBuilder::BuildUsd,
            vec!["--no-tests".into()],
        )
        .unwrap();
        assert!(legacy.compatibility.is_none());
        assert_eq!(legacy.args, ["--no-tests"]);
    }

    #[test]
    fn runtime_artifact_manifest_exposes_host_requirements() {
        let mut manifest = exportable_manifest();
        manifest.set_host_requirements(vec![HostRequirement {
            manager: HostPackageManager::Apt,
            name: "libx11-dev".into(),
        }]);
        let packed = ost_build::PackResult {
            archive_digest: format!("sha256:{}", "ab".repeat(32)),
            archive_size: 42,
            total_size: 21,
            files: Vec::new(),
        };
        let producer =
            runtime_artifact_manifest(&manifest, "runtime.tar.zst", &packed, 1_760_000_000);
        assert_eq!(producer["host_requirements"][0]["manager"], "apt");
        assert_eq!(
            producer["provenance"]["runtime_manifest"]["host_requirements"][0]["name"],
            "libx11-dev"
        );
    }

    #[test]
    fn runtime_artifact_manifest_exposes_openusd_compatibility() {
        let mut manifest = exportable_manifest();
        manifest
            .set_openusd_compatibility(Some(verified_compatibility(OpenUsdVariantId::Vulkan)))
            .unwrap();
        let packed = ost_build::PackResult {
            archive_digest: format!("sha256:{}", "ab".repeat(32)),
            archive_size: 42,
            total_size: 21,
            files: Vec::new(),
        };
        let producer =
            runtime_artifact_manifest(&manifest, "runtime.tar.zst", &packed, 1_760_000_000);
        assert_eq!(producer["openusd_compatibility"]["variant"], "vulkan");
        assert_eq!(
            producer["openusd_compatibility"]["python"]["version"],
            "3.13.7"
        );
    }

    #[test]
    fn provider_constraints_match_exact_and_wildcard_versions() {
        assert!(version_satisfies_constraint("14.2.0", "14.2"));
        assert!(version_satisfies_constraint("3.13.7", "3.13.x"));
        assert!(version_satisfies_constraint("2022.1.0", "2022.x"));
        assert!(version_satisfies_constraint("2.39", ">=2.28"));
        assert!(!version_satisfies_constraint("3.12.9", "3.13.x"));
        assert!(!version_satisfies_constraint("15.0", "14.2"));
    }

    #[test]
    fn tbb_version_probe_skips_forwarding_headers_without_version_macros() {
        let first = temp_dir("tbb-forwarder");
        let second = temp_dir("tbb-version");
        std::fs::create_dir_all(first.join("include/tbb").as_std_path()).unwrap();
        std::fs::write(
            first.join("include/tbb/version.h").as_std_path(),
            "#include <oneapi/tbb/version.h>\n",
        )
        .unwrap();
        std::fs::create_dir_all(second.join("include/oneapi/tbb").as_std_path()).unwrap();
        std::fs::write(
            second.join("include/oneapi/tbb/version.h").as_std_path(),
            "#define TBB_VERSION_MAJOR 2022\n#define TBB_VERSION_MINOR 1\n#define TBB_VERSION_PATCH 0\n",
        )
        .unwrap();

        assert_eq!(
            tbb_version_from_roots(&[first.clone(), second.clone()]).as_deref(),
            Some("2022.1.0")
        );
        std::fs::remove_dir_all(first.as_std_path()).unwrap();
        std::fs::remove_dir_all(second.as_std_path()).unwrap();
    }

    #[test]
    fn managed_reset_is_bounded_to_the_store() {
        let scratch = temp_dir("managed-reset");
        let store = scratch.join("store");
        let managed = store.join("runtimes/cy2026-usd");
        let outside = scratch.join("outside");
        let traversal_parent = store.join("cache/inner");
        std::fs::create_dir_all(managed.as_std_path()).unwrap();
        std::fs::create_dir_all(outside.as_std_path()).unwrap();
        std::fs::create_dir_all(traversal_parent.as_std_path()).unwrap();

        reset_managed_path(&store, &managed).unwrap();
        assert!(!managed.as_std_path().exists());
        let error = reset_managed_path(&store, &outside).unwrap_err();
        assert_eq!(error.code(), "INTERNAL_ERROR");
        assert!(outside.as_std_path().exists());

        let traversing = traversal_parent.join("../../../outside");
        let error = reset_managed_path(&store, &traversing).unwrap_err();
        assert_eq!(error.code(), "INTERNAL_ERROR");
        assert!(outside.as_std_path().exists());

        std::fs::remove_dir_all(scratch.as_std_path()).unwrap();
    }

    #[test]
    fn measured_glibc_floor_relabels_a_linux_runtime() {
        let m = exportable_manifest();
        // The recorded nominal is the defaulted glibc228.
        assert_eq!(m.variant.slug(), "linux-x86_64-glibc228-py313");

        // A higher measured floor (built on a newer host) overrides the nominal.
        let corrected = variant_for_measured_floor(
            &m.variant,
            Some(GlibcVersion {
                major: 2,
                minor: 43,
            }),
        )
        .expect("a glibc variant with a measured floor is relabeled");
        assert_eq!(corrected.slug(), "linux-x86_64-glibc243-py313");

        // A *lower* real floor is also authoritative — the label must describe
        // the binaries, not flatter them.
        let lower = variant_for_measured_floor(
            &m.variant,
            Some(GlibcVersion {
                major: 2,
                minor: 17,
            }),
        )
        .unwrap();
        assert_eq!(lower.slug(), "linux-x86_64-glibc217-py313");

        // No measurement (no ELF glibc references) leaves the nominal untouched.
        assert!(variant_for_measured_floor(&m.variant, None).is_none());
    }

    #[test]
    fn measured_glibc_floor_ignores_non_glibc_variants() {
        // A Windows/MSVC runtime carries no glibc ABI to correct, even if a stray
        // floor were somehow measured.
        let host = ost_core::Host {
            os: Os::Windows,
            arch: Arch::X86_64,
        };
        let rt = Runtime::resolve("cy2026", "usd", &host, "3.13.x");
        let win = RuntimeManifest::build(
            &rt,
            "3.13.x",
            vec![],
            vec![],
            vec![],
            1_750_000_000,
            RuntimeSource::Build,
        );
        assert!(matches!(win.variant.abi, Abi::Msvc { .. }));
        assert!(variant_for_measured_floor(
            &win.variant,
            Some(GlibcVersion {
                major: 2,
                minor: 43
            })
        )
        .is_none());
    }

    /// Report 30 §1: macOS recorded `"abi": "native"`, which asserts nothing,
    /// so `--require-target macos-arm64-py313` passed for a runtime the host
    /// could not load. The measured deployment floor goes into the target the
    /// way Linux's glibc floor does.
    #[test]
    fn a_measured_macos_floor_replaces_the_empty_native_abi() {
        let host = ost_core::Host {
            os: Os::Macos,
            arch: Arch::Arm64,
        };
        let rt = Runtime::resolve("cy2026", "usd", &host, "3.13.x");
        let mac = RuntimeManifest::build(
            &rt,
            "3.13.x",
            vec![],
            vec![],
            vec![],
            1_750_000_000,
            RuntimeSource::Build,
        );
        assert_eq!(mac.variant.slug(), "macos-arm64-py313");

        let floor = ost_build::MacosFloor {
            deployment_target: Some(ost_build::MacosVersion {
                major: 14,
                minor: 5,
            }),
            sdk: Some(ost_build::MacosVersion {
                major: 15,
                minor: 2,
            }),
        };
        let corrected = variant_for_macos_floor(&mac.variant, floor)
            .expect("a measured deployment target relabels the variant");
        assert_eq!(corrected.slug(), "macos-arm64-macos145-py313");

        // Re-measuring an already-labeled runtime at the same floor is a no-op,
        // and a floor that moved relabels rather than keeping the old claim.
        assert!(variant_for_macos_floor(&corrected, floor).is_none());
        let raised = variant_for_macos_floor(
            &corrected,
            ost_build::MacosFloor {
                deployment_target: Some(ost_build::MacosVersion {
                    major: 15,
                    minor: 0,
                }),
                sdk: None,
            },
        )
        .unwrap();
        assert_eq!(raised.slug(), "macos-arm64-macos150-py313");

        // No measurement, and a non-macOS runtime, both leave the label alone.
        assert!(variant_for_macos_floor(&mac.variant, ost_build::MacosFloor::default()).is_none());
        let linux = exportable_manifest();
        assert!(variant_for_macos_floor(&linux.variant, floor).is_none());
    }

    /// The macOS build knobs reach every dependency `build_usd.py` builds,
    /// because they travel as the environment CMake initializes from.
    #[test]
    fn the_macos_build_knobs_render_as_environment_and_cache_entries() {
        let env = vec![
            ("SDKROOT".to_string(), "/SDKs/MacOSX15.2.sdk".to_string()),
            ("MACOSX_DEPLOYMENT_TARGET".to_string(), "14.5".to_string()),
            ("VSCMD_VER".to_string(), "17.0".to_string()),
        ];
        assert_eq!(
            macos_cmake_args(&env),
            vec![
                "-DCMAKE_OSX_SYSROOT=/SDKs/MacOSX15.2.sdk".to_string(),
                "-DCMAKE_OSX_DEPLOYMENT_TARGET=14.5".to_string(),
            ]
        );

        // Nothing requested is nothing applied, on every host.
        assert!(macos_build_env(&MacosBuildOpts::default())
            .unwrap()
            .is_empty());
    }

    /// A macOS-only knob on another host is refused rather than silently
    /// dropped: a build that ignored `--sdk` would produce the artifact the
    /// caller was trying to avoid.
    #[test]
    #[cfg(not(target_os = "macos"))]
    fn the_macos_build_knobs_are_refused_off_macos() {
        let err = macos_build_env(&MacosBuildOpts {
            sdk: Some("15.2".into()),
            deployment_target: None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("not macOS"), "{err}");
    }

    #[test]
    fn a_configure_failure_tail_keeps_the_last_meaningful_lines() {
        let output = "-- Detecting CXX compiler\n\n\
                      CMake Error at pxrConfig.cmake:1 (find_dependency):\n\
                      \x20 Could NOT find X11 (missing: X11_Xt_LIB)\n";
        let tail = configure_failure_tail(output);
        assert!(tail.contains("Could NOT find X11"), "{tail}");
        assert!(!tail.contains("\n"), "the tail is one line: {tail}");
        assert_eq!(configure_failure_tail("   \n\n"), "<no output>");
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
    fn build_usd_download_urls_become_exact_dependency_identities() {
        let cases = [
            (
                BuildUsdDownload {
                    dependency: "oneTBB".into(),
                    url: "https://github.com/oneapi-src/oneTBB/archive/refs/tags/v2021.12.0.zip"
                        .into(),
                    archive_sha256: "aa".repeat(32),
                },
                "https://github.com/oneapi-src/oneTBB",
                "v2021.12.0",
                "2021.12.0",
            ),
            (
                BuildUsdDownload {
                    dependency: "TIFF".into(),
                    url: "https://gitlab.com/libtiff/libtiff/-/archive/v4.0.7/libtiff-v4.0.7.zip"
                        .into(),
                    archive_sha256: "bb".repeat(32),
                },
                "https://gitlab.com/libtiff/libtiff",
                "v4.0.7",
                "4.0.7",
            ),
            (
                BuildUsdDownload {
                    dependency: "boost".into(),
                    url: "https://archives.boost.io/release/1.86.0/source/boost_1_86_0.zip".into(),
                    archive_sha256: "cc".repeat(32),
                },
                "https://github.com/boostorg/boost",
                "boost-1.86.0",
                "1.86.0",
            ),
        ];
        for (download, repository, revision, version) in cases {
            let archive_digest = format!("sha256:{}", download.archive_sha256);
            let identity = dependency_identity_from_download(download).unwrap();
            assert_eq!(identity.source.repository, repository);
            assert_eq!(identity.source.revision, revision);
            assert_eq!(identity.version, version);
            assert_eq!(
                identity.archive_digest.as_deref(),
                Some(archive_digest.as_str())
            );
        }
    }

    #[test]
    fn captured_build_usd_dependencies_are_sorted_and_duplicate_safe() {
        let scratch = temp_dir("build-usd-capture");
        let capture = scratch.join("dependencies.jsonl");
        std::fs::write(
            capture.as_std_path(),
            concat!(
                "{\"dependency\":\"oneTBB\",\"url\":\"https://github.com/oneapi-src/oneTBB/archive/refs/tags/v2021.12.0.zip\",\"archive_sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}\n",
                "{\"dependency\":\"MaterialX\",\"url\":\"https://github.com/AcademySoftwareFoundation/MaterialX/archive/v1.39.4.zip\",\"archive_sha256\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}\n"
            ),
        )
        .unwrap();
        let dependencies = read_build_usd_dependencies(&capture).unwrap();
        assert_eq!(dependencies[0].name, "MaterialX");
        assert_eq!(dependencies[0].version, "1.39.4");
        assert_eq!(dependencies[1].name, "oneTBB");

        // A retry may download the same dependency again. Identical ledger
        // entries merge; only a conflicting source/digest is ambiguous.
        let identical = std::fs::read_to_string(capture.as_std_path()).unwrap();
        std::fs::write(capture.as_std_path(), format!("{identical}{identical}")).unwrap();
        assert_eq!(read_build_usd_dependencies(&capture).unwrap().len(), 2);

        std::fs::write(
            capture.as_std_path(),
            concat!(
                "{\"dependency\":\"oneTBB\",\"url\":\"https://github.com/oneapi-src/oneTBB/archive/v1.zip\",\"archive_sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}\n",
                "{\"dependency\":\"oneTBB\",\"url\":\"https://github.com/oneapi-src/oneTBB/archive/v2.zip\",\"archive_sha256\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}\n"
            ),
        )
        .unwrap();
        assert_eq!(
            read_build_usd_dependencies(&capture).unwrap_err().code(),
            "OPENUSD_DEPENDENCY_CAPTURE_INVALID"
        );
        std::fs::remove_dir_all(scratch.as_std_path()).unwrap();
    }

    #[test]
    fn capture_runner_observes_the_download_selected_by_the_upstream_script() {
        let Some(python) = tools::which("python").or_else(|| tools::which("python3")) else {
            return;
        };
        let scratch = temp_dir("build-usd-runner");
        let script = scratch.join("build_usd.py");
        let capture = scratch.join("dependencies.jsonl");
        std::fs::write(
            scratch.join("apple_utils.py").as_std_path(),
            "SIBLING_IMPORT_WORKED = True\n",
        )
        .unwrap();
        std::fs::write(
            script.as_std_path(),
            r#"import os
import apple_utils

assert apple_utils.SIBLING_IMPORT_WORKED

def DownloadURL(url, context, force, extractDir=None, dontExtract=None, destFileName=None, expectedSHA256=None):
    filename = destFileName or url.split("/")[-1]
    with open(os.path.join(context.srcDir, filename), "wb") as archive:
        archive.write(b"selected dependency bytes")
    return context.srcDir

class Context:
    srcDir = os.path.dirname(__file__)

class Dependency:
    name = "oneTBB"

context = Context()
dep = Dependency()
try:
    # Download and install 3rd-Party dependencies
    DownloadURL("https://github.com/oneapi-src/oneTBB/archive/refs/tags/v2021.12.0.zip", context, False)
except Exception:
    raise
"#,
        )
        .unwrap();
        let status = Command::new(python)
            .args(["-c", BUILD_USD_CAPTURE_RUNNER, script.as_str()])
            .env("OST_BUILD_USD_DEPENDENCY_CAPTURE", capture.as_str())
            .status()
            .unwrap();
        assert!(status.success());
        let dependencies = read_build_usd_dependencies(&capture).unwrap();
        assert_eq!(dependencies.len(), 1);
        assert_eq!(dependencies[0].name, "oneTBB");
        assert_eq!(dependencies[0].version, "2021.12.0");
        assert!(dependencies[0].archive_digest.is_some());
        assert_eq!(
            dependencies[0].source.repository,
            "https://github.com/oneapi-src/oneTBB"
        );
        std::fs::remove_dir_all(scratch.as_std_path()).unwrap();
    }

    #[test]
    fn managed_build_identities_enrich_and_check_export_metadata() {
        let mut manifest = exportable_manifest();
        manifest
            .set_build_identities(
                Some(ResolvedSourceIdentity {
                    repository: "https://github.com/PixarAnimationStudios/OpenUSD".into(),
                    revision: "2095fafafd033fa23386d7ec6d58c7cc33974518".into(),
                }),
                vec![ResolvedDependencyIdentity {
                    name: "oneTBB".into(),
                    version: "2021.12.0".into(),
                    source: ResolvedSourceIdentity {
                        repository: "https://github.com/oneapi-src/oneTBB".into(),
                        revision: "v2021.12.0".into(),
                    },
                    archive_digest: None,
                }],
            )
            .unwrap();
        let metadata = artifact_build_metadata(&manifest, None, false)
            .unwrap()
            .unwrap();
        assert_eq!(
            metadata["source"]["revision"],
            "2095fafafd033fa23386d7ec6d58c7cc33974518"
        );
        assert_eq!(metadata["dependencies"][0]["name"], "oneTBB");
        assert_eq!(
            metadata["builder"]["id"],
            "https://openstrata.dev/builders/ost/runtime-pull"
        );

        let supplied = serde_json::json!({
            "source": {
                "repository": "https://example.invalid/not-openusd",
                "revision": "different"
            },
            "builder": {
                "id": "https://build.example/openusd",
                "identity": { "host": "builder-01" }
            }
        });
        assert_eq!(
            artifact_build_metadata(&manifest, Some(supplied), true)
                .unwrap_err()
                .code(),
            "BUILD_METADATA_SOURCE_MISMATCH"
        );

        let mut archive_manifest = exportable_manifest();
        archive_manifest
            .set_build_identities(None, manifest.build_dependencies.clone())
            .unwrap();
        assert_eq!(
            artifact_build_metadata(&archive_manifest, None, false)
                .unwrap_err()
                .code(),
            "BUILD_METADATA_SOURCE_REQUIRED"
        );
        let supplied = serde_json::json!({
            "source": {
                "repository": "https://downloads.example/openusd",
                "revision": "sha256:source-tree"
            },
            "builder": {
                "id": "https://build.example/openusd",
                "identity": { "host": "builder-01" }
            }
        });
        let metadata = artifact_build_metadata(&archive_manifest, Some(supplied), true)
            .unwrap()
            .unwrap();
        assert_eq!(metadata["dependencies"][0]["name"], "oneTBB");
    }

    #[test]
    fn source_urls_are_sanitized_before_identity_storage() {
        assert_eq!(
            normalize_git_repository(
                "https://secret-token@github.com/PixarAnimationStudios/OpenUSD.git?access=secret#fragment"
            ),
            "https://github.com/PixarAnimationStudios/OpenUSD"
        );
        let identity = dependency_identity_from_download(BuildUsdDownload {
            dependency: "custom".into(),
            url: "https://user:secret@example.invalid/source-1.0.zip?token=secret".into(),
            archive_sha256: "dd".repeat(32),
        })
        .unwrap();
        assert_eq!(
            identity.source.repository,
            "https://example.invalid/source-1.0.zip"
        );
        assert!(!identity.version.contains("secret"));
    }

    #[test]
    fn dirty_git_source_is_rejected_before_build() {
        let Some(git) = tools::which("git") else {
            return;
        };
        let scratch = temp_dir("dirty-openusd-source");
        let run = |args: &[&str]| {
            let status = Command::new(&git)
                .args(args)
                .current_dir(scratch.as_std_path())
                .status()
                .unwrap();
            assert!(status.success(), "git command failed: {args:?}");
        };
        run(&["init", "-q"]);
        run(&["config", "user.name", "OpenStrata Test"]);
        run(&["config", "user.email", "test@openstrata.invalid"]);
        std::fs::write(scratch.join("source.cpp").as_std_path(), "clean\n").unwrap();
        run(&["add", "source.cpp"]);
        run(&["commit", "-q", "-m", "source"]);
        run(&[
            "remote",
            "add",
            "origin",
            "https://token@github.com/PixarAnimationStudios/OpenUSD.git",
        ]);

        let identity = git_source_identity(&scratch).unwrap().unwrap();
        assert_eq!(
            identity.repository,
            "https://github.com/PixarAnimationStudios/OpenUSD"
        );
        std::fs::write(scratch.join("untracked.py").as_std_path(), "dirty\n").unwrap();
        assert_eq!(
            git_source_identity(&scratch).unwrap_err().code(),
            "OPENUSD_SOURCE_DIRTY"
        );
        std::fs::remove_dir_all(scratch.as_std_path()).unwrap();
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
    fn missing_dep_warning_names_the_missing_packages_and_pip_fix() {
        // Nothing missing -> no warning (a preflight must not cry wolf).
        let needed = build_dep_requirements(&["usd-stage-read".into(), "qt-ui".into()]);
        assert!(missing_dep_warning("/usr/bin/python3", &needed, &[]).is_empty());

        // A clean interpreter missing jinja2 (but with PySide6) warns with the
        // exact missing name and a pip line naming only Jinja2's pip package.
        let lines = missing_dep_warning("/usr/bin/python3", &needed, &["jinja2".to_string()]);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("not importable by /usr/bin/python3"));
        assert!(lines[0].contains("jinja2"));
        assert!(lines[1].contains("-m pip install Jinja2"));
        assert!(
            !lines[1].contains("PySide6"),
            "only the missing package is offered"
        );
        assert!(lines[2].contains("usdview needs PySide6 + PyOpenGL"));

        // All three missing -> the pip line offers all three pip names in order.
        let all = build_dep_requirements(&["usd-stage-read".into(), "hydra-preview".into()]);
        let missing: Vec<String> = all.iter().map(|(i, _)| i.to_string()).collect();
        let lines = missing_dep_warning("python", &all, &missing);
        assert!(lines[1].contains("pip install Jinja2 PySide6 PyOpenGL"));
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

    /// The forced-off components are defaults, not constraints: forwarding the
    /// positive half must *replace* ost's negative half, because build_usd.py
    /// rejects the pair rather than letting the later flag win (report 29 §1).
    #[test]
    fn forwarded_component_flag_replaces_the_default_trim() {
        let args = build_usd_args(
            Utf8Path::new("/src/build_scripts/build_usd.py"),
            Utf8Path::new("/store/rt"),
            None,
            &["--examples".to_string()],
        );
        assert!(args.iter().any(|a| a == "--examples"));
        assert!(!args.iter().any(|a| a == "--no-examples"));
        // The components the caller said nothing about keep their default.
        for untouched in ["--no-tutorials", "--no-docs", "--no-tests"] {
            assert!(args.iter().any(|a| a == untouched), "missing {untouched}");
        }
        assert!(conflicting_component_flags(&args).is_empty());
    }

    /// Re-passing the negative half is a no-op, not a duplicate.
    #[test]
    fn forwarded_negative_half_is_not_duplicated() {
        let args = build_usd_args(
            Utf8Path::new("/src/build_scripts/build_usd.py"),
            Utf8Path::new("/store/rt"),
            None,
            &["--no-docs".to_string()],
        );
        assert_eq!(args.iter().filter(|a| *a == "--no-docs").count(), 1);
    }

    /// `--x=value` is the other spelling argparse accepts for the same option.
    #[test]
    fn valued_forwarded_flag_still_suppresses_the_default() {
        let args = build_usd_args(
            Utf8Path::new("/src/build_scripts/build_usd.py"),
            Utf8Path::new("/store/rt"),
            None,
            &["--tests=all".to_string()],
        );
        assert!(!args.iter().any(|a| a == "--no-tests"));
    }

    /// A caller who names both halves themselves is caught before the spawn.
    #[test]
    fn both_halves_from_the_caller_are_reported_as_a_conflict() {
        let args = build_usd_args(
            Utf8Path::new("/src/build_scripts/build_usd.py"),
            Utf8Path::new("/store/rt"),
            None,
            &["--tools".to_string(), "--no-tools".to_string()],
        );
        assert_eq!(
            conflicting_component_flags(&args),
            vec!["tools".to_string()]
        );
    }

    /// The default argv must never trip its own conflict check.
    #[test]
    fn default_component_argv_has_no_conflicts() {
        let args = build_usd_args(
            Utf8Path::new("/src/build_scripts/build_usd.py"),
            Utf8Path::new("/store/rt"),
            Some(4),
            &[],
        );
        assert!(conflicting_component_flags(&args).is_empty());
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

    #[test]
    fn reproducible_export_uses_a_host_independent_worker_default() {
        assert_eq!(default_pack_workers(true), 0);
        assert!(default_pack_workers(false) >= 1);
    }

    /// A slim export drops whole trees. If the shipped CMake config still points
    /// into one, `find_package` resolves a path the artifact does not contain —
    /// and it fails at the consumer's configure step, far from `runtime export`.
    #[test]
    fn slim_export_reports_dropped_layout_a_shipped_config_references() {
        let dir = temp_dir("slim-ref");
        let config = dir.join("pxrConfig.cmake");
        std::fs::write(
            config.as_std_path(),
            "set(PXR_INCLUDE_DIRS \"${PXR_ROOT}/include\")
             set(PXR_SRC \"${PXR_ROOT}/src/pxr\")
",
        )
        .unwrap();

        let files = vec![config.clone()];
        let found = referenced_excluded_dirs(&files, &["src".to_string(), "build".to_string()]);
        assert_eq!(
            found,
            vec![("src".to_string(), "pxrConfig.cmake".to_string())],
            "only the referenced excluded tree is reported"
        );

        // Nothing excluded, nothing to warn about.
        assert!(referenced_excluded_dirs(&files, &[]).is_empty());
        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    /// The match is on a path segment, so a config that merely mentions a
    /// similar word does not produce a warning nobody can act on.
    #[test]
    fn a_similar_word_is_not_mistaken_for_a_dropped_directory() {
        let dir = temp_dir("slim-noise");
        let config = dir.join("pxrTargets.cmake");
        std::fs::write(
            config.as_std_path(),
            "set(REBUILD_FLAGS on)
set(X \"${ROOT}/buildinfo/x\")
",
        )
        .unwrap();
        assert!(
            referenced_excluded_dirs(&[config], &["build".to_string()]).is_empty(),
            "'buildinfo' and 'REBUILD_FLAGS' are not references to 'build/'"
        );
        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    /// Only shipped `.cmake` files matter: a reference from something the
    /// artifact does not carry cannot break a consumer.
    #[test]
    fn non_cmake_files_are_not_scanned_for_references() {
        let dir = temp_dir("slim-nontext");
        let readme = dir.join("README.md");
        std::fs::write(
            readme.as_std_path(),
            "see ${ROOT}/src/pxr for sources
",
        )
        .unwrap();
        assert!(referenced_excluded_dirs(&[readme], &["src".to_string()]).is_empty());
        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }
}
