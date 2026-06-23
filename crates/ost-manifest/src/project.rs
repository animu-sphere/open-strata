// SPDX-License-Identifier: Apache-2.0
//! The project manifest: `openstrata.toml`.
//!
//! Capabilities are requested by *what they do*, not by package name (§3.5).
//! A project pins a platform year and a profile, and may request additional
//! capabilities and named extensions on top of that profile.

use serde::{Deserialize, Serialize};

use ost_core::paths::PROJECT_MANIFEST;
use ost_core::{Error, Result};

/// `[project]` table — identity and metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn default_version() -> String {
    "0.1.0".into()
}

/// `[requires]` table — the runtime contract this project builds against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requires {
    /// Platform calendar-year id, e.g. `cy2026`.
    pub platform: String,
    /// Profile name, e.g. `usd` or `lookdev`.
    #[serde(default = "default_profile")]
    pub profile: String,
    /// Extra capabilities beyond those implied by the profile (§4.5).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// Named certified extensions to include (§4.4).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
}

fn default_profile() -> String {
    "core".into()
}

/// The whole `openstrata.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub project: ProjectMeta,
    pub requires: Requires,
}

impl Project {
    /// A sensible starter manifest for `ost init`.
    pub fn scaffold(name: impl Into<String>, platform: impl Into<String>) -> Project {
        Project {
            project: ProjectMeta {
                name: name.into(),
                version: default_version(),
                description: None,
            },
            requires: Requires {
                platform: platform.into(),
                profile: "usd".into(),
                capabilities: Vec::new(),
                extensions: Vec::new(),
            },
        }
    }

    pub fn from_toml(src: &str) -> Result<Project> {
        toml::from_str(src).map_err(|e| Error::parse(PROJECT_MANIFEST, anyhow::Error::new(e)))
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self)
            .map_err(|e| Error::parse(PROJECT_MANIFEST, anyhow::Error::new(e)))
    }
}

/// Add `name` to `[requires].extensions` in raw manifest TOML, preserving the
/// rest of the document (comments, formatting, and any tables this model does
/// not capture). The list is kept sorted. Returns the rewritten TOML, or `None`
/// when the extension is already present (idempotent).
///
/// This edits the source in place rather than round-tripping through [`Project`],
/// which would drop comments and silently delete unmodelled sections.
pub fn add_extension(src: &str, name: &str) -> Result<Option<String>> {
    use toml_edit::{Array, DocumentMut, Item, Value};

    let mut doc: DocumentMut = src
        .parse()
        .map_err(|e| Error::parse(PROJECT_MANIFEST, anyhow::Error::new(e)))?;

    let requires = doc
        .get_mut("requires")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| {
            Error::InvalidManifest(format!(
                "{PROJECT_MANIFEST} is missing the [requires] table"
            ))
        })?;

    let mut names: Vec<String> = requires
        .get("extensions")
        .and_then(Item::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    if names.iter().any(|e| e == name) {
        return Ok(None);
    }
    names.push(name.to_string());
    names.sort();

    let array: Array = names.into_iter().collect();
    requires["extensions"] = Item::Value(Value::Array(array));

    Ok(Some(doc.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# my project
[project]
name = \"demo\"

[requires]
platform = \"cy2026\"  # pinned year
profile = \"lookdev\"
";

    #[test]
    fn add_extension_preserves_comments_and_sorts() {
        let out = add_extension(SAMPLE, "openusd").unwrap().expect("changed");
        assert!(out.contains("# my project"));
        assert!(out.contains("# pinned year"));
        assert!(out.contains("extensions = [\"openusd\"]"));

        // Adding a second one keeps the list sorted.
        let out = add_extension(&out, "materialx").unwrap().expect("changed");
        let idx_mtlx = out.find("materialx").unwrap();
        let idx_usd = out.find("openusd").unwrap();
        assert!(idx_mtlx < idx_usd, "extensions must stay sorted");
    }

    #[test]
    fn add_extension_is_idempotent() {
        let out = add_extension(SAMPLE, "openusd").unwrap().unwrap();
        assert!(add_extension(&out, "openusd").unwrap().is_none());
    }

    #[test]
    fn add_extension_keeps_unmodelled_sections() {
        let src = format!("{SAMPLE}\n[tools.cmake]\ngenerator = \"Ninja\"\n");
        let out = add_extension(&src, "openusd").unwrap().unwrap();
        assert!(out.contains("[tools.cmake]"));
        assert!(out.contains("generator = \"Ninja\""));
    }
}
