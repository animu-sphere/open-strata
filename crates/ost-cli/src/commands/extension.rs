// SPDX-License-Identifier: Apache-2.0
//! `ost extension` — inspect and request controlled extensions (§4.4, §3.5).
//!
//! - `list` shows the extension catalog.
//! - `why` traces why an extension is required by a profile (the capability and
//!   feature that pull it in, directly or transitively).
//! - `add` records an extension in the project manifest.

use clap::Subcommand;

use ost_core::paths::{find_project_root, PROJECT_MANIFEST};
use ost_core::{Error, Result};
use ost_extension::{load_all, RequirementReason};

use crate::commands::configure::resolve_selection;
use crate::commands::resolve as resolve_runtime;
use crate::output::{self, Format};

#[derive(Debug, Subcommand)]
pub enum ExtensionCmd {
    /// List the known extensions.
    List,
    /// Explain why an extension is required by a profile.
    Why {
        /// Extension id, e.g. `materialx`.
        name: String,
        /// Profile to trace. Defaults to the project's profile.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Add an extension to the project manifest.
    Add {
        /// Extension id, e.g. `materialx`.
        name: String,
    },
}

pub fn run(cmd: ExtensionCmd, fmt: Format) -> Result<()> {
    match cmd {
        ExtensionCmd::List => list(fmt),
        ExtensionCmd::Why { name, profile } => why(&name, profile, fmt),
        ExtensionCmd::Add { name } => add(&name, fmt),
    }
}

fn list(fmt: Format) -> Result<()> {
    let catalog = load_all()?;

    if fmt.is_json() {
        let items: Vec<_> = catalog
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "type": e.kind,
                    "version": e.version,
                    "tier": e.tier,
                    "provides": e.provides.keys().collect::<Vec<_>>(),
                })
            })
            .collect();
        output::json(&serde_json::json!({ "extensions": items }));
        return Ok(());
    }

    println!(
        "{:<12}  {:<20}  {:<10}  PROVIDES",
        "EXTENSION", "TYPE", "VERSION"
    );
    for e in catalog.iter() {
        let provides: Vec<&str> = e.provides.keys().map(String::as_str).collect();
        println!(
            "{:<12}  {:<20}  {:<10}  {}",
            e.id,
            e.kind,
            e.version,
            provides.join(", ")
        );
    }
    Ok(())
}

fn why(name: &str, profile_override: Option<String>, fmt: Format) -> Result<()> {
    let (_root, platform, profile) = resolve_selection(None, profile_override)?;
    let r = resolve_runtime(&platform, &profile)?;
    let catalog = load_all()?;
    if catalog.get(name).is_none() {
        return Err(Error::usage(format!(
            "unknown extension '{name}' (try `ost extension list`)"
        )));
    }
    let resolution = ost_extension::resolve(&catalog, &r.capabilities);
    let reasons = ost_extension::why(&catalog, &resolution, name);
    let required = resolution.extensions.iter().any(|e| e.id == name);

    if fmt.is_json() {
        let rendered: Vec<String> = reasons.iter().map(|r| render_reason(name, r)).collect();
        output::json(&serde_json::json!({
            "extension": name,
            "platform": platform,
            "profile": profile,
            "required": required,
            "reasons": rendered,
        }));
        return Ok(());
    }

    println!("Why is '{name}' required by profile '{profile}'?");
    if reasons.is_empty() {
        println!("  It is not required by this profile.");
    } else {
        for reason in &reasons {
            println!("  - {}", render_reason(name, reason));
        }
    }
    Ok(())
}

/// Render a [`RequirementReason`] as a human-readable line.
fn render_reason(name: &str, reason: &RequirementReason) -> String {
    match reason {
        RequirementReason::Direct {
            capability,
            feature,
        } => match feature {
            Some(f) => format!("capability '{capability}' is provided by {name}[{f}]"),
            None => format!("capability '{capability}' is provided by {name}"),
        },
        RequirementReason::Transitive {
            extension,
            feature,
            capability,
        } => match capability {
            Some(c) => {
                format!("pulled in by {extension}[{feature}] (required by capability '{c}')")
            }
            None => format!("pulled in by {extension}[{feature}]"),
        },
    }
}

fn add(name: &str, fmt: Format) -> Result<()> {
    let catalog = load_all()?;
    if catalog.get(name).is_none() {
        return Err(Error::usage(format!(
            "unknown extension '{name}' (try `ost extension list`)"
        )));
    }

    let cwd = std::env::current_dir().map_err(|e| Error::io(".", e))?;
    let root =
        find_project_root(&cwd).ok_or_else(|| Error::ProjectNotFound(cwd.display().to_string()))?;
    let manifest_path = root.join(PROJECT_MANIFEST);
    let src = std::fs::read_to_string(&manifest_path)
        .map_err(|e| Error::io(manifest_path.display().to_string(), e))?;

    // Edit the manifest in place so comments and unmodelled sections survive.
    match ost_manifest::add_extension(&src, name)? {
        None => {
            if fmt.is_json() {
                output::json(
                    &serde_json::json!({ "added": false, "extension": name, "reason": "already present" }),
                );
            } else {
                println!("Extension '{name}' is already in the project manifest.");
            }
        }
        Some(updated) => {
            std::fs::write(&manifest_path, updated)
                .map_err(|e| Error::io(manifest_path.display().to_string(), e))?;
            if fmt.is_json() {
                output::json(&serde_json::json!({ "added": true, "extension": name }));
            } else {
                println!("Added extension '{name}' to {}", manifest_path.display());
            }
        }
    }
    Ok(())
}
