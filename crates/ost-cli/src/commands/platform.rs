//! `ost platform` — list / show / diff calendar-year definitions (§4.1).

use clap::Subcommand;

use ost_core::Result;
use ost_platform::{diff, Catalog, ComponentChange, Platform};

use crate::output::{self, Format};

#[derive(Debug, Subcommand)]
pub enum PlatformCmd {
    /// List all known platform definitions.
    List,
    /// Show the full definition of one platform year.
    Show {
        /// Platform id, e.g. `cy2026`.
        id: String,
    },
    /// Show component differences between two platform years.
    Diff {
        /// Older platform id, e.g. `cy2025`.
        from: String,
        /// Newer platform id, e.g. `cy2026`.
        to: String,
    },
}

pub fn run(cmd: PlatformCmd, fmt: Format) -> Result<()> {
    let catalog = Catalog::load()?;
    match cmd {
        PlatformCmd::List => list(&catalog, fmt),
        PlatformCmd::Show { id } => show(catalog.get(&id)?, fmt),
        PlatformCmd::Diff { from, to } => {
            let d = diff(catalog.get(&from)?, catalog.get(&to)?);
            show_diff(&d, fmt)
        }
    }
}

fn list(catalog: &Catalog, fmt: Format) -> Result<()> {
    if fmt.is_json() {
        let items: Vec<_> = catalog
            .iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id,
                    "status": p.source.status,
                    "python": p.component("python"),
                })
            })
            .collect();
        output::json(&serde_json::json!({ "platforms": items }));
        return Ok(());
    }

    println!("{:<10}  {:<11}  {}", "PLATFORM", "STATUS", "PYTHON");
    for p in catalog.iter() {
        let status = format!("{:?}", p.source.status).to_lowercase();
        println!(
            "{:<10}  {:<11}  {}",
            p.id,
            status,
            p.component("python").unwrap_or("-")
        );
    }
    Ok(())
}

fn show(p: &Platform, fmt: Format) -> Result<()> {
    if fmt.is_json() {
        output::json(&serde_json::to_value(p).expect("platform serializes"));
        return Ok(());
    }

    let status = format!("{:?}", p.source.status).to_lowercase();
    let kind = format!("{:?}", p.source.kind);
    println!("Platform: {}", p.id);
    println!("Source:   {kind} ({status})");
    println!("Components:");
    let width = p.core.keys().map(String::len).max().unwrap_or(0);
    for (name, version) in &p.core {
        println!("  {name:<width$}  {version}");
    }
    if let Some(notes) = &p.notes {
        println!("\nNotes:\n  {}", notes.trim());
    }
    Ok(())
}

fn show_diff(d: &ost_platform::PlatformDiff, fmt: Format) -> Result<()> {
    if fmt.is_json() {
        let changes: Vec<_> = d
            .changes
            .iter()
            .map(|(name, change)| match change {
                ComponentChange::Added { to } => serde_json::json!({
                    "component": name, "change": "added", "to": to,
                }),
                ComponentChange::Removed { from } => serde_json::json!({
                    "component": name, "change": "removed", "from": from,
                }),
                ComponentChange::Changed { from, to } => serde_json::json!({
                    "component": name, "change": "changed", "from": from, "to": to,
                }),
            })
            .collect();
        output::json(&serde_json::json!({
            "from": d.from_id,
            "to": d.to_id,
            "changes": changes,
        }));
        return Ok(());
    }

    println!("Diff {} -> {}", d.from_id, d.to_id);
    if d.is_empty() {
        println!("  (no component differences)");
        return Ok(());
    }
    let width = d.changes.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
    for (name, change) in &d.changes {
        match change {
            ComponentChange::Added { to } => println!("  + {name:<width$}  {to}"),
            ComponentChange::Removed { from } => println!("  - {name:<width$}  {from}"),
            ComponentChange::Changed { from, to } => {
                println!("  ~ {name:<width$}  {from} -> {to}")
            }
        }
    }
    Ok(())
}
