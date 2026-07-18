// SPDX-License-Identifier: Apache-2.0
//! `ost package` — install a built target and pack it as a `tar.zst` artifact.
//!
//! Flow (§8.2 tail, §10): `cmake --install` the built target into a clean stage
//! tree, pack it to `dist/<name>/<version>/<target>/<name>-<version>-<target>.tar.zst`,
//! and write a content-addressed `manifest.json` plus `SHA256SUMS`. Every file
//! and the archive itself are hashed, and the manifest records provenance and
//! the runtime's validation status.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use camino::Utf8PathBuf;
use clap::Args;

use ost_build::{
    pack_dir, stage_files, BuildCompletion, BuildIntent, TargetLock, BUILD_COMPLETION_FILE,
};
use ost_core::paths::STATE_DIR;
use ost_core::{tools, Error, Result};
use ost_runtime::{RuntimeManifest, MANIFEST_FILE};

use crate::commands::configure::{build_target, load_project, resolve_selection};
use crate::output::{self, Format};

#[derive(Debug, Args)]
pub struct PackageArgs {
    /// Platform target, e.g. `cy2026`. Defaults to the project's platform.
    #[arg(long)]
    target: Option<String>,

    /// Profile to package. Defaults to the project's profile.
    #[arg(long)]
    profile: Option<String>,

    /// Allow an empty install tree (a metadata-only artifact). By default an
    /// empty tree is an error.
    #[arg(long)]
    allow_empty: bool,

    /// Reclaim the stable package stage harder and sweep stale fallback stages a
    /// previous locked run left behind, instead of quietly staging into yet
    /// another sibling. Use once the process holding the old stage has exited.
    #[arg(long)]
    clean_stage: bool,
}

pub fn run(args: PackageArgs, fmt: Format) -> Result<()> {
    let (root, platform, profile) = resolve_selection(args.target, args.profile)?;
    let project = load_project(&root)?;
    let project_version = project.effective_version(&root)?;
    let (target, r) = build_target(&platform, &profile)?;
    let id = target.id();

    let build_dir = root.join("build").join(&id);
    if !build_dir.as_std_path().is_dir() {
        return Err(Error::Operation(format!(
            "target '{id}' is not built — run `ost build` first"
        )));
    }
    let lock_path = root
        .join(STATE_DIR)
        .join("targets")
        .join(&id)
        .join("target.lock.json");
    let completion_path = build_dir.join(BUILD_COMPLETION_FILE);
    let lock: TargetLock = read_json(&lock_path)?;
    let completion: BuildCompletion = read_json(&completion_path)?;
    completion
        .validate_against(
            &lock,
            &project.project.name,
            &project_version,
            &Utf8PathBuf::from(format!("build/{id}")),
        )
        .map_err(|detail| {
            Error::precondition(format!(
                "target '{id}' has stale or incompatible build completion: {detail}"
            ))
            .with_hint("rerun `ost build` before packaging")
        })?;
    let configuration = completed_configuration(&completion.intent)?;

    let cmake = tools::which("cmake").ok_or_else(|| {
        Error::coded(
            "REQUIRED_TOOL_MISSING",
            ost_core::Category::Precondition,
            "`cmake` not found on PATH",
        )
    })?;

    // Install into a clean stage tree. Reruns must not fail on a stage the
    // previous run left temporarily undeletable (scanner-held handles,
    // dogfooding report #9): stage into a fresh sibling instead, and surface
    // that as an actionable warning (`--clean-stage` reclaims the stable name).
    let preferred_stage = root.join(STATE_DIR).join("targets").join(&id).join("stage");
    let (stage, stage_warnings) = super::prepare_package_stage(&preferred_stage, args.clean_stage)?;

    // Apply the runtime environment to `cmake --install` for the same reason
    // `ost build` and `ost run` do: install rules may invoke USD/Python tooling
    // that needs PATH, PYTHONPATH and the loader path set consistently.
    let mut install = Command::new(&cmake);
    install
        .args(["--install", &format!("build/{id}"), "--config"])
        .arg(configuration)
        .arg("--prefix")
        .arg(stage.as_std_path())
        .current_dir(root.as_std_path());
    r.env.apply(&mut install);
    let status = install
        .status()
        .map_err(|e| Error::io(format!("run {}", cmake.display()), e))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    // Walk the stage tree once, then reuse the list for the empty check and the
    // pack below. Reject an empty install tree before writing anything — packing
    // it would produce a useless artifact that silently "succeeds".
    // `--allow-empty` opts in to a metadata-only artifact.
    // A rejected unsafe entry (symlink/special file, §SEC-001) is `InvalidData`,
    // not a disk failure — surface it as a validation error so callers branching
    // on category don't mistake a planted-link attack for transient I/O.
    let staged = stage_files(&stage).map_err(|e| {
        if e.kind() == std::io::ErrorKind::InvalidData {
            Error::validation(e.to_string())
        } else {
            Error::io(stage.to_string(), e)
        }
    })?;
    if staged.is_empty() && !args.allow_empty {
        return Err(Error::validation(format!(
            "the install tree for '{id}' is empty — nothing to package"
        ))
        .with_hint(
            "add `install(TARGETS ...)` (and resource install rules) to CMakeLists.txt, \
             or pass --allow-empty for a metadata-only artifact",
        ));
    }

    // Pack the stage tree.
    let name = &project.project.name;
    let version = project_version;
    let archive_name = format!("{name}-{version}-{id}.tar.zst");
    let dist_dir = root.join("dist").join(name).join(&version).join(&id);
    let archive_path = dist_dir.join(&archive_name);

    let packed = pack_dir(&stage, &archive_path, &staged)
        .map_err(|e| Error::io(archive_path.to_string(), e))?;

    if packed.files.is_empty() {
        // Only reachable with --allow-empty (the empty tree was rejected above).
        eprintln!("note: packaging a metadata-only artifact (empty install tree, --allow-empty)");
    }

    // Runtime validation status, for the artifact's provenance.
    let validation = std::fs::read_to_string(r.prefix.join(MANIFEST_FILE).as_std_path())
        .ok()
        .and_then(|s| RuntimeManifest::from_json(&s).ok())
        .map(|m| format!("{:?}", m.validation).to_lowercase())
        .unwrap_or_else(|| "unknown".to_string());

    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // manifest.json
    let files_json: Vec<_> = packed.files.iter().map(|f| f.manifest_json()).collect();
    let mut manifest = serde_json::json!({
        "schema": 1,
        "name": name,
        "version": version,
        "target": id,
        "archive": archive_name,
        "archive_digest": packed.archive_digest,
        "archive_size": packed.archive_size,
        "total_size": packed.total_size,
        "created_unix": created,
        // The producing tool names itself here so the registry can
        // record the artifact's origin instead of whoever imported it.
        "producer": format!("ost {}", env!("CARGO_PKG_VERSION")),
        "provenance": {
            "platform": lock.platform,
            "profile": lock.profile,
            "variant": lock.variant.slug(),
            "cxx_standard": lock.cxx_standard,
            "generator": lock.generator,
            "runtime": { "id": lock.runtime.id, "digest": lock.runtime.digest },
            "validation": validation,
        },
        "files": files_json,
    });
    let evidence = ost_artifact::generate_evidence(&dist_dir, &mut manifest)?;
    let manifest_path = dist_dir.join("manifest.json");
    write(&manifest_path, &pretty(&manifest)?)?;

    // SHA256SUMS — bare-hex line for the archive, so `sha256sum -c` validates it.
    let bare = packed
        .archive_digest
        .strip_prefix("sha256:")
        .unwrap_or(&packed.archive_digest);
    let mut sums = vec![format!("{bare}  {archive_name}")];
    sums.extend(evidence.iter().map(|layer| {
        format!(
            "{}  {}",
            layer
                .digest
                .strip_prefix("sha256:")
                .unwrap_or(&layer.digest),
            layer.path
        )
    }));
    write(&dist_dir.join("SHA256SUMS"), &sums.join("\n"))?;

    report(
        &id,
        &archive_path,
        &packed,
        &validation,
        &stage_warnings,
        fmt,
    );
    Ok(())
}

