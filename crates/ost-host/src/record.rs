// SPDX-License-Identifier: Apache-2.0
//! The versioned host record: what OpenStrata knows about one DCC install.
//!
//! A record is deliberately *evidence*, not an assertion. It carries where the
//! install was found, what confirmed it, how its version was resolved and how
//! much that resolution is worth, and — when validation failed — why. Nothing
//! here claims OpenStrata installed, licensed, or may modify the host.

use std::collections::BTreeMap;

use camino::{Utf8Path, Utf8PathBuf};
use ost_core::host::{Arch, Os};
use ost_core::{digest, Category, Error, Result};
use serde::{Deserialize, Serialize};

/// Schema of a persisted host record. Distinct from the `--json` envelope
/// `schema`, exactly as `runtime.json` and the plugin report are.
pub const HOST_RECORD_SCHEMA: &str = "openstrata.host-record/v1alpha1";

/// Version of the *input set* a standard fingerprint is computed over. Bumping
/// this invalidates cached fingerprints without pretending an install changed.
pub const FINGERPRINT_INPUTS_VERSION: u32 = 1;

/// A supported third-party DCC product family.
///
/// "Host" is a DCC install already on the machine; [`ost_core::Host`] is the
/// machine itself. The two never mix in this crate: the machine appears only as
/// [`HostPlatform`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostFamily {
    Maya,
    Houdini,
}

impl HostFamily {
    /// Every family this `ost` version can validate, in a stable order.
    pub const ALL: [HostFamily; 2] = [HostFamily::Maya, HostFamily::Houdini];

    pub fn as_str(self) -> &'static str {
        match self {
            HostFamily::Maya => "maya",
            HostFamily::Houdini => "houdini",
        }
    }

    /// The vendor's product name, recorded so a report reads as a product
    /// rather than an internal slug.
    pub fn product(self) -> &'static str {
        match self {
            HostFamily::Maya => "Autodesk Maya",
            HostFamily::Houdini => "SideFX Houdini",
        }
    }

    /// Parse a family selector, naming the supported families on failure —
    /// this is the one place that list is authoritative.
    pub fn parse(value: &str) -> Result<HostFamily> {
        HostFamily::ALL
            .into_iter()
            .find(|family| family.as_str() == value)
            .ok_or_else(|| {
                let supported = HostFamily::ALL
                    .iter()
                    .map(|family| family.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                Error::coded(
                    "HOST_FAMILY_UNKNOWN",
                    Category::Usage,
                    format!("unknown host family '{value}'"),
                )
                .with_hint(format!("supported families: {supported}"))
            })
    }
}

/// Lifecycle of a record, mirroring the plugin harness's "candidate →
/// validated, never a false PASS".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostStatus {
    /// A provider suggested this root; no validator has confirmed it.
    Candidate,
    /// A family validator confirmed markers, executables, and identity.
    Validated,
    /// A validator looked and refused. The reason is in [`HostRecord::rejection`].
    Rejected,
    /// Previously validated, but the install's bytes have changed since.
    Stale,
    /// Previously validated, and the root is no longer readable.
    Unreachable,
    /// Recorded under an input set or schema this `ost` no longer honours.
    Invalidated,
}

impl HostStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            HostStatus::Candidate => "candidate",
            HostStatus::Validated => "validated",
            HostStatus::Rejected => "rejected",
            HostStatus::Stale => "stale",
            HostStatus::Unreachable => "unreachable",
            HostStatus::Invalidated => "invalidated",
        }
    }

    /// Whether this record may be used to run something. Only a fresh,
    /// confirmed validation qualifies.
    pub fn is_usable(self) -> bool {
        matches!(self, HostStatus::Validated)
    }

    pub fn parse(value: &str) -> Result<HostStatus> {
        const ALL: [HostStatus; 6] = [
            HostStatus::Candidate,
            HostStatus::Validated,
            HostStatus::Rejected,
            HostStatus::Stale,
            HostStatus::Unreachable,
            HostStatus::Invalidated,
        ];
        ALL.into_iter()
            .find(|status| status.as_str() == value)
            .ok_or_else(|| {
                let supported = ALL
                    .iter()
                    .map(|status| status.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                Error::coded(
                    "HOST_STATUS_UNKNOWN",
                    Category::Usage,
                    format!("unknown host status '{value}'"),
                )
                .with_hint(format!("supported statuses: {supported}"))
            })
    }
}

