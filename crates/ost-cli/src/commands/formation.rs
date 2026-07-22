// SPDX-License-Identifier: Apache-2.0
//! `ost formation` — digest-pinned cross-repository composition and launch.

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};
use clap::{Args, Subcommand};
use ost_artifact::{extract_archive, ArtifactKind, ArtifactRecord, ArtifactStore};
use ost_core::fs::write_atomic;
use ost_core::{digest, Category, Error, Result};
use ost_formation::{
    ActivationInput, ComponentInput, FormationLock, FormationManifest, MaterializedFormation,
    ResolutionInput,
};
use ost_plugin::Bundle;
use ost_runtime::RuntimeManifest;
use serde::Deserialize;

use crate::output::{self, Format};

#[derive(Debug, Subcommand)]
pub enum FormationCmd {
    /// Resolve immutable artifacts, compatibility, and environment without launching.
    Resolve(FormationPathArgs),
    /// Inspect the current resolution and any adjacent lock state.
    Inspect(FormationPathArgs),
    /// Write a deterministic, digest-pinned formation.lock.
    Lock(FormationLockArgs),
    /// Launch the Formation's command in the foreground and record evidence.
    Run(FormationRunArgs),
}

#[derive(Debug, Clone, Args)]
pub struct FormationPathArgs {
    /// Formation manifest to resolve.
    #[arg(default_value = "formation.toml")]
    pub path: Utf8PathBuf,
}

#[derive(Debug, Clone, Args)]
pub struct FormationLockArgs {
    /// Formation manifest to lock.
    #[arg(default_value = "formation.toml")]
    pub path: Utf8PathBuf,

    /// Lock path (defaults to formation.lock beside the manifest).
    #[arg(long)]
    pub output: Option<Utf8PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct FormationRunArgs {
    /// Formation manifest to run.
    #[arg(default_value = "formation.toml")]
    pub path: Utf8PathBuf,

    /// Override `[command]` with a program and arguments after `--`.
    #[arg(last = true)]
    pub command: Vec<String>,
}

pub fn run(command: FormationCmd, format: Format) -> Result<()> {
    match command {
        FormationCmd::Resolve(args) => resolve_command(&args.path, format),
        FormationCmd::Inspect(args) => inspect_command(&args.path, format),
        FormationCmd::Lock(args) => lock_command(&args, format),
        FormationCmd::Run(args) => run_command(&args, format),
    }
}

fn resolve_command(path: &Utf8Path, format: Format) -> Result<()> {
    let resolution = resolve_path(path)?;
    let materialized = &resolution.materialized;
    match format {
        Format::Json => output::success(&serde_json::to_value(&materialized.resolved).map_err(
            |error| Error::Operation(format!("cannot serialize Formation resolution: {error}")),
        )?),
        Format::Human => print_resolution(materialized),
    }
    Ok(())
}

fn inspect_command(path: &Utf8Path, format: Format) -> Result<()> {
    let resolution = resolve_path(path)?;
    let materialized = &resolution.materialized;
    let absolute_path = &resolution.manifest_path;
    let lock_path = default_lock_path(absolute_path);
    let lock = read_lock_if_present(&lock_path)?;
    let lock_matches = lock
        .as_ref()
        .map(|lock| lock.resolution.manifest_digest == materialized.resolved.manifest_digest)
        .unwrap_or(false);
    let data = serde_json::json!({
        "resolution": materialized.resolved,
        "lock": {
            "path": lock_path,
            "present": lock.is_some(),
            "matches_manifest": lock_matches,
            "digest": lock.as_ref().map(FormationLock::digest).transpose()?,
        }
    });
    match format {
        Format::Json => output::success(&data),
        Format::Human => {
            print_resolution(materialized);
            if lock_matches {
                println!("  lock: {} (current)", lock_path);
            } else if lock.is_some() {
                println!("  lock: {} (stale)", lock_path);
            } else {
                println!("  lock: not written (run `ost formation lock {path}`)");
            }
        }
    }
    Ok(())
}

fn lock_command(args: &FormationLockArgs, format: Format) -> Result<()> {
    let resolution = resolve_path(&args.path)?;
    let materialized = &resolution.materialized;
    let absolute_path = &resolution.manifest_path;
    let output_path = args
        .output
        .clone()
        .unwrap_or_else(|| default_lock_path(absolute_path));
    let output_path = absolute_from_current(&output_path)?;
    let lock = FormationLock::from_resolved(&materialized.resolved);
    let body = serde_json::to_string_pretty(&lock)
        .map_err(|error| Error::Operation(format!("cannot serialize Formation lock: {error}")))?;
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent.as_std_path())
            .map_err(|error| Error::io(parent.to_string(), error))?;
    }
    write_atomic(output_path.as_std_path(), format!("{body}\n").as_bytes())?;
    let data = serde_json::json!({
        "schema": lock.schema,
        "path": output_path,
        "digest": lock.digest()?,
        "manifest_digest": materialized.resolved.manifest_digest,
        "components": materialized.resolved.components.len(),
    });
    match format {
        Format::Json => output::success(&data),
        Format::Human => println!(
            "Locked Formation '{}' ({} component(s)) -> {}\n  {}",
            materialized.resolved.name,
            materialized.resolved.components.len(),
            output_path,
            data["digest"].as_str().unwrap_or_default()
        ),
    }
    Ok(())
}

