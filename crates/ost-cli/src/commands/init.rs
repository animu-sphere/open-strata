// SPDX-License-Identifier: Apache-2.0
//! `ost init` — scaffold an OpenStrata project in the current directory.

use std::path::Path;

use clap::Args;

use ost_core::paths::{PROJECT_MANIFEST, STATE_DIR};
use ost_core::{Error, Result};
use ost_manifest::Project;
use ost_platform::Catalog;

use crate::output::{self, Format};

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Project name. Defaults to the current directory name.
    #[arg(long)]
    name: Option<String>,

    /// Platform calendar-year to target, e.g. `cy2026`. Defaults to the latest.
    #[arg(long)]
    platform: Option<String>,

    /// Overwrite an existing manifest if one is present.
    #[arg(long)]
    force: bool,
}

pub fn run(args: InitArgs, fmt: Format) -> Result<()> {
    let cwd = std::env::current_dir().map_err(|e| Error::io(".", e))?;
    let manifest_path = cwd.join(PROJECT_MANIFEST);

    if manifest_path.exists() && !args.force {
        return Err(Error::ProjectExists(manifest_path.display().to_string()));
    }

    let catalog = Catalog::load()?;
    let platform = match args.platform {
        Some(id) => {
            // Validate the requested platform exists before writing anything.
            catalog.get(&id)?;
            id
        }
        None => latest_platform(&catalog)?,
    };

    let name = args.name.unwrap_or_else(|| {
        cwd.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("openstrata-project")
            .to_string()
    });

    let project = Project::scaffold(&name, &platform);
    let toml = project.to_toml()?;
    std::fs::write(&manifest_path, toml)
        .map_err(|e| Error::io(manifest_path.display().to_string(), e))?;

    // Create the generated-state directory and keep it out of git.
    let state_dir = cwd.join(STATE_DIR);
    std::fs::create_dir_all(&state_dir)
        .map_err(|e| Error::io(state_dir.display().to_string(), e))?;
    write_state_gitignore(&state_dir)?;

    report(&manifest_path, &name, &platform, fmt);
    Ok(())
}

/// The newest platform by id (BTreeMap order means the last entry wins).
fn latest_platform(catalog: &Catalog) -> Result<String> {
    catalog
        .iter()
        .last()
        .map(|p| p.id.clone())
        .ok_or_else(|| Error::InvalidManifest("no platform definitions available".into()))
}

fn write_state_gitignore(state_dir: &std::path::Path) -> Result<()> {
    let gitignore = state_dir.join(".gitignore");
    if gitignore.exists() {
        return Ok(());
    }
    // The whole generated-state tree is reproducible; never commit it.
    std::fs::write(&gitignore, "*\n")
        .map_err(|e| Error::io(gitignore.display().to_string(), e))
}

fn report(manifest_path: &Path, name: &str, platform: &str, fmt: Format) {
    if fmt.is_json() {
        output::json(&serde_json::json!({
            "initialized": true,
            "manifest": manifest_path.display().to_string(),
            "name": name,
            "platform": platform,
        }));
        return;
    }
    println!("Initialized OpenStrata project '{name}'");
    println!("  manifest: {}", manifest_path.display());
    println!("  platform: {platform}");
    println!("\nNext:");
    println!("  ost platform show {platform}");
}