/// Which provider suggested a candidate root. Order is the resolution order,
/// and every source that reached a root is retained on the record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiscoverySource {
    /// The operator named this exact path.
    ExplicitPath,
    /// A `[host.discovery].roots` entry in the project manifest.
    ConfiguredRoot,
    /// A vendor environment variable (`MAYA_LOCATION`, `HFS`).
    Environment,
    /// A documented default install location for this OS.
    KnownInstallRoot,
    /// An executable found on `PATH`. Opt-in: never consulted by default.
    ExecutablePath,
}

impl DiscoverySource {
    pub fn as_str(self) -> &'static str {
        match self {
            DiscoverySource::ExplicitPath => "explicit-path",
            DiscoverySource::ConfiguredRoot => "configured-root",
            DiscoverySource::Environment => "environment",
            DiscoverySource::KnownInstallRoot => "known-install-root",
            DiscoverySource::ExecutablePath => "executable-path",
        }
    }
}

/// One provider's account of how a root was reached. Records keep *all* of
/// them: an install found three ways is one install with three provenances,
/// and dropping two of them would hide a site's actual configuration.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DiscoveryEvidence {
    pub source: DiscoverySource,
    /// What the provider actually used: the variable name, the configured
    /// root, the scanned default, or the resolved executable.
    pub detail: String,
}

/// A selected executable, recorded relative to the install root so the record
/// describes a *layout* rather than repeating a machine-local prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostExecutable {
    /// Root-relative, forward-slash normalized.
    pub path: String,
    pub size: u64,
    /// Seconds since the Unix epoch, when the platform reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_unix: Option<i64>,
}

impl HostExecutable {
    pub fn absolute(&self, root: &Utf8Path) -> Utf8PathBuf {
        root.join(&self.path)
    }
}

/// Where a version came from, which is the whole difference between a fact and
/// a directory name that looks like one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VersionSource {
    /// Read out of a file the vendor ships (an SDK header, a version file).
    Metadata,
    /// Reported by the host's own executable under a bounded timeout.
    Probe,
    /// Inferred from the install directory's name.
    DirectoryName,
}

impl VersionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            VersionSource::Metadata => "metadata",
            VersionSource::Probe => "probe",
            VersionSource::DirectoryName => "directory-name",
        }
    }

    /// A directory name is a convention, not a guarantee: a site can rename
    /// `2025.3` to `current`, or patch an install without renaming anything.
    /// Callers that need certainty must require `high`.
    pub fn confidence(self) -> &'static str {
        match self {
            VersionSource::Metadata | VersionSource::Probe => "high",
            VersionSource::DirectoryName => "low",
        }
    }
}

/// A resolved host version and how much the resolution is worth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostVersion {
    /// Exactly as resolved, e.g. `2026` or `20.5.550`.
    pub raw: String,
    pub major: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minor: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    pub source: VersionSource,
    /// Denormalized from `source` so a consumer branching on trust does not
    /// have to know which sources are trustworthy.
    pub confidence: String,
}

impl HostVersion {
    /// Build a version from a dotted string, keeping the raw text intact.
    pub fn from_raw(raw: &str, source: VersionSource) -> Option<HostVersion> {
        let raw = raw.trim();
        let mut parts = raw.split('.');
        let major: u32 = parts.next()?.trim().parse().ok()?;
        let minor = parts
            .next()
            .and_then(|part| part.trim().parse::<u32>().ok());
        let rest: Vec<&str> = parts.collect();
        let build = if rest.is_empty() {
            None
        } else {
            Some(rest.join("."))
        };
        Some(HostVersion {
            raw: raw.to_string(),
            major,
            minor,
            build,
            source,
            confidence: source.confidence().to_string(),
        })
    }
}

/// The host's embedded Python, which is the ABI a plugin built for this host
/// must match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostPython {
    /// `3.11` — major.minor, which is what ABI compatibility turns on.
    pub version: String,
    /// `cp311`, the conventional ABI tag.
    pub abi_tag: String,
    pub source: VersionSource,
    /// Root-relative evidence for the detection, so a wrong answer is
    /// debuggable without re-running discovery.
    pub evidence: String,
}

impl HostPython {
    pub fn from_layout(version: &str, evidence: impl Into<String>) -> Option<HostPython> {
        let mut parts = version.split('.');
        let major: u32 = parts.next()?.parse().ok()?;
        let minor: u32 = parts.next()?.parse().ok()?;
        Some(HostPython {
            version: format!("{major}.{minor}"),
            abi_tag: format!("cp{major}{minor}"),
            source: VersionSource::Metadata,
            evidence: evidence.into(),
        })
    }
}

