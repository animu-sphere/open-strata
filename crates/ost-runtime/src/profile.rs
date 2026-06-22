//! Profiles: named bundles of capabilities (§4.3).
//!
//! Loading mirrors `ost-platform`: built-in profiles are embedded so they work
//! on a fresh install, and user profiles in `~/.ost/profiles/*.yaml` are layered
//! on top, overriding built-ins by id. (Factoring this shared "embed + overlay"
//! loader into `ost-core` is tracked as a cross-cutting cleanup.)

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use ost_core::paths::Store;
use ost_core::{Error, Result};

/// The `requires` block of a profile.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Requires {
    /// Logical capabilities this profile pulls in (§4.5).
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// A profile definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub requires: Requires,
}

impl Profile {
    pub fn capabilities(&self) -> &[String] {
        &self.requires.capabilities
    }
}

impl ost_core::catalog::Identified for Profile {
    fn id(&self) -> &str {
        &self.id
    }
}

const BUILTINS: &[(&str, &str)] = &[
    ("core", include_str!("../../../profiles/core.yaml")),
    ("dev", include_str!("../../../profiles/dev.yaml")),
    ("usd", include_str!("../../../profiles/usd.yaml")),
    ("lookdev", include_str!("../../../profiles/lookdev.yaml")),
];

/// All known profiles, keyed and ordered by id.
pub struct ProfileCatalog {
    profiles: BTreeMap<String, Profile>,
}

impl ProfileCatalog {
    pub fn load() -> Result<ProfileCatalog> {
        let user_dir = Store::discover().root.join("profiles");
        let profiles = ost_core::catalog::load(BUILTINS, &user_dir, parse)?;
        Ok(ProfileCatalog { profiles })
    }

    pub fn iter(&self) -> impl Iterator<Item = &Profile> {
        self.profiles.values()
    }

    pub fn get(&self, id: &str) -> Result<&Profile> {
        self.profiles
            .get(id)
            .ok_or_else(|| Error::InvalidManifest(format!("unknown profile '{id}'")))
    }
}

fn parse(label: &str, src: &str) -> Result<Profile> {
    serde_yaml::from_str(src).map_err(|e| Error::parse(format!("profile '{label}'"), e))
}
