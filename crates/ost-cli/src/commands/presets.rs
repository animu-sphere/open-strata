// SPDX-License-Identifier: Apache-2.0
//! `ost presets` — manage OpenStrata's CMake preset includes (§8.3).
//!
//! By default `ost configure`/`ost build` keep their preset includes in the
//! tool-owned `CMakeUserPresets.json` and never touch the user's committed
//! `CMakePresets.json`. These commands let a user opt in to wiring those
//! includes into their `CMakePresets.json` (e.g. to commit them), non-
//! destructively:
//!
//! * `install`   — add the per-target includes, preserving every other field.
//! * `diff`      — show what `install` would change, without writing.
//! * `uninstall` — remove only the OpenStrata-managed includes.
//!
//! Writes are atomic (temp file → fsync → rename) and a malformed
//! `CMakePresets.json` is always an error, never silently overwritten.

use camino::Utf8Path;
use clap::{Args, Subcommand};
use serde_json::{Map, Value};

use ost_build::{ensure_includes, includes_of, managed_include, remove_managed_includes};
use ost_core::fs::write_atomic;
use ost_core::paths::STATE_DIR;
use ost_core::{Error, Result};

use crate::commands::configure::resolve_selection;
use crate::output::{self, Format};

/// The user's committed root presets file (never written without an explicit
/// `ost presets install`).
pub const ROOT_PRESETS: &str = "CMakePresets.json";

/// OpenStrata's tool-owned, developer-local presets file.
pub const USER_PRESETS: &str = "CMakeUserPresets.json";

#[derive(Debug, Subcommand)]
pub enum PresetsCmd {
    /// Wire OpenStrata's per-target presets into the project CMakePresets.json.
    Install(InstallArgs),

    /// Show what `install` would add to CMakePresets.json, without writing.
    Diff,

    /// Remove OpenStrata-managed includes from the project CMakePresets.json.
    Uninstall(UninstallArgs),
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    /// Show the planned changes without writing.
    #[arg(long)]
    dry_run: bool,

    /// Back up CMakePresets.json to CMakePresets.json.bak before writing.
    #[arg(long)]
    backup: bool,
}

#[derive(Debug, Args)]
pub struct UninstallArgs {
    /// Show the planned changes without writing.
    #[arg(long)]
    dry_run: bool,

    /// Back up CMakePresets.json to CMakePresets.json.bak before writing.
    #[arg(long)]
    backup: bool,
}

pub fn run(cmd: PresetsCmd, fmt: Format) -> Result<()> {
    // Resolve the project root (platform/profile are irrelevant here).
    let (root, _, _) = resolve_selection(None, None)?;
    match cmd {
        PresetsCmd::Install(args) => install(&root, args, fmt),
        PresetsCmd::Diff => diff(&root, fmt),
        PresetsCmd::Uninstall(args) => uninstall(&root, args, fmt),
    }
}

fn install(root: &Utf8Path, args: InstallArgs, fmt: Format) -> Result<()> {
    let path = root.join(ROOT_PRESETS);
    let wants = configured_includes(root)?;
    if wants.is_empty() {
        return Err(Error::Operation(
            "no configured targets found — run `ost configure` first".to_string(),
        ));
    }

    let mut map = read_presets_object(&path)?.unwrap_or_default();
    let before = includes_of(&Value::Object(map.clone()));
    let changed = ensure_includes(&mut map, &wants);
    let after = includes_of(&Value::Object(map.clone()));
    let added: Vec<String> = after.into_iter().filter(|i| !before.contains(i)).collect();

    if args.dry_run {
        report_changes(fmt, "install", &path, &added, &[], true);
        return Ok(());
    }
    if !changed {
        report_changes(fmt, "install", &path, &[], &[], false);
        return Ok(());
    }

    if args.backup {
        back_up(&path)?;
    }
    write_object(&path, &map)?;
    // The includes now live in CMakePresets.json; drop them from the tool-owned
    // file so the same preset name is never defined in both (CMake errors).
    prune_user_presets(root)?;

    report_changes(fmt, "install", &path, &added, &[], false);
    Ok(())
}

fn diff(root: &Utf8Path, fmt: Format) -> Result<()> {
    let path = root.join(ROOT_PRESETS);
    let wants = configured_includes(root)?;
    let mut map = read_presets_object(&path)?.unwrap_or_default();
    let before = includes_of(&Value::Object(map.clone()));
    ensure_includes(&mut map, &wants);
    let after = includes_of(&Value::Object(map));
    let added: Vec<String> = after.into_iter().filter(|i| !before.contains(i)).collect();

    report_changes(fmt, "diff", &path, &added, &[], true);
    Ok(())
}

fn uninstall(root: &Utf8Path, args: UninstallArgs, fmt: Format) -> Result<()> {
    let path = root.join(ROOT_PRESETS);
    let Some(mut map) = read_presets_object(&path)? else {
        report_changes(fmt, "uninstall", &path, &[], &[], false);
        return Ok(());
    };
    let removed = remove_managed_includes(&mut map);

    if args.dry_run {
        report_changes(fmt, "uninstall", &path, &[], &removed, true);
        return Ok(());
    }
    if removed.is_empty() {
        report_changes(fmt, "uninstall", &path, &[], &[], false);
        return Ok(());
    }

    if args.backup {
        back_up(&path)?;
    }
    write_object(&path, &map)?;
    report_changes(fmt, "uninstall", &path, &[], &removed, false);
    Ok(())
}