/// The machine the install was observed on. A record does not travel to a
/// different platform, and saying so is cheaper than discovering it later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostPlatform {
    pub os: Os,
    pub arch: Arch,
}

impl HostPlatform {
    pub fn detect() -> HostPlatform {
        HostPlatform::for_os(Os::current())
    }

    /// This machine's architecture, against the OS a scan was run for — which
    /// tests inject. A record must state the OS whose layout rules produced it,
    /// or a fixture pass would validate Linux paths and stamp them `windows`.
    pub fn for_os(os: Os) -> HostPlatform {
        HostPlatform {
            os,
            arch: Arch::current(),
        }
    }
}

/// How much of the install a fingerprint looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FingerprintMode {
    /// Cheap and bounded: identity, layout, and the selected executables'
    /// size/mtime. Never hashes an install.
    Standard,
    /// Opt-in: additionally hashes the selected executables' contents.
    Deep,
}

impl FingerprintMode {
    pub fn as_str(self) -> &'static str {
        match self {
            FingerprintMode::Standard => "standard",
            FingerprintMode::Deep => "deep",
        }
    }

    pub fn parse(value: &str) -> Result<FingerprintMode> {
        match value {
            "standard" => Ok(FingerprintMode::Standard),
            "deep" => Ok(FingerprintMode::Deep),
            other => Err(Error::coded(
                "HOST_FINGERPRINT_MODE_UNKNOWN",
                Category::Usage,
                format!("unknown fingerprint mode '{other}'"),
            )
            .with_hint("supported modes: standard, deep")),
        }
    }
}

/// A digest over a versioned input set, distinguishing installs that share a
/// nominal version but differ in patch level, runtime, or layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostFingerprint {
    pub mode: FingerprintMode,
    pub inputs_version: u32,
    /// `sha256:<hex>`, the same shape as every other digest in the tree.
    pub digest: String,
}

/// Why a validator refused a candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostRejection {
    /// Stable machine code, e.g. `HOST_MARKERS_MISSING`.
    pub code: String,
    pub message: String,
}

/// Everything OpenStrata knows about one DCC install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRecord {
    pub schema_version: String,
    /// Deterministic instance id, stable across re-discovery of the same root.
    pub id: String,
    pub family: HostFamily,
    pub product: String,
    pub status: HostStatus,
    /// The canonical install root on this machine.
    pub root: Utf8PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<HostVersion>,
    /// Selected executables by role (`interactive`, `interpreter`, `batch`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub executables: BTreeMap<String, HostExecutable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python: Option<HostPython>,
    pub platform: HostPlatform,
    /// Root-relative paths that confirmed this is the product it claims to be.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<String>,
    /// Every provider that reached this root, in resolution order.
    pub evidence: Vec<DiscoveryEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<HostFingerprint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection: Option<HostRejection>,
}

impl HostRecord {
    /// The deterministic instance id for an install.
    ///
    /// Derived from family, version and *canonical root* — deliberately not
    /// from the fingerprint. Patching an install in place must keep its
    /// identity (so a pin still names the same thing) while changing its
    /// fingerprint (so the change is visible). Two ids that differ are two
    /// installs; one id whose fingerprint moved is one install that changed.
    pub fn instance_id(
        family: HostFamily,
        version: Option<&HostVersion>,
        root: &Utf8Path,
    ) -> String {
        let version_slug = version
            .map(|version| sanitize_id_component(&version.raw))
            .unwrap_or_else(|| "unknown".to_string());
        let root_digest = hex_digest(canonical_root_key(root).as_bytes());
        format!("{}-{version_slug}-{}", family.as_str(), &root_digest[..8])
    }

    /// The `sha256:`-prefixed fingerprint digest, when one was computed.
    pub fn fingerprint_digest(&self) -> Option<&str> {
        self.fingerprint
            .as_ref()
            .map(|fingerprint| fingerprint.digest.as_str())
    }

    /// The order records are listed in: usable installs first, then family, then
    /// id. Shared by the scan and every listing so a printed inventory cannot
    /// drift from the order a scan produced.
    pub fn listing_order(&self) -> (bool, HostFamily, &str) {
        (!self.status.is_usable(), self.family, self.id.as_str())
    }

