// SPDX-License-Identifier: Apache-2.0
//! `ost formation` — digest-pinned cross-repository composition and launch.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};
use clap::{Args, Subcommand};
use ost_artifact::{extract_archive, ArtifactKind, ArtifactRecord, ArtifactStore};
use ost_core::fs::write_atomic;
use ost_core::{digest, Category, Error, Result};
use ost_formation::{
    ActivationInput, ComponentInput, EnvironmentContribution, FormationLock, FormationManifest,
    MaterializedFormation, ResolutionInput,
};
use ost_host::{HostFamily, HostRecord};
use ost_plugin::Bundle;
use ost_runtime::{EnvOp, EnvVar, RuntimeManifest};
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
    /// Print the fully composed Formation environment.
    Env(FormationEnvArgs),
    /// Diagnose resolution, lock state, environment, and command reachability.
    Doctor(FormationPathArgs),
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
pub struct FormationEnvArgs {
    /// Formation manifest to resolve.
    #[arg(default_value = "formation.toml")]
    pub path: Utf8PathBuf,

    /// Target shell. Defaults to the host's conventional shell.
    #[arg(long)]
    pub shell: Option<String>,
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
        FormationCmd::Env(args) => env_command(&args, format),
        FormationCmd::Doctor(args) => doctor_command(&args.path, format),
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

fn env_command(args: &FormationEnvArgs, format: Format) -> Result<()> {
    let mut resolution = resolve_path(&args.path)?;
    resolution.activate_python();
    let shell = super::devshell::pick_shell(args.shell.as_deref(), ost_core::Host::detect().os)?;
    let env = resolution.materialized.env.resolve_over(&HashMap::new());
    match format {
        Format::Json => output::success(&serde_json::json!({
            "formation": resolution.materialized.resolved.name,
            "manifest_digest": resolution.materialized.resolved.manifest_digest,
            "target": resolution.materialized.resolved.target,
            "shell": format!("{shell:?}").to_lowercase(),
            "env": env
                .iter()
                .map(|(name, value)| serde_json::json!({"name": name, "value": value}))
                .collect::<Vec<_>>(),
            "materialized": resolution.staging.path,
        })),
        Format::Human => {
            println!(
                "# OpenStrata Formation environment for {}",
                resolution.materialized.resolved.name
            );
            print!("{}", resolution.materialized.env.render(shell));
        }
    }
    // Unlike resolve/inspect, `env` exports paths for a later shell process.
    // Keep this verified materialization alive so those paths remain usable.
    resolution.staging.retain();
    Ok(())
}

fn doctor_command(path: &Utf8Path, format: Format) -> Result<()> {
    let mut resolution = resolve_path(path)?;
    let python = resolution.activate_python();
    let materialized = &resolution.materialized;
    let lock_path = default_lock_path(&resolution.manifest_path);
    let lock = read_lock_if_present(&lock_path)?;
    let lock_state = match &lock {
        Some(lock)
            if lock.resolution.manifest_digest == materialized.resolved.manifest_digest
                && lock.resolution == materialized.resolved =>
        {
            ("pass", "formation.lock matches the current resolution")
        }
        Some(_) => ("fail", "formation.lock is stale or has resolution drift"),
        None => ("fail", "formation.lock is absent"),
    };
    let environment = materialized.env.resolve_over(&HashMap::new());
    let launch = formation_launch(
        &materialized.resolved.command.program,
        &environment,
        python.as_deref(),
    );
    let checks = serde_json::json!([
        {
            "id": "formation.resolution",
            "status": "pass",
            "detail": format!("{} digest-pinned component(s) resolved", materialized.resolved.components.len()),
        },
        {
            "id": "formation.lock",
            "status": lock_state.0,
            "detail": lock_state.1,
        },
        {
            "id": "formation.environment",
            "status": if materialized.resolved.conflicts.is_empty() { "pass" } else { "fail" },
            "detail": format!("{} composed variable(s), {} conflict(s)", environment.len(), materialized.resolved.conflicts.len()),
        },
        {
            "id": "formation.command",
            "status": if launch.is_ok() { "pass" } else { "fail" },
            "detail": match &launch {
                Ok(_) => format!("'{}' is reachable in the composed environment", materialized.resolved.command.program),
                Err(error) => error.to_string(),
            },
        },
    ]);
    let passed = checks
        .as_array()
        .is_some_and(|items| items.iter().all(|item| item["status"] != "fail"));
    if format.is_json() {
        output::report(
            passed,
            &serde_json::json!({
                "formation": materialized.resolved.name,
                "manifest_digest": materialized.resolved.manifest_digest,
                "target": materialized.resolved.target,
                "checks": checks,
            }),
        );
    } else {
        println!("Diagnosing Formation {}", materialized.resolved.name);
        for check in checks.as_array().into_iter().flatten() {
            println!(
                "  [{}] {} — {}",
                if check["status"] == "pass" {
                    "✓"
                } else {
                    "✗"
                },
                check["id"].as_str().unwrap_or_default(),
                check["detail"].as_str().unwrap_or_default()
            );
        }
    }
    if !passed {
        resolution.cleanup();
        std::process::exit(Category::Validation.exit_code() as i32);
    }
    Ok(())
}

#[derive(Debug)]
struct FormationLaunch {
    executable: std::path::PathBuf,
    script: Option<std::path::PathBuf>,
}

/// Resolve once for both doctor and run; never let the OS search the parent PATH.
fn formation_launch(
    program: &str,
    env: &[(String, String)],
    python: Option<&str>,
) -> Result<FormationLaunch> {
    let path = std::path::Path::new(program);
    let bases = if path.is_absolute() || path.components().count() > 1 {
        vec![path.to_path_buf()]
    } else {
        env.iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("PATH"))
            .map(|(_, value)| {
                std::env::split_paths(value)
                    .filter(|dir| !dir.as_os_str().is_empty())
                    .map(|dir| dir.join(program))
                    .collect()
            })
            .unwrap_or_default()
    };
    for base in bases {
        let mut candidates = vec![base.clone()];
        if cfg!(windows) && base.extension().is_none() {
            candidates.extend(["exe", "com", "cmd", "bat"].map(|ext| base.with_extension(ext)));
        }
        for candidate in candidates.into_iter().filter(|path| path.is_file()) {
            let script = if is_python_script(&candidate) {
                Some(candidate.clone())
            } else if candidate.extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat")
            }) {
                let sibling = candidate.with_extension("");
                let expected = format!(
                    "python \"%~dp0{}\" %*",
                    sibling.file_name().unwrap_or_default().to_string_lossy()
                );
                let shim = std::fs::read_to_string(&candidate).unwrap_or_default();
                (is_python_script(&sibling)
                    && shim
                        .trim()
                        .trim_start_matches('@')
                        .eq_ignore_ascii_case(&expected))
                .then_some(sibling)
            } else {
                None
            };
            if let Some(script) = script {
                let executable = python.ok_or_else(|| Error::coded(
                    "FORMATION_PYTHON_UNAVAILABLE", Category::Precondition,
                    format!("cannot launch '{program}': no Python matching the runtime ABI was found"),
                ).with_hint("install the runtime's declared Python major/minor, then run formation doctor again"))?;
                return Ok(FormationLaunch {
                    executable: executable.into(),
                    script: Some(script),
                });
            }
            if native_launchable(&candidate) {
                return Ok(FormationLaunch {
                    executable: candidate,
                    script: None,
                });
            }
        }
    }
    Err(Error::coded("FORMATION_LAUNCH_FAILED", Category::Precondition,
        format!("cannot launch '{program}': no executable or Python script found in the composed environment")))
}

