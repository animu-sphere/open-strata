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
        let entries =
            std::fs::read_dir(user_dir.as_std_path()).map_err(|e| Error::io(user_dir.to_string(), e))?;
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
            let src =
                std::fs::read_to_string(&path).map_err(|e| Error::io(label.clone(), e))?;
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
