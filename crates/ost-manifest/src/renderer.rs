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

/// How a producer session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionOutcome {
    /// The producing command or declared check ran to completion and succeeded.
    Success,
    /// It ran to completion and failed. Its FAIL/SKIP findings are truthful;
    /// its PASSes are not, because the run that would have justified them did
    /// not finish successfully.
    Failure,
    /// It never reached a conclusion — killed, timed out, or still running.
    Incomplete,
}

impl SessionOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Incomplete => "incomplete",
        }
    }
}

/// The producing invocation behind one overlay: who wrote it, against what, and
/// whether it actually finished.
///
/// v0.17.0 had no such binding, so a renderer assertion could read PASS from a
/// CTest that later timed out: the overlay was written mid-run and nothing
/// downstream could tell it apart from one written by a completed check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProducerSession {
    /// Unique per invocation, so a later session supersedes an earlier one.
    pub id: String,
    /// What produced it, e.g. `ctest`, `ost-build`, `renderer-harness`.
    pub kind: String,
    /// The managed target this session wrote.
    pub target: String,
    pub started_unix: u64,
    /// When the session concluded. `None` means it never did — the overlay was
    /// observed mid-flight and is not a completed record of anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_unix: Option<u64>,
    pub outcome: SessionOutcome,
}

impl ProducerSession {
    /// Whether this session may stand behind a PASS: it both concluded and
    /// concluded successfully. Anything else can contribute FAIL/SKIP only.
    pub fn can_assert_pass(&self) -> bool {
        self.outcome == SessionOutcome::Success && self.completed_unix.is_some()
    }