fn is_python_script(path: &std::path::Path) -> bool {
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("py"))
    {
        return true;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 512];
    let Ok(count) = file.read(&mut head) else {
        return false;
    };
    let first = String::from_utf8_lossy(&head[..count]);
    let first = first.lines().next().unwrap_or_default();
    first.starts_with("#!") && first.to_ascii_lowercase().contains("python")
}

fn native_launchable(path: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        path.extension().is_some_and(|ext| {
            ["exe", "com", "cmd", "bat"]
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
        })
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
}

fn run_command(args: &FormationRunArgs, format: Format) -> Result<()> {
    let mut resolution = resolve_path(&args.path)?;
    let python = resolution.activate_python();
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
    let host_record = materialized
        .resolved
        .host
        .as_ref()
        .map(|host| super::host::checked_pinned_host(&host.id, &host.fingerprint))
        .transpose()?;

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
    let launch = formation_launch(&program, &env, python.as_deref())?;
    let mut command = Command::new(&launch.executable);
    if let Some(script) = &launch.script {
        command.arg(script);
    }
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
        "host": materialized.resolved.host,
        "host_record": host_record,
        "components": materialized.resolved.components,
        "program": program,
        "args": command_args,
        "executable": launch.executable,
        "script": launch.script,
        "python": python,
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
    runtime_root: Utf8PathBuf,
    runtime_python: String,
}

