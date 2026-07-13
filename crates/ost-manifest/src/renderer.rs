// SPDX-License-Identifier: Apache-2.0
//! Renderer composition intent and validation evidence.
//!
//! This model describes logical source/build boundaries. It deliberately does
//! not require one CMake package, library descriptor, or plugin bundle per unit.

use std::collections::{BTreeMap, BTreeSet};

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use ost_core::{Error, Result};

pub const RENDERER_MANIFEST: &str = "openstrata.renderer.yaml";
pub const RENDERER_SCHEMA: &str = "openstrata.renderer/v1alpha1";
pub const RENDERER_REPORT_FILE: &str = "renderer-report.json";
pub const RENDERER_REPORT_SCHEMA: &str = "openstrata.renderer-report/v1alpha1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererIdentity {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererComposition {
    /// Selected backend family, for example `vulkan`.
    pub backend: String,
    /// Host-neutral sources which feed the renderer, for example `headless`.
    pub scene_inputs: Vec<String>,
    /// Logical scaffold units. Values are project-owned target labels, not
    /// installed package or artifact identities.
    pub units: BTreeMap<String, String>,
    /// Optional adapter labels keyed by host/API family.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adapters: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderProducts {
    pub required: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameContract {
    pub contexts: u32,
    /// Ownership/completion contract label. The schema does not prescribe a
    /// semaphore, fence, queue, or frame-graph implementation.
    pub completion: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererValidation {
    pub gpu_smoke: bool,
    pub validation_messages_are_errors: bool,
    /// Stable evidence ids expected in `renderer-report.json`.
    pub assertions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererManifest {
    pub schema: String,
    pub renderer: RendererIdentity,
    pub composition: RendererComposition,
    pub render_products: RenderProducts,
    pub frame: FrameContract,
    pub validation: RendererValidation,
}

impl RendererManifest {
    pub fn parse(source: &str) -> Result<Self> {
        let manifest: Self = serde_yaml::from_str(source)
            .map_err(|error| Error::parse(RENDERER_MANIFEST, anyhow::Error::new(error)))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn load(root: &Utf8Path) -> Result<Self> {
        let path = root.join(RENDERER_MANIFEST);
        let source = std::fs::read_to_string(path.as_std_path())
            .map_err(|error| Error::io(path.to_string(), error))?;
        Self::parse(&source)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != RENDERER_SCHEMA {
            return Err(invalid(format!(
                "renderer schema '{}' is unsupported (expected '{RENDERER_SCHEMA}')",
                self.schema
            )));
        }
        validate_portable_id("renderer.name", &self.renderer.name)?;
        validate_portable_id("composition.backend", &self.composition.backend)?;
        validate_unique_ids("composition.scene_inputs", &self.composition.scene_inputs)?;
        validate_label_map("composition.units", &self.composition.units)?;
        validate_label_map_entries("composition.adapters", &self.composition.adapters)?;
        validate_unique_ids("render_products.required", &self.render_products.required)?;
        if self.frame.contexts == 0 {
            return Err(invalid("frame.contexts must be at least 1"));
        }
        if self.frame.completion.trim().is_empty() {
            return Err(invalid("frame.completion must not be empty"));
        }
        if self.validation.assertions.is_empty() {
            return Err(invalid("validation.assertions must not be empty"));
        }
        let mut assertions = BTreeSet::new();
        for assertion in &self.validation.assertions {
            validate_evidence_id(assertion)?;
            if !assertions.insert(assertion.as_str()) {
                return Err(invalid(format!(
                    "validation.assertions repeats '{assertion}'"
                )));
            }
        }
        Ok(())
    }

    pub fn report_path(&self, build_dir: &Utf8Path) -> Utf8PathBuf {
        build_dir.join(RENDERER_REPORT_FILE)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RendererCheckStatus {
    Pass,
    Fail,
    Skip,
}

impl RendererCheckStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Skip => "skip",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererCheck {
    pub id: String,
    pub status: RendererCheckStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererReportIdentity {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererDevice {
    pub backend: String,
    pub name: String,
    pub api_version: String,
    pub driver_version: String,
    pub vendor_id: u32,
    pub device_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererReport {
    pub schema: String,
    pub renderer: RendererReportIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<RendererDevice>,
    pub checks: Vec<RendererCheck>,
}

impl RendererReport {
    pub fn parse(source: &str) -> Result<Self> {
        let report: Self = serde_json::from_str(source)
            .map_err(|error| Error::parse(RENDERER_REPORT_FILE, anyhow::Error::new(error)))?;
        if report.schema != RENDERER_REPORT_SCHEMA {
            return Err(invalid(format!(
                "renderer report schema '{}' is unsupported (expected '{RENDERER_REPORT_SCHEMA}')",
                report.schema
            )));
        }
        Ok(report)
    }

    pub fn load(path: &Utf8Path) -> Result<Self> {
        let source = std::fs::read_to_string(path.as_std_path())
            .map_err(|error| Error::io(path.to_string(), error))?;
        Self::parse(&source)
    }

    pub fn validate_against(&self, manifest: &RendererManifest) -> Result<()> {
        if self.renderer.name != manifest.renderer.name {
            return Err(invalid(format!(
                "renderer report names '{}', but the manifest names '{}'",
                self.renderer.name, manifest.renderer.name
            )));
        }
        if let Some(device) = &self.device {
            validate_portable_id("renderer report device.backend", &device.backend)?;
            if device.name.trim().is_empty()
                || device.api_version.trim().is_empty()
                || device.driver_version.trim().is_empty()
            {
                return Err(invalid(
                    "renderer report device name/API/driver must not be empty",
                ));
            }
        }
        let mut observed = BTreeSet::new();
        for check in &self.checks {
            validate_evidence_id(&check.id)?;
            if !observed.insert(check.id.as_str()) {
                return Err(invalid(format!(
                    "renderer report repeats check '{}'",
                    check.id
                )));
            }
            if check.status != RendererCheckStatus::Pass
                && check
                    .detail
                    .as_deref()
                    .is_none_or(|detail| detail.trim().is_empty())
            {
                return Err(invalid(format!(
                    "renderer report check '{}' must explain {}",
                    check.id,
                    check.status.as_str()
                )));
            }
        }
        for required in &manifest.validation.assertions {
            if !observed.contains(required.as_str()) {
                return Err(invalid(format!(
                    "renderer report is missing required assertion '{required}'"
                )));
            }
        }
        Ok(())
    }

    pub fn passed(&self) -> bool {
        !self
            .checks
            .iter()
            .any(|check| check.status == RendererCheckStatus::Fail)
    }
}

fn validate_label_map(label: &str, values: &BTreeMap<String, String>) -> Result<()> {
    if values.is_empty() {
        return Err(invalid(format!("{label} must not be empty")));
    }
    validate_label_map_entries(label, values)
}

fn validate_label_map_entries(label: &str, values: &BTreeMap<String, String>) -> Result<()> {
    for (key, value) in values {
        validate_portable_id(label, key)?;
        validate_portable_id(label, value)?;
    }
    Ok(())
}

fn validate_unique_ids(label: &str, values: &[String]) -> Result<()> {
    if values.is_empty() {
        return Err(invalid(format!("{label} must not be empty")));
    }
    let mut unique = BTreeSet::new();
    for value in values {
        validate_portable_id(label, value)?;
        if !unique.insert(value.as_str()) {
            return Err(invalid(format!("{label} repeats '{value}'")));
        }
    }
    Ok(())
}

fn validate_portable_id(label: &str, value: &str) -> Result<()> {
    let valid = value
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        });
    if valid {
        Ok(())
    } else {
        Err(invalid(format!(
            "{label} '{value}' is not a portable identifier"
        )))
    }
}

fn validate_evidence_id(value: &str) -> Result<()> {
    // Match schemas/renderer.schema.json: at least two segments, so every
    // evidence id carries a namespace (`renderer.gpu.frame`, never `frame`).
    let segments: Vec<&str> = value.split('.').collect();
    let valid = segments.len() >= 2
        && segments.iter().all(|segment| {
            segment
                .chars()
                .next()
                .is_some_and(|first| first.is_ascii_lowercase())
                && segment.chars().all(|character| {
                    character.is_ascii_lowercase()
                        || character.is_ascii_digit()
                        || character == '_'
                        || character == '-'
                })
        });
    if valid {
        Ok(())
    } else {
        Err(invalid(format!(
            "evidence id '{value}' must be at least two lowercase dot-separated portable segments"
        )))
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidManifest(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
schema: openstrata.renderer/v1alpha1
renderer:
  name: sample-renderer
composition:
  backend: vulkan
  scene_inputs: [headless]
  units:
    core: sample-render-core
    extraction: sample-render-extraction
    backend: sample-render-vulkan
  adapters:
    headless: sample-render-headless
render_products:
  required: [color, depth]
frame:
  contexts: 3
  completion: explicit
validation:
  gpu_smoke: true
  validation_messages_are_errors: true
  assertions:
    - renderer.core.boundary
    - renderer.backend.capability
    - renderer.gpu.frame
"#;

    #[test]
    fn parses_logical_units_without_implying_packages() {
        let manifest = RendererManifest::parse(MANIFEST).unwrap();
        assert_eq!(manifest.composition.units["core"], "sample-render-core");
        assert_eq!(manifest.frame.contexts, 3);
    }

    #[test]
    fn manifest_is_strict_and_rejects_duplicate_assertions() {
        assert!(RendererManifest::parse(
            &MANIFEST.replace("gpu_smoke: true", "gpu_smoke: true\n  typo: true")
        )
        .is_err());
        assert!(RendererManifest::parse(&MANIFEST.replace(
            "    - renderer.gpu.frame",
            "    - renderer.gpu.frame\n    - renderer.gpu.frame"
        ))
        .is_err());
        // Single-segment ids carry no namespace; the JSON schema also rejects
        // them, so the Rust validation must agree.
        assert!(RendererManifest::parse(&MANIFEST.replace("renderer.gpu.frame", "frame")).is_err());
    }

    #[test]
    fn report_requires_manifest_assertions_and_skip_reasons() {
        let manifest = RendererManifest::parse(MANIFEST).unwrap();
        let source = r#"{
          "schema":"openstrata.renderer-report/v1alpha1",
          "renderer":{"name":"sample-renderer"},
          "device":{"backend":"vulkan","name":"Example GPU","api_version":"1.3.0","driver_version":"42","vendor_id":1,"device_id":2},
          "checks":[
            {"id":"renderer.core.boundary","status":"pass"},
            {"id":"renderer.backend.capability","status":"skip","detail":"Vulkan loader unavailable"},
            {"id":"renderer.gpu.frame","status":"skip","detail":"backend implementation is project-owned"}
          ]
        }"#;
        let report = RendererReport::parse(source).unwrap();
        report.validate_against(&manifest).unwrap();
        assert!(report.passed());
        assert_eq!(report.device.as_ref().unwrap().backend, "vulkan");

        let missing = RendererReport::parse(&source.replace(
            ",\n            {\"id\":\"renderer.gpu.frame\",\"status\":\"skip\",\"detail\":\"backend implementation is project-owned\"}",
            ""
        ))
        .unwrap();
        assert!(missing.validate_against(&manifest).is_err());

        let unexplained = RendererReport::parse(&source.replace(
            "\"detail\":\"Vulkan loader unavailable\"",
            "\"detail\":\"\"",
        ))
        .unwrap();
        assert!(unexplained.validate_against(&manifest).is_err());
    }
}
