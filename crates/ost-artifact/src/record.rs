// SPDX-License-Identifier: Apache-2.0
//! Artifact identity records (Phase 6, §10).
//!
//! Every artifact in the registry is described by one [`ArtifactRecord`]: what
//! it is (`kind`/`name`/`version`/`target`/`profile`), its content identity
//! (the archive `digest`), and its provenance (`producer`/`source`/
//! `validation`/`licenses`). The record is a fixed-field struct, so its JSON
//! serialization is deterministic (§23) and diffs cleanly in the index.
//!
//! Records are *derived* from a producer manifest — the `manifest.json` that
//! `ost package` / `ost plugin package` write beside their archives — never
//! authored by hand. The producer manifest itself is stored verbatim next to
//! the archive, so the registry adds identity without rewriting provenance.

use serde::{Deserialize, Serialize};

use ost_core::{Error, Result};
use ost_platform::{
    OpenUsdVerification, ResolvedDependencyIdentity, ResolvedOpenUsdCompatibility,
    ResolvedSourceIdentity,
};

use crate::policy::TrustLevel;
use crate::ComponentContract;

/// Filename of the registry record within an artifact's object directory.
pub const RECORD_FILE: &str = "record.json";

/// Filename of the producer manifest, both in a dist dir and in the store.
pub const MANIFEST_FILE: &str = "manifest.json";

/// Schema version of [`ArtifactRecord`]. Extend additively; bump on a breaking
/// shape change.
pub const RECORD_SCHEMA: u32 = 1;

/// Producer-manifest `kind` tag for plugin bundles (`ost plugin package`).
pub const PLUGIN_BUNDLE_KIND: &str = "openstrata.plugin-bundle";

/// Producer-manifest `kind` tag for an aggregate plugin product.
pub const PLUGIN_PRODUCT_KIND: &str = "openstrata.plugin-product";

/// Producer-manifest `kind` tag for a workspace-built executable
/// (`ost plugin package --workspace`, from an `openstrata.tool.yaml` member).
pub const TOOL_KIND: &str = "openstrata.tool";

/// Producer-manifest `kind` tag for runtime artifacts (future `runtime export`).
pub const RUNTIME_KIND: &str = "openstrata.runtime";
/// A locked multi-artifact composition, not a legacy OpenUSD runtime manifest.
pub const COMPOSED_RUNTIME_KIND: &str = "openstrata.composed-runtime";

/// Producer-manifest field selecting the source/dependency-aware OpenUSD
/// selector algorithm. Manifests without this field predate that algorithm and
/// may still carry the legacy selector annotation.
pub const OPENUSD_SELECTOR_SCHEMA_FIELD: &str = "openusd_selector_schema";

/// Current normalized OpenUSD selector algorithm. Schema 3 adds the separate
/// profile/graphics axes, producer release, consumer constraint, and macOS ABI
/// facts. Schema 2 remains readable for already-published leaves.
pub const OPENUSD_SELECTOR_SCHEMA: u32 = 3;

/// What an artifact is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactKind {
    /// A prebuilt OpenUSD runtime (consumable via `RuntimeSource::Artifact`).
    Runtime,
    #[serde(rename = "composed-runtime")]
    ComposedRuntime,
    /// A packaged plugin bundle (`ost plugin package` output).
    Plugin,
    /// An aggregate of exact packaged plugin members.
    Product,
    /// A packaged project target (`ost package` output).
    Package,
}

impl ArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ArtifactKind::Runtime => "runtime",
            ArtifactKind::ComposedRuntime => "composed-runtime",
            ArtifactKind::Plugin => "plugin",
            ArtifactKind::Product => "product",
            ArtifactKind::Package => "package",
        }
    }

    pub fn from_tag(tag: &str) -> Option<ArtifactKind> {
        match tag {
            "runtime" => Some(ArtifactKind::Runtime),
            "composed-runtime" => Some(ArtifactKind::ComposedRuntime),
            "plugin" => Some(ArtifactKind::Plugin),
            "product" => Some(ArtifactKind::Product),
            "package" => Some(ArtifactKind::Package),
            _ => None,
        }
    }
}

/// How an artifact entered the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactSource {
    /// Imported as-is via `ost artifact import` (no publish-gate checks).
    Imported,
    /// Published via a gated command (`ost plugin publish`): validation,
    /// provenance, and license requirements were enforced at entry.
    Published,
}

impl ArtifactSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ArtifactSource::Imported => "imported",
            ArtifactSource::Published => "published",
        }
    }
}

/// The registry's identity record for one artifact.
///
/// Field order is fixed and collection-free (no maps), so serialization is
/// deterministic. The `digest` is the content identity: two records with the
/// same digest describe the same bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub schema: u32,
    pub kind: ArtifactKind,
    pub name: String,
    pub version: String,
    /// Target id, e.g. `cy2026-windows-x86_64-msvc143-py313-usd`.
    pub target: String,
    /// Profile the artifact was produced against, when the producer records it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// `sha256:<hex>` of the archive — the artifact's content identity.
    pub digest: String,
    /// Archive filename within the object directory (as produced).
    pub archive: String,
    pub archive_size: u64,
    /// Total uncompressed bytes across the archived files.
    pub total_size: u64,
    /// Number of files in the archive (from the producer manifest).
    pub file_count: u64,
    /// Seconds since the Unix epoch when the artifact entered the registry.
    pub created_unix: u64,
    /// Tool that produced the *artifact*, as recorded by that tool in its own
    /// dist manifest — e.g. `ost 0.18.0`. `None` when the manifest does not say,
    /// which is every manifest written before v0.18.0.
    ///
    /// Never inferred from the importing process: the same image used to read
    /// `ost 0.10.0` on one machine and `ost 0.17.0` on another purely because
    /// of who imported it. Absent is honest; a guess is not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer: Option<String>,
    /// Tool that created this registry entry, e.g. `ost 0.18.0`. This is what
    /// `producer` held before v0.18.0.
    ///
    /// Defaulted for deserialization so pre-v0.18.0 records still load; they
    /// carry only `producer`, and [`ArtifactRecord::migrate_legacy_producer`]
    /// moves that value here, where it is true. Always serialized, so a record
    /// written by this version is never mistaken for a legacy one.
    #[serde(default)]
    pub imported_by: String,
    pub source: ArtifactSource,
    /// Assurance currently established for this artifact. Old records predate
    /// trust policy and therefore deserialize conservatively as `local`.
    #[serde(default)]
    pub trust: TrustLevel,
    /// Validation outcome carried over from the producer manifest:
    /// `passed` / `failed` / `pending` / `unknown`.
    pub validation: String,
    /// SPDX license expressions recorded by the producer (may be empty for an
    /// `imported` artifact; a `published` one is required to carry at least one).
    #[serde(default)]
    pub licenses: Vec<String>,
    /// Object-relative path of the attached SPDX SBOM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sbom: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sbom_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sbom_size: Option<u64>,
    /// Object-relative path and content identity of SLSA/in-toto provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance_size: Option<u64>,
    /// Runtime the artifact was built/validated against (provenance link).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_digest: Option<String>,
    /// Normalized OpenUSD compatibility identity carried by runtime artifacts.
    ///
    /// This is copied from the producer manifest only after its provider
    /// versions and its platform/target binding have been verified. Older and
    /// non-runtime artifacts omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openusd_compatibility: Option<ResolvedOpenUsdCompatibility>,
    /// Independent verification stages claimed by a runtime producer. This is
    /// kept separate from the aggregate `validation` word so compile/link
    /// success cannot imply loader, physical-device, or render success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openusd_verification: Option<OpenUsdVerification>,
    /// Exact upstream repository and revision declared by the producer.
    ///
    /// The archive digest remains the immutable artifact identity; this field
    /// explains which source revision contributed those bytes and is included
    /// in the deterministic OpenUSD compatibility selector when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_identity: Option<ResolvedSourceIdentity>,
    /// Exact resolved dependency versions and source revisions declared by the
    /// producer, sorted by name for deterministic records and selectors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependency_identities: Vec<ResolvedDependencyIdentity>,
    /// Versioned requirements, provisions, activation, and install ownership
    /// used by runtime composition. Older artifacts omit this field and remain
    /// valid registry entries, but cannot participate in a composed runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<ComponentContract>,
}

