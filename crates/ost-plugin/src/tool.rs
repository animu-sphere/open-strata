// SPDX-License-Identifier: Apache-2.0
//! Workspace-built executable descriptors.
//!
//! A tool is a user-facing deliverable a workspace builds against the runtime —
//! a CLI executable, not an OpenUSD plugin bundle and not a library other
//! members link. It has no plugin kind, `plugInfo.json`, or registration
//! contract, and nothing in the workspace graph depends on it.
//!
//! That last property is why it needs a descriptor of its own. `plugin package
//! --workspace --product` composes member archives from bundles, so a tool no
//! bundle requires had no member archive that could carry it: a release either
//! omitted the tool or the repository hand-rolled a second packaging path
//! (usd-vrm-plugins report 28 §3). The descriptor gives OpenStrata a portable
//! identity, the executables to ship, and where the workspace build puts them.

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use ost_core::{Error, Result};

use crate::bundle::{canonicalize_root, check_safe_relative};
use crate::library::is_portable_id;
use crate::model::LibraryDependency;
use crate::satisfies;

/// Filename of a tool descriptor at a workspace member root.
pub const TOOL_MANIFEST: &str = "openstrata.tool.yaml";

/// Initial tool descriptor schema.
pub const TOOL_SCHEMA: &str = "openstrata.tool/v1alpha1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolIdentity {
    pub id: String,
    pub version: String,
    /// SPDX identifier, carried into the product's aggregate license set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolRequires {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<LibraryDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolManifest {
    pub schema: String,
    pub tool: ToolIdentity,
    /// Executable names as the build produces them, without a platform
    /// extension: `motion_retarget`, never `motion_retarget.exe`. Packaging
    /// resolves the extension for the target OS and fails if none is staged, so
    /// a release cannot quietly ship a tool package with no tool in it.
    pub executables: Vec<String>,
    /// Directories below the member root the build writes into, searched in
    /// order. Defaults to `bin` (executables) and `lib` (the shared libraries
    /// they load beside them); a directory the build did not produce is skipped.
    #[serde(default = "default_tool_directories")]
    pub directories: Vec<String>,
    #[serde(default)]
    pub requires: ToolRequires,
}

fn default_tool_directories() -> Vec<String> {
    vec!["bin".into(), "lib".into()]
}