    /// A one-line human summary: id, version, and what the version is worth.
    pub fn summary(&self) -> String {
        let version = match &self.version {
            Some(version) if version.source == VersionSource::DirectoryName => {
                format!("{} (from directory name)", version.raw)
            }
            Some(version) => version.raw.clone(),
            None => "unknown version".to_string(),
        };
        format!("{} {version} [{}]", self.id, self.status.as_str())
    }
}

/// Record a path the way every other path-bearing field in this tree is
/// written: forward slashes, no trailing separator.
///
/// Windows accepts `/` everywhere, and a record whose separators depend on
/// which provider happened to find the install — one built from an environment
/// variable, one from a `join` — is a record two runs can disagree about while
/// describing the same directory.
pub fn normalize_path(path: &Utf8Path) -> Utf8PathBuf {
    let normalized = path.as_str().replace('\\', "/");
    let trimmed = normalized.trim_end_matches('/');
    // A bare root (`/`, `C:/`) has nothing left to trim to.
    if trimmed.is_empty() || trimmed.ends_with(':') {
        return Utf8PathBuf::from(normalized);
    }
    Utf8PathBuf::from(trimmed)
}

/// The key a root is identified by. Case is folded on platforms whose paths
/// are case-insensitive, so `C:\Program Files\...` and `c:\program files\...`
/// are one install rather than two.
///
/// The fold follows the *running* machine, not the OS a scan was requested for:
/// whether two paths are one directory is a fact about the filesystem doing the
/// lookup, and a record never travels to another platform anyway.
pub fn canonical_root_key(root: &Utf8Path) -> String {
    let normalized = normalize_path(root).into_string();
    if matches!(Os::current(), Os::Windows | Os::Macos) {
        normalized.to_lowercase()
    } else {
        normalized
    }
}

/// The bare hex of a SHA-256.
///
/// [`digest::sha256_hex`] renders `sha256:<hex>` so the algorithm travels with
/// the value, which is right for a digest field and wrong inside an id — an id
/// is constrained to `[A-Za-z0-9._-]` and a colon is not in it.
fn hex_digest(bytes: &[u8]) -> String {
    let rendered = digest::sha256_hex(bytes);
    rendered
        .strip_prefix("sha256:")
        .map(str::to_string)
        .unwrap_or(rendered)
}

/// Keep an id component inside `[A-Za-z0-9._-]`, the shape every other
/// OpenStrata identifier uses.
fn sanitize_id_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

/// The confirmed facts about one install: everything a fingerprint covers and
/// everything a record is built from.
#[derive(Debug, Clone, Copy)]
pub struct HostIdentity<'a> {
    pub family: HostFamily,
    pub root: &'a Utf8Path,
    pub version: Option<&'a HostVersion>,
    pub executables: &'a BTreeMap<String, HostExecutable>,
    pub python: Option<&'a HostPython>,
    pub markers: &'a [String],
    pub platform: HostPlatform,
}