impl ArtifactRecord {
    /// The bare hex of the digest (identity key in the object store).
    pub fn digest_hex(&self) -> &str {
        self.digest
            .strip_prefix("sha256:")
            .unwrap_or(self.digest.as_str())
    }

    /// A short human reference, e.g. `sha256:3fa9c1d2…` (12 hex chars).
    pub fn short_digest(&self) -> String {
        let hex = self.digest_hex();
        format!("sha256:{}", &hex[..hex.len().min(12)])
    }

    /// Deterministic OCI-compatible selector for a normalized OpenUSD runtime.
    /// Legacy and non-runtime records do not have enough identity to produce
    /// one and therefore return `None`.
    pub fn openusd_selector(&self) -> Option<String> {
        self.openusd_compatibility
            .as_ref()
            .and_then(|compatibility| {
                compatibility.selector(
                    &self.target,
                    &self.version,
                    self.source_identity.as_ref(),
                    &self.dependency_identities,
                )
            })
    }

    /// Selector emitted before source and dependency identities became part of
    /// the hash. Pull verification uses this only for producer manifests that
    /// do not opt into [`OPENUSD_SELECTOR_SCHEMA`].
    pub(crate) fn legacy_openusd_selector(&self) -> Option<String> {
        self.openusd_compatibility
            .as_ref()
            .and_then(|compatibility| {
                compatibility.selector(&self.target, &self.version, None, &[])
            })
    }

    /// Reinterpret a pre-v0.18.0 record, whose `producer` field held the tool
    /// that *imported* the artifact rather than the one that produced it.
    ///
    /// Such a record has no `imported_by` at all, so an empty one after
    /// deserialization identifies it unambiguously: every record this version
    /// writes serializes `imported_by`. The value moves to the field it was
    /// always describing, and the origin goes back to being unknown rather
    /// than staying a claim the artifact never made.
    ///
    /// Idempotent, and a no-op on records written by this version.
    pub fn migrate_legacy_producer(&mut self) {
        if self.imported_by.is_empty() {
            self.imported_by = self.producer.take().unwrap_or_default();
        }
    }

