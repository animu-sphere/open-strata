// SPDX-License-Identifier: Apache-2.0
//! Plain CMake library descriptors used by plugin workspace composition.
//!
//! A library is deliberately not an OpenUSD plugin bundle: it has no plugin
//! kind, `plugInfo.json`, registration metadata, or OpenUSD runtime contract.
//! Its descriptor only gives OST a portable identity, an installed CMake
//! package/target contract, dependency edges, and installed loader paths.

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use ost_core::{Error, Result};

use crate::bundle::{canonicalize_root, check_safe_relative};
use crate::model::LibraryDependency;
use crate::satisfies;

/// Filename of a plain-library descriptor at a CMake project root.
pub const LIBRARY_MANIFEST: &str = "openstrata.library.yaml";

/// Initial plain-library descriptor schema.
pub const LIBRARY_SCHEMA: &str = "openstrata.library/v1alpha1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryIdentity {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryCmake {
    /// Installed config-package identity consumed by `find_package(...)`.
    pub package: String,
    /// Installed exported target, normally `<package>::<target>`.
    pub target: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryRequires {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<LibraryDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryRuntime {
    /// Paths below the installed prefix which may contain shared libraries.
    /// Both `bin` and `lib` may be listed for one cross-platform source project;
    /// only directories materialized by install are injected or packaged.
    #[serde(default = "default_runtime_directories")]
    pub directories: Vec<String>,
}

impl Default for LibraryRuntime {
    fn default() -> Self {
        Self {
            directories: default_runtime_directories(),
        }
    }
}

fn default_runtime_directories() -> Vec<String> {
    vec!["bin".into(), "lib".into()]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryManifest {
    pub schema: String,
    pub library: LibraryIdentity,
    pub cmake: LibraryCmake,
    #[serde(default)]
    pub requires: LibraryRequires,
    #[serde(default)]
    pub runtime: LibraryRuntime,
}

impl LibraryManifest {
    pub fn parse(source: &str) -> std::result::Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(source)
    }

    fn validate(&self) -> Result<()> {
        if self.schema != LIBRARY_SCHEMA {
            return Err(Error::config(format!(
                "library schema '{}' is unsupported (expected '{LIBRARY_SCHEMA}')",
                self.schema
            )));
        }
        if !is_portable_id(&self.library.id) {
            return Err(Error::config(format!(
                "library.id '{}' is not a portable identifier",
                self.library.id
            )));
        }
        satisfies(&self.library.version, &self.library.version).map_err(|error| {
            Error::config(format!(
                "library '{}' has an invalid version '{}': {error}",
                self.library.id, self.library.version
            ))
        })?;
        if self.cmake.package.trim().is_empty() {
            return Err(Error::config("library cmake.package must not be empty"));
        }
        // Match schemas/library.schema.json: two or more non-empty `::`
        // segments (nested export namespaces are legal CMake), no stray ':'.
        let segments: Vec<&str> = self.cmake.target.split("::").collect();
        if segments.len() < 2
            || segments
                .iter()
                .any(|segment| segment.is_empty() || segment.contains(':'))
        {
            return Err(Error::config(format!(
                "library cmake.target '{}' must be a namespaced exported target such as Package::Target",
                self.cmake.target
            )));
        }
        for directory in &self.runtime.directories {
            check_safe_relative("runtime.directories", directory)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Library {
    pub root: Utf8PathBuf,
    pub manifest: LibraryManifest,
}

impl Library {
    pub fn load(root: &Utf8Path) -> Result<Self> {
        let manifest_path = root.join(LIBRARY_MANIFEST);
        if !manifest_path.as_std_path().is_file() {
            return Err(Error::Operation(format!(
                "no {LIBRARY_MANIFEST} in '{root}' (is this a plain CMake library?)"
            )));
        }
        let root = canonicalize_root(root)?;
        let manifest_path = root.join(LIBRARY_MANIFEST);
        let source = std::fs::read_to_string(manifest_path.as_std_path())
            .map_err(|error| Error::io(manifest_path.to_string(), error))?;
        let manifest = LibraryManifest::parse(&source)
            .map_err(|error| Error::parse(LIBRARY_MANIFEST, anyhow::Error::new(error)))?;
        manifest.validate()?;

        Ok(Self { root, manifest })
    }

    pub fn id(&self) -> &str {
        &self.manifest.library.id
    }

    pub fn version(&self) -> &str {
        &self.manifest.library.version
    }

    /// Runtime directories below an installed workspace prefix. Missing
    /// directories are omitted: a header-only or static build contributes no
    /// loader path, while a shared build materializes `bin` and/or `lib`.
    pub fn installed_runtime_dirs(&self, prefix: &Utf8Path) -> Vec<Utf8PathBuf> {
        self.manifest
            .runtime
            .directories
            .iter()
            .map(|directory| prefix.join(directory))
            .filter(|directory| directory.as_std_path().is_dir())
            .collect()
    }
}

pub(crate) fn is_portable_id(id: &str) -> bool {
    id.chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(extra: &str) -> String {
        format!(
            "schema: {LIBRARY_SCHEMA}\nlibrary: {{ id: vrmContainer, version: 0.1.0 }}\ncmake: {{ package: vrmContainer, target: 'vrmContainer::vrmContainer' }}\n{extra}"
        )
    }

    #[test]
    fn parses_plain_cmake_contract_and_defaults_runtime_layout() {
        let manifest = LibraryManifest::parse(&descriptor("")).unwrap();
        assert_eq!(manifest.library.id, "vrmContainer");
        assert_eq!(manifest.runtime.directories, vec!["bin", "lib"]);
        assert!(manifest.requires.libraries.is_empty());
    }

    #[test]
    fn dependency_entries_are_strict() {
        let source = descriptor(
            "requires:\n  libraries:\n    - { id: bytes, version: '>=1.0,<2.0', typo: true }\n",
        );
        assert!(LibraryManifest::parse(&source).is_err());
    }

    #[test]
    fn cmake_target_must_be_a_namespaced_export() {
        let manifest = |target: &str| {
            let source = descriptor("").replace("'vrmContainer::vrmContainer'", target);
            LibraryManifest::parse(&source).unwrap()
        };
        // Nested export namespaces are legal CMake and must stay accepted.
        assert!(manifest("'ns::inner::target'").validate().is_ok());
        for bad in [
            "vrmContainer",
            "'::vrmContainer'",
            "'vrmContainer::'",
            "'a:::b'",
            "'a:b'",
        ] {
            assert!(manifest(bad).validate().is_err(), "{bad} must be rejected");
        }
    }
}
