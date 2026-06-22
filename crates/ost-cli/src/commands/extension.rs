//! `ost extension` — inspect and request controlled extensions (§4.4, §3.5).
//!
//! - `list` shows the extension catalog.
//! - `why` traces why an extension is required by a profile (the capability and
//!   feature that pull it in, directly or transitively).
//! - `add` records an extension in the project manifest.

use clap::Subcommand;

use ost_core::paths::PROJECT_MANIFEST;
use ost_core::{Error, Result};
use ost_extension::load_all;

use crate::commands::configure::{load_project, resolve_selection};
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

    println!("{:<12}  {:<20}  {:<10}  {}", "EXTENSION", "TYPE", "VERSION", "PROVIDES");
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
        return Err(Error::Operation(format!(
            "unknown extension '{name}' (try `ost extension list`)"
        )));
    }
    let resolution = ost_extension::resolve(&catalog, &r.capabilities);

    let mut reasons: Vec<String> = Vec::new();

    // Direct: a requested capability is provided by this extension.
    for edge in &resolution.edges {
        if edge.extension.as_deref() == Some(name) {
            match &edge.feature {
                Some(f) => reasons.push(format!(
                    "capability '{}' is provided by {name}[{f}]",
                    edge.capability
                )),
                None => reasons.push(format!(
                    "capability '{}' is provided by {name}",
                    edge.capability
                )),
            }
        }
    }

    // Transitive: another resolved extension's feature requires this one.
    for ext in &resolution.extensions {
        if ext.id == name {
            continue;
        }
        let Some(src) = catalog.get(&ext.id) else { continue };
        for feature in &ext.features {
            let Some(spec) = src.feature(feature) else { continue };
            if spec.requires_extensions.iter().any(|d| d == name) {
                let cap = resolution
                    .edges
                    .iter()
                    .find(|e| {
                        e.extension.as_deref() == Some(ext.id.as_str())
                            && e.feature.as_deref() == Some(feature.as_str())
                    })
                    .map(|e| e.capability.clone());
                match cap {
                    Some(c) => reasons.push(format!(
                        "pulled in by {}[{feature}] (required by capability '{c}')",
                        ext.id
                    )),
                    None => reasons.push(format!("pulled in by {}[{feature}]", ext.id)),
                }
            }
        }
    }

    let required = resolution.extensions.iter().any(|e| e.id == name);

    if fmt.is_json() {
        output::json(&serde_json::json!({
            "extension": name,
            "platform": platform,
            "profile": profile,
            "required": required,
            "reasons": reasons,
        }));
        return Ok(());
    }

    println!("Why is '{name}' required by profile '{profile}'?");
    if reasons.is_empty() {
        println!("  It is not required by this profile.");
    } else {
        for reason in &reasons {
            println!("  - {reason}");
        }
    }
    Ok(())
}

fn add(name: &str, fmt: Format) -> Result<()> {
    let (root, _platform, _profile) = resolve_selection(None, None)?;

    let catalog = load_all()?;
    if catalog.get(name).is_none() {
        return Err(Error::Operation(format!(
            "unknown extension '{name}' (try `ost extension list`)"
        )));
    }

    let mut project = load_project(&root)?;
    if project.requires.extensions.iter().any(|e| e == name) {
        if fmt.is_json() {
            output::json(&serde_json::json!({ "added": false, "extension": name, "reason": "already present" }));
        } else {
            println!("Extension '{name}' is already in the project manifest.");
        }
        return Ok(());
    }

    project.requires.extensions.push(name.to_string());
    project.requires.extensions.sort();
    let toml = project.to_toml()?;
    let manifest_path = root.join(PROJECT_MANIFEST);
    std::fs::write(manifest_path.as_std_path(), toml)
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;

    if fmt.is_json() {
        output::json(&serde_json::json!({ "added": true, "extension": name }));
    } else {
        println!("Added extension '{name}' to {}", manifest_path);
    }
    Ok(())
}