fn run_command(args: &FormationRunArgs, format: Format) -> Result<()> {
    let resolution = resolve_path(&args.path)?;
    let materialized = &resolution.materialized;
    let absolute_path = &resolution.manifest_path;
    let lock_path = default_lock_path(absolute_path);
    let lock = read_lock_if_present(&lock_path)?.ok_or_else(|| {
        Error::coded(
            "FORMATION_LOCK_REQUIRED",
            Category::Precondition,
            format!("Formation lock not found at '{lock_path}'"),
        )
        .with_hint(format!("run `ost formation lock {}` first", absolute_path))
    })?;
    if lock.resolution.manifest_digest != materialized.resolved.manifest_digest {
        return Err(Error::coded(
            "FORMATION_LOCK_STALE",
            Category::Validation,
            format!("'{lock_path}' does not match the current Formation manifest"),
        )
        .with_hint(format!(
            "refresh it with `ost formation lock {absolute_path}`"
        )));
    }
    let current_lock = FormationLock::from_resolved(&materialized.resolved);
    if lock != current_lock {
        return Err(Error::coded(
            "FORMATION_LOCK_DRIFT",
            Category::Validation,
            "resolved artifacts/environment differ from formation.lock",
        )
        .with_hint("restore the locked artifacts or deliberately refresh the lock"));
    }

    let (program, command_args) = if args.command.is_empty() {
        (
            materialized.resolved.command.program.clone(),
            materialized.resolved.command.args.clone(),
        )
    } else {
        (args.command[0].clone(), args.command[1..].to_vec())
    };
    let env = materialized.env.resolve_over(&HashMap::new());
    let env_fingerprint =
        digest::sha256_hex(&serde_json::to_vec(&env).map_err(|error| {
            Error::Operation(format!("cannot fingerprint environment: {error}"))
        })?);
    let started = now_unix();
    let mut command = Command::new(&program);
    command.args(&command_args);
    for key in [
        "PATH",
        "LD_LIBRARY_PATH",
        "DYLD_LIBRARY_PATH",
        "PYTHONPATH",
        "CMAKE_PREFIX_PATH",
        "PXR_PLUGINPATH_NAME",
    ] {
        command.env_remove(key);
    }
    command.envs(env.iter().cloned());

    let (status, stdout, stderr) = if format.is_json() {
        let output = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|error| {
                Error::coded(
                    "FORMATION_LAUNCH_FAILED",
                    Category::Precondition,
                    format!("cannot launch '{program}': {error}"),
                )
            })?;
        (
            output.status,
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    } else {
        let status = command.status().map_err(|error| {
            Error::coded(
                "FORMATION_LAUNCH_FAILED",
                Category::Precondition,
                format!("cannot launch '{program}': {error}"),
            )
        })?;
        (status, String::new(), String::new())
    };
    let finished = now_unix();
    let lock_digest = lock.digest()?;
    let evidence = serde_json::json!({
        "schema": "openstrata.formation-run/v1alpha1",
        "formation": materialized.resolved.name,
        "manifest_digest": materialized.resolved.manifest_digest,
        "lock_digest": lock_digest,
        "runtime": materialized.resolved.runtime,
        "components": materialized.resolved.components,
        "program": program,
        "args": command_args,
        "environment_digest": env_fingerprint,
        "started_unix": started,
        "finished_unix": finished,
        "exit_code": status.code(),
        "success": status.success(),
        "stdout": stdout,
        "stderr": stderr,
    });
    let evidence_path = evidence_path(absolute_path, &materialized.resolved.name, started);
    let body = serde_json::to_string_pretty(&evidence)
        .map_err(|error| Error::Operation(format!("cannot serialize run evidence: {error}")))?;
    if let Some(parent) = evidence_path.parent() {
        std::fs::create_dir_all(parent.as_std_path())
            .map_err(|error| Error::io(parent.to_string(), error))?;
    }
    write_atomic(evidence_path.as_std_path(), format!("{body}\n").as_bytes())?;

    if format.is_json() {
        output::report(
            status.success(),
            &serde_json::json!({
                "run": evidence,
                "evidence": evidence_path,
            }),
        );
    } else {
        println!("Formation evidence: {evidence_path}");
    }
    if !status.success() {
        resolution.cleanup();
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

struct ResolvedPath {
    materialized: MaterializedFormation,
    manifest_path: Utf8PathBuf,
    staging: StagingRoot,
}

impl ResolvedPath {
    fn cleanup(&self) {
        self.staging.cleanup();
    }
}

struct StagingRoot {
    path: Utf8PathBuf,
}

impl StagingRoot {
    fn cleanup(&self) {
        let _ = ost_core::fs::remove_dir_all_robust(self.path.as_std_path());
    }
}

impl Drop for StagingRoot {
    fn drop(&mut self) {
        self.cleanup();
    }
}

fn resolve_path(path: &Utf8Path) -> Result<ResolvedPath> {
    let absolute_path = absolute_from_current(path)?;
    let declared = FormationManifest::load(&absolute_path)?;
    let parent = absolute_path.parent().unwrap_or(Utf8Path::new("."));
    let staging = StagingRoot {
        path: parent
            .join(".strata")
            .join("formations")
            .join(&declared.formation.name)
            .join("materialized")
            .join(format!("{}-{}", std::process::id(), now_nanos())),
    };
    let staging_root = &staging.path;
    let store = ArtifactStore::discover();
    let runtime_record = checked_record(&store, &declared.runtime.artifact)?;
    if runtime_record.kind != ArtifactKind::Runtime {
        return Err(Error::coded(
            "FORMATION_ARTIFACT_KIND_MISMATCH",
            Category::Validation,
            format!(
                "Formation runtime {} is a {} artifact",
                runtime_record.short_digest(),
                runtime_record.kind.as_str()
            ),
        ));
    }
    let runtime_manifest = embedded_runtime_manifest(&store, &runtime_record)?;
    let runtime_root = materialize(&store, &runtime_record, &staging_root.join("runtime"))?;
    let mut components = Vec::new();
    for component in &declared.components {
        let record = checked_record(&store, &component.artifact)?;
        let root = materialize(
            &store,
            &record,
            &staging_root.join("components").join(&component.id),
        )?;
        let (bundles, activation) = component_payload(&record, &root)?;
        components.push(ComponentInput {
            declared: component.clone(),
            record,
            root,
            bundles,
            activation,
        });
    }
    let materialized = ost_formation::resolve(
        &declared,
        ResolutionInput {
            runtime_record,
            runtime_manifest,
            runtime_root,
            components,
        },
    )?;
    Ok(ResolvedPath {
        materialized,
        manifest_path: absolute_path,
        staging,
    })
}

fn checked_record(store: &ArtifactStore, digest_ref: &str) -> Result<ArtifactRecord> {
    let record = store.resolve(digest_ref)?;
    if record.digest != digest_ref {
        return Err(Error::validation(format!(
            "artifact resolver returned {}, but exact identity {} was pinned",
            record.digest, digest_ref
        )));
    }
    let verification = store.verify(digest_ref)?;
    if !verification.passed() {
        return Err(Error::coded(
            "FORMATION_ARTIFACT_VERIFICATION_FAILED",
            Category::Validation,
            format!(
                "artifact {} failed local integrity verification",
                record.short_digest()
            ),
        )
        .with_hint("re-import or pull the artifact from its trusted producer"));
    }
    Ok(record)
}

fn embedded_runtime_manifest(
    store: &ArtifactStore,
    record: &ArtifactRecord,
) -> Result<RuntimeManifest> {
    let producer = store.producer_manifest(record)?;
    let embedded = producer
        .pointer("/provenance/runtime_manifest")
        .ok_or_else(|| {
            Error::InvalidManifest("runtime artifact carries no provenance.runtime_manifest".into())
        })?;
    serde_json::from_value(embedded.clone()).map_err(|error| {
        Error::parse(
            "runtime artifact runtime_manifest",
            anyhow::Error::new(error),
        )
    })
}

fn materialize(
    store: &ArtifactStore,
    record: &ArtifactRecord,
    root: &Utf8Path,
) -> Result<Utf8PathBuf> {
    if !root.as_std_path().exists() {
        if let Some(parent) = root.parent() {
            std::fs::create_dir_all(parent.as_std_path())
                .map_err(|error| Error::io(parent.to_string(), error))?;
        }
        store.extract(&record.digest, root)?;
    } else if !root.as_std_path().is_dir() {
        return Err(Error::validation(format!(
            "Formation materialization path '{root}' is not a directory"
        )));
    }
    Ok(root.to_path_buf())
}

fn component_payload(
    record: &ArtifactRecord,
    root: &Utf8Path,
) -> Result<(Vec<Bundle>, ActivationInput)> {
    match record.kind {
        ArtifactKind::Plugin => {
            let bundle = Bundle::load(root)?;
            let activation = activation_for_bundle(&bundle)?;
            Ok((vec![bundle], activation))
        }
        ArtifactKind::Product => product_payload(root),
        ArtifactKind::Package => Ok((Vec::new(), activation_for_root(root)?)),
        ArtifactKind::Runtime => Err(Error::validation(
            "runtime artifact cannot be a Formation component",
        )),
    }
}

#[derive(Debug, Deserialize)]
struct ProductContract {
    schema: String,
    install: ProductInstall,
    members: Vec<ProductMember>,
}

#[derive(Debug, Deserialize)]
struct ProductInstall {
    order: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ProductMember {
    id: String,
    archive: String,
    archive_digest: String,
}

fn product_payload(root: &Utf8Path) -> Result<(Vec<Bundle>, ActivationInput)> {
    let contract_path = root.join("openstrata.product.json");
    let source = std::fs::read_to_string(contract_path.as_std_path())
        .map_err(|error| Error::io(contract_path.to_string(), error))?;
    let contract: ProductContract = serde_json::from_str(&source)
        .map_err(|error| Error::parse(contract_path.to_string(), anyhow::Error::new(error)))?;
    if contract.schema != "openstrata.plugin-product/v1alpha1" {
        return Err(Error::config(format!(
            "unsupported plugin product schema '{}'",
            contract.schema
        )));
    }
    let by_id = contract
        .members
        .iter()
        .map(|member| (member.id.as_str(), member))
        .collect::<HashMap<_, _>>();
    let mut bundles = Vec::new();
    let mut activation = ActivationInput::default();
    for id in &contract.install.order {
        let member = by_id.get(id.as_str()).ok_or_else(|| {
            Error::InvalidManifest(format!(
                "plugin product install order names missing member '{id}'"
            ))
        })?;
        ost_formation::validate_full_digest(
            &format!("product member '{id}' archive_digest"),
            &member.archive_digest,
        )?;
        let archive = safe_join(root, &member.archive, "product member archive")?;
        let member_root = safe_join(&root.join("expanded"), id, "product member id")?;
        if !member_root.as_std_path().exists() {
            if let Some(parent) = member_root.parent() {
                std::fs::create_dir_all(parent.as_std_path())
                    .map_err(|error| Error::io(parent.to_string(), error))?;
            }
            extract_archive(&archive, &member.archive_digest, &member_root)?;
        }
        let bundle = Bundle::load(&member_root)?;
        merge_activation(&mut activation, activation_for_bundle(&bundle)?);
        bundles.push(bundle);
    }
    Ok((bundles, activation))
}

#[derive(Debug, Deserialize)]
struct ActivationContract {
    schema: String,
    #[serde(default)]
    plugin_paths: Vec<String>,
    #[serde(default)]
    library_paths: Vec<String>,
    #[serde(default)]
    python_paths: Vec<String>,
    #[serde(default)]
    bin_paths: Vec<String>,
}

fn activation_for_bundle(bundle: &Bundle) -> Result<ActivationInput> {
    let contract = bundle.root.join("openstrata.activation.json");
    if contract.as_std_path().is_file() {
        activation_for_root(&bundle.root)
    } else {
        Ok(ActivationInput {
            plugin_paths: vec![bundle.plug_info_root()],
            library_paths: std::iter::once(bundle.lib_dir())
                .chain(bundle.runtime_lib_dirs())
                .collect(),
            python_paths: vec![bundle.python_dir()],
            bin_paths: Vec::new(),
        })
    }
}

fn activation_for_root(root: &Utf8Path) -> Result<ActivationInput> {
    let path = root.join("openstrata.activation.json");
    if !path.as_std_path().is_file() {
        return Ok(ActivationInput {
            plugin_paths: existing_dirs([root.join("plugin").join("usd")]),
            library_paths: existing_dirs([root.join("lib")]),
            python_paths: existing_dirs([root.join("python")]),
            bin_paths: existing_dirs([root.join("bin")]),
        });
    }
    let source = std::fs::read_to_string(path.as_std_path())
        .map_err(|error| Error::io(path.to_string(), error))?;
    let activation: ActivationContract = serde_json::from_str(&source)
        .map_err(|error| Error::parse(path.to_string(), anyhow::Error::new(error)))?;
    if activation.schema != "openstrata.activation/v1alpha1" {
        return Err(Error::config(format!(
            "unsupported activation schema '{}' in '{path}'",
            activation.schema
        )));
    }
    Ok(ActivationInput {
        plugin_paths: activation
            .plugin_paths
            .iter()
            .map(|relative| safe_join(root, relative, "plugin activation path"))
            .collect::<Result<_>>()?,
        library_paths: activation
            .library_paths
            .iter()
            .map(|relative| safe_join(root, relative, "library activation path"))
            .collect::<Result<_>>()?,
        python_paths: activation
            .python_paths
            .iter()
            .map(|relative| safe_join(root, relative, "Python activation path"))
            .collect::<Result<_>>()?,
        bin_paths: activation
            .bin_paths
            .iter()
            .map(|relative| safe_join(root, relative, "binary activation path"))
            .collect::<Result<_>>()?,
    })
}

fn safe_join(root: &Utf8Path, relative: &str, field: &str) -> Result<Utf8PathBuf> {
    let bytes = relative.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if relative.is_empty()
        || relative.starts_with('/')
        || relative.starts_with('\\')
        || drive
        || relative.split(['/', '\\']).any(|part| part == "..")
    {
        return Err(Error::coded(
            "FORMATION_ACTIVATION_ESCAPE",
            Category::Validation,
            format!("{field} '{relative}' is not a safe artifact-relative path"),
        ));
    }
    Ok(root.join(relative))
}

fn merge_activation(target: &mut ActivationInput, mut source: ActivationInput) {
    target.plugin_paths.append(&mut source.plugin_paths);
    target.library_paths.append(&mut source.library_paths);
    target.python_paths.append(&mut source.python_paths);
    target.bin_paths.append(&mut source.bin_paths);
}

fn existing_dirs<const N: usize>(paths: [Utf8PathBuf; N]) -> Vec<Utf8PathBuf> {
    paths
        .into_iter()
        .filter(|path| path.as_std_path().is_dir())
        .collect()
}

fn print_resolution(materialized: &MaterializedFormation) {
    let resolved = &materialized.resolved;
    println!("Formation {}", resolved.name);
    println!("  target: {}", resolved.target);
    println!(
        "  runtime: {} {} ({})",
        resolved.runtime.name, resolved.runtime.version, resolved.runtime.digest
    );
    for component in &resolved.components {
        println!(
            "  {} {}: {} {} ({})",
            component.declared_kind.as_str(),
            component.id,
            component.artifact.name,
            component.artifact.version,
            component.artifact.digest
        );
        for bundle in &component.bundles {
            println!(
                "    bundle: {} {} ({})",
                bundle.name, bundle.version, bundle.kind
            );
        }
    }
    println!(
        "  command: {} {}",
        resolved.command.program,
        resolved.command.args.join(" ")
    );
    println!("  conflicts: none");
}

fn absolute_from_current(path: &Utf8Path) -> Result<Utf8PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let current = std::env::current_dir().map_err(|error| Error::io("current directory", error))?;
    Utf8PathBuf::from_path_buf(current.join(path.as_std_path()))
        .map_err(|path| Error::config(format!("path is not valid UTF-8: {}", path.display())))
}

fn default_lock_path(manifest: &Utf8Path) -> Utf8PathBuf {
    manifest
        .parent()
        .unwrap_or(Utf8Path::new("."))
        .join("formation.lock")
}

fn read_lock_if_present(path: &Utf8Path) -> Result<Option<FormationLock>> {
    if !path.as_std_path().is_file() {
        return Ok(None);
    }
    let source = std::fs::read_to_string(path.as_std_path())
        .map_err(|error| Error::io(path.to_string(), error))?;
    let lock: FormationLock = serde_json::from_str(&source)
        .map_err(|error| Error::parse(path.to_string(), anyhow::Error::new(error)))?;
    if lock.schema != ost_formation::LOCK_SCHEMA {
        return Err(Error::config(format!(
            "unsupported Formation lock schema '{}' in '{path}'",
            lock.schema
        )));
    }
    Ok(Some(lock))
}

fn evidence_path(manifest: &Utf8Path, name: &str, started: u64) -> Utf8PathBuf {
    let parent = manifest.parent().unwrap_or(Utf8Path::new("."));
    let process = std::process::id();
    parent
        .join(".strata")
        .join("formations")
        .join(name)
        .join("runs")
        .join(format!("{started}-{process}.json"))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_paths_cannot_escape_artifact() {
        let root = Utf8Path::new("C:/formation/object");
        assert!(safe_join(root, "plugin/usd", "path").is_ok());
        assert!(safe_join(root, "../outside", "path").is_err());
        assert!(safe_join(root, "D:/outside", "path").is_err());
        assert!(safe_join(root, "\\\\server\\share", "path").is_err());
    }

    #[test]
    fn default_lock_is_adjacent_to_manifest() {
        assert_eq!(
            default_lock_path(Utf8Path::new("C:/project/formation.toml")),
            Utf8PathBuf::from("C:/project/formation.lock")
        );
    }
}