    /// Derive a record from a producer `manifest.json`.
    ///
    /// Accepts plugin-bundle and aggregate-product manifests, project package
    /// manifests (no `kind` tag), and the runtime tag.
    /// `imported_by` names the tool building this registry entry; the artifact's
    /// own producer is read from the manifest's `producer` field and left `None`
    /// when the manifest does not carry one.
    pub fn from_producer_manifest(
        manifest: &serde_json::Value,
        source: ArtifactSource,
        created_unix: u64,
        imported_by: &str,
    ) -> Result<ArtifactRecord> {
        let kind = detect_kind(manifest)?;
        if kind == ArtifactKind::ComposedRuntime {
            let composition = manifest.get("composition").ok_or_else(|| {
                Error::InvalidManifest("composed runtime requires composition metadata".into())
            })?;
            if composition.get("schema").and_then(|v| v.as_str())
                != Some("openstrata.composed-runtime/v1alpha1")
                || composition.get("lock").and_then(|v| v.as_str())
                    != Some("metadata/composition.lock.json")
                || !composition
                    .get("runtime_digest")
                    .and_then(|v| v.as_str())
                    .is_some_and(is_sha256_ref)
            {
                return Err(Error::InvalidManifest(
                    "composed runtime has an invalid schema, identity or lock path".into(),
                ));
            }
        }
        validate_openusd_selector_schema(manifest, kind)?;

        let manifest_tag = manifest.get("kind").and_then(|value| value.as_str());
        let (name, version, licenses) = match kind {
            ArtifactKind::Plugin => {
                let plugin = require_object(manifest, "plugin")?;
                let licenses = plugin
                    .get("license")
                    .and_then(|v| v.as_str())
                    .map(|s| vec![s.to_string()])
                    .unwrap_or_default();
                (
                    require_str(plugin, "name")?,
                    require_str(plugin, "version")?,
                    licenses,
                )
            }
            ArtifactKind::Package if manifest_tag == Some(TOOL_KIND) => {
                let tool = require_object(manifest, "tool")?;
                let licenses = tool
                    .get("license")
                    .and_then(|value| value.as_str())
                    .map(|value| vec![value.to_string()])
                    .unwrap_or_default();
                (
                    require_str(tool, "id")?,
                    require_str(tool, "version")?,
                    licenses,
                )
            }
            ArtifactKind::Runtime
            | ArtifactKind::ComposedRuntime
            | ArtifactKind::Product
            | ArtifactKind::Package => {
                let licenses = manifest
                    .get("licenses")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                (
                    require_str(manifest, "name")?,
                    require_str(manifest, "version")?,
                    licenses,
                )
            }
        };

        let digest = require_str(manifest, "archive_digest")?;
        if !is_sha256_ref(&digest) {
            return Err(Error::InvalidManifest(format!(
                "producer manifest carries a malformed archive_digest '{digest}' \
                 (expected sha256:<64 hex chars>)"
            )));
        }

        let provenance = manifest.get("provenance");
        let producer_platform = provenance
            .and_then(|p| p.get("platform"))
            .and_then(|v| v.as_str());
        let profile = provenance
            .and_then(|p| p.get("profile"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let runtime = provenance.and_then(|p| p.get("runtime"));
        let runtime_id = runtime
            .and_then(|r| r.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let runtime_digest = runtime
            .and_then(|r| r.get("digest"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let target = require_str(manifest, "target")?;
        let openusd_compatibility =
            normalize_openusd_compatibility(manifest, kind, producer_platform, &target)?;
        let openusd_verification = normalize_openusd_verification(manifest, kind)?;
        let source_identity = normalize_source_identity(manifest)?;
        let dependency_identities = normalize_dependency_identities(manifest)?;
        let component = normalize_component(manifest, &name, &version)?;

        // The two producers record validation differently: the plugin manifest
        // nests `{passed: bool}`, the package manifest carries the runtime's
        // validation string. Normalize both to one word.
        let validation = match provenance.and_then(|p| p.get("validation")) {
            Some(serde_json::Value::Object(v)) => match v.get("passed").and_then(|b| b.as_bool()) {
                Some(true) => "passed".to_string(),
                Some(false) => "failed".to_string(),
                None => "unknown".to_string(),
            },
            Some(serde_json::Value::String(s)) => s.clone(),
            _ => "unknown".to_string(),
        };

        let archive = require_archive_filename(manifest)?;

        Ok(ArtifactRecord {
            schema: RECORD_SCHEMA,
            kind,
            name,
            version,
            target,
            profile,
            digest,
            archive,
            archive_size: require_u64(manifest, "archive_size")?,
            total_size: require_u64(manifest, "total_size")?,
            file_count: manifest
                .get("files")
                .and_then(|v| v.as_array())
                .map(|a| a.len() as u64)
                .unwrap_or(0),
            created_unix,
            producer: manifest
                .get("producer")
                .and_then(|value| value.as_str())
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            imported_by: imported_by.to_string(),
            source,
            trust: match source {
                ArtifactSource::Imported => TrustLevel::Local,
                ArtifactSource::Published => TrustLevel::Unsigned,
            },
            validation,
            licenses,
            sbom: None,
            sbom_digest: None,
            sbom_size: None,
            provenance: None,
            provenance_digest: None,
            provenance_size: None,
            runtime_id,
            runtime_digest,
            openusd_compatibility,
            openusd_verification,
            source_identity,
            dependency_identities,
            component,
        })
    }
}

fn normalize_component(
    manifest: &serde_json::Value,
    artifact_name: &str,
    artifact_version: &str,
) -> Result<Option<ComponentContract>> {
    let Some(value) = manifest.get("component") else {
        return Ok(None);
    };
    let contract: ComponentContract = serde_json::from_value(value.clone()).map_err(|error| {
        Error::InvalidManifest(format!(
            "producer manifest 'component' contract is invalid: {error}"
        ))
    })?;
    contract.validate()?;
    if contract.id != artifact_name {
        return Err(Error::InvalidManifest(format!(
            "component id '{}' does not match artifact name '{artifact_name}'",
            contract.id
        )));
    }
    if contract.version != artifact_version {
        return Err(Error::InvalidManifest(format!(
            "component version '{}' does not match artifact version '{artifact_version}'",
            contract.version
        )));
    }
    Ok(Some(contract))
}

/// Parse the versioned, split OpenUSD verification state and require the
/// artifact-facing copy to match the embedded runtime that pull materializes.
fn normalize_openusd_verification(
    manifest: &serde_json::Value,
    kind: ArtifactKind,
) -> Result<Option<OpenUsdVerification>> {
    let top_level = manifest
        .get("openusd_verification")
        .filter(|value| !value.is_null());
    if kind != ArtifactKind::Runtime && top_level.is_some() {
        return Err(Error::InvalidManifest(
            "only runtime producer manifests may carry 'openusd_verification'".to_string(),
        ));
    }
    if kind != ArtifactKind::Runtime {
        return Ok(None);
    }

    let embedded = manifest
        .pointer("/provenance/runtime_manifest/openusd_verification")
        .filter(|value| !value.is_null());
    let value = match (top_level, embedded) {
        (None, None) => return Ok(None),
        (Some(top_level), Some(embedded)) if top_level == embedded => embedded,
        (Some(_), Some(_)) => {
            return Err(Error::InvalidManifest(
                "producer manifest top-level 'openusd_verification' does not match \
                 provenance.runtime_manifest.openusd_verification"
                    .to_string(),
            ));
        }
        (top_level, embedded) => {
            return Err(Error::InvalidManifest(format!(
                "producer manifest OpenUSD verification state is incomplete \
                 (top-level present={}, embedded runtime present={})",
                top_level.is_some(),
                embedded.is_some()
            )));
        }
    };

    let verification: OpenUsdVerification =
        serde_json::from_value(value.clone()).map_err(|error| {
            Error::InvalidManifest(format!(
                "producer manifest 'openusd_verification' is invalid: {error}"
            ))
        })?;
    if !verification.is_supported() {
        return Err(Error::InvalidManifest(format!(
            "producer manifest carries unsupported OpenUSD verification schema {} (expected 1)",
            verification.schema
        )));
    }
    Ok(Some(verification))
}

fn validate_openusd_selector_schema(
    manifest: &serde_json::Value,
    kind: ArtifactKind,
) -> Result<()> {
    let Some(value) = manifest.get(OPENUSD_SELECTOR_SCHEMA_FIELD) else {
        return Ok(());
    };
    if kind != ArtifactKind::Runtime {
        return Err(Error::InvalidManifest(format!(
            "only runtime producer manifests may carry '{OPENUSD_SELECTOR_SCHEMA_FIELD}'"
        )));
    }
    if !matches!(value.as_u64(), Some(2 | 3)) {
        return Err(Error::InvalidManifest(format!(
            "producer manifest carries unsupported OpenUSD selector schema {} (expected 2 or {OPENUSD_SELECTOR_SCHEMA})",
            value
        )));
    }
    Ok(())
}

/// Normalize the exact producer source revision when build metadata is present.
/// A partial identity is rejected instead of being silently dropped: once a
/// producer emits `build.source`, consumers must be able to rely on both fields.
fn normalize_source_identity(
    manifest: &serde_json::Value,
) -> Result<Option<ResolvedSourceIdentity>> {
    let Some(build) = manifest.get("build").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let source = build.get("source").ok_or_else(|| {
        Error::InvalidManifest("producer manifest build metadata is missing 'source'".to_string())
    })?;
    let repository = source
        .get("repository")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty() && *value == value.trim())
        .ok_or_else(|| {
            Error::InvalidManifest(
                "producer manifest build source is missing an exact, whitespace-normalized 'repository'"
                    .to_string(),
            )
        })?;
    let revision = source
        .get("revision")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty() && *value == value.trim())
        .ok_or_else(|| {
            Error::InvalidManifest(
                "producer manifest build source is missing an exact, whitespace-normalized 'revision'"
                    .to_string(),
            )
        })?;
    Ok(Some(ResolvedSourceIdentity {
        repository: repository.to_string(),
        revision: revision.to_string(),
    }))
}

/// Normalize exact dependency versions and revisions from producer build
/// metadata. Duplicate names are rejected because a selector cannot explain
/// which of two conflicting identities a single dependency name means.
fn normalize_dependency_identities(
    manifest: &serde_json::Value,
) -> Result<Vec<ResolvedDependencyIdentity>> {
    let Some(values) = manifest
        .pointer("/build/dependencies")
        .filter(|value| !value.is_null())
    else {
        return Ok(Vec::new());
    };
    let values = values.as_array().ok_or_else(|| {
        Error::InvalidManifest("producer manifest build dependencies must be an array".to_string())
    })?;
    let mut dependencies = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let dependency: ResolvedDependencyIdentity = serde_json::from_value(value.clone())
                .map_err(|error| {
                    Error::InvalidManifest(format!(
                        "producer manifest build dependency {index} is invalid: {error}"
                    ))
                })?;
            if !dependency.is_verified() {
                return Err(Error::InvalidManifest(format!(
                    "producer manifest build dependency {index} has a blank name, version, repository, or revision"
                )));
            }
            Ok(dependency)
        })
        .collect::<Result<Vec<_>>>()?;
    dependencies.sort_unstable();
    if let Some(pair) = dependencies
        .windows(2)
        .find(|pair| pair[0].name == pair[1].name)
    {
        return Err(Error::InvalidManifest(format!(
            "producer manifest build dependencies repeat name '{}'",
            pair[0].name
        )));
    }
    Ok(dependencies)
}

/// Parse and validate the optional normalized OpenUSD identity on a producer
/// manifest. The compatibility object is artifact identity, so accepting a
/// partially resolved or contradictory value would make the record claim more
/// than the produced bytes establish.
fn normalize_openusd_compatibility(
    manifest: &serde_json::Value,
    kind: ArtifactKind,
    producer_platform: Option<&str>,
    target: &str,
) -> Result<Option<ResolvedOpenUsdCompatibility>> {
    let top_level = manifest
        .get("openusd_compatibility")
        .filter(|value| !value.is_null());
    if kind != ArtifactKind::Runtime && top_level.is_some() {
        return Err(Error::InvalidManifest(
            "only runtime producer manifests may carry 'openusd_compatibility'".to_string(),
        ));
    }
    if kind != ArtifactKind::Runtime {
        return Ok(None);
    }

    // Runtime pull restores the embedded manifest, so it is the compatibility
    // identity that the materialized runtime will actually use. The top-level
    // copy exists for artifact inspection and selection; requiring exact
    // agreement prevents those consumers from seeing a different variant than
    // runtime pull installs.
    let embedded = manifest
        .pointer("/provenance/runtime_manifest/openusd_compatibility")
        .filter(|value| !value.is_null());
    let value = match (top_level, embedded) {
        (None, None) => return Ok(None),
        (Some(top_level), Some(embedded)) if top_level == embedded => embedded,
        (Some(_), Some(_)) => {
            return Err(Error::InvalidManifest(
                "producer manifest top-level 'openusd_compatibility' does not match \
                 provenance.runtime_manifest.openusd_compatibility"
                    .to_string(),
            ));
        }
        (top_level, embedded) => {
            return Err(Error::InvalidManifest(format!(
                "producer manifest OpenUSD compatibility identity is incomplete \
                 (top-level present={}, embedded runtime present={})",
                top_level.is_some(),
                embedded.is_some()
            )));
        }
    };

    let compatibility: ResolvedOpenUsdCompatibility = serde_json::from_value(value.clone())
        .map_err(|error| {
            Error::InvalidManifest(format!(
                "producer manifest 'openusd_compatibility' is invalid: {error}"
            ))
        })?;
    if !matches!(compatibility.schema, 1 | 2) {
        return Err(Error::InvalidManifest(format!(
            "producer manifest carries unsupported OpenUSD compatibility schema {} (expected 1 or 2)",
            compatibility.schema
        )));
    }
    let failure = if compatibility.schema == 1 {
        compatibility.providers_verification_failure()
    } else {
        compatibility.verification_failure()
    };
    if let Some(failure) = failure {
        return Err(Error::InvalidManifest(format!(
            "producer manifest OpenUSD compatibility identity is not verifiable: {failure}"
        )));
    }
    if producer_platform != Some(compatibility.platform.as_str()) {
        return Err(Error::InvalidManifest(format!(
            "producer manifest OpenUSD platform '{}' does not match provenance platform '{}'",
            compatibility.platform,
            producer_platform.unwrap_or("<missing>")
        )));
    }
    let target_prefix = format!(
        "{}-{}-",
        compatibility.os.as_str(),
        compatibility.arch.as_str()
    );
    if !target.starts_with(&target_prefix) {
        return Err(Error::InvalidManifest(format!(
            "producer manifest OpenUSD target {}-{} does not match artifact target '{target}'",
            compatibility.os.as_str(),
            compatibility.arch.as_str()
        )));
    }
    let python_version = compatibility
        .python
        .version
        .as_deref()
        .and_then(python_abi_token)
        .ok_or_else(|| {
            Error::InvalidManifest(
                "producer manifest OpenUSD Python version cannot produce a major/minor ABI tag"
                    .to_string(),
            )
        })?;
    if !target.split('-').any(|token| token == python_version) {
        return Err(Error::InvalidManifest(format!(
            "producer manifest OpenUSD Python ABI '{python_version}' does not match artifact target '{target}'"
        )));
    }

    Ok(Some(compatibility))
}

fn python_abi_token(version: &str) -> Option<String> {
    let mut parts = version.trim().split('.');
    let major = parts.next()?;
    let minor = parts.next()?;
    if major.is_empty()
        || minor.is_empty()
        || !major.bytes().all(|byte| byte.is_ascii_digit())
        || !minor.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    Some(format!("py{major}{minor}"))
}

/// One archived file as listed by the producer manifest (`files[]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    /// For a symlink entry, its (in-tree, relative) target; `sha256`/`size` then
    /// describe the target string, not file contents. Absent for a regular file,
    /// so a pre-symlink manifest still round-trips unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_target: Option<String>,
    /// `true` if the archive entry carries a Unix execute bit. Absent in a
    /// pre-executable-bit manifest (defaults to `false`), so old manifests still
    /// round-trip and a runtime of ordinary data files is unaffected.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub executable: bool,
}

