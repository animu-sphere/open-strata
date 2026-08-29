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

/// Distribution modes owned by the component descriptor.  These are explicit
/// because directory layout cannot tell whether a workspace member is useful
/// outside an aggregate product.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryPackage {
    pub standalone: bool,
    pub aggregate_member: bool,
}

/// Optional source used by the generated installed-package consumer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryConsumer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
}

/// Public installed CMake surface promised to consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryPackageContract {
    pub package_name: String,
    pub exported_targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_headers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumer: Option<LibraryConsumer>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<LibraryPackage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_contract: Option<LibraryPackageContract>,
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
        if self
            .package
            .as_ref()
            .is_some_and(|package| !package.standalone && !package.aggregate_member)
        {
            return Err(Error::config(
                "library package must be standalone, an aggregate member, or both",
            ));
        }
        if let Some(contract) = &self.package_contract {
            let package = self.package.as_ref().ok_or_else(|| {
                Error::config("library package_contract requires an explicit package mode")
            })?;
            validate_cmake_atom("package name", &contract.package_name)?;
            if contract.package_name != self.cmake.package {
                return Err(Error::config(format!(
                    "library package_contract.package_name '{}' differs from cmake.package '{}'",
                    contract.package_name, self.cmake.package
                )));
            }
            if contract.exported_targets.is_empty() {
                return Err(Error::config(
                    "library package_contract.exported_targets must not be empty",
                ));
            }
            for target in &contract.exported_targets {
                validate_exported_target(target)?;
            }
            if contract
                .exported_targets
                .iter()
                .enumerate()
                .any(|(index, target)| contract.exported_targets[..index].contains(target))
            {
                return Err(Error::config(
                    "library package_contract.exported_targets contains duplicates",
                ));
            }
            if !contract.exported_targets.contains(&self.cmake.target) {
                return Err(Error::config(format!(
                    "library package_contract.exported_targets must include cmake.target '{}'",
                    self.cmake.target
                )));
            }
            for header in &contract.public_headers {
                validate_header_pattern(header)?;
            }
            if let Some(consumer) = &contract.consumer {
                if let Some(include) = &consumer.include {
                    check_safe_relative("package_contract.consumer.include", include)?;
                }
                if let Some(symbol) = &consumer.symbol {
                    validate_cpp_symbol(symbol)?;
                    if consumer.include.is_none() {
                        return Err(Error::config(
                            "library package_contract.consumer.symbol requires consumer.include",
                        ));
                    }
                }
            }
            if !package.standalone && contract.consumer.is_some() {
                return Err(Error::config(
                    "aggregate-only library package cannot declare a standalone consumer probe",
                ));
            }
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

fn validate_exported_target(target: &str) -> Result<()> {
    let segments: Vec<&str> = target.split("::").collect();
    if segments.len() < 2
        || segments
            .iter()
            .any(|segment| validate_cmake_atom("target segment", segment).is_err())
    {
        return Err(Error::config(format!(
            "library exported target '{target}' must be namespaced such as Package::Target"
        )));
    }
    Ok(())
}

fn validate_cmake_atom(label: &str, value: &str) -> Result<()> {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'.' | b'-'))
    {
        Ok(())
    } else {
        Err(Error::config(format!(
            "library {label} '{value}' contains characters unsafe for generated CMake"
        )))
    }
}

fn validate_header_pattern(pattern: &str) -> Result<()> {
    if pattern.contains(['?', '[', ']']) {
        return Err(Error::config(format!(
            "library public header pattern '{pattern}' uses an unsupported glob"
        )));
    }
    let path = pattern.replace('*', "placeholder");
    check_safe_relative("package_contract.public_headers", &path)
}

fn validate_cpp_symbol(symbol: &str) -> Result<()> {
    if symbol.split("::").all(|segment| {
        segment
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
            && segment
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
    }) {
        Ok(())
    } else {
        Err(Error::config(format!(
            "library consumer symbol '{symbol}' must be a C++ qualified identifier"
        )))
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
        assert!(manifest.package_contract.is_none());
    }

    #[test]
    fn validates_installed_package_contract() {
        let manifest = LibraryManifest::parse(&descriptor(
            "package: { standalone: true, aggregate_member: true }\npackage_contract:\n  package_name: vrmContainer\n  exported_targets: ['vrmContainer::vrmContainer']\n  public_headers: ['include/vrmContainer/**']\n  consumer: { include: vrmContainer/api.hpp, symbol: 'vrm::version' }\n",
        ))
        .unwrap();
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn package_contract_cannot_diverge_from_existing_cmake_identity() {
        let manifest = LibraryManifest::parse(&descriptor(
            "package: { standalone: true, aggregate_member: false }\npackage_contract:\n  package_name: other\n  exported_targets: ['other::target']\n",
        ))
        .unwrap();
        let error = manifest.validate().unwrap_err().to_string();
        assert!(error.contains("differs from cmake.package"), "{error}");
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
