// SPDX-License-Identifier: Apache-2.0
//! Portable metadata used to compose independently published artifacts.
//!
//! The contract deliberately describes intent rather than host paths.  It is
//! copied into [`crate::ArtifactRecord`] when a producer manifest is imported,
//! so resolution never has to extract an archive or trust ambient state.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use ost_core::{Error, Result};

/// Initial additive component contract carried by producer manifests.
pub const COMPONENT_SCHEMA: &str = "openstrata.component/v1alpha1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComponentKind {
    Runtime,
    Plugin,
    Library,
    Tool,
    Renderer,
    Data,
}

impl ComponentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Plugin => "plugin",
            Self::Library => "library",
            Self::Tool => "tool",
            Self::Renderer => "renderer",
            Self::Data => "data",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityProvision {
    pub capability: String,
    pub version: String,
    /// Singleton capabilities may have only one selected provider unless the
    /// composition explicitly pins that provider.
    #[serde(default = "default_true")]
    pub singleton: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequirement {
    pub capability: String,
    /// A version constraint accepted by the OpenStrata version matcher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Optional component-id pin local to this requirement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnvironmentOperation {
    Prepend,
    Append,
    Set,
}

impl EnvironmentOperation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Prepend => "prepend",
            Self::Append => "append",
            Self::Set => "set",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentContribution {
    pub variable: String,
    pub operation: EnvironmentOperation,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallMapping {
    pub source: String,
    pub destination: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentCompatibility {
    /// Accepted concrete artifact targets. Empty means the artifact's own
    /// `target` field is authoritative.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openusd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentContract {
    pub schema: String,
    pub id: String,
    pub kind: ComponentKind,
    pub version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provides: Vec<CapabilityProvision>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<CapabilityRequirement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub environment: Vec<EnvironmentContribution>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub install: Vec<InstallMapping>,
    #[serde(default, skip_serializing_if = "is_default_compatibility")]
    pub compatibility: ComponentCompatibility,
    /// Existing v0.22.3 library-package evidence remains in-place while the
    /// common contract grows around it. These fields are intentionally opaque
    /// to the resolver and preserve the producer manifest's additive contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmake: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<serde_json::Value>,
}

impl ComponentContract {
    pub fn validate(&self) -> Result<()> {
        if self.schema != COMPONENT_SCHEMA {
            return Err(Error::InvalidManifest(format!(
                "unsupported component schema '{}' (expected '{COMPONENT_SCHEMA}')",
                self.schema
            )));
        }
        portable_id("component.id", &self.id)?;
        if self.version.trim().is_empty() {
            return Err(Error::InvalidManifest(
                "component.version must not be empty".into(),
            ));
        }
        let mut provided = BTreeSet::new();
        for provision in &self.provides {
            capability("component.provides.capability", &provision.capability)?;
            if !provided.insert(&provision.capability) {
                return Err(Error::InvalidManifest(format!(
                    "component provides capability '{}' more than once",
                    provision.capability
                )));
            }
            if provision.version.trim().is_empty() {
                return Err(Error::InvalidManifest(format!(
                    "component provision '{}' has an empty version",
                    provision.capability
                )));
            }
        }
        for requirement in &self.requires {
            capability("component.requires.capability", &requirement.capability)?;
            if requirement
                .version
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
            {
                return Err(Error::InvalidManifest(format!(
                    "component requirement '{}' has an empty version constraint",
                    requirement.capability
                )));
            }
            if let Some(provider) = &requirement.provider {
                portable_id("component.requires.provider", provider)?;
            }
        }
        for contribution in &self.environment {
            if contribution.variable.is_empty()
                || !contribution
                    .variable
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            {
                return Err(Error::InvalidManifest(format!(
                    "component environment variable '{}' is not a portable uppercase name",
                    contribution.variable
                )));
            }
            if contribution.values.is_empty() {
                return Err(Error::InvalidManifest(format!(
                    "component environment contribution '{}' has no values",
                    contribution.variable
                )));
            }
            for value in &contribution.values {
                safe_relative("component.environment.values", value, true)?;
            }
        }
        for mapping in &self.install {
            safe_relative("component.install.source", &mapping.source, false)?;
            safe_relative("component.install.destination", &mapping.destination, false)?;
        }
        Ok(())
    }
}

fn default_true() -> bool {
    true
}

fn is_default_compatibility(value: &ComponentCompatibility) -> bool {
    value == &ComponentCompatibility::default()
}

fn portable_id(field: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(Error::InvalidManifest(format!(
            "{field} '{value}' is not a portable identifier"
        )));
    }
    Ok(())
}

fn capability(field: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':'))
    {
        return Err(Error::InvalidManifest(format!(
            "{field} '{value}' is not a portable capability"
        )));
    }
    Ok(())
}

fn safe_relative(field: &str, value: &str, allow_dot: bool) -> Result<()> {
    let drive = value.as_bytes().get(1) == Some(&b':');
    let only_current_directory = value
        .split(['/', '\\'])
        .all(|part| part.is_empty() || part == ".");
    let invalid = value.is_empty()
        || (!allow_dot && only_current_directory)
        || value.starts_with(['/', '\\'])
        || drive
        || value.split(['/', '\\']).any(|part| part == "..");
    if invalid {
        return Err(Error::InvalidManifest(format!(
            "{field} '{value}' must be an artifact-relative path"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_the_portable_contract() {
        let contract = ComponentContract {
            schema: COMPONENT_SCHEMA.into(),
            id: "usd-http-resolver".into(),
            kind: ComponentKind::Plugin,
            version: "0.4.0".into(),
            provides: vec![CapabilityProvision {
                capability: "usd.resolve.http".into(),
                version: "0.4.0".into(),
                singleton: true,
            }],
            requires: vec![CapabilityRequirement {
                capability: "usd".into(),
                version: Some(">=26.05,<26.09".into()),
                provider: None,
            }],
            environment: vec![EnvironmentContribution {
                variable: "PXR_PLUGINPATH_NAME".into(),
                operation: EnvironmentOperation::Prepend,
                values: vec!["plugin/usd".into()],
            }],
            install: vec![InstallMapping {
                source: "lib/http.so".into(),
                destination: "lib/http.so".into(),
            }],
            compatibility: ComponentCompatibility::default(),
            descriptor: None,
            descriptor_sha256: None,
            cmake: None,
            dependencies: None,
        };
        contract.validate().unwrap();
    }

    #[test]
    fn rejects_install_escapes() {
        let mut contract = ComponentContract {
            schema: COMPONENT_SCHEMA.into(),
            id: "data".into(),
            kind: ComponentKind::Data,
            version: "1".into(),
            provides: Vec::new(),
            requires: Vec::new(),
            environment: Vec::new(),
            install: vec![InstallMapping {
                source: "payload".into(),
                destination: "../outside".into(),
            }],
            compatibility: ComponentCompatibility::default(),
            descriptor: None,
            descriptor_sha256: None,
            cmake: None,
            dependencies: None,
        };
        assert!(contract.validate().is_err());
        contract.install[0].destination = "share/data".into();
        contract.validate().unwrap();
        contract.install[0].destination = "./".into();
        assert!(contract.validate().is_err());
    }
}
