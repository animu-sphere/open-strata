// SPDX-License-Identifier: Apache-2.0
//! Portable, verified composition lock. No cache timestamps or host paths are identity.

use ost_artifact::{ArtifactRecord, ArtifactSource, ManifestFile, TrustLevel};
use ost_core::{digest, Category, Error, Result};
use serde::{Deserialize, Serialize};

use crate::{
    resolve_runtime_composition, validate_full_digest, CompositionInput,
    ResolvedRuntimeComposition, RuntimeCompositionManifest,
};

pub const COMPOSITION_LOCK_SCHEMA: &str = "openstrata.runtime-composition-lock/v1alpha1";

pub fn composition_error(code: &'static str, message: impl Into<String>) -> Error {
    Error::coded(code, Category::Validation, message)
}

/// JSON objects use serde_json's sorted map representation, including nested objects.
pub fn canonical_json_digest(value: &impl Serialize) -> Result<String> {
    let value = serde_json::to_value(value)
        .map_err(|e| Error::Operation(format!("cannot serialize composition identity: {e}")))?;
    let bytes = serde_json::to_vec(&value)
        .map_err(|e| Error::Operation(format!("cannot serialize composition identity: {e}")))?;
    Ok(digest::sha256_hex(&bytes))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedCompositionArtifact {
    /// Producer metadata is pinned separately from the payload archive.
    pub manifest_digest: String,
    /// Importer, trust and timestamps are normalized, never inherited from a cache.
    pub record: ArtifactRecord,
}

impl LockedCompositionArtifact {
    pub fn new(mut record: ArtifactRecord, manifest: &serde_json::Value) -> Result<Self> {
        record.created_unix = 0;
        record.imported_by.clear();
        record.source = ArtifactSource::Imported;
        record.trust = TrustLevel::Local;
        Ok(Self {
            manifest_digest: canonical_json_digest(manifest)?,
            record,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionInventoryEntry {
    pub component: String,
    pub artifact: String,
    pub file: ManifestFile,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionDependency {
    pub consumer: String,
    pub capability: String,
    pub provider: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCompositionLock {
    pub schema: String,
    pub manifest: RuntimeCompositionManifest,
    pub resolved: ResolvedRuntimeComposition,
    pub artifacts: Vec<LockedCompositionArtifact>,
    pub dependencies: Vec<CompositionDependency>,
    pub inventory: Vec<CompositionInventoryEntry>,
    pub runtime_digest: String,
}

impl RuntimeCompositionLock {
    pub fn new(
        manifest: RuntimeCompositionManifest,
        mut artifacts: Vec<LockedCompositionArtifact>,
        mut inventory: Vec<CompositionInventoryEntry>,
    ) -> Result<Self> {
        for artifact in &mut artifacts {
            artifact.record.created_unix = 0;
            artifact.record.imported_by.clear();
            artifact.record.source = ArtifactSource::Imported;
            artifact.record.trust = TrustLevel::Local;
        }
        artifacts.sort_by(|a, b| a.record.digest.cmp(&b.record.digest));
        inventory.sort_by(|a, b| a.file.path.cmp(&b.file.path));
        let resolved = resolve_runtime_composition(
            &manifest,
            CompositionInput {
                records: artifacts
                    .iter()
                    .map(|artifact| artifact.record.clone())
                    .collect(),
            },
        )?;
        let mut dependencies = Vec::new();
        for component in &resolved.components {
            let contract = artifacts
                .iter()
                .find(|a| a.record.digest == component.digest)
                .and_then(|a| a.record.component.as_ref())
                .ok_or_else(|| {
                    composition_error("COMPOSITION_LOCK_INVALID", "missing locked component")
                })?;
            for requirement in &contract.requires {
                let provider = resolved
                    .providers
                    .iter()
                    .find(|p| p.capability == requirement.capability)
                    .ok_or_else(|| {
                        composition_error("COMPOSITION_LOCK_INVALID", "missing locked provider")
                    })?;
                dependencies.push(CompositionDependency {
                    consumer: component.id.clone(),
                    capability: requirement.capability.clone(),
                    provider: provider.component.clone(),
                    digest: provider.digest.clone(),
                });
            }
        }
        dependencies.sort();
        dependencies.dedup();
        // Keep acquisition locations for clean reconstruction but give set-like
        // declarations one ordering for stable lock output.
        let mut portable = manifest.canonical();
        for artifact in &mut portable.artifacts {
            artifact.source = manifest
                .artifacts
                .iter()
                .find(|a| a.artifact == artifact.artifact)
                .and_then(|a| a.source.clone());
        }
        let mut lock = Self {
            schema: COMPOSITION_LOCK_SCHEMA.into(),
            manifest: portable,
            resolved,
            artifacts,
            dependencies,
            inventory,
            runtime_digest: String::new(),
        };
        lock.validate_inventory()?;
        lock.runtime_digest = lock.identity()?;
        Ok(lock)
    }

    /// Acquisition URLs, registry records, producer timestamps and evidence
    /// locations do not affect execution identity. Immutable source revisions,
    /// compatibility decisions, contracts, and actual payload inventory do.
    fn identity(&self) -> Result<String> {
        let identities = self
            .artifacts
            .iter()
            .map(|a| {
                serde_json::json!({
                    "digest": a.record.digest,
                    "component": a.record.component,
                    "target": a.record.target,
                    "openusd_compatibility": a.record.openusd_compatibility,
                    "source_identity": a.record.source_identity,
                    "dependency_identities": a.record.dependency_identities,
                })
            })
            .collect::<Vec<_>>();
        canonical_json_digest(&serde_json::json!({
            "schema": self.schema, "manifest": self.manifest.canonical(),
            "resolved": self.resolved, "artifacts": identities,
            "dependencies": self.dependencies, "inventory": self.inventory,
            "layout": "component-prefixes/v1",
        }))
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != COMPOSITION_LOCK_SCHEMA {
            return Err(composition_error(
                "COMPOSITION_LOCK_INVALID",
                "unsupported composition lock schema",
            ));
        }
        validate_full_digest("runtime identity", &self.runtime_digest)?;
        for artifact in &self.artifacts {
            validate_full_digest("producer manifest identity", &artifact.manifest_digest)?;
        }
        let rebuilt = Self::new(
            self.manifest.clone(),
            self.artifacts.clone(),
            self.inventory.clone(),
        )?;
        if rebuilt != *self {
            return Err(composition_error("COMPOSITION_LOCK_MISMATCH",
                "lock does not match its canonical provider, compatibility, dependency or inventory decisions"));
        }
        Ok(())
    }

    fn validate_inventory(&self) -> Result<()> {
        let mut ids = std::collections::BTreeSet::new();
        for component in &self.resolved.components {
            if component.id.ends_with('.') || !ids.insert(component.id.to_ascii_lowercase()) {
                return Err(composition_error(
                    "COMPOSITION_INVENTORY_INVALID",
                    "component prefixes collide or use a nonportable id",
                ));
            }
        }
        let mut paths = std::collections::BTreeSet::new();
        for entry in &self.inventory {
            let component = self
                .resolved
                .components
                .iter()
                .find(|c| c.id == entry.component && c.digest == entry.artifact)
                .ok_or_else(|| {
                    composition_error("COMPOSITION_LOCK_INVALID", "inventory has an unknown owner")
                })?;
            let prefix = format!("components/{}/", component.id);
            let path = &entry.file.path;
            if !path.starts_with(&prefix)
                || path.contains('\\')
                || path.contains(':')
                || path
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
                || !paths.insert(path.to_ascii_lowercase())
            {
                return Err(composition_error(
                    "COMPOSITION_INVENTORY_INVALID",
                    format!("unsafe or colliding inventory path '{path}'"),
                ));
            }
            validate_full_digest("inventory file", &entry.file.sha256)?;
        }
        Ok(())
    }
}
