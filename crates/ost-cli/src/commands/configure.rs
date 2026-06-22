//! `ost configure` — generate CMake toolchain and presets for a target (§8).
//!
//! Resolves the project's platform+profile to a runtime, then writes
//! `.strata/targets/<id>/{toolchain.cmake,env.json,target.lock.json,
//! CMakePresets.json}` and updates the project-root `CMakePresets.json` to
//! include the per-target presets. CMake then drives the actual configure via
//! `cmake --preset <id>`.
//!
//! The generation here is shared with `ost build` via [`generate`].

use std::time::{SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};
use clap::Args;

use ost_build::{
    render_target_presets, render_toolchain, root_presets_with_include, Target, TargetLock,
};
use ost_core::paths::{find_project_root, PROJECT_MANIFEST, STATE_DIR};
use ost_core::{Error, Result};
use ost_manifest::Project;
use ost_runtime::{RuntimeManifest, MANIFEST_FILE};

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
}

/// The result of generating a target's CMake files.
pub(crate) struct Generated {
    pub id: String,
    pub target: Target,
    pub pulled: bool,
    pub root: Utf8PathBuf,
}

pub fn run(args: ConfigureArgs, fmt: Format) -> Result<()> {
    let (root, platform, profile) = resolve_selection(args.target, args.profile)?;
    let g = generate(&root, &platform, &profile)?;
    report(&g, fmt);
    Ok(())
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
        generator: "Ninja".to_string(),
    };
    Ok((target, r))
}

/// Resolve the runtime and write all of a target's CMake files. Returns the
/// generated target so callers (e.g. `ost build`) can act on it.
pub(crate) fn generate(root: &Utf8Path, platform: &str, profile: &str) -> Result<Generated> {
    let (target, r) = build_target(platform, profile)?;
    let id = target.id();

    let target_dir = root.join(STATE_DIR).join("targets").join(&id);
    std::fs::create_dir_all(target_dir.as_std_path())
        .map_err(|e| Error::io(target_dir.to_string(), e))?;

    // 1. toolchain.cmake
    write(&target_dir.join("toolchain.cmake"), &render_toolchain(&target, &r.prefix))?;

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
    let lock = TargetLock::from_target(&target, &toolchain_rel, created);
    let lock_json = lock
        .to_json()
        .map_err(|e| Error::parse("target.lock.json", anyhow::Error::new(e)))?;
    write(&target_dir.join("target.lock.json"), &lock_json)?;

    // 4. per-target CMakePresets.json
    write(
        &target_dir.join("CMakePresets.json"),
        &pretty(&render_target_presets(&target))?,
    )?;

    // 5. root CMakePresets.json (include the per-target file)
    let include_rel = format!(".strata/targets/{id}/CMakePresets.json");
    let root_presets_path = root.join("CMakePresets.json");
    let existing: Option<serde_json::Value> =
        std::fs::read_to_string(root_presets_path.as_std_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());
    let root_presets = root_presets_with_include(existing.as_ref(), &include_rel);
    write(&root_presets_path, &pretty(&root_presets)?)?;

    // 6. Refresh the project lockfile so it tracks the configured runtime.
    let lock = crate::commands::lock::build_lock(root, platform, profile)?;
    crate::commands::lock::write_lock(root, &lock)?;

    Ok(Generated {
        id,
        target,
        pulled: r.pulled,
        root: root.to_path_buf(),
    })
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
        output::json(&serde_json::json!({
            "configured": true,
            "target": id,
            "runtime": g.target.runtime_id,
            "pulled": g.pulled,
            "cxx_standard": g.target.cxx_standard,
            "generator": g.target.generator,
            "files": {
                "toolchain": format!(".strata/targets/{id}/toolchain.cmake"),
                "env": format!(".strata/targets/{id}/env.json"),
                "lock": format!(".strata/targets/{id}/target.lock.json"),
                "presets": format!(".strata/targets/{id}/CMakePresets.json"),
                "root_presets": "CMakePresets.json",
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
    println!("  updated CMakePresets.json (include) at project root");
    println!("  refreshed strata.lock");
    println!("\nNext:");
    println!("  cmake --preset {id}    (or `ost build`)");
}
