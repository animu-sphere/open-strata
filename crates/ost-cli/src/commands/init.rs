// SPDX-License-Identifier: Apache-2.0
//! `ost init` — scaffold an OpenStrata project in the current directory.
//!
//! By default this also scaffolds a minimal, buildable CMake project (the
//! `cpp-library` template) so `ost build` works right after `ost runtime pull`.
//! Use `--bare` to write only the manifest when adopting OpenStrata into an
//! existing CMake project.

use std::path::Path;

use camino::Utf8PathBuf;
use clap::Args;

use ost_core::paths::{PROJECT_MANIFEST, STATE_DIR};
use ost_core::{Error, Result};
use ost_manifest::Project;
use ost_platform::Catalog;

use crate::output::{self, Format};
use crate::project_template::{self, Template};

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Project name. Defaults to the current directory name.
    #[arg(long)]
    name: Option<String>,

    /// Platform calendar-year to target, e.g. `cy2026`. Defaults to the latest.
    #[arg(long)]
    platform: Option<String>,

    /// Project template to scaffold: `cpp-library` (default) or `usd-plugin`.
    #[arg(long, default_value = "cpp-library", conflicts_with = "bare")]
    template: String,

    /// Write only the manifest (no CMake files) — for an existing CMake project.
    #[arg(long)]
    bare: bool,

    /// Overwrite an existing manifest and template files if present.
    #[arg(long)]
    force: bool,
}

pub fn run(args: InitArgs, fmt: Format) -> Result<()> {
    let template = if args.bare {
        Template::Bare
    } else {
        Template::parse(&args.template)?
    };

    let cwd = std::env::current_dir().map_err(|e| Error::io(".", e))?;
    let root = Utf8PathBuf::from_path_buf(cwd.clone())
        .map_err(|p| Error::InvalidManifest(format!("non-UTF-8 project path: {}", p.display())))?;
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

    // Validate the name and detect template-file conflicts before any write, so
    // a failure leaves the work tree untouched. The name only needs to be a
    // portable identifier when it is substituted into template files; `--bare`
    // writes none, so any directory name accepted by the manifest is fine there
    // (VFX project dirs are routinely `2026_show`, `seq010`, `show.v2`, …).
    if template != Template::Bare {
        project_template::validate_name(&name)?;
    }
    if !args.force {
        if let Some(first) = project_template::conflicts(template, &name, &root).first() {
            return Err(Error::Operation(format!(
                "refusing to overwrite existing '{first}'; \
                 pass --force, or use --bare to skip template files"
            )));
        }
    }

    let project = Project::scaffold(&name, &platform);
    let toml = project.to_toml()?;
    std::fs::write(&manifest_path, toml)
        .map_err(|e| Error::io(manifest_path.display().to_string(), e))?;

    // Create the generated-state directory and keep it out of git.
    let state_dir = cwd.join(STATE_DIR);
    std::fs::create_dir_all(&state_dir)
        .map_err(|e| Error::io(state_dir.display().to_string(), e))?;
    write_state_gitignore(&state_dir)?;

    let scaffolded = project_template::scaffold(template, &name, &root, args.force)?;

    report(&manifest_path, &name, &platform, template, &scaffolded, fmt);
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
    std::fs::write(&gitignore, "*\n").map_err(|e| Error::io(gitignore.display().to_string(), e))
}

fn report(
    manifest_path: &Path,
    name: &str,
    platform: &str,
    template: Template,
    scaffolded: &[Utf8PathBuf],
    fmt: Format,
) {
    if fmt.is_json() {
        output::success(&serde_json::json!({
            "initialized": true,
            "manifest": manifest_path.display().to_string(),
            "name": name,
            "platform": platform,
            "template": template.as_str(),
            "files": scaffolded.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
        }));
        return;
    }
    println!("Initialized OpenStrata project '{name}'");
    println!("  manifest: {}", manifest_path.display());
    println!("  platform: {platform}");
    println!("  template: {}", template.as_str());
    if !scaffolded.is_empty() {
        println!("  files:");
        for f in scaffolded {
            println!("    {f}");
        }
    }
    println!("\nNext:");
    if template == Template::Bare {
        println!("  (bare) ensure your project has a CMakeLists.txt, then:");
    }
    println!("  ost runtime pull {platform} --profile <profile>");
    println!("  ost build");
}