/// An optional debug-symbol archive carried alongside the primary artifact.
///
/// Plugin packages use this for their lean-by-default `*-debug.tar.zst`
/// sidecar. It remains subordinate to the primary artifact identity, but every
/// movement edge must preserve and verify it because `manifest.json` promises
/// that these bytes are available to consumers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugArchive {
    pub archive: String,
    pub digest: String,
    pub archive_size: u64,
    pub files: Vec<ManifestFile>,
}

/// Extract the per-file integrity list from a producer manifest.
pub fn manifest_files(manifest: &serde_json::Value) -> Result<Vec<ManifestFile>> {
    let files = manifest
        .get("files")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            Error::InvalidManifest("producer manifest has no 'files' array".to_string())
        })?;
    files
        .iter()
        .map(|f| {
            serde_json::from_value(f.clone()).map_err(|e| {
                Error::InvalidManifest(format!("producer manifest 'files' entry is invalid: {e}"))
            })
        })
        .collect()
}

/// Parse the optional plugin debug-symbol sidecar recorded under `debug`.
/// Missing `debug` is the ordinary single-archive shape; when present, all
/// identity fields are required and the filename must be safe to join beneath
/// a dist/store directory.
pub fn manifest_debug_archive(manifest: &serde_json::Value) -> Result<Option<DebugArchive>> {
    let Some(debug) = manifest.get("debug") else {
        return Ok(None);
    };
    if !debug.is_object() {
        return Err(Error::InvalidManifest(
            "producer manifest 'debug' must be an object".to_string(),
        ));
    }

    let archive = require_archive_filename(debug)?;
    let digest = require_str(debug, "archive_digest")?;
    if !is_sha256_ref(&digest) {
        return Err(Error::InvalidManifest(format!(
            "producer manifest debug archive carries a malformed archive_digest '{digest}' \
             (expected sha256:<64 hex chars>)"
        )));
    }
    let archive_size = require_u64(debug, "archive_size")?;
    let files = manifest_files(debug)?;

    if manifest
        .get("archive")
        .and_then(|v| v.as_str())
        .is_some_and(|main| main == archive)
    {
        return Err(Error::InvalidManifest(
            "producer manifest debug archive must have a distinct filename".to_string(),
        ));
    }

    Ok(Some(DebugArchive {
        archive,
        digest,
        archive_size,
        files,
    }))
}

