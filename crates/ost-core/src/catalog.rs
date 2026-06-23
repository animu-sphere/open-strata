// SPDX-License-Identifier: Apache-2.0
//! A generic "embedded built-ins + user overlay" catalog loader.
//!
//! Platforms, profiles, and extensions all load the same way: a set of built-in
//! YAML documents compiled into the binary, overlaid by user-provided `*.yaml`
//! files in a store directory, keyed by id (user entries override built-ins).
//! This module factors out that pattern so each crate only provides its built-in
//! list, the store directory, and a parse closure.

use std::collections::BTreeMap;

use camino::Utf8Path;

use crate::{Error, Result};

/// Anything that can live in a [`load`]ed catalog: it knows its own id.
pub trait Identified {
    fn id(&self) -> &str;
}

/// Load a catalog: parse the `builtins`, then overlay `*.yaml` from `user_dir`.
///
/// `parse` deserializes one document given a `(label, source)` pair, where the
/// label is used only for error messages. Items with an empty id are rejected.
/// User entries override built-ins with the same id; the result is ordered by id.
pub fn load<T, F>(
    builtins: &[(&str, &str)],
    user_dir: &Utf8Path,
    parse: F,
) -> Result<BTreeMap<String, T>>
where
    T: Identified,
    F: Fn(&str, &str) -> Result<T>,
{
    let mut items = BTreeMap::new();

    for (label, src) in builtins {
        insert(&mut items, parse(label, src)?, label)?;
    }

    if user_dir.as_std_path().is_dir() {
        let entries = std::fs::read_dir(user_dir.as_std_path())
            .map_err(|e| Error::io(user_dir.to_string(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| Error::io(user_dir.to_string(), e))?;
            let path = entry.path();
            let is_yaml = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e == "yaml" || e == "yml")
                .unwrap_or(false);
            if !is_yaml {
                continue;
            }
            let label = path.display().to_string();
            let src = std::fs::read_to_string(&path).map_err(|e| Error::io(label.clone(), e))?;
            insert(&mut items, parse(&label, &src)?, &label)?;
        }
    }

    Ok(items)
}

fn insert<T: Identified>(items: &mut BTreeMap<String, T>, item: T, label: &str) -> Result<()> {
    if item.id().is_empty() {
        return Err(Error::InvalidManifest(format!(
            "'{label}' is missing an 'id'"
        )));
    }
    items.insert(item.id().to_string(), item);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct Item {
        id: String,
        value: u32,
    }

    impl Identified for Item {
        fn id(&self) -> &str {
            &self.id
        }
    }

    // The loader is format-agnostic; the test parser reads JSON (a YAML subset),
    // so files are named `*.yaml` but contain JSON we deserialize with serde_json.
    fn parse(label: &str, src: &str) -> Result<Item> {
        serde_json::from_str(src).map_err(|e| Error::parse(label, anyhow::Error::new(e)))
    }

    const BUILTINS: &[(&str, &str)] = &[
        ("base-a", r#"{"id":"a","value":1}"#),
        ("base-b", r#"{"id":"b","value":2}"#),
    ];

    /// Create a unique temp directory for a test's user overlay.
    fn temp_dir(tag: &str) -> Utf8PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut dir = Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
        dir.push(format!("ost-catalog-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(dir.as_std_path()).unwrap();
        dir
    }

    fn write(dir: &Utf8Path, name: &str, contents: &str) {
        std::fs::write(dir.join(name).as_std_path(), contents).unwrap();
    }

    #[test]
    fn builtins_load_when_no_overlay() {
        let dir = temp_dir("none");
        std::fs::remove_dir_all(dir.as_std_path()).unwrap(); // non-existent dir
        let items = load(BUILTINS, &dir, parse).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items["a"].value, 1);
    }

    #[test]
    fn overlay_overrides_by_id_adds_new_and_ignores_non_yaml() {
        let dir = temp_dir("overlay");
        write(&dir, "a.yaml", r#"{"id":"a","value":99}"#); // override built-in
        write(&dir, "c.yml", r#"{"id":"c","value":3}"#); // new
        write(&dir, "notes.txt", "ignored"); // non-yaml ignored

        let items = load(BUILTINS, &dir, parse).unwrap();
        std::fs::remove_dir_all(dir.as_std_path()).unwrap();

        assert_eq!(items.len(), 3);
        assert_eq!(items["a"].value, 99, "user file overrides built-in by id");
        assert_eq!(items["b"].value, 2);
        assert_eq!(items["c"].value, 3);
    }

    #[test]
    fn empty_id_is_rejected() {
        let dir = temp_dir("badid");
        write(&dir, "bad.yaml", r#"{"id":"","value":1}"#);
        let result = load(BUILTINS, &dir, parse);
        std::fs::remove_dir_all(dir.as_std_path()).unwrap();
        assert!(result.is_err());
    }
}