fn completed_configuration(intent: &BuildIntent) -> Result<&str> {
    intent
        .cache
        .get("CMAKE_BUILD_TYPE")
        .map(String::as_str)
        .filter(|configuration| !configuration.trim().is_empty())
        .ok_or_else(|| {
            Error::precondition("build completion does not record a CMake configuration")
                .with_hint("rerun `ost build` before packaging")
        })
}

fn read_json<T: serde::de::DeserializeOwned>(path: &camino::Utf8Path) -> Result<T> {
    let source = std::fs::read_to_string(path.as_std_path()).map_err(|error| {
        Error::precondition(format!(
            "required build evidence is missing at '{path}': {error}"
        ))
        .with_hint("rerun `ost build` before packaging")
    })?;
    serde_json::from_str(&source).map_err(|error| {
        Error::precondition(format!("invalid build evidence at '{path}': {error}"))
            .with_hint("rerun `ost build` before packaging")
    })
}

fn write(path: &Utf8PathBuf, contents: &str) -> Result<()> {
    std::fs::write(path.as_std_path(), format!("{contents}\n"))
        .map_err(|e| Error::io(path.to_string(), e))
}

fn pretty(value: &serde_json::Value) -> Result<String> {
    serde_json::to_string_pretty(value).map_err(|e| Error::parse("json", anyhow::Error::new(e)))
}

fn report(
    id: &str,
    archive: &Utf8PathBuf,
    packed: &ost_build::PackResult,
    validation: &str,
    warnings: &[serde_json::Value],
    fmt: Format,
) {
    if fmt.is_json() {
        output::report_with_warnings(
            true,
            &serde_json::json!({
                "packaged": true,
                "target": id,
                "archive": archive.to_string(),
                "archive_digest": packed.archive_digest,
                "archive_size": packed.archive_size,
                "files": packed.files.len(),
                "validation": validation,
            }),
            warnings,
        );
        return;
    }
    for w in warnings {
        if let Some(msg) = w["message"].as_str() {
            eprintln!("warning: {msg}");
        }
    }
    println!("Packaged target {id}");
    println!("  archive:  {archive}");
    println!("  digest:   {}", packed.archive_digest);
    println!(
        "  size:     {} bytes ({} file(s), {} uncompressed)",
        packed.archive_size,
        packed.files.len(),
        packed.total_size
    );
    println!("  validation: {validation}");
    println!("  manifest.json + SHA256SUMS written alongside the archive");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_configuration_is_required_for_packaging() {
        let mut intent = BuildIntent::default();
        assert!(completed_configuration(&intent).is_err());

        intent
            .cache
            .insert("CMAKE_BUILD_TYPE".into(), "Debug".into());
        assert_eq!(completed_configuration(&intent).unwrap(), "Debug");
    }
}
