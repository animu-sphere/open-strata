//! Loading platform manifests.
//!
//! Built-in CY manifests are embedded in the binary so `ost platform list`
//! works on a fresh install with no network or store. User-provided YAML in
//! `~/.ost/platforms/*.yaml` is layered on top and overrides built-ins with
//! the same id (§3.5 resolver philosophy; §17.3 layout).

use std::collections::BTreeMap;

use ost_core::paths::Store;
use ost_core::{Error, Result};

use crate::model::Platform;

/// A built-in manifest: `(id, yaml-source)`.
const BUILTINS: &[(&str, &str)] = &[
    ("cy2025", include_str!("../../../platforms/cy2025.yaml")),
    ("cy2026", include_str!("../../../platforms/cy2026.yaml")),
    ("cy2027", include_str!("../../../platforms/cy2027.yaml")),
];

/// All known platforms, keyed and ordered by id.
pub struct Catalog {
    platforms: BTreeMap<String, Platform>,
}

impl Catalog {
    /// Load built-in manifests, then overlay any user manifests.
    pub fn load() -> Result<Catalog> {
        let mut platforms = BTreeMap::new();

        for (id, src) in BUILTINS {
            let p = parse(id, src)?;
            platforms.insert(p.id.clone(), p);
        }

        let user_dir = Store::discover().platforms();
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
                let src = std::fs::read_to_string(&path)
                    .map_err(|e| Error::io(path.display().to_string(), e))?;
                let p = parse(&path.display().to_string(), &src)?;
                platforms.insert(p.id.clone(), p);
            }
        }

        Ok(Catalog { platforms })
    }

    /// Platforms ordered by id (BTreeMap iteration is sorted).
    pub fn iter(&self) -> impl Iterator<Item = &Platform> {
        self.platforms.values()
    }

    pub fn get(&self, id: &str) -> Result<&Platform> {
        self.platforms
            .get(id)
            .ok_or_else(|| Error::PlatformNotFound(id.to_string()))
    }

    pub fn len(&self) -> usize {
        self.platforms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.platforms.is_empty()
    }
}

fn parse(label: &str, src: &str) -> Result<Platform> {
    let p: Platform =
        serde_yaml::from_str(src).map_err(|e| Error::parse(format!("platform '{label}'"), e))?;
    if p.id.is_empty() {
        return Err(Error::InvalidManifest(format!(
            "platform '{label}' is missing an 'id'"
        )));
    }
    Ok(p)
}

/// Convenience: load the whole catalog.
pub fn load_all() -> Result<Catalog> {
    Catalog::load()
}

/// Convenience: load a single platform by id.
pub fn load_one(id: &str) -> Result<Platform> {
    Catalog::load()?.get(id).cloned()
}
