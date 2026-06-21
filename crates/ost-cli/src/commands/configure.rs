//! `ost configure` — generate CMake toolchain and presets for a target (§8).
//!
//! Resolves the project's platform+profile to a runtime, then writes
//! `.strata/targets/<id>/{toolchain.cmake,env.json,target.lock.json,
//! CMakePresets.json}` and updates the project-root `CMakePresets.json` to
//! include the per-target presets. CMake then drives the actual configure via
//! `cmake --preset <id>`.

use std::time::{SystemTime, UNIX_EPOCH};

use camino::Utf8PathBuf;
use clap::Args;

use ost_build::{
    render_target_presets, render_toolchain, root_presets_with_include, Target, TargetLock,
};
use ost_core::paths::{find_project_root, PROJECT_MANIFEST, STATE_DIR};
use ost_core::{Error, Result};
use ost_manifest::Project;
use ost_runtime::{RuntimeManifest, MANIFEST_FILE};

use crate::commands::resolve;
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

pub fn run(args: ConfigureArgs, fmt: Format) -> Result<()> {
    // Configure is project-centric: presets live at the project root.
    let cwd = std::env::current_dir().map_err(|e| Error::io(".", e))?;
    let root = find_project_root(&cwd)
        .ok_or_else(|| Error::ProjectNotFound(cwd.display().to_string()))?;
    let root = Utf8PathBuf::from_path_buf(root)
        .map_err(|p| Error::InvalidManifest(format!("non-UTF-8 project path: {}", p.display())))?;

    let manifest_src = std::fs::read_to_string(root.join(PROJECT_MANIFEST).as_std_path())
        .map_err(|e| Error::io(root.join(PROJECT_MANIFEST).to_string(), e))?;
    let project = Project::from_toml(&manifest_src)?;

    let platform = args
        .target
        .clone()
        .unwrap_or_else(|| project.requires.platform.clone());
    let profile = args
        .profile
        .clone()
        .unwrap_or_else(|| project.requires.profile.clone());

    let r = resolve(&platform, &profile)?;

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
        platform: platform.clone(),
        profile: profile.clone(),
        variant: r.runtime.variant.clone(),
        runtime_id: r.runtime.id(),
        runtime_digest,
        python_version: r.python_version.clone(),
        cxx_standard: r.cxx_standard.clone(),
        capabilities: r.capabilities.clone(),
        generator: "Ninja".to_string(),
    };
    let id = target.id();

    // .strata/targets/<id>/
    let target_dir = root.join(STATE_DIR).join("targets").join(&id);
    std::fs::create_dir_all(target_dir.as_std_path())
        .map_err(|e| Error::io(target_dir.to_string(), e))?;

    // 1. toolchain.cmake
    let toolchain = render_toolchain(&target, &r.prefix);
    let toolchain_path = target_dir.join("toolchain.cmake");
    write(&toolchain_path, &toolchain)?;

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
    let target_presets = render_target_presets(&target);
    write(
        &target_dir.join("CMakePresets.json"),
        &pretty(&target_presets)?,
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

    report(&id, &target, &r, &root, fmt);
    Ok(())
}

fn write(path: &Utf8PathBuf, contents: &str) -> Result<()> {
    std::fs::write(path.as_std_path(), format!("{contents}\n"))
        .map_err(|e| Error::io(path.to_string(), e))
}

fn pretty(value: &serde_json::Value) -> Result<String> {
    serde_json::to_string_pretty(value).map_err(|e| Error::parse("json", anyhow::Error::new(e)))
}

fn report(id: &str, target: &Target, r: &crate::commands::Resolved, root: &Utf8PathBuf, fmt: Format) {
    if fmt.is_json() {
        output::json(&serde_json::json!({
            "configured": true,
            "target": id,
            "runtime": target.runtime_id,
            "pulled": r.pulled,
            "cxx_standard": target.cxx_standard,
            "generator": target.generator,
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
    println!("  runtime:   {}", target.runtime_id);
    println!("  generator: {} (C++{})", target.generator, target.cxx_standard);
    if !r.pulled {
        println!("  warning:   runtime not pulled; toolchain paths are prospective");
        println!("             run `ost runtime pull {} --profile {}`", target.platform, target.profile);
    }
    println!("  generated under {}:", root.join(STATE_DIR).join("targets").join(id));
    println!("    toolchain.cmake  env.json  target.lock.json  CMakePresets.json");
    println!("  updated CMakePresets.json (include) at project root");
    println!("\nNext:");
    println!("  cmake --preset {id}");
}