/// Classify a producer manifest by its `kind` tag (absent = project package).
fn detect_kind(manifest: &serde_json::Value) -> Result<ArtifactKind> {
    match manifest.get("kind").and_then(|v| v.as_str()) {
        Some(PLUGIN_BUNDLE_KIND) => Ok(ArtifactKind::Plugin),
        Some(PLUGIN_PRODUCT_KIND) => Ok(ArtifactKind::Product),
        Some(RUNTIME_KIND) => Ok(ArtifactKind::Runtime),
        Some(COMPOSED_RUNTIME_KIND) => Ok(ArtifactKind::ComposedRuntime),
        Some(TOOL_KIND) => Ok(ArtifactKind::Package),
        Some(other) => Err(Error::InvalidManifest(format!(
            "unrecognized producer manifest kind '{other}' \
             (expected {PLUGIN_BUNDLE_KIND}, {PLUGIN_PRODUCT_KIND}, {TOOL_KIND}, {RUNTIME_KIND}, or a project package manifest)"
        ))),
        None => Ok(ArtifactKind::Package),
    }
}

/// `true` for a well-formed `sha256:<64 lowercase hex>` reference.
pub fn is_sha256_ref(s: &str) -> bool {
    match s.strip_prefix("sha256:") {
        Some(hex) => hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()),
        None => false,
    }
}

fn require_archive_filename(manifest: &serde_json::Value) -> Result<String> {
    let archive = require_str(manifest, "archive")?;
    if !is_plain_archive_filename(&archive) {
        return Err(Error::InvalidManifest(format!(
            "producer manifest 'archive' must be a plain filename, got '{archive}'"
        )));
    }
    Ok(archive)
}

fn is_plain_archive_filename(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains(':')
        && !name.chars().any(char::is_control)
}

fn require_object<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a serde_json::Value> {
    value.get(key).filter(|v| v.is_object()).ok_or_else(|| {
        Error::InvalidManifest(format!("producer manifest is missing the '{key}' object"))
    })
}

fn require_str(value: &serde_json::Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            Error::InvalidManifest(format!("producer manifest is missing '{key}' (string)"))
        })
}

