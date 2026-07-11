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

use crate::policy::TrustLevel;

/// Filename of the registry record within an artifact's object directory.
pub const RECORD_FILE: &str = "record.json";

/// Filename of the producer manifest, both in a dist dir and in the store.
pub const MANIFEST_FILE: &str = "manifest.json";

/// Schema version of [`ArtifactRecord`]. Extend additively; bump on a breaking
/// shape change.
pub const RECORD_SCHEMA: u32 = 1;

/// Producer-manifest `kind` tag for plugin bundles (`ost plugin package`).
pub const PLUGIN_BUNDLE_KIND: &str = "openstrata.plugin-bundle";

/// Producer-manifest `kind` tag for runtime artifacts (future `runtime export`).
pub const RUNTIME_KIND: &str = "openstrata.runtime";

/// What an artifact is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactKind {
    /// A prebuilt OpenUSD runtime (consumable via `RuntimeSource::Artifact`).
    Runtime,
    /// A packaged plugin bundle (`ost plugin package` output).
    Plugin,
    /// A packaged project target (`ost package` output).
    Package,
}

impl ArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ArtifactKind::Runtime => "runtime",
            ArtifactKind::Plugin => "plugin",
            ArtifactKind::Package => "package",
        }
    }

    pub fn from_tag(tag: &str) -> Option<ArtifactKind> {
        match tag {
            "runtime" => Some(ArtifactKind::Runtime),
            "plugin" => Some(ArtifactKind::Plugin),
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
    /// Tool that produced the registry entry, e.g. `ost 0.6.0`.
    pub producer: String,
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
    /// Object-relative path of a generated SBOM, once SBOM generation lands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sbom: Option<String>,
    /// Runtime the artifact was built/validated against (provenance link).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_digest: Option<String>,
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

    /// Derive a record from a producer `manifest.json`.
    ///
    /// Accepts the two manifests OpenStrata produces today — the plugin-bundle
    /// manifest (`kind: openstrata.plugin-bundle`) and the project package
    /// manifest (no `kind` tag) — plus the future `openstrata.runtime` tag.
    pub fn from_producer_manifest(
        manifest: &serde_json::Value,
        source: ArtifactSource,
        created_unix: u64,
        producer: &str,
    ) -> Result<ArtifactRecord> {
        let kind = detect_kind(manifest)?;

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
            ArtifactKind::Runtime | ArtifactKind::Package => {
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
            target: require_str(manifest, "target")?,
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
            producer: producer.to_string(),
            source,
            trust: match source {
                ArtifactSource::Imported => TrustLevel::Local,
                ArtifactSource::Published => TrustLevel::Unsigned,
            },
            validation,
            licenses,
            sbom: None,
            runtime_id,
            runtime_digest,
        })
    }
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
        Some(RUNTIME_KIND) => Ok(ArtifactKind::Runtime),
        Some(other) => Err(Error::InvalidManifest(format!(
            "unrecognized producer manifest kind '{other}' \
             (expected {PLUGIN_BUNDLE_KIND}, {RUNTIME_KIND}, or a project package manifest)"
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
}