impl ToolManifest {
    pub fn parse(source: &str) -> std::result::Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(source)
    }

    fn validate(&self) -> Result<()> {
        if self.schema != TOOL_SCHEMA {
            return Err(Error::config(format!(
                "tool schema '{}' is unsupported (expected '{TOOL_SCHEMA}')",
                self.schema
            )));
        }
        if !is_portable_id(&self.tool.id) {
            return Err(Error::config(format!(
                "tool.id '{}' is not a portable identifier",
                self.tool.id
            )));
        }
        satisfies(&self.tool.version, &self.tool.version).map_err(|error| {
            Error::config(format!(
                "tool '{}' has an invalid version '{}': {error}",
                self.tool.id, self.tool.version
            ))
        })?;
        if self.executables.is_empty() {
            return Err(Error::config(format!(
                "tool '{}' declares no executables — a tool package with no \
                 executable is not a deliverable",
                self.tool.id
            )));
        }
        for name in &self.executables {
            // The name is joined onto a staged path and reported to a consumer,
            // so it is a bare filename, never a path or a platform extension.
            if name.is_empty()
                || name.contains(['/', '\\', ':'])
                || name.starts_with('.')
                || name.ends_with(".exe")
            {
                return Err(Error::config(format!(
                    "tool '{}' executable '{name}' must be a bare name without a \
                     path or platform extension",
                    self.tool.id
                )));
            }
        }
        for directory in &self.directories {
            check_safe_relative("directories", directory)?;
        }
        if self.directories.is_empty() {
            return Err(Error::config(format!(
                "tool '{}' declares no directories to package",
                self.tool.id
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Tool {
    pub root: Utf8PathBuf,
    pub manifest: ToolManifest,
}

impl Tool {
    pub fn load(root: &Utf8Path) -> Result<Self> {
        let manifest_path = root.join(TOOL_MANIFEST);
        if !manifest_path.as_std_path().is_file() {
            return Err(Error::Operation(format!(
                "no {TOOL_MANIFEST} in '{root}' (is this a workspace-built executable?)"
            )));
        }
        let root = canonicalize_root(root)?;
        let manifest_path = root.join(TOOL_MANIFEST);
        let source = std::fs::read_to_string(manifest_path.as_std_path())
            .map_err(|error| Error::io(manifest_path.to_string(), error))?;
        let manifest = ToolManifest::parse(&source)
            .map_err(|error| Error::parse(TOOL_MANIFEST, anyhow::Error::new(error)))?;
        manifest.validate()?;

        Ok(Self { root, manifest })
    }

    pub fn id(&self) -> &str {
        &self.manifest.tool.id
    }

    pub fn version(&self) -> &str {
        &self.manifest.tool.version
    }

    /// Declared directories that the build actually produced, in declared
    /// order. A tool that installs only `bin` contributes only `bin`.
    pub fn built_directories(&self) -> Vec<String> {
        self.manifest
            .directories
            .iter()
            .filter(|directory| self.root.join(directory).as_std_path().is_dir())
            .cloned()
            .collect()
    }

    /// Locate each declared executable below `root`, returning its portable
    /// relative path. `windows` selects the `.exe` suffix.
    ///
    /// An executable that is not there is reported by name: a workspace that has
    /// not been built yet is the common case, and "no such file" for a path the
    /// caller never typed is not a useful answer.
    pub fn locate_executables(&self, root: &Utf8Path, windows: bool) -> Result<Vec<String>> {
        let suffix = if windows { ".exe" } else { "" };
        let mut found = Vec::new();
        for name in &self.manifest.executables {
            let filename = format!("{name}{suffix}");
            let hit = self
                .manifest
                .directories
                .iter()
                .map(|directory| format!("{directory}/{filename}"))
                .find(|relative| root.join(relative).as_std_path().is_file());
            match hit {
                Some(relative) => found.push(relative),
                None => {
                    return Err(Error::precondition(format!(
                        "tool '{}' declares executable '{filename}', which is not in {} \
                         under '{root}'",
                        self.manifest.tool.id,
                        self.manifest.directories.join(", ")
                    ))
                    .with_hint(
                        "build the workspace first (`ost build`), or correct \
                         `executables` / `directories` in openstrata.tool.yaml",
                    ))
                }
            }
        }
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(extra: &str) -> String {
        format!(
            "schema: {TOOL_SCHEMA}\n\
             tool: {{ id: motion_retarget, version: 0.4.0 }}\n\
             executables: [motion_retarget]\n{extra}"
        )
    }

    fn parse(extra: &str) -> ToolManifest {
        ToolManifest::parse(&descriptor(extra)).unwrap()
    }

    #[test]
    fn parses_identity_executables_and_default_layout() {
        let manifest = parse("");
        assert_eq!(manifest.tool.id, "motion_retarget");
        assert_eq!(manifest.executables, vec!["motion_retarget"]);
        assert_eq!(manifest.directories, vec!["bin", "lib"]);
        assert!(manifest.requires.libraries.is_empty());
        manifest.validate().unwrap();
    }

    #[test]
    fn a_tool_must_declare_at_least_one_executable() {
        let manifest = parse("").clone();
        let empty = ToolManifest {
            executables: vec![],
            ..manifest
        };
        let err = empty.validate().unwrap_err().to_string();
        assert!(err.contains("declares no executables"), "{err}");
    }

    #[test]
    fn executable_names_are_bare() {
        for bad in ["bin/motion", "motion.exe", "../motion", ".hidden"] {
            let source = descriptor("").replace("motion_retarget]", &format!("'{bad}']"));
            let manifest = ToolManifest::parse(&source).unwrap();
            assert!(
                manifest.validate().is_err(),
                "'{bad}' must be rejected as an executable name"
            );
        }
    }

    #[test]
    fn dependency_entries_are_strict() {
        let source = descriptor(
            "requires:\n  libraries:\n    - { id: bytes, version: '>=1.0,<2.0', typo: true }\n",
        );
        assert!(ToolManifest::parse(&source).is_err());
    }

    #[test]
    fn directories_stay_below_the_member_root() {
        let source = descriptor("directories: ['../elsewhere']\n");
        let manifest = ToolManifest::parse(&source).unwrap();
        assert!(manifest.validate().is_err());
    }
}
