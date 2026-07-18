// SPDX-License-Identifier: Apache-2.0
//! `ost configure` — generate CMake toolchain and presets for a target (§8).
//!
//! Resolves the project's platform+profile to a runtime, then writes
//! `.strata/targets/<id>/{toolchain.cmake,env.json,target.lock.json,
//! CMakePresets.json}` and refreshes the tool-owned `CMakeUserPresets.json` to
//! include the per-target presets. CMake then drives the actual configure via
//! `cmake --preset <id>`.
//!
//! The user's committed `CMakePresets.json` is never touched by default; see
//! `ost presets` to wire the includes into it explicitly and non-destructively.
//!
//! The generation here is shared with `ost build` via [`generate`].

use std::time::{SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};
use clap::Args;

use serde_json::{Map, Value};

use ost_build::{
    ensure_includes, includes_of, managed_include, render_target_presets, render_toolchain,
    Compiler, LeaseMode, Target, TargetLease, TargetLock, TARGET_LEASE_FILE,
};
use ost_core::fs::write_atomic;
use ost_core::paths::{find_project_root, PROJECT_MANIFEST, STATE_DIR};
use ost_core::{Error, Result};
use ost_manifest::Project;
use ost_runtime::{RuntimeManifest, MANIFEST_FILE};

use crate::commands::compiler::{self, CompilerOpts};
use crate::commands::presets::{read_presets_object, ROOT_PRESETS, USER_PRESETS};
use crate::commands::{resolve, Resolved};
use crate::output::{self, Format};

#[derive(Debug, Args)]
pub struct ConfigureArgs {
    /// Platform target, e.g. `cy2026`. Defaults to the project's platform.
    #[arg(long)]
    target: Option<String>,

    /// Profile to build. Defaults to the project's profile.
    #[arg(long)]
    profile: Option<String>,

    /// What to do when another invocation is already writing this target:
    /// `fail` immediately, `wait` for it (see --busy-timeout), or `read-only`
    /// to proceed without taking the target lease.
    #[arg(long, default_value = "fail", value_parser = ["fail", "wait", "read-only"])]
    on_busy: String,

    /// How long `--on-busy wait` waits, in seconds; 0 waits indefinitely.
    #[arg(long, default_value_t = 600)]
    busy_timeout: u64,

    #[command(flatten)]
    compiler: CompilerOpts,
}

/// The result of generating a target's CMake files.
pub(crate) struct Generated {
    pub id: String,
    pub target: Target,
    pub pulled: bool,
    pub root: Utf8PathBuf,
    pub compiler: Compiler,
}

pub fn run(args: ConfigureArgs, fmt: Format) -> Result<()> {
    let (root, platform, profile) = resolve_selection(args.target, args.profile)?;
    let compiler = resolve_compiler(&root, &args.compiler)?;

    // Resolve the target id without writing anything, so the lease is held
    // before the first generated file lands. `ost build` takes the same lease
    // around its own call to `generate`, which is why it is taken here rather
    // than inside `generate` — nesting the two would deadlock a build against
    // itself.
    let (target, _) = build_target(&platform, &profile)?;
    let id = target.id();
    let mode = LeaseMode::parse(&args.on_busy, args.busy_timeout)?;
    let lease_path = root
        .join(STATE_DIR)
        .join("targets")
        .join(&id)
        .join(TARGET_LEASE_FILE);
    let lease = TargetLease::acquire(&lease_path, &id, "ost configure", mode)?;
    if let Some(takeover) = lease.takeover() {
        eprintln!("warning: {}", takeover.describe());
    }

    let g = generate(&root, &platform, &profile, &compiler)?;
    lease.release();
    report(&g, fmt);
    Ok(())
}

/// Resolve the compiler policy from CLI flags over the project's `[build]` table.
pub(crate) fn resolve_compiler(root: &Utf8Path, opts: &CompilerOpts) -> Result<Compiler> {
    let build = load_project(root)?.build;
    compiler::resolve(opts, build.as_ref())
}