impl ResolvedPath {
    /// Machine-local interpreter selection is shared with runtime validation.
    /// Keep it out of the portable lock, but expose its directory to scripts
    /// and shims without importing the rest of the parent PATH.
    fn activate_python(&mut self) -> Option<String> {
        let argv = ost_build::resolve_run_python(&self.runtime_root, &self.runtime_python)?;
        let executable = argv.first()?.clone();
        let directory = Utf8Path::new(&executable).parent()?;
        self.materialized.env.vars.push(EnvVar {
            key: "PATH".into(),
            op: EnvOp::Prepend(directory.to_string().replace('\\', "/")),
        });
        Some(executable)
    }

    fn cleanup(&self) {
        self.staging.cleanup();
    }
}

struct StagingRoot {
    path: Utf8PathBuf,
    retained: bool,
}

impl StagingRoot {
    fn cleanup(&self) {
        let _ = ost_core::fs::remove_dir_all_robust(self.path.as_std_path());
    }

    fn retain(&mut self) {
        self.retained = true;
    }
}

impl Drop for StagingRoot {
    fn drop(&mut self) {
        if !self.retained {
            self.cleanup();
        }
    }
}

fn resolve_path(path: &Utf8Path) -> Result<ResolvedPath> {
    let absolute_path = absolute_from_current(path)?;
    let declared = FormationManifest::load(&absolute_path)?;
    let host_record = declared
        .host
        .as_ref()
        .map(|host| super::host::checked_pinned_host(&host.id, &host.fingerprint))
        .transpose()?;
    let store = ArtifactStore::discover();
    // Fresh extraction per invocation prevents modified retained `env` trees
    // from being trusted. The short root is independent of project/name depth.
    let nonce = digest::sha256_hex(format!("{}-{}", std::process::id(), now_nanos()).as_bytes());
    let staging_path = store.root().join("f").join(&nonce[7..23]);
    std::fs::create_dir_all(staging_path.parent().unwrap().as_std_path())
        .map_err(|error| Error::io(staging_path.to_string(), error))?;
    std::fs::create_dir(staging_path.as_std_path())
        .map_err(|error| Error::io(staging_path.to_string(), error))?;
    // Own cleanup only after exclusive creation succeeded.
    let staging = StagingRoot {
        path: staging_path,
        retained: false,
    };
    let staging_root = &staging.path;
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
    if let Some((host, python)) = host_record
        .as_ref()
        .and_then(|host| host.python.as_ref().map(|python| (host, python)))
    {
        let runtime_python = runtime_manifest
            .python
            .split('.')
            .take(2)
            .collect::<Vec<_>>()
            .join(".");
        if python.version != runtime_python {
            return Err(Error::coded(
                "HOST_PYTHON_ABI_MISMATCH",
                Category::Validation,
                format!(
                    "host '{}' embeds Python {}, but the pinned runtime uses {}",
                    host.id, python.version, runtime_manifest.python
                ),
            )
            .with_hint("select a runtime artifact built for the host's Python ABI"));
        }
    }
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
    let runtime_python = runtime_manifest.python.clone();
    let mut materialized = ost_formation::resolve(
        &declared,
        ResolutionInput {
            runtime_record,
            runtime_manifest,
            runtime_root: runtime_root.clone(),
            components,
        },
    )?;
    if let Some(host) = &host_record {
        append_host_environment(&mut materialized, host);
    }
    Ok(ResolvedPath {
        materialized,
        manifest_path: absolute_path,
        staging,
        runtime_root,
        runtime_python,
    })
}