/// Parse a presets file into its top-level object.
///
/// `Ok(None)` only when the file does not exist. A read error, invalid JSON
/// (including JSON-with-comments, which is unsupported), or a non-object root is
/// an error — never silently treated as empty.
pub fn read_presets_object(path: &Utf8Path) -> Result<Option<Map<String, Value>>> {
    let text = match std::fs::read_to_string(path.as_std_path()) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io(path.to_string(), e)),
    };
    let value: Value = serde_json::from_str(&text).map_err(|e| {
        Error::Operation(format!(
            "{path} is not valid JSON, refusing to overwrite it \
             (note: JSON with comments is not supported): {e}"
        ))
    })?;
    match value {
        Value::Object(map) => Ok(Some(map)),
        _ => Err(Error::Operation(format!(
            "{path} must contain a JSON object at the top level"
        ))),
    }
}

/// The OpenStrata-managed includes for every configured target on disk.
fn configured_includes(root: &Utf8Path) -> Result<Vec<String>> {
    let targets = root.join(STATE_DIR).join("targets");
    let mut out = Vec::new();
    match std::fs::read_dir(targets.as_std_path()) {
        Ok(entries) => {
            for entry in entries.flatten() {
                if entry.path().join(ROOT_PRESETS).is_file() {
                    if let Some(name) = entry.file_name().to_str() {
                        out.push(managed_include(name));
                    }
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(Error::io(targets.to_string(), e)),
    }
    out.sort();
    Ok(out)
}

/// Remove all OpenStrata-managed includes from `CMakeUserPresets.json`; delete
/// the file if nothing meaningful remains.
fn prune_user_presets(root: &Utf8Path) -> Result<()> {
    let path = root.join(USER_PRESETS);
    let Some(mut map) = read_presets_object(&path)? else {
        return Ok(());
    };
    if remove_managed_includes(&mut map).is_empty() {
        return Ok(());
    }
    // If only `version` is left, the tool-owned file has no purpose anymore.
    let trivial = map.keys().all(|k| k == "version");
    if trivial {
        std::fs::remove_file(path.as_std_path()).map_err(|e| Error::io(path.to_string(), e))?;
    } else {
        write_object(&path, &map)?;
    }
    Ok(())
}

fn back_up(path: &Utf8Path) -> Result<()> {
    if !path.as_std_path().exists() {
        return Ok(());
    }
    let backup = format!("{path}.bak");
    std::fs::copy(path.as_std_path(), &backup)
        .map(|_| ())
        .map_err(|e| Error::io(backup, e))
}

fn write_object(path: &Utf8Path, map: &Map<String, Value>) -> Result<()> {
    let body = serde_json::to_string_pretty(&Value::Object(map.clone()))
        .map_err(|e| Error::parse(path.to_string(), anyhow::Error::new(e)))?;
    write_atomic(path.as_std_path(), format!("{body}\n").as_bytes())
}

fn report_changes(
    fmt: Format,
    action: &str,
    path: &Utf8Path,
    added: &[String],
    removed: &[String],
    planned: bool,
) {
    if fmt.is_json() {
        output::json(&serde_json::json!({
            "action": action,
            "file": path.to_string(),
            "planned": planned,
            "added": added,
            "removed": removed,
        }));
        return;
    }

    let verb = if planned { "would update" } else { "updated" };
    if added.is_empty() && removed.is_empty() {
        println!("{path}: already up to date — nothing to {action}.");
        return;
    }
    println!("{verb} {path}:");
    for a in added {
        println!("  + include {a}");
    }
    for r in removed {
        println!("  - include {r}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;

    fn scratch(name: &str) -> Utf8PathBuf {
        let dir = std::env::temp_dir().join(format!("ost-presets-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Utf8PathBuf::from_path_buf(dir.join("CMakePresets.json")).unwrap()
    }

    #[test]
    fn missing_file_is_none_not_error() {
        let path = scratch("missing").with_file_name("does-not-exist.json");
        assert!(read_presets_object(&path).unwrap().is_none());
    }

    #[test]
    fn malformed_json_is_an_error() {
        let path = scratch("malformed");
        std::fs::write(path.as_std_path(), "{ not valid // comment").unwrap();
        let err = read_presets_object(&path).unwrap_err().to_string();
        assert!(err.contains("not valid JSON"));
        assert!(err.contains("refusing to overwrite"));
        std::fs::remove_file(path.as_std_path()).ok();
    }

    #[test]
    fn non_object_root_is_an_error() {
        let path = scratch("array");
        std::fs::write(path.as_std_path(), "[1, 2, 3]").unwrap();
        let err = read_presets_object(&path).unwrap_err().to_string();
        assert!(err.contains("must contain a JSON object"));
        std::fs::remove_file(path.as_std_path()).ok();
    }

    #[test]
    fn valid_object_round_trips() {
        let path = scratch("valid");
        std::fs::write(path.as_std_path(), r#"{"version":6}"#).unwrap();
        let map = read_presets_object(&path).unwrap().unwrap();
        assert_eq!(map["version"], serde_json::json!(6));
        std::fs::remove_file(path.as_std_path()).ok();
    }
}