/// Find the project root and resolve the effective platform + profile, applying
/// CLI overrides over the values in `openstrata.toml`. Shared by configure/build.
pub(crate) fn resolve_selection(
    target: Option<String>,
    profile: Option<String>,
) -> Result<(Utf8PathBuf, String, String)> {
    let cwd = std::env::current_dir().map_err(|e| Error::io(".", e))?;
    let root =
        find_project_root(&cwd).ok_or_else(|| Error::ProjectNotFound(cwd.display().to_string()))?;
    let root = Utf8PathBuf::from_path_buf(root)
        .map_err(|p| Error::InvalidManifest(format!("non-UTF-8 project path: {}", p.display())))?;

    let manifest_path = root.join(PROJECT_MANIFEST);
    let manifest_src = std::fs::read_to_string(manifest_path.as_std_path())
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;
    let project = Project::from_toml(&manifest_src)?;

    let platform = target.unwrap_or(project.requires.platform);
    let profile = profile.unwrap_or(project.requires.profile);
    Ok((root, platform, profile))
}

/// Load the project manifest at a known project root.
pub(crate) fn load_project(root: &Utf8Path) -> Result<Project> {
    let manifest_path = root.join(PROJECT_MANIFEST);
    let src = std::fs::read_to_string(manifest_path.as_std_path())
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;
    Project::from_toml(&src)
}

/// Resolve a platform+profile into a build [`Target`] and its [`Resolved`]
/// runtime, without writing anything. Shared by configure/build/package.
pub(crate) fn build_target(platform: &str, profile: &str) -> Result<(Target, Resolved)> {
    build_target_with_generator(platform, profile, "Ninja")
}

pub(crate) fn build_target_with_generator(
    platform: &str,
    profile: &str,
    generator: &str,
) -> Result<(Target, Resolved)> {
    let r = resolve(platform, profile)?;

    // Pull the digest from the runtime manifest when available.
    let runtime_digest = if r.pulled {
        std::fs::read_to_string(r.prefix.join(MANIFEST_FILE).as_std_path())
            .ok()
            .and_then(|s| RuntimeManifest::from_json(&s).ok())
            .map(|m| m.digest)
            .unwrap_or_default()
    } else {
        String::new()
    };

    let target = Target {
        platform: platform.to_string(),
        profile: profile.to_string(),
        variant: r.runtime.variant.clone(),
        runtime_id: r.runtime.id(),
        runtime_digest,
        python_version: r.python_version.clone(),
        cxx_standard: r.cxx_standard.clone(),
        capabilities: r.capabilities.clone(),
        generator: generator.to_string(),
    };
    Ok((target, r))
}

/// Resolve the runtime and write all of a target's CMake files. Returns the
/// generated target so callers (e.g. `ost build`) can act on it.
pub(crate) fn generate(
    root: &Utf8Path,
    platform: &str,
    profile: &str,
    compiler: &Compiler,
) -> Result<Generated> {
    generate_with_generator(root, platform, profile, compiler, "Ninja")
}