fn require_u64(value: &serde_json::Value, key: &str) -> Result<u64> {
    value.get(key).and_then(|v| v.as_u64()).ok_or_else(|| {
        Error::InvalidManifest(format!("producer manifest is missing '{key}' (integer)"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin_manifest() -> serde_json::Value {
        serde_json::json!({
            "schema": 1,
            "kind": "openstrata.plugin-bundle",
            "plugin": {
                "name": "toy",
                "version": "0.1.0",
                "kind": "usd-fileformat",
                "license": "Apache-2.0",
            },
            "target": "cy2026-windows-x86_64-msvc143-py313-usd",
            "archive": "toy-0.1.0-cy2026-windows-x86_64-msvc143-py313-usd.tar.zst",
            "archive_digest": format!("sha256:{}", "ab".repeat(32)),
            "archive_size": 1234,
            "total_size": 5678,
            "created_unix": 1_750_000_000,
            "provenance": {
                "platform": "cy2026",
                "profile": "usd",
                "variant": "windows-x86_64-msvc143",
                "cxx_abi": "msvc143",
                "runtime": { "id": "openstrata-cy2026-usd", "digest": "sha256:feed", "source": "local", "validation": "passed" },
                "validation": { "passed": true, "report": "validation/report.json" },
            },
            "files": [
                { "path": "lib/toy.dll", "sha256": "sha256:aa", "size": 10 },
                { "path": "plugInfo.json", "sha256": "sha256:bb", "size": 20 },
            ],
        })
    }

    fn package_manifest() -> serde_json::Value {
        serde_json::json!({
            "schema": 1,
            "name": "demo",
            "version": "1.2.3",
            "target": "cy2026-linux-x86_64-gcc11-py313-usd",
            "archive": "demo-1.2.3.tar.zst",
            "archive_digest": format!("sha256:{}", "cd".repeat(32)),
            "archive_size": 10,
            "total_size": 20,
            "created_unix": 1_750_000_000,
            "provenance": {
                "platform": "cy2026",
                "profile": "usd",
                "runtime": { "id": "rt", "digest": "sha256:beef" },
                "validation": "pending",
            },
            "files": [],
        })
    }

    fn runtime_manifest_with_openusd() -> serde_json::Value {
        let mut manifest = package_manifest();
        manifest["kind"] = serde_json::json!(RUNTIME_KIND);
        manifest["name"] = serde_json::json!("openstrata-cy2026-usd");
        manifest["version"] = serde_json::json!("26.05");
        manifest["target"] = serde_json::json!("linux-x86_64-glibc228-py313");
        let compatibility = serde_json::json!({
            "schema": 2,
            "platform": "cy2026",
            "profile": "usd",
            "os": "linux",
            "arch": "x86_64",
            "toolchain": {
                "family": "gcc",
                "provider": "managed",
                "version": "14.2.0",
                "version_constraint": "14.x",
                "cxx_standard": "20",
                "runtime": {
                    "family": "glibc",
                    "provider": "system",
                    "version": "2.28",
                    "version_constraint": ">=2.28"
                }
            },
            "python": {
                "family": "cpython",
                "provider": "managed",
                "version": "3.13.7",
                "version_constraint": "3.13.x"
            },
            "tbb": {
                "family": "onetbb",
                "provider": "managed",
                "version": "2022.1.0",
                "version_constraint": "2022.x"
            },
            "variant": "vulkan",
            "capabilities": ["hgi-gl", "hgi-vulkan"],
            "producer_openusd_version": "26.05",
            "consumer_openusd_constraint": ">=26.05,<26.09"
        });
        manifest["openusd_compatibility"] = compatibility.clone();
        let verification = serde_json::json!({
            "schema": 1,
            "compile": "passed",
            "link": "passed",
            "loader": "not-run",
            "physical_device": "not-run",
            "render": "not-run"
        });
        manifest["openusd_verification"] = verification.clone();
        manifest["provenance"]["runtime_manifest"] = serde_json::json!({
            "openusd_compatibility": compatibility,
            "openusd_verification": verification,
        });
        manifest
    }

    fn update_openusd_compatibility(
        manifest: &mut serde_json::Value,
        update: impl Fn(&mut serde_json::Value),
    ) {
        update(&mut manifest["openusd_compatibility"]);
        update(&mut manifest["provenance"]["runtime_manifest"]["openusd_compatibility"]);
    }

    #[test]
    fn plugin_manifest_derives_a_plugin_record() {
        let r = ArtifactRecord::from_producer_manifest(
            &plugin_manifest(),
            ArtifactSource::Published,
            1_760_000_000,
            "ost 0.6.0",
        )
        .unwrap();
        assert_eq!(r.kind, ArtifactKind::Plugin);
        assert_eq!(r.name, "toy");
        assert_eq!(r.version, "0.1.0");
        assert_eq!(r.profile.as_deref(), Some("usd"));
        assert_eq!(r.validation, "passed");
        assert_eq!(r.licenses, vec!["Apache-2.0".to_string()]);
        assert_eq!(r.runtime_digest.as_deref(), Some("sha256:feed"));
        assert_eq!(r.file_count, 2);
        assert_eq!(r.source, ArtifactSource::Published);
    }

    #[test]
    fn package_manifest_derives_a_package_record() {
        let r = ArtifactRecord::from_producer_manifest(
            &package_manifest(),
            ArtifactSource::Imported,
            1_760_000_000,
            "ost 0.6.0",
        )
        .unwrap();
        assert_eq!(r.kind, ArtifactKind::Package);
        assert_eq!(r.name, "demo");
        assert_eq!(r.validation, "pending");
        assert!(r.licenses.is_empty());
    }

    #[test]
    fn versioned_component_contract_is_normalized_into_the_record() {
        let mut manifest = package_manifest();
        manifest["component"] = serde_json::json!({
            "schema": crate::COMPONENT_SCHEMA,
            "id": "demo",
            "kind": "library",
            "version": "1.2.3",
            "provides": [
                {"capability": "library:demo", "version": "1.2.3", "singleton": true}
            ],
            "environment": [
                {"variable": "CMAKE_PREFIX_PATH", "operation": "prepend", "values": ["."]}
            ],
            "install": [
                {"source": "lib/demo.lib", "destination": "lib/demo.lib"}
            ]
        });
        let record = ArtifactRecord::from_producer_manifest(
            &manifest,
            ArtifactSource::Imported,
            1_760_000_000,
            "ost test",
        )
        .unwrap();
        let component = record.component.unwrap();
        assert_eq!(component.id, "demo");
        assert_eq!(component.kind, crate::ComponentKind::Library);
        assert_eq!(component.provides[0].capability, "library:demo");
    }

    #[test]
    fn tool_manifest_uses_its_nested_identity() {
        let mut manifest = package_manifest();
        manifest["kind"] = serde_json::json!(TOOL_KIND);
        manifest.as_object_mut().unwrap().remove("name");
        manifest.as_object_mut().unwrap().remove("version");
        manifest["tool"] = serde_json::json!({
            "id": "copc-info",
            "version": "0.4.0",
            "license": "Apache-2.0"
        });
        manifest["component"] = serde_json::json!({
            "schema": crate::COMPONENT_SCHEMA,
            "id": "copc-info",
            "kind": "tool",
            "version": "0.4.0",
            "provides": [
                {"capability": "tool:copc-info", "version": "0.4.0", "singleton": true}
            ]
        });
        let record = ArtifactRecord::from_producer_manifest(
            &manifest,
            ArtifactSource::Imported,
            1_760_000_000,
            "ost test",
        )
        .unwrap();
        assert_eq!(record.kind, ArtifactKind::Package);
        assert_eq!(record.name, "copc-info");
        assert_eq!(record.component.unwrap().kind, crate::ComponentKind::Tool);
    }

    #[test]
    fn aggregate_plugin_product_derives_a_product_record() {
        let mut manifest = package_manifest();
        manifest["kind"] = serde_json::json!(PLUGIN_PRODUCT_KIND);
        manifest["name"] = serde_json::json!("vrm-plugins");
        manifest["licenses"] = serde_json::json!(["Apache-2.0"]);
        manifest["provenance"]["validation"] = serde_json::json!({ "passed": true, "members": 3 });

        let record = ArtifactRecord::from_producer_manifest(
            &manifest,
            ArtifactSource::Imported,
            1_760_000_000,
            "ost 0.19.0",
        )
        .unwrap();

        assert_eq!(record.kind, ArtifactKind::Product);
        assert_eq!(record.name, "vrm-plugins");
        assert_eq!(record.validation, "passed");
        assert_eq!(record.licenses, vec!["Apache-2.0"]);
        assert_eq!(
            ArtifactKind::from_tag("product"),
            Some(ArtifactKind::Product)
        );
    }

    #[test]
    fn runtime_record_preserves_verified_openusd_compatibility() {
        let mut manifest = runtime_manifest_with_openusd();
        manifest["build"] = serde_json::json!({
            "source": {
                "repository": "github.com/PixarAnimationStudios/OpenUSD",
                "revision": "v26.05"
            },
            "dependencies": [
                {
                    "name": "onetbb",
                    "version": "2022.1.0",
                    "source": {
                        "repository": "github.com/uxlfoundation/oneTBB",
                        "revision": "v2022.1.0"
                    }
                }
            ]
        });
        let record = ArtifactRecord::from_producer_manifest(
            &manifest,
            ArtifactSource::Imported,
            1_760_000_000,
            "ost test",
        )
        .unwrap();

        let selector = record.openusd_selector().unwrap();
        let compatibility = record.openusd_compatibility.as_ref().unwrap();
        assert_eq!(compatibility.platform, "cy2026");
        assert_eq!(
            compatibility.variant,
            ost_platform::OpenUsdVariantId::Vulkan
        );
        assert_eq!(compatibility.python.version.as_deref(), Some("3.13.7"));
        assert_eq!(compatibility.capabilities, ["hgi-gl", "hgi-vulkan"]);
        assert_eq!(record.source_identity.as_ref().unwrap().revision, "v26.05");
        assert_eq!(record.dependency_identities[0].name, "onetbb");
        let verification = record.openusd_verification.as_ref().unwrap();
        assert_eq!(
            verification.compile,
            ost_platform::OpenUsdVerificationStatus::Passed
        );
        assert_eq!(
            verification.render,
            ost_platform::OpenUsdVerificationStatus::NotRun
        );
        assert!(selector.starts_with("openusd-cy2026-linux-x86_64-vulkan-"));
        assert_eq!(selector.rsplit_once('-').unwrap().1.len(), 64);

        let mut different_abi = record.clone();
        different_abi.target = "linux-x86_64-glibc234-py313".into();
        assert_ne!(record.openusd_selector(), different_abi.openusd_selector());

        let mut different_openusd = record.clone();
        different_openusd.version = "25.11".into();
        assert_ne!(
            record.openusd_selector(),
            different_openusd.openusd_selector()
        );

        let mut different_source = record.clone();
        different_source.source_identity.as_mut().unwrap().revision = "v26.08".into();
        assert_ne!(
            record.openusd_selector(),
            different_source.openusd_selector()
        );

        let mut different_dependency = record.clone();
        different_dependency.dependency_identities[0]
            .source
            .revision = "v2022.2.0".into();
        assert_ne!(
            record.openusd_selector(),
            different_dependency.openusd_selector()
        );
    }

    #[test]
    fn producer_build_source_is_exact_or_rejected() {
        let mut complete = package_manifest();
        complete["build"] = serde_json::json!({
            "source": { "repository": "owner/repo", "revision": "deadbeef" }
        });
        let record = ArtifactRecord::from_producer_manifest(
            &complete,
            ArtifactSource::Imported,
            0,
            "ost test",
        )
        .unwrap();
        assert_eq!(
            record.source_identity,
            Some(ResolvedSourceIdentity {
                repository: "owner/repo".into(),
                revision: "deadbeef".into(),
            })
        );

        for (field, source) in [
            ("repository", serde_json::json!({ "revision": "deadbeef" })),
            (
                "revision",
                serde_json::json!({ "repository": "owner/repo" }),
            ),
        ] {
            let mut incomplete = package_manifest();
            incomplete["build"] = serde_json::json!({ "source": source });
            let error = ArtifactRecord::from_producer_manifest(
                &incomplete,
                ArtifactSource::Imported,
                0,
                "ost test",
            )
            .unwrap_err();
            assert!(error.to_string().contains(field), "{error}");
        }
    }

    #[test]
    fn producer_build_dependencies_are_exact_sorted_and_unique() {
        let dependency = |name: &str, version: &str| {
            serde_json::json!({
                "name": name,
                "version": version,
                "source": {
                    "repository": format!("github.com/example/{name}"),
                    "revision": format!("v{version}")
                }
            })
        };
        let mut manifest = package_manifest();
        manifest["build"] = serde_json::json!({
            "source": { "repository": "owner/repo", "revision": "deadbeef" },
            "dependencies": [dependency("zlib", "1.3.1"), dependency("onetbb", "2022.1.0")]
        });
        let record = ArtifactRecord::from_producer_manifest(
            &manifest,
            ArtifactSource::Imported,
            0,
            "ost test",
        )
        .unwrap();
        assert_eq!(
            record
                .dependency_identities
                .iter()
                .map(|dependency| dependency.name.as_str())
                .collect::<Vec<_>>(),
            ["onetbb", "zlib"]
        );

        manifest["build"]["dependencies"] =
            serde_json::json!([dependency("zlib", "1.3.1"), dependency("zlib", "1.3.2")]);
        let error = ArtifactRecord::from_producer_manifest(
            &manifest,
            ArtifactSource::Imported,
            0,
            "ost test",
        )
        .unwrap_err();
        assert!(error.to_string().contains("repeat name 'zlib'"), "{error}");

        manifest["build"]["dependencies"] = serde_json::json!([{
            "name": "zlib",
            "version": "",
            "source": { "repository": "zlib/zlib", "revision": "v1.3.1" }
        }]);
        let error = ArtifactRecord::from_producer_manifest(
            &manifest,
            ArtifactSource::Imported,
            0,
            "ost test",
        )
        .unwrap_err();
        assert!(error.to_string().contains("blank name, version"), "{error}");
    }

    #[test]
    fn runtime_record_rejects_unverified_or_contradictory_openusd_identity() {
        let mut unverified = runtime_manifest_with_openusd();
        update_openusd_compatibility(&mut unverified, |compatibility| {
            compatibility["tbb"]
                .as_object_mut()
                .unwrap()
                .remove("version");
        });
        let error = ArtifactRecord::from_producer_manifest(
            &unverified,
            ArtifactSource::Imported,
            0,
            "ost test",
        )
        .unwrap_err();
        // The two halves point at opposite owners, so the message names which
        // one held and for which provider (report 36 §7.1).
        let message = error.to_string();
        assert!(
            message.contains("tbb") && message.contains("(unverified)"),
            "a provider with no observed version is unverified: {message}"
        );

        let mut mismatched_version = runtime_manifest_with_openusd();
        update_openusd_compatibility(&mut mismatched_version, |compatibility| {
            compatibility["python"]["version"] = serde_json::json!("3.12.9");
        });
        let error = ArtifactRecord::from_producer_manifest(
            &mismatched_version,
            ArtifactSource::Imported,
            0,
            "ost test",
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("python")
                && message.contains("3.12.9")
                && message.contains("(contradictory)"),
            "an observed version that fails its own constraint is contradictory: {message}"
        );

        let mut wrong_platform = runtime_manifest_with_openusd();
        update_openusd_compatibility(&mut wrong_platform, |compatibility| {
            compatibility["platform"] = serde_json::json!("cy2025");
        });
        let error = ArtifactRecord::from_producer_manifest(
            &wrong_platform,
            ArtifactSource::Imported,
            0,
            "ost test",
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("does not match provenance platform"));

        let mut wrong_target = runtime_manifest_with_openusd();
        update_openusd_compatibility(&mut wrong_target, |compatibility| {
            compatibility["os"] = serde_json::json!("windows");
        });
        let error = ArtifactRecord::from_producer_manifest(
            &wrong_target,
            ArtifactSource::Imported,
            0,
            "ost test",
        )
        .unwrap_err();
        assert!(error.to_string().contains("does not match artifact target"));

        let mut wrong_python_abi = runtime_manifest_with_openusd();
        wrong_python_abi["target"] = serde_json::json!("linux-x86_64-glibc228-py312");
        let error = ArtifactRecord::from_producer_manifest(
            &wrong_python_abi,
            ArtifactSource::Imported,
            0,
            "ost test",
        )
        .unwrap_err();
        assert!(error.to_string().contains("Python ABI 'py313'"));
    }

    #[test]
    fn runtime_record_rejects_split_openusd_identity() {
        let mut mismatched = runtime_manifest_with_openusd();
        mismatched["openusd_compatibility"]["variant"] = serde_json::json!("headless");
        let error = ArtifactRecord::from_producer_manifest(
            &mismatched,
            ArtifactSource::Imported,
            0,
            "ost test",
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("does not match provenance.runtime_manifest"));

        let mut missing_top_level = runtime_manifest_with_openusd();
        missing_top_level
            .as_object_mut()
            .unwrap()
            .remove("openusd_compatibility");
        let error = ArtifactRecord::from_producer_manifest(
            &missing_top_level,
            ArtifactSource::Imported,
            0,
            "ost test",
        )
        .unwrap_err();
        assert!(error.to_string().contains("identity is incomplete"));

        let mut missing_embedded = runtime_manifest_with_openusd();
        missing_embedded["provenance"]["runtime_manifest"]
            .as_object_mut()
            .unwrap()
            .remove("openusd_compatibility");
        let error = ArtifactRecord::from_producer_manifest(
            &missing_embedded,
            ArtifactSource::Imported,
            0,
            "ost test",
        )
        .unwrap_err();
        assert!(error.to_string().contains("identity is incomplete"));
    }

    #[test]
    fn runtime_record_rejects_split_or_unsupported_verification_state() {
        let mut mismatched = runtime_manifest_with_openusd();
        mismatched["openusd_verification"]["render"] = serde_json::json!("passed");
        let error = ArtifactRecord::from_producer_manifest(
            &mismatched,
            ArtifactSource::Imported,
            0,
            "ost test",
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("top-level 'openusd_verification' does not match"));

        let mut unsupported = runtime_manifest_with_openusd();
        unsupported["openusd_verification"]["schema"] = serde_json::json!(2);
        unsupported["provenance"]["runtime_manifest"]["openusd_verification"]["schema"] =
            serde_json::json!(2);
        let error = ArtifactRecord::from_producer_manifest(
            &unsupported,
            ArtifactSource::Imported,
            0,
            "ost test",
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("unsupported OpenUSD verification schema 2"));
    }

    #[test]
    fn non_runtime_record_cannot_claim_openusd_compatibility() {
        let mut manifest = plugin_manifest();
        manifest["openusd_compatibility"] =
            runtime_manifest_with_openusd()["openusd_compatibility"].clone();
        let error = ArtifactRecord::from_producer_manifest(
            &manifest,
            ArtifactSource::Imported,
            0,
            "ost test",
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("only runtime producer manifests"));
    }

    #[test]
    fn malformed_digest_is_rejected() {
        let mut m = plugin_manifest();
        m["archive_digest"] = serde_json::json!("sha256:short");
        let err =
            ArtifactRecord::from_producer_manifest(&m, ArtifactSource::Imported, 0, "ost test")
                .unwrap_err();
        assert!(err.to_string().contains("archive_digest"), "got: {err}");
    }

    #[test]
    fn pathy_archive_filename_is_rejected() {
        for archive in [
            "",
            ".",
            "..",
            "../toy.tar.zst",
            "nested/toy.tar.zst",
            "nested\\toy.tar.zst",
            "/tmp/toy.tar.zst",
            "C:toy.tar.zst",
            "toy\nextra.tar.zst",
        ] {
            let mut m = plugin_manifest();
            m["archive"] = serde_json::json!(archive);
            let err =
                ArtifactRecord::from_producer_manifest(&m, ArtifactSource::Imported, 0, "ost test")
                    .unwrap_err();
            assert!(err.to_string().contains("archive"), "got: {err}");
        }
    }

    #[test]
    fn debug_archive_identity_is_validated() {
        let mut m = plugin_manifest();
        m["debug"] = serde_json::json!({
            "archive": "toy-debug.tar.zst",
            "archive_digest": format!("sha256:{}", "cd".repeat(32)),
            "archive_size": 42,
            "files": [],
        });
        let debug = manifest_debug_archive(&m).unwrap().unwrap();
        assert_eq!(debug.archive, "toy-debug.tar.zst");

        m["debug"]["archive"] = serde_json::json!("../toy-debug.tar.zst");
        assert!(manifest_debug_archive(&m).is_err());
    }

    #[test]
    fn unknown_kind_is_rejected() {
        let mut m = plugin_manifest();
        m["kind"] = serde_json::json!("openstrata.mystery");
        assert!(ArtifactRecord::from_producer_manifest(
            &m,
            ArtifactSource::Imported,
            0,
            "ost test"
        )
        .is_err());
    }

    #[test]
    fn record_json_is_deterministic_and_roundtrips() {
        let r = ArtifactRecord::from_producer_manifest(
            &plugin_manifest(),
            ArtifactSource::Published,
            1_760_000_000,
            "ost 0.6.0",
        )
        .unwrap();
        let a = serde_json::to_string_pretty(&r).unwrap();
        let b = serde_json::to_string_pretty(&r).unwrap();
        assert_eq!(a, b);
        let back: ArtifactRecord = serde_json::from_str(&a).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn digest_helpers() {
        let r = ArtifactRecord::from_producer_manifest(
            &plugin_manifest(),
            ArtifactSource::Published,
            0,
            "ost",
        )
        .unwrap();
        assert_eq!(r.digest_hex(), "ab".repeat(32));
        assert_eq!(r.short_digest(), format!("sha256:{}", "ab".repeat(6)));
        assert!(is_sha256_ref(&format!("sha256:{}", "0".repeat(64))));
        assert!(!is_sha256_ref("sha256:xyz"));
        assert!(!is_sha256_ref(&"0".repeat(64)));
    }

    #[test]
    fn producer_comes_from_the_manifest_and_never_from_the_importer() {
        // The same bytes used to read `ost 0.10.0` on one machine and
        // `ost 0.17.0` on another, because the field recorded whoever imported
        // it. Origin now comes from the manifest or not at all.
        let mut manifest = plugin_manifest();
        manifest["producer"] = "ost 0.18.0".into();
        let r = ArtifactRecord::from_producer_manifest(
            &manifest,
            ArtifactSource::Published,
            1_750_000_000,
            "ost 0.99.0",
        )
        .unwrap();
        assert_eq!(r.producer.as_deref(), Some("ost 0.18.0"));
        assert_eq!(r.imported_by, "ost 0.99.0", "the importer is recorded too");

        // A manifest that predates the field claims nothing.
        let r = ArtifactRecord::from_producer_manifest(
            &plugin_manifest(),
            ArtifactSource::Published,
            1_750_000_000,
            "ost 0.99.0",
        )
        .unwrap();
        assert_eq!(r.producer, None, "absent is honest; a guess is not");
        assert_eq!(r.imported_by, "ost 0.99.0");

        // An empty string is not a producer either.
        let mut blank = plugin_manifest();
        blank["producer"] = "".into();
        let r = ArtifactRecord::from_producer_manifest(
            &blank,
            ArtifactSource::Published,
            1_750_000_000,
            "ost 0.99.0",
        )
        .unwrap();
        assert_eq!(r.producer, None);

        // A record written before v0.18.0 stored the *importer* under
        // `producer`. Deserializing must land that value in `imported_by`,
        // where it is true, and leave the origin unknown.
        let mut legacy = serde_json::to_value(&r).unwrap();
        let object = legacy.as_object_mut().unwrap();
        object.remove("imported_by");
        object.insert("producer".into(), "ost 0.10.0".into());
        let mut migrated: ArtifactRecord = serde_json::from_value(legacy).unwrap();
        migrated.migrate_legacy_producer();
        assert_eq!(migrated.imported_by, "ost 0.10.0");
        assert_eq!(migrated.producer, None, "the origin was never recorded");

        // Idempotent, and never disturbs a record written by this version.
        let before = migrated.clone();
        migrated.migrate_legacy_producer();
        assert_eq!(migrated, before);
        let mut current = ArtifactRecord {
            producer: Some("ost 0.18.0".into()),
            imported_by: "ost 0.18.0".into(),
            ..before
        };
        let expected = current.clone();
        current.migrate_legacy_producer();
        assert_eq!(current, expected);
    }
}