/// The host contributes to the same Formation EnvSet and portable lock report
/// as the runtime and packaged components. The lock names host-relative paths;
/// only the materialized EnvSet carries the machine-local install root.
fn append_host_environment(materialized: &mut MaterializedFormation, host: &HostRecord) {
    let source = format!("host:{}", host.id);
    let mut directories = BTreeMap::new();
    for executable in host.executables.values() {
        let relative = Utf8Path::new(&executable.path)
            .parent()
            .unwrap_or(Utf8Path::new("."));
        let portable = format!("host/{}", relative.as_str().replace('\\', "/"));
        let absolute = host.root.join(relative).to_string().replace('\\', "/");
        directories.insert(portable, absolute);
    }
    // EnvSet::Prepend applies in sequence, so reverse the sorted declarations
    // to keep the portable report and effective PATH in the same order.
    for absolute in directories.values().rev() {
        materialized.env.vars.push(EnvVar {
            key: "PATH".into(),
            op: EnvOp::Prepend(absolute.clone()),
        });
    }
    materialized
        .resolved
        .environment
        .push(EnvironmentContribution {
            key: "PATH".into(),
            operation: "prepend".into(),
            source: source.clone(),
            paths: directories.into_keys().collect(),
        });

    let variable = match host.family {
        HostFamily::Maya => "MAYA_LOCATION",
        HostFamily::Houdini => "HFS",
    };
    materialized.env.vars.push(EnvVar {
        key: variable.into(),
        op: EnvOp::Set(host.root.to_string().replace('\\', "/")),
    });
    materialized
        .resolved
        .environment
        .push(EnvironmentContribution {
            key: variable.into(),
            operation: "set".into(),
            source,
            paths: vec!["host".into()],
        });
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
    // Bundle::load canonicalizes roots. Use the same identity here so Windows
    // short TEMP aliases (RUNNER~1) and their long names compare as one root.
    let canonical = std::fs::canonicalize(root.as_std_path())
        .map_err(|error| Error::io(root.to_string(), error))?;
    let canonical = Utf8PathBuf::from_path_buf(canonical)
        .map_err(|path| Error::config(format!("non-UTF-8 artifact path: {}", path.display())))?;
    #[cfg(windows)]
    {
        if let Some(rest) = canonical.as_str().strip_prefix(r"\\?\UNC\") {
            return Ok(Utf8PathBuf::from(format!(r"\\{rest}")));
        }
        if let Some(rest) = canonical.as_str().strip_prefix(r"\\?\") {
            return Ok(Utf8PathBuf::from(rest));
        }
    }
    Ok(canonical)
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
        ArtifactKind::Package => {
            let mut activation = activation_for_root(root)?;
            if let Some(contract) = &record.component {
                for contribution in &contract.environment {
                    if contribution.operation != ost_artifact::EnvironmentOperation::Prepend {
                        return Err(Error::validation(
                            "Formation component paths require prepend environment contributions",
                        ));
                    }
                    let paths = match contribution.variable.as_str() {
                        "PXR_PLUGINPATH_NAME" => &mut activation.plugin_paths,
                        "PYTHONPATH" => &mut activation.python_paths,
                        "LD_LIBRARY_PATH" | "DYLD_LIBRARY_PATH" => &mut activation.library_paths,
                        "PATH" => &mut activation.bin_paths,
                        _ => continue,
                    };
                    for relative in &contribution.values {
                        paths.push(safe_join(root, relative, "component environment path")?);
                    }
                }
            }
            Ok((Vec::new(), activation))
        }
        ArtifactKind::Runtime | ArtifactKind::ComposedRuntime => Err(Error::validation(
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
