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

use ost_build::{pack_dir, stage_files};
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
}

pub fn run(args: PackageArgs, fmt: Format) -> Result<()> {
    let (root, platform, profile) = resolve_selection(args.target, args.profile)?;
    let project = load_project(&root)?;
    let (target, r) = build_target(&platform, &profile)?;
    let id = target.id();

    let build_dir = root.join("build").join(&id);
    if !build_dir.as_std_path().is_dir() {
        return Err(Error::Operation(format!(
            "target '{id}' is not built — run `ost build` first"
        )));
    }

    let cmake = tools::which("cmake").ok_or_else(|| {
        Error::coded(
            "REQUIRED_TOOL_MISSING",
            ost_core::Category::Precondition,
            "`cmake` not found on PATH",
        )
    })?;

    // Install into a clean stage tree.
    let stage = root.join(STATE_DIR).join("targets").join(&id).join("stage");
    if stage.as_std_path().exists() {
        std::fs::remove_dir_all(stage.as_std_path())
            .map_err(|e| Error::io(stage.to_string(), e))?;
    }
    std::fs::create_dir_all(stage.as_std_path()).map_err(|e| Error::io(stage.to_string(), e))?;

    // Apply the runtime environment to `cmake --install` for the same reason
    // `ost build` and `ost run` do: install rules may invoke USD/Python tooling
    // that needs PATH, PYTHONPATH and the loader path set consistently.
    let mut install = Command::new(&cmake);
    install
        .args(["--install", &format!("build/{id}"), "--prefix"])
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
    let version = &project.project.version;
    let archive_name = format!("{name}-{version}-{id}.tar.zst");
    let dist_dir = root.join("dist").join(name).join(version).join(&id);
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
    let files_json: Vec<_> = packed
        .files
        .iter()
        .map(|f| serde_json::json!({ "path": f.path, "sha256": f.sha256, "size": f.size }))
        .collect();
    let manifest = serde_json::json!({
        "schema": 1,
        "name": name,
        "version": version,
        "target": id,
        "archive": archive_name,
        "archive_digest": packed.archive_digest,
        "archive_size": packed.archive_size,
        "total_size": packed.total_size,
        "created_unix": created,
        "provenance": {
            "platform": target.platform,
            "profile": target.profile,
            "variant": target.variant.slug(),
            "cxx_standard": target.cxx_standard,
            "generator": target.generator,
            "runtime": { "id": target.runtime_id, "digest": target.runtime_digest },
            "validation": validation,
        },
        "files": files_json,
    });
    let manifest_path = dist_dir.join("manifest.json");
    write(&manifest_path, &pretty(&manifest)?)?;

    // SHA256SUMS — bare-hex line for the archive, so `sha256sum -c` validates it.
    let bare = packed
        .archive_digest
        .strip_prefix("sha256:")
        .unwrap_or(&packed.archive_digest);
    write(
        &dist_dir.join("SHA256SUMS"),
        &format!("{bare}  {archive_name}"),
    )?;

    report(&id, &archive_path, &packed, &validation, fmt);
    Ok(())
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
    fmt: Format,
) {
    if fmt.is_json() {
        output::success(&serde_json::json!({
            "packaged": true,
            "target": id,
            "archive": archive.to_string(),
            "archive_digest": packed.archive_digest,
            "archive_size": packed.archive_size,
            "files": packed.files.len(),
            "validation": validation,
        }));
        return;
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