pub(crate) fn generate_with_generator(
    root: &Utf8Path,
    platform: &str,
    profile: &str,
    compiler: &Compiler,
    generator: &str,
) -> Result<Generated> {
    let (target, r) = build_target_with_generator(platform, profile, generator)?;
    let id = target.id();

    let target_dir = root.join(STATE_DIR).join("targets").join(&id);
    std::fs::create_dir_all(target_dir.as_std_path())
        .map_err(|e| Error::io(target_dir.to_string(), e))?;

    // The compiler record (policy + resolved paths + versions) goes in the lock.
    let lock_compiler = compiler::to_lock(compiler, &r.artifact_prefix, target.os());

    // If a previous configure used a different compiler, the CMake cache under
    // build/<id> is stale (cached compiler/ABI) — drop it so the next configure
    // is clean. The toolchain/presets themselves are always regenerated below.
    invalidate_build_tree_if_configuration_changed(root, &id, &lock_compiler, generator);

    // 1. toolchain.cmake — pin a host interpreter's Development artifacts so
    // an adopted runtime's pxrConfig (which bakes the export machine's Python
    // paths) configures on this host; `None` falls back to the runtime prefix.
    let python = ost_build::resolve_for_runtime(&r.artifact_prefix, &target.python_version);
    crate::commands::relocate_baked_python_if_stale(&r.artifact_prefix, python.as_ref());
    write(
        &target_dir.join("toolchain.cmake"),
        &render_toolchain(&target, &r.artifact_prefix, compiler, python.as_ref()),
    )?;

    // 2. env.json (resolved env for build steps to reuse)
    let env_vars: Vec<_> = r
        .env
        .pairs()
        .into_iter()
        .map(|(k, v)| serde_json::json!({ "name": k, "value": v }))
        .collect();
    let env_json = serde_json::json!({
        "runtime": target.runtime_id,
        "prefix": r.prefix.to_string(),
        "vars": env_vars,
    });
    write(&target_dir.join("env.json"), &pretty(&env_json)?)?;

    // 3. target.lock.json
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let toolchain_rel = format!(".strata/targets/{id}/toolchain.cmake");
    let lock = TargetLock::from_target(&target, lock_compiler, &toolchain_rel, created);
    let lock_json = lock
        .to_json()
        .map_err(|e| Error::parse("target.lock.json", anyhow::Error::new(e)))?;
    write(&target_dir.join("target.lock.json"), &lock_json)?;

    // 4. per-target CMakePresets.json
    write(
        &target_dir.join("CMakePresets.json"),
        &pretty(&render_target_presets(&target))?,
    )?;

    // 5. Wire the per-target presets into the tool-owned CMakeUserPresets.json
    //    so `cmake --preset <id>` works out of the box. We never touch the
    //    user's CMakePresets.json by default (see `ost presets install`).
    refresh_user_presets(root, &id)?;

    // 6. Refresh the project lockfile so it tracks the configured runtime.
    let lock = crate::commands::lock::build_lock(root, platform, profile)?;
    crate::commands::lock::write_lock(root, &lock)?;

    Ok(Generated {
        id,
        target,
        pulled: r.pulled,
        root: root.to_path_buf(),
        compiler: compiler.clone(),
    })
}

/// The files [`generate`] writes for a target, relative to the project root.
///
/// Single source of truth shared by `ost build`'s `--dry-run` planner so it
/// cannot drift from what a real build actually produces. Keep this in lockstep
/// with the writes in [`generate`].
pub(crate) fn target_output_paths(id: &str) -> Vec<String> {
    let t = format!("{STATE_DIR}/targets/{id}");
    vec![
        format!("{t}/toolchain.cmake"),
        format!("{t}/env.json"),
        format!("{t}/target.lock.json"),
        format!("{t}/CMakePresets.json"),
        USER_PRESETS.to_string(),
        "strata.lock".to_string(),
    ]
}

/// Remove `build/<id>` when the compiler differs from the last configure.
///
/// CMake caches the compiler and its ABI on first configure; reusing that cache
/// with a different compiler produces incoherent builds. We compare only the
/// compiler fingerprint (policy + paths) recorded in the previous
/// `target.lock.json`; a missing/unreadable lock means nothing to invalidate.
fn invalidate_build_tree_if_configuration_changed(
    root: &Utf8Path,
    id: &str,
    next: &ost_build::LockCompiler,
    generator: &str,
) {
    let lock_path = root
        .join(STATE_DIR)
        .join("targets")
        .join(id)
        .join("target.lock.json");
    let previous = std::fs::read_to_string(lock_path.as_std_path())
        .ok()
        .and_then(|s| serde_json::from_str::<TargetLock>(&s).ok());

    if let Some(prev) = previous {
        if prev.compiler.fingerprint() != next.fingerprint() || prev.generator != generator {
            let build_dir = root.join("build").join(id);
            if build_dir.as_std_path().exists() {
                let _ = std::fs::remove_dir_all(build_dir.as_std_path());
            }
        }
    }
}

