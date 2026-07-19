// SPDX-License-Identifier: Apache-2.0
//! `ost external` — import and inspect provenance for a build OpenStrata did
//! not perform.
//!
//! `ost validate --build-dir` has always accepted an external tree, and has
//! always had to skip most of what it would like to say about one: nothing tied
//! the tree to a runtime OpenStrata knows. `ost external import` supplies that
//! tie by inspecting the tree's own `CMakeCache.txt` and recording the identity
//! CMake wrote there.
//!
//! The import is explicit on purpose. Silently adopting whatever build tree
//! happened to be lying in `build/` is how an unrelated tree ends up backing a
//! compatibility claim; making it a command means someone decided this tree
//! describes this runtime.
//!
//! What an import grants is deliberately narrow: `validate --build-dir` may
//! report `runtime-compatible` on a full identity match. `configured` and
//! `built` stay skipped no matter what — they assert OpenStrata did the work.

use camino::Utf8PathBuf;
use clap::{Args, Subcommand};

use ost_build::{
    CMakeCache, ExternalBuildProvenance, ExternalImportScope, ExternalRequirementStatus,
    ExternalRuntime, EXTERNAL_BUILD_FILE,
};
use ost_core::fs::write_atomic;
use ost_core::{Error, Result};

use crate::commands::configure::{build_target, resolve_selection};
use crate::commands::runtime::detect_openusd_version;
use crate::output::{self, Format};

#[derive(Debug, Subcommand)]
pub enum ExternalCmd {
    /// Inspect an external build tree's CMake cache and record its provenance.
    Import(ImportArgs),
    /// Show the provenance recorded for an external build tree.
    Show(ShowArgs),
}

#[derive(Debug, Args)]
pub struct ImportArgs {
    /// The external build tree to inspect.
    #[arg(long)]
    build_dir: Utf8PathBuf,

    /// Platform target, e.g. `cy2026`. Defaults to the project's platform.
    #[arg(long)]
    target: Option<String>,

    /// Profile. Defaults to the project's profile.
    #[arg(long)]
    profile: Option<String>,

    /// Additional capability this external tree is intended to exercise. May
    /// be repeated; requirements are combined with the resolved profile.
    #[arg(long = "capability")]
    capabilities: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// The external build tree whose record should be shown.
    #[arg(long)]
    build_dir: Utf8PathBuf,
}

pub fn run(cmd: ExternalCmd, fmt: Format) -> Result<()> {
    match cmd {
        ExternalCmd::Import(args) => import(args, fmt),
        ExternalCmd::Show(args) => show(args, fmt),
    }
}