/// Compute a fingerprint over a versioned, explicitly listed input set.
///
/// The inputs are serialized as canonical JSON and hashed, so what the digest
/// covers is readable in one place and a change to it is a deliberate
/// [`FINGERPRINT_INPUTS_VERSION`] bump rather than a silent drift.
pub fn fingerprint(identity: &HostIdentity<'_>, mode: FingerprintMode) -> Result<HostFingerprint> {
    let mut inputs = serde_json::json!({
        "inputs_version": FINGERPRINT_INPUTS_VERSION,
        "mode": mode.as_str(),
        "family": identity.family.as_str(),
        "root": canonical_root_key(identity.root),
        "os": identity.platform.os.as_str(),
        "arch": identity.platform.arch.as_str(),
        "version": identity.version.map(|version| version.raw.clone()),
        "python": identity.python.map(|python| python.version.clone()),
        "markers": identity.markers,
        "executables": identity.executables,
    });

    if mode == FingerprintMode::Deep {
        // Deep mode is the opt-in answer to "same version, different bytes":
        // it hashes the executables the standard mode only sizes.
        let mut hashes = serde_json::Map::new();
        for (role, executable) in identity.executables {
            let path = executable.absolute(identity.root);
            let mut file = std::fs::File::open(path.as_std_path())
                .map_err(|error| Error::io(path.to_string(), error))?;
            // Already rendered `sha256:<hex>` by the shared helper.
            let (digest, _) = digest::sha256_hex_reader(&mut file)
                .map_err(|error| Error::io(path.to_string(), error))?;
            hashes.insert(role.clone(), serde_json::Value::String(digest));
        }
        inputs["executable_digests"] = serde_json::Value::Object(hashes);
    }

    let canonical = serde_json::to_vec(&inputs).map_err(|error| {
        Error::Operation(format!("cannot serialize fingerprint inputs: {error}"))
    })?;
    Ok(HostFingerprint {
        mode,
        inputs_version: FINGERPRINT_INPUTS_VERSION,
        digest: digest::sha256_hex(&canonical),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_id_survives_a_patch_but_separates_two_installs() {
        let root = Utf8Path::new("/tools/maya/2025.3");
        let version = HostVersion::from_raw("2025.3", VersionSource::Metadata).unwrap();
        let first = HostRecord::instance_id(HostFamily::Maya, Some(&version), root);
        let again = HostRecord::instance_id(HostFamily::Maya, Some(&version), root);
        assert_eq!(first, again, "the same install keeps its identity");

        let sibling = HostRecord::instance_id(
            HostFamily::Maya,
            Some(&version),
            Utf8Path::new("/tools/maya/2025.3-patched"),
        );
        assert_ne!(first, sibling, "a different root is a different install");

        // An id must stay inside `[A-Za-z0-9._-]` like every other OpenStrata
        // identifier — the shared digest helper renders `sha256:<hex>`, and that
        // colon has no business in an id.
        assert_eq!(first, "maya-2025.3-".to_string() + &first[12..], "{first}");
        assert!(
            first
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')),
            "{first}"
        );
        let suffix = &first[12..];
        assert_eq!(suffix.len(), 8, "{first}");
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()), "{first}");
    }

    #[test]
    fn version_records_what_its_source_is_worth() {
        let metadata = HostVersion::from_raw("20.5.550", VersionSource::Metadata).unwrap();
        assert_eq!(metadata.major, 20);
        assert_eq!(metadata.minor, Some(5));
        assert_eq!(metadata.build.as_deref(), Some("550"));
        assert_eq!(metadata.confidence, "high");

        let guessed = HostVersion::from_raw("2026", VersionSource::DirectoryName).unwrap();
        assert_eq!(
            guessed.confidence, "low",
            "a directory name is a convention"
        );
        assert!(HostVersion::from_raw("current", VersionSource::DirectoryName).is_none());
    }

    #[test]
    fn fingerprint_changes_with_bytes_and_holds_across_recomputation() {
        let root = Utf8Path::new("/tools/maya/2026");
        let mut executables = BTreeMap::new();
        executables.insert(
            "interpreter".to_string(),
            HostExecutable {
                path: "bin/mayapy".into(),
                size: 128,
                modified_unix: Some(1_700_000_000),
            },
        );
        let markers = vec!["bin".to_string()];
        let platform = HostPlatform {
            os: Os::Linux,
            arch: Arch::X86_64,
        };
        let compute = |executables: &BTreeMap<String, HostExecutable>| {
            fingerprint(
                &HostIdentity {
                    family: HostFamily::Maya,
                    root,
                    version: None,
                    executables,
                    python: None,
                    markers: &markers,
                    platform,
                },
                FingerprintMode::Standard,
            )
            .unwrap()
        };

        let first = compute(&executables);
        assert_eq!(first, compute(&executables), "recomputation is stable");
        let hex = first
            .digest
            .strip_prefix("sha256:")
            .unwrap_or_else(|| panic!("digest must be `sha256:<hex>`: {}", first.digest));
        assert_eq!(hex.len(), 64, "{}", first.digest);
        assert!(
            hex.chars().all(|c| c.is_ascii_hexdigit()),
            "{}",
            first.digest
        );

        executables.get_mut("interpreter").unwrap().size = 129;
        assert_ne!(
            first.digest,
            compute(&executables).digest,
            "a re-installed executable must not read as the same install"
        );
    }

    #[test]
    fn family_and_status_parsing_names_the_supported_values() {
        assert_eq!(HostFamily::parse("houdini").unwrap(), HostFamily::Houdini);
        let error = HostFamily::parse("blender").unwrap_err();
        assert_eq!(error.code(), "HOST_FAMILY_UNKNOWN");
        assert!(error.hint().unwrap().contains("maya"));
        assert_eq!(
            HostStatus::parse("validated").unwrap(),
            HostStatus::Validated
        );
        assert!(HostStatus::parse("passing").is_err());
    }
}
