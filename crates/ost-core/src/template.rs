// SPDX-License-Identifier: Apache-2.0
//! Versioned template descriptors and deterministic generated provenance.

use std::collections::{BTreeMap, BTreeSet};

use camino::{Utf8Component, Utf8Path};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

pub const TEMPLATE_SCHEMA: &str = "openstrata.template/v1alpha1";
pub const SCAFFOLD_SCHEMA: &str = "openstrata.scaffold/v1alpha1";
pub const SCAFFOLD_PROVENANCE: &str = "openstrata.scaffold.yaml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TemplateMaturity {
    Reference,
    Skeleton,
    Template,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TemplateArtifactKind {
    Plugin,
    Project,
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateIdentity {
    pub id: String,
    pub version: String,
    pub maturity: TemplateMaturity,
    pub artifact_kind: TemplateArtifactKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TemplateVariableType {
    PortableIdentifier,
    FileExtension,
    UriScheme,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateVariable {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: TemplateVariableType,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateOutputs {
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateVerification {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assertions: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateCompatibility {
    pub ost: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openusd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateDescriptor {
    pub schema: String,
    pub template: TemplateIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_kind: Option<String>,
    #[serde(default)]
    pub variables: Vec<TemplateVariable>,
    pub outputs: TemplateOutputs,
    pub verification: TemplateVerification,
    pub compatibility: TemplateCompatibility,
}

impl TemplateDescriptor {
    pub fn parse(source: &str) -> Result<Self> {
        let descriptor: Self = serde_yaml::from_str(source)
            .map_err(|e| Error::parse("template.yaml", anyhow::Error::new(e)))?;
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != TEMPLATE_SCHEMA {
            return Err(Error::InvalidManifest(format!(
                "template.yaml schema '{}' is unsupported (expected '{TEMPLATE_SCHEMA}')",
                self.schema
            )));
        }
        validate_portable_id("template id", &self.template.id)?;
        if !is_semantic_version(&self.template.version) {
            return Err(Error::InvalidManifest(
                "template.yaml template.version must be a semantic version (major.minor.patch)"
                    .into(),
            ));
        }
        if self.compatibility.ost.trim().is_empty() {
            return Err(Error::InvalidManifest(
                "template.yaml compatibility.ost must not be empty".into(),
            ));
        }

        let mut variables = BTreeSet::new();
        for variable in &self.variables {
            validate_portable_id("template variable", &variable.name)?;
            if !variables.insert(variable.name.as_str()) {
                return Err(Error::InvalidManifest(format!(
                    "template.yaml declares duplicate variable '{}'",
                    variable.name
                )));
            }
        }

        if self.outputs.files.is_empty() {
            return Err(Error::InvalidManifest(
                "template.yaml outputs.files must not be empty".into(),
            ));
        }
        let mut outputs = BTreeSet::new();
        for output in &self.outputs.files {
            validate_output_path(output)?;
            if !outputs.insert(output.as_str()) {
                return Err(Error::InvalidManifest(format!(
                    "template.yaml declares duplicate output '{output}'"
                )));
            }
        }
        if !outputs.contains(SCAFFOLD_PROVENANCE) {
            return Err(Error::InvalidManifest(format!(
                "template.yaml outputs.files must include '{SCAFFOLD_PROVENANCE}'"
            )));
        }
        Ok(())
    }
}

fn is_semantic_version(version: &str) -> bool {
    // Peel optional build metadata (`+...`) then pre-release (`-...`); both, when
    // their delimiter is present, must carry a non-empty identifier so trailing
    // `1.0.0-` / `1.0.0+` do not slip through as valid.
    let (rest, build) = match version.split_once('+') {
        Some((rest, build)) => (rest, Some(build)),
        None => (version, None),
    };
    let (core, pre) = match rest.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (rest, None),
    };
    if build.is_some_and(str::is_empty) || pre.is_some_and(str::is_empty) {
        return false;
    }
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

fn validate_portable_id(label: &str, value: &str) -> Result<()> {
    let valid = value
        .chars()
        .next()
        .map(|c| c.is_ascii_alphabetic())
        .unwrap_or(false)
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidManifest(format!(
            "invalid {label} '{value}': use letters, digits, '-' or '_', starting with a letter"
        )))
    }
}

fn validate_output_path(path: &str) -> Result<()> {
    let path = Utf8Path::new(path);
    let unsafe_component = path.components().any(|component| {
        matches!(
            component,
            Utf8Component::Prefix(_) | Utf8Component::RootDir | Utf8Component::ParentDir
        )
    });
    if path.as_str().is_empty() || unsafe_component {
        return Err(Error::InvalidManifest(format!(
            "template.yaml output '{path}' must be a safe relative path"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScaffoldTemplateIdentity {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScaffoldGenerator {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScaffoldProvenance {
    pub schema: String,
    pub template: ScaffoldTemplateIdentity,
    pub generator: ScaffoldGenerator,
    pub inputs: BTreeMap<String, String>,
}

impl ScaffoldProvenance {
    pub fn new(
        descriptor: &TemplateDescriptor,
        generator_version: impl Into<String>,
        inputs: BTreeMap<String, String>,
    ) -> Self {
        Self {
            schema: SCAFFOLD_SCHEMA.into(),
            template: ScaffoldTemplateIdentity {
                id: descriptor.template.id.clone(),
                version: descriptor.template.version.clone(),
            },
            generator: ScaffoldGenerator {
                name: "ost".into(),
                version: generator_version.into(),
            },
            inputs,
        }
    }

    pub fn to_yaml(&self) -> Result<String> {
        serde_yaml::to_string(self)
            .map_err(|e| Error::parse(SCAFFOLD_PROVENANCE, anyhow::Error::new(e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESCRIPTOR: &str = r#"
schema: openstrata.template/v1alpha1
template:
  id: usd-fileformat-cpp
  version: 1.0.0
  maturity: template
  artifact_kind: plugin
plugin_kind: usd-fileformat
variables:
  - name: name
    type: portable-identifier
    required: true
outputs:
  files: [openstrata.plugin.yaml, openstrata.scaffold.yaml]
verification:
  levels: [0, 1]
compatibility:
  ost: ">=0.13,<1.0"
  openusd: ">=25.05,<26.0"
"#;

    #[test]
    fn descriptor_parses_and_validates() {
        let descriptor = TemplateDescriptor::parse(DESCRIPTOR).unwrap();
        assert_eq!(descriptor.template.id, "usd-fileformat-cpp");
        assert_eq!(descriptor.plugin_kind.as_deref(), Some("usd-fileformat"));
    }

    #[test]
    fn descriptor_rejects_unsafe_and_duplicate_outputs() {
        let unsafe_path = DESCRIPTOR.replace(
            "openstrata.plugin.yaml, openstrata.scaffold.yaml",
            "../escape, openstrata.scaffold.yaml",
        );
        assert!(TemplateDescriptor::parse(&unsafe_path).is_err());

        let duplicate = DESCRIPTOR.replace(
            "openstrata.plugin.yaml, openstrata.scaffold.yaml",
            "openstrata.scaffold.yaml, openstrata.scaffold.yaml",
        );
        assert!(TemplateDescriptor::parse(&duplicate).is_err());

        let bad_version = DESCRIPTOR.replace("version: 1.0.0", "version: latest");
        assert!(TemplateDescriptor::parse(&bad_version).is_err());
    }

    #[test]
    fn semantic_version_accepts_valid_and_rejects_malformed() {
        for good in [
            "1.0.0",
            "0.13.2",
            "1.2.3-alpha.1",
            "1.2.3+build.5",
            "1.2.3-rc.1+meta",
        ] {
            assert!(is_semantic_version(good), "expected '{good}' to be valid");
        }
        for bad in [
            "1.0",
            "1.0.0.0",
            "v1.0.0",
            "latest",
            "1.0.0-",
            "1.0.0+",
            "1.0.0-+meta",
            "1..0",
        ] {
            assert!(!is_semantic_version(bad), "expected '{bad}' to be invalid");
        }
    }

    #[test]
    fn provenance_is_byte_deterministic_and_has_no_timestamp() {
        let descriptor = TemplateDescriptor::parse(DESCRIPTOR).unwrap();
        let inputs = BTreeMap::from([("name".into(), "toy".into())]);
        let first = ScaffoldProvenance::new(&descriptor, "0.13.0", inputs.clone())
            .to_yaml()
            .unwrap();
        let second = ScaffoldProvenance::new(&descriptor, "0.13.0", inputs)
            .to_yaml()
            .unwrap();
        assert_eq!(first, second);
        assert!(first.contains("id: usd-fileformat-cpp"));
        assert!(first.contains("name: toy"));
        assert!(!first.contains("generated_at"));
    }
}