    /// Why this session cannot stand behind a PASS, for an error message.
    fn pass_refusal(&self) -> &'static str {
        match (self.outcome, self.completed_unix) {
            (SessionOutcome::Success, Some(_)) => "completed successfully",
            (SessionOutcome::Success, None) => {
                "claims success but never recorded a completion time"
            }
            (SessionOutcome::Failure, _) => "failed",
            (SessionOutcome::Incomplete, _) => "never completed",
        }
    }

    fn validate(&self) -> Result<()> {
        validate_portable_id("renderer producer session id", &self.id)?;
        validate_portable_id("renderer producer session kind", &self.kind)?;
        if self.target.trim().is_empty() {
            return Err(invalid(
                "renderer producer session target must not be empty",
            ));
        }
        if let Some(completed) = self.completed_unix {
            if completed < self.started_unix {
                return Err(invalid(format!(
                    "renderer producer session '{}' completed before it started",
                    self.id
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererCheck {
    pub id: String,
    pub status: RendererCheckStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Id of the producer session that contributed this check, stamped by
    /// merge. Lets `ost validate` name the producer behind every assertion
    /// instead of presenting a merged report as one anonymous verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer: Option<String>,
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
    /// The invocation that produced this report. Required for an overlay to
    /// contribute a PASS; absent on reports written before v0.18.0, which is
    /// why those cannot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer: Option<ProducerSession>,
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
        if let Some(session) = &self.producer {
            session.validate()?;
            // A report standing on an unfinished or failed producer must not
            // present PASSes as established.
            if !session.can_assert_pass() {
                if let Some(check) = self
                    .checks
                    .iter()
                    .find(|check| check.status == RendererCheckStatus::Pass)
                {
                    return Err(invalid(format!(
                        "renderer report asserts PASS for '{}' but its producer session '{}' {}",
                        check.id,
                        session.id,
                        session.pass_refusal()
                    )));
                }
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

    /// Merge independently produced renderer evidence without hiding conflicts.
    /// Existing ids require an explicit replacement policy, and a recorded FAIL
    /// can never be silently downgraded.
    ///
    /// Every PASS the overlay contributes must be backed by a producer session
    /// that ran to completion and succeeded. A session that failed, never
    /// finished, wrote a different target, or has already been superseded may
    /// still contribute FAIL and SKIP — those remain true — but its PASSes are
    /// refused, because the run that would have justified them did not happen.
    pub fn merge(&self, overlay: &RendererReport, replace: bool) -> Result<RendererReport> {
        if self.renderer != overlay.renderer {
            return Err(invalid(format!(
                "renderer report identity conflict: '{}' != '{}'",
                self.renderer.name, overlay.renderer.name
            )));
        }
        if let Some(session) = &overlay.producer {
            session.validate()?;
        }
        if let Some(session) = &self.producer {
            session.validate()?;
        }

        // A session that wrote a different target describes a different build;
        // its findings are not evidence about this one.
        if let (Some(base), Some(incoming)) = (&self.producer, &overlay.producer) {
            if base.target != incoming.target {
                return Err(invalid(format!(
                    "renderer producer session '{}' wrote target '{}', but this report's \
                     session '{}' wrote '{}' — these describe different builds",
                    incoming.id, incoming.target, base.id, base.target
                )));
            }
            // Replaying an older session over a newer one would resurrect
            // findings the newer run already superseded.
            if incoming.started_unix < base.started_unix {
                return Err(invalid(format!(
                    "renderer producer session '{}' is superseded by '{}', which started later",
                    incoming.id, base.id
                )));
            }
        }

        let overlay_may_pass = overlay
            .producer
            .as_ref()
            .is_some_and(ProducerSession::can_assert_pass);
        if !overlay_may_pass {
            if let Some(check) = overlay
                .checks
                .iter()
                .find(|check| check.status == RendererCheckStatus::Pass)
            {
                let reason = match &overlay.producer {
                    Some(session) => format!(
                        "its producer session '{}' {}",
                        session.id,
                        session.pass_refusal()
                    ),
                    None => "it records no producer session".to_string(),
                };
                return Err(invalid(format!(
                    "renderer report cannot merge PASS for '{}' because {reason}; \
                     a PASS requires a producer that ran to completion and succeeded",
                    check.id
                )));
            }
        }
        let device = match (&self.device, &overlay.device) {
            (Some(left), Some(right)) if left != right => {
                return Err(invalid(format!(
                    "renderer report device/runtime context conflicts for '{}'",
                    self.renderer.name
                )))
            }
            (Some(device), _) | (None, Some(device)) => Some(device.clone()),
            (None, None) => None,
        };

        let mut checks = self.checks.clone();
        for incoming in &overlay.checks {
            // Stamp provenance so a merged report can still name the producer
            // behind each assertion. A check that already carries one keeps it:
            // it came from a still-earlier producer through a prior merge.
            let mut incoming = incoming.clone();
            if incoming.producer.is_none() {
                incoming.producer = overlay.producer.as_ref().map(|s| s.id.clone());
            }
            if let Some(index) = checks.iter().position(|check| check.id == incoming.id) {
                if !replace {
                    return Err(invalid(format!(
                        "renderer report repeats '{}' across inputs; pass an explicit replacement policy",
                        incoming.id
                    )));
                }
                if checks[index].status == RendererCheckStatus::Fail
                    && incoming.status != RendererCheckStatus::Fail
                {
                    return Err(invalid(format!(
                        "renderer report cannot downgrade FAIL for '{}' to {}",
                        incoming.id,
                        incoming.status.as_str()
                    )));
                }
                checks[index] = incoming;
            } else {
                checks.push(incoming);
            }
        }

        // The merged report is owned by the later session: it is the one whose
        // completion the combined evidence now rests on.
        let producer = match (&self.producer, &overlay.producer) {
            (Some(base), Some(incoming)) => Some(if incoming.started_unix >= base.started_unix {
                incoming.clone()
            } else {
                base.clone()
            }),
            (Some(session), None) | (None, Some(session)) => Some(session.clone()),
            (None, None) => None,
        };

        Ok(RendererReport {
            schema: self.schema.clone(),
            renderer: self.renderer.clone(),
            device,
            producer,
            checks,
        })
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

    #[test]
    fn report_merge_requires_replacement_and_preserves_failures() {
        let base = RendererReport::parse(
            r#"{
              "schema":"openstrata.renderer-report/v1alpha1",
              "renderer":{"name":"sample-renderer"},
              "producer":{"id":"s1","kind":"ctest","target":"hdSample",
                          "started_unix":100,"completed_unix":150,"outcome":"success"},
              "checks":[
                {"id":"renderer.core.boundary","status":"pass"},
                {"id":"renderer.gpu.frame","status":"fail","detail":"device lost"}
              ]
            }"#,
        )
        .unwrap();
        let overlay = RendererReport::parse(
            r#"{
              "schema":"openstrata.renderer-report/v1alpha1",
              "renderer":{"name":"sample-renderer"},
              "producer":{"id":"s2","kind":"ctest","target":"hdSample",
                          "started_unix":200,"completed_unix":300,"outcome":"success"},
              "checks":[
                {"id":"renderer.core.boundary","status":"pass"},
                {"id":"renderer.host.first_frame","status":"pass"}
              ]
            }"#,
        )
        .unwrap();
        assert!(base.merge(&overlay, false).is_err());
        let merged = base.merge(&overlay, true).unwrap();
        assert_eq!(merged.checks.len(), 3);
        // Merged checks name the producer that contributed them.
        let first_frame = merged
            .checks
            .iter()
            .find(|c| c.id == "renderer.host.first_frame")
            .unwrap();
        assert_eq!(first_frame.producer.as_deref(), Some("s2"));

        let downgrade = RendererReport::parse(
            r#"{
              "schema":"openstrata.renderer-report/v1alpha1",
              "renderer":{"name":"sample-renderer"},
              "producer":{"id":"s3","kind":"ctest","target":"hdSample",
                          "started_unix":400,"completed_unix":500,"outcome":"success"},
              "checks":[{"id":"renderer.gpu.frame","status":"pass"}]
            }"#,
        )
        .unwrap();
        assert!(base.merge(&downgrade, true).is_err());
    }

    /// Build a report with an explicit producer session.
    fn report_with(session: &str, checks: &str) -> RendererReport {
        RendererReport::parse(&format!(
            r#"{{
              "schema":"openstrata.renderer-report/v1alpha1",
              "renderer":{{"name":"sample-renderer"}},
              {session}
              "checks":[{checks}]
            }}"#
        ))
        .unwrap()
    }

    #[test]
    fn a_pass_requires_a_producer_that_completed_successfully() {
        // The hdMerlin defect: a renderer assertion read PASS from a CTest that
        // later timed out. The overlay looked identical to one from a finished
        // check, so nothing downstream could refuse it.
        let base = report_with(
            r#""producer":{"id":"base","kind":"ost-build","target":"hdSample",
                "started_unix":100,"completed_unix":150,"outcome":"success"},"#,
            r#"{"id":"renderer.core.boundary","status":"pass"}"#,
        );

        // Timed out mid-run: no completion, so its PASS is refused.
        let timed_out = report_with(
            r#""producer":{"id":"t1","kind":"ctest","target":"hdSample",
                "started_unix":200,"outcome":"incomplete"},"#,
            r#"{"id":"renderer.gpu.frame","status":"pass"}"#,
        );
        let error = base.merge(&timed_out, true).unwrap_err().to_string();
        assert!(error.contains("never completed"), "{error}");

        // Ran to completion but failed: PASS is still refused.
        let failed = report_with(
            r#""producer":{"id":"f1","kind":"ctest","target":"hdSample",
                "started_unix":200,"completed_unix":260,"outcome":"failure"},"#,
            r#"{"id":"renderer.gpu.frame","status":"pass"}"#,
        );
        assert!(base
            .merge(&failed, true)
            .unwrap_err()
            .to_string()
            .contains("failed"));

        // …but that same failed session's FAIL is truthful and merges.
        let honest_failure = report_with(
            r#""producer":{"id":"f1","kind":"ctest","target":"hdSample",
                "started_unix":200,"completed_unix":260,"outcome":"failure"},"#,
            r#"{"id":"renderer.gpu.frame","status":"fail","detail":"device lost"}"#,
        );
        let merged = base.merge(&honest_failure, true).unwrap();
        assert_eq!(merged.checks.len(), 2);
        assert!(!merged.passed());

        // A completed, successful session is what a PASS actually requires.
        let good = report_with(
            r#""producer":{"id":"g1","kind":"ctest","target":"hdSample",
                "started_unix":300,"completed_unix":360,"outcome":"success"},"#,
            r#"{"id":"renderer.gpu.frame","status":"pass"}"#,
        );
        let merged = base.merge(&good, true).unwrap();
        assert!(merged.passed());
        assert_eq!(
            merged
                .checks
                .iter()
                .find(|c| c.id == "renderer.gpu.frame")
                .unwrap()
                .producer
                .as_deref(),
            Some("g1")
        );
        // The merged report is owned by the later session.
        assert_eq!(merged.producer.unwrap().id, "g1");
    }

    #[test]
    fn merge_refuses_mismatched_and_superseded_producer_sessions() {
        let base = report_with(
            r#""producer":{"id":"base","kind":"ost-build","target":"hdSample",
                "started_unix":200,"completed_unix":250,"outcome":"success"},"#,
            r#"{"id":"renderer.core.boundary","status":"pass"}"#,
        );

        // A session that wrote a different target is evidence about a
        // different build, however successful it was.
        let other_target = report_with(
            r#""producer":{"id":"x1","kind":"ctest","target":"hdOther",
                "started_unix":300,"completed_unix":360,"outcome":"success"},"#,
            r#"{"id":"renderer.gpu.frame","status":"pass"}"#,
        );
        assert!(base
            .merge(&other_target, true)
            .unwrap_err()
            .to_string()
            .contains("different builds"));

        // Replaying an older session would resurrect superseded findings.
        let older = report_with(
            r#""producer":{"id":"old","kind":"ctest","target":"hdSample",
                "started_unix":100,"completed_unix":150,"outcome":"success"},"#,
            r#"{"id":"renderer.gpu.frame","status":"pass"}"#,
        );
        assert!(base
            .merge(&older, true)
            .unwrap_err()
            .to_string()
            .contains("superseded"));

        // A session claiming success without a completion time is incoherent.
        let no_completion = report_with(
            r#""producer":{"id":"n1","kind":"ctest","target":"hdSample",
                "started_unix":300,"outcome":"success"},"#,
            r#"{"id":"renderer.gpu.frame","status":"pass"}"#,
        );
        assert!(base
            .merge(&no_completion, true)
            .unwrap_err()
            .to_string()
            .contains("never recorded a completion time"));

        // And one that finished before it started is rejected outright.
        let backwards = report_with(
            r#""producer":{"id":"b1","kind":"ctest","target":"hdSample",
                "started_unix":300,"completed_unix":10,"outcome":"success"},"#,
            r#"{"id":"renderer.gpu.frame","status":"pass"}"#,
        );
        assert!(base
            .merge(&backwards, true)
            .unwrap_err()
            .to_string()
            .contains("completed before it started"));
    }
}
