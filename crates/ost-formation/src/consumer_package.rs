// SPDX-License-Identifier: Apache-2.0
//! Registry-neutral identity contract for ecosystem consumer packages.
//!
//! A wheel, npm package, or native SDK package is a derived entry point. It
//! never becomes a second runtime lock or dependency graph: the exact exported
//! composed-runtime artifact and every selected component remain authoritative.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use ost_core::{Category, Error, Result};
use serde::{Deserialize, Serialize};

use crate::validate_full_digest;

pub const CONSUMER_PACKAGE_SCHEMA: &str = "openstrata.consumer-package/v1alpha1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConsumerPackageKind {
    NativeSdk,
    PythonWheel,
    NpmJavascript,
    NpmWasm,
}

impl ConsumerPackageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NativeSdk => "native-sdk",
            Self::PythonWheel => "python-wheel",
            Self::NpmJavascript => "npm-javascript",
            Self::NpmWasm => "npm-wasm",
        }
    }
}

impl fmt::Display for ConsumerPackageKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ConsumerPackageKind {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "native-sdk" => Ok(Self::NativeSdk),
            "python-wheel" => Ok(Self::PythonWheel),
            "npm-javascript" => Ok(Self::NpmJavascript),
            "npm-wasm" => Ok(Self::NpmWasm),
            _ => Err(format!(
                "unknown consumer package kind '{value}' (expected native-sdk, python-wheel, npm-javascript, or npm-wasm)"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumerPackage {
    pub kind: ConsumerPackageKind,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumerComponentIdentity {
    pub id: String,
    pub version: String,
    pub digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sbom_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumerRuntimeIdentity {
    /// Canonical OST artifact bytes embedded by or fetched for the package.
    pub artifact_digest: String,
    /// Composition identity embedded in that artifact's verified lock.
    pub runtime_digest: String,
    /// Evidence sidecars travel with the canonical artifact and are pinned here
    /// so an ecosystem registry cannot silently detach or replace them.
    pub sbom_digest: String,
    pub provenance_digest: String,
    pub target: String,
    pub components: Vec<ConsumerComponentIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumerPublicApi {
    /// CMake package names, Python import modules, or JavaScript export keys.
    pub entrypoints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumerPrivateLoader {
    /// Makes the public API boundary explicit: package code may implement this
    /// protocol, but callers do not parse OST locks or activation metadata.
    pub scope: String,
    /// Verify the artifact, materialize it, then apply its SDK activation.
    pub strategy: String,
    pub artifact_kind: String,
    pub sdk_schema: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumerPackageManifest {
    pub schema: String,
    pub package: ConsumerPackage,
    pub runtime: ConsumerRuntimeIdentity,
    pub public_api: ConsumerPublicApi,
    pub private_loader: ConsumerPrivateLoader,
}

impl ConsumerPackageManifest {
    pub fn new(
        kind: ConsumerPackageKind,
        name: String,
        version: String,
        mut runtime: ConsumerRuntimeIdentity,
        mut entrypoints: Vec<String>,
    ) -> Result<Self> {
        runtime.components.sort();
        entrypoints.sort();
        let manifest = Self {
            schema: CONSUMER_PACKAGE_SCHEMA.into(),
            package: ConsumerPackage {
                kind,
                name,
                version,
            },
            runtime,
            public_api: ConsumerPublicApi { entrypoints },
            private_loader: ConsumerPrivateLoader {
                scope: "package-private".into(),
                strategy: "verify-extract-activate".into(),
                artifact_kind: "openstrata.composed-runtime".into(),
                sdk_schema: "openstrata.runtime-sdk/v1alpha1".into(),
            },
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != CONSUMER_PACKAGE_SCHEMA {
            return Err(consumer_error("unsupported consumer package schema"));
        }
        validate_label("package name", &self.package.name)?;
        validate_label("package version", &self.package.version)?;
        validate_full_digest("consumer runtime artifact", &self.runtime.artifact_digest)?;
        validate_full_digest("consumer runtime identity", &self.runtime.runtime_digest)?;
        validate_full_digest("consumer runtime SBOM", &self.runtime.sbom_digest)?;
        validate_full_digest(
            "consumer runtime provenance",
            &self.runtime.provenance_digest,
        )?;
        validate_label("runtime target", &self.runtime.target)?;
        if self.runtime.components.is_empty() {
            return Err(consumer_error(
                "consumer runtime must retain at least one component identity",
            ));
        }
        if !self
            .runtime
            .components
            .windows(2)
            .all(|pair| pair[0] < pair[1])
        {
            return Err(consumer_error(
                "consumer components must be unique and canonically sorted",
            ));
        }
        let mut component_ids = BTreeSet::new();
        for component in &self.runtime.components {
            validate_label("component id", &component.id)?;
            validate_label("component version", &component.version)?;
            validate_full_digest("consumer component", &component.digest)?;
            if let Some(digest) = &component.sbom_digest {
                validate_full_digest("consumer component SBOM", digest)?;
            }
            if let Some(digest) = &component.provenance_digest {
                validate_full_digest("consumer component provenance", digest)?;
            }
            if !component_ids.insert(component.id.as_str()) {
                return Err(consumer_error(format!(
                    "consumer component '{}' is duplicated",
                    component.id
                )));
            }
        }
        if self.public_api.entrypoints.is_empty() {
            return Err(consumer_error(
                "consumer package must declare at least one public entrypoint",
            ));
        }
        if !self
            .public_api
            .entrypoints
            .windows(2)
            .all(|pair| pair[0] < pair[1])
        {
            return Err(consumer_error(
                "consumer entrypoints must be unique and canonically sorted",
            ));
        }
        let mut entrypoints = BTreeSet::new();
        for entrypoint in &self.public_api.entrypoints {
            validate_entrypoint(self.package.kind, entrypoint)?;
            if !entrypoints.insert(entrypoint.as_str()) {
                return Err(consumer_error(format!(
                    "consumer entrypoint '{entrypoint}' is duplicated"
                )));
            }
        }
        if self.private_loader.scope != "package-private"
            || self.private_loader.strategy != "verify-extract-activate"
            || self.private_loader.artifact_kind != "openstrata.composed-runtime"
            || self.private_loader.sdk_schema != "openstrata.runtime-sdk/v1alpha1"
        {
            return Err(consumer_error(
                "consumer private loader contract is unsupported",
            ));
        }
        Ok(())
    }
}

fn consumer_error(message: impl Into<String>) -> Error {
    Error::coded("CONSUMER_PACKAGE_INVALID", Category::Validation, message)
}

fn validate_label(label: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.trim() != value
        || value.contains(['\0', '\n', '\r', '\\'])
        || value == "."
        || value == ".."
    {
        return Err(consumer_error(format!("{label} '{value}' is not portable")));
    }
    Ok(())
}

fn validate_entrypoint(kind: ConsumerPackageKind, value: &str) -> Result<()> {
    if value.is_empty() || value.trim() != value || value.contains(['\0', '\n', '\r', '\\']) {
        return Err(consumer_error(format!(
            "consumer entrypoint '{value}' is not portable"
        )));
    }
    let valid = match kind {
        ConsumerPackageKind::PythonWheel => value.split('.').all(identifier),
        ConsumerPackageKind::NativeSdk => {
            value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_+-.".contains(&byte))
                && value
                    .bytes()
                    .next()
                    .is_some_and(|byte| byte.is_ascii_alphanumeric())
        }
        ConsumerPackageKind::NpmJavascript | ConsumerPackageKind::NpmWasm => {
            value == "."
                || value.strip_prefix("./").is_some_and(|tail| {
                    !tail.is_empty()
                        && !tail
                            .split('/')
                            .any(|part| part.is_empty() || part == "." || part == "..")
                        && !tail.contains(':')
                })
        }
    };
    if !valid {
        return Err(consumer_error(format!(
            "entrypoint '{value}' is invalid for {}",
            kind.as_str()
        )));
    }
    Ok(())
}

fn identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn runtime() -> ConsumerRuntimeIdentity {
        ConsumerRuntimeIdentity {
            artifact_digest: digest('a'),
            runtime_digest: digest('b'),
            sbom_digest: digest('d'),
            provenance_digest: digest('e'),
            target: "cy2026-windows-x86_64-msvc143-py313-usd".into(),
            components: vec![ConsumerComponentIdentity {
                id: "openusd".into(),
                version: "26.08".into(),
                digest: digest('c'),
                sbom_digest: Some(digest('f')),
                provenance_digest: None,
            }],
        }
    }

    #[test]
    fn every_consumer_kind_keeps_one_private_loader_contract() {
        for (kind, entrypoint) in [
            (ConsumerPackageKind::NativeSdk, "pxr"),
            (ConsumerPackageKind::PythonWheel, "pxr.Usd"),
            (ConsumerPackageKind::NpmJavascript, "."),
            (ConsumerPackageKind::NpmWasm, "./wasm"),
        ] {
            let manifest = ConsumerPackageManifest::new(
                kind,
                "openusd-consumer".into(),
                "26.8.0".into(),
                runtime(),
                vec![entrypoint.into()],
            )
            .unwrap();
            assert_eq!(manifest.private_loader.scope, "package-private");
            assert_eq!(manifest.runtime.components[0].digest, digest('c'));
        }
    }

    #[test]
    fn public_entrypoints_are_kind_specific_and_unique() {
        let error = ConsumerPackageManifest::new(
            ConsumerPackageKind::PythonWheel,
            "usd-python".into(),
            "1.0.0".into(),
            runtime(),
            vec!["not-a-module".into()],
        )
        .unwrap_err();
        assert!(error.to_string().contains("invalid for python-wheel"));

        let error = ConsumerPackageManifest::new(
            ConsumerPackageKind::NpmJavascript,
            "@openstrata/usd".into(),
            "1.0.0".into(),
            runtime(),
            vec![".".into(), ".".into()],
        )
        .unwrap_err();
        assert!(error.to_string().contains("unique and canonically sorted"));

        let manifest = ConsumerPackageManifest::new(
            ConsumerPackageKind::NativeSdk,
            "usd-native".into(),
            "1.0.0".into(),
            runtime(),
            vec!["UsdGeom".into(), "Sdf".into()],
        )
        .unwrap();
        assert_eq!(manifest.public_api.entrypoints, ["Sdf", "UsdGeom"]);
    }

    #[test]
    fn json_schema_tracks_the_serialized_contract() {
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../../schemas/consumer-package.schema.json"
        ))
        .unwrap();
        for field in [
            "schema",
            "package",
            "runtime",
            "public_api",
            "private_loader",
        ] {
            assert!(schema["properties"].get(field).is_some(), "missing {field}");
        }
        assert_eq!(
            schema["properties"]["package"]["properties"]["kind"]["enum"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        let entrypoint_pattern = |kind: &str| {
            schema["allOf"]
                .as_array()
                .unwrap()
                .iter()
                .find(|condition| {
                    let selected = &condition["if"]["properties"]["package"]["properties"]["kind"];
                    selected["const"] == kind
                        || selected["enum"]
                            .as_array()
                            .is_some_and(|values| values.iter().any(|value| value == kind))
                })
                .and_then(|condition| {
                    condition["then"]["properties"]["public_api"]["properties"]["entrypoints"]
                        ["items"]["pattern"]
                        .as_str()
                })
                .unwrap()
        };
        assert_eq!(
            entrypoint_pattern("native-sdk"),
            "^[A-Za-z0-9][A-Za-z0-9_+.-]*$"
        );
        assert_eq!(
            entrypoint_pattern("python-wheel"),
            "^[A-Za-z_][A-Za-z0-9_]*(\\.[A-Za-z_][A-Za-z0-9_]*)*$"
        );
        assert_eq!(
            entrypoint_pattern("npm-javascript"),
            "^(?!.*\\s$)(?:\\.|\\./(?!\\.{1,2}(?:/|$))[^/\\\\:\\u0000\\r\\n]+(?:/(?!\\.{1,2}(?:/|$))[^/\\\\:\\u0000\\r\\n]+)*)$"
        );
        assert_eq!(
            entrypoint_pattern("npm-wasm"),
            entrypoint_pattern("npm-javascript")
        );
    }
}