fn import(args: ImportArgs, fmt: Format) -> Result<()> {
    let (root, platform, profile) = resolve_selection(args.target, args.profile)?;
    let build_dir = absolute(&root, &args.build_dir);
    let cache = load_cache(&build_dir)?;

    let (target, resolved) = build_target(&platform, &profile)?;
    if !resolved.pulled {
        return Err(
            Error::precondition(format!("runtime '{}' not pulled", target.runtime_id)).with_hint(
                format!(
                    "run `ost runtime pull {} --profile {}` first",
                    target.platform, target.profile
                ),
            ),
        );
    }

    // The runtime's *artifact* prefix is what a tree resolves `pxr` from: for an
    // adopted runtime that is the external USD install, not the store entry.
    let runtime = ExternalRuntime {
        id: target.runtime_id.clone(),
        digest: target.runtime_digest.clone(),
        root: resolved.artifact_prefix.to_string().replace('\\', "/"),
    };
    let openusd_version = detect_openusd_version(&resolved.artifact_prefix);

    let mut capabilities = target.capabilities.clone();
    capabilities.extend(args.capabilities);
    capabilities.sort();
    capabilities.dedup();
    let scope = ExternalImportScope {
        profile: target.profile.clone(),
        requires_openusd: capabilities
            .iter()
            .any(|capability| crate::commands::needs_openusd(capability)),
        capabilities,
    };

    let record =
        ExternalBuildProvenance::from_cache(&cache, runtime, openusd_version, scope, now_unix())
            .map_err(|error| Error::validation(error.to_string()).with_hint(error.remediation()))?;

    let body = record
        .to_json()
        .map_err(|error| Error::parse(EXTERNAL_BUILD_FILE, anyhow::Error::new(error)))?;
    let path = build_dir.join(EXTERNAL_BUILD_FILE);
    write_atomic(path.as_std_path(), format!("{body}\n").as_bytes())?;

    if fmt.is_json() {
        output::success(&serde_json::json!({
            "imported": path.to_string(),
            "provenance": record,
        }));
    } else {
        println!("Imported external build provenance for {}", target.id());
        println!("  build dir:   {}", record.build_dir);
        println!("  source root: {}", record.source_root);
        println!(
            "  runtime:     {} ({})",
            record.runtime.id, record.runtime.root
        );
        if let Some(version) = &record.openusd_version {
            println!("  OpenUSD:     {version}");
        }
        println!(
            "  toolchain:   {} ({}) / {} / {}",
            record.toolchain.generator,
            record.toolchain.generator_flavor,
            if record.toolchain.multi_config {
                if record.toolchain.configurations.is_empty() {
                    "<multi-config>"
                } else {
                    "<selectable configurations>"
                }
            } else if record.toolchain.configuration.is_empty() {
                "<default>"
            } else {
                &record.toolchain.configuration
            },
            record.toolchain.cxx_compiler
        );
        if !record.toolchain.configurations.is_empty() {
            println!(
                "  configs:     {}",
                record.toolchain.configurations.join(", ")
            );
        }
        for requirement in &record.requirements {
            println!(
                "  requirement: {} = {} — {}",
                requirement.name,
                match requirement.status {
                    ExternalRequirementStatus::Applied => "applied",
                    ExternalRequirementStatus::NotApplicable => "not-applicable",
                },
                requirement.detail
            );
        }
        if let Some(python) = &record.toolchain.python_version {
            println!("  Python:      {python}");
        }
        println!("  recorded at  {path}");
        println!(
            "\n`ost validate --build-dir {}` can now report runtime compatibility.\n\
             It still will not claim `ost build` configured or built this tree.",
            args.build_dir
        );
    }
    Ok(())
}

fn show(args: ShowArgs, fmt: Format) -> Result<()> {
    let cwd = std::env::current_dir().map_err(|error| Error::io(".", error))?;
    let cwd = Utf8PathBuf::from_path_buf(cwd)
        .map_err(|path| Error::Operation(format!("non-UTF-8 path: {}", path.display())))?;
    let build_dir = absolute(&cwd, &args.build_dir);
    let record = read_provenance(&build_dir)?;

    if fmt.is_json() {
        output::success(&serde_json::json!({ "provenance": record }));
    } else {
        println!("{}", record.describe());
        println!("  source root: {}", record.source_root);
        println!(
            "  runtime:     {} ({})",
            record.runtime.id, record.runtime.root
        );
        println!("  cache digest {}", record.cache_digest);
    }
    Ok(())
}

/// Read an external tree's provenance record, if it has one.
pub(crate) fn read_provenance(build_dir: &camino::Utf8Path) -> Result<ExternalBuildProvenance> {
    let path = build_dir.join(EXTERNAL_BUILD_FILE);
    let source = std::fs::read_to_string(path.as_std_path()).map_err(|_| {
        Error::precondition(format!("no external build provenance at {build_dir}")).with_hint(
            format!("run `ost external import --build-dir {build_dir}` first"),
        )
    })?;
    serde_json::from_str(&source)
        .map_err(|error| Error::parse(path.to_string(), anyhow::Error::new(error)))
}

pub(crate) fn load_cache(build_dir: &camino::Utf8Path) -> Result<CMakeCache> {
    let path = build_dir.join("CMakeCache.txt");
    CMakeCache::load(&path).map_err(|_| {
        Error::precondition(format!("{path} not found"))
            .with_hint("point --build-dir at a configured CMake build tree")
    })
}

fn absolute(root: &camino::Utf8Path, path: &camino::Utf8Path) -> Utf8PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
