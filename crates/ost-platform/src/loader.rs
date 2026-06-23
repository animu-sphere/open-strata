// SPDX-License-Identifier: Apache-2.0
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
        let user_dir = Store::discover().platforms();
        let platforms = ost_core::catalog::load(BUILTINS, &user_dir, parse)?;
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
    serde_yaml::from_str(src).map_err(|e| Error::parse(format!("platform '{label}'"), e))
}

/// Convenience: load the whole catalog.
pub fn load_all() -> Result<Catalog> {
    Catalog::load()
}

/// Convenience: load a single platform by id.
pub fn load_one(id: &str) -> Result<Platform> {
    Catalog::load()?.get(id).cloned()
}
