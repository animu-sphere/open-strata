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
        let mut profiles = BTreeMap::new();
        for (id, src) in BUILTINS {
            let p = parse(id, src)?;
            profiles.insert(p.id.clone(), p);
        }

        let user_dir = Store::discover().root.join("profiles");
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
                profiles.insert(p.id.clone(), p);
            }
        }

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
    let p: Profile =
        serde_yaml::from_str(src).map_err(|e| Error::parse(format!("profile '{label}'"), e))?;
    if p.id.is_empty() {
        return Err(Error::InvalidManifest(format!(
            "profile '{label}' is missing an 'id'"
        )));
    }
    Ok(p)
}