/// Ensure OpenStrata's `CMakeUserPresets.json` includes target `id`'s presets.
///
/// `CMakeUserPresets.json` is tool-owned and developer-local (git-ignored), so
/// refreshing it never disturbs the user's committed `CMakePresets.json`. If the
/// user has explicitly wired this include into their own `CMakePresets.json`
/// (via `ost presets install`), we skip it here — CMake errors on a preset name
/// defined twice.
fn refresh_user_presets(root: &Utf8Path, id: &str) -> Result<()> {
    let include = managed_include(id);

    // Parse-or-error: a malformed CMakePresets.json is never treated as empty.
    let root_presets_path = root.join(ROOT_PRESETS);
    if let Some(map) = read_presets_object(&root_presets_path)? {
        if includes_of(&Value::Object(map))
            .iter()
            .any(|i| i == &include)
        {
            return Ok(());
        }
    }

    let user_path = root.join(USER_PRESETS);
    let mut map: Map<String, Value> = read_presets_object(&user_path)?.unwrap_or_default();
    ensure_includes(&mut map, std::slice::from_ref(&include));
    let body = pretty(&Value::Object(map))?;
    write_atomic(user_path.as_std_path(), format!("{body}\n").as_bytes())?;

    ignore_user_presets(root);
    Ok(())
}

/// Best-effort: keep `CMakeUserPresets.json` out of git (it is developer-local).
/// Appends a single idempotent line to the project root `.gitignore`; failures
/// are non-fatal since this is a convenience, not correctness.
fn ignore_user_presets(root: &Utf8Path) {
    let path = root.join(".gitignore");
    let current = std::fs::read_to_string(path.as_std_path()).unwrap_or_default();
    if current.lines().any(|l| l.trim() == USER_PRESETS) {
        return;
    }
    let mut next = current;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(USER_PRESETS);
    next.push('\n');
    let _ = std::fs::write(path.as_std_path(), next);
}

fn write(path: &Utf8PathBuf, contents: &str) -> Result<()> {
    std::fs::write(path.as_std_path(), format!("{contents}\n"))
        .map_err(|e| Error::io(path.to_string(), e))
}

fn pretty(value: &serde_json::Value) -> Result<String> {
    serde_json::to_string_pretty(value).map_err(|e| Error::parse("json", anyhow::Error::new(e)))
}

fn report(g: &Generated, fmt: Format) {
    let id = &g.id;
    if fmt.is_json() {
        output::success(&serde_json::json!({
            "configured": true,
            "target": id,
            "runtime": g.target.runtime_id,
            "pulled": g.pulled,
            "cxx_standard": g.target.cxx_standard,
            "generator": g.target.generator,
            "compiler": g.compiler.policy(),
            "files": {
                "toolchain": format!(".strata/targets/{id}/toolchain.cmake"),
                "env": format!(".strata/targets/{id}/env.json"),
                "lock": format!(".strata/targets/{id}/target.lock.json"),
                "presets": format!(".strata/targets/{id}/CMakePresets.json"),
                "user_presets": USER_PRESETS,
            },
        }));
        return;
    }

    println!("Configured target {id}");
    println!("  runtime:   {}", g.target.runtime_id);
    println!(
        "  generator: {} (C++{})",
        g.target.generator, g.target.cxx_standard
    );
    println!("  compiler:  {}", g.compiler.policy());
    if !g.pulled {
        println!("  warning:   runtime not pulled; toolchain paths are prospective");
        println!(
            "             run `ost runtime pull {} --profile {}`",
            g.target.platform, g.target.profile
        );
    }
    println!(
        "  generated under {}:",
        g.root.join(STATE_DIR).join("targets").join(id)
    );
    println!("    toolchain.cmake  env.json  target.lock.json  CMakePresets.json");
    println!("  refreshed {USER_PRESETS} (your CMakePresets.json is untouched)");
    println!("  refreshed strata.lock");
    println!("\nNext:");
    println!("  cmake --preset {id}    (or `ost build`)");
    println!("  to wire presets into your committed CMakePresets.json: `ost presets install`");
}
