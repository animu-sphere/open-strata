// SPDX-License-Identifier: Apache-2.0
//! The per-runtime manifest written into the store on `pull` (§4.2, §10.2).
//!
//! This is the identity record for an installed runtime: what it is, what it
//! provides, and its digest. The digest is computed over the *canonical* fields
//! only (not the creation time), so the same runtime always digests identically
//! — satisfying the "manifests must be deterministic" bar (§23) while still
//! recording provenance.

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use ost_core::{digest, Variant};

use crate::runtime::Runtime;

/// Native package manager that can satisfy a runtime's host-side dependency.
///
/// These requirements are deliberately separate from `runtime_deps`: the
/// latter are artifact prefixes already resolved into the runtime environment,
/// while these packages must exist on the consuming machine before the runtime
/// can be configured or launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostPackageManager {
    /// Debian/Ubuntu package installed through APT.
    Apt,
    /// macOS formula installed through Homebrew.
    Brew,
}

impl HostPackageManager {
    pub fn as_str(self) -> &'static str {
        match self {
            HostPackageManager::Apt => "apt",
            HostPackageManager::Brew => "brew",
        }
    }

    /// Whether `name` is safe to pass as one package-manager argument and is
    /// representable by that manager's native package syntax.
    pub fn accepts_name(self, name: &str) -> bool {
        let bare = |part: &str| {
            !part.is_empty()
                && !part.starts_with('-')
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
        };
        match self {
            HostPackageManager::Apt => bare(name),
            // Homebrew uses `@` for versioned formulae such as python@3.13.
            // Keep accepting only one shell-safe argv token: taps and local
            // formula paths remain outside this declarative contract.
            HostPackageManager::Brew => match name.split_once('@') {
                Some((formula, version)) => {
                    bare(formula) && bare(version) && !version.contains('@')
                }
                None => bare(name),
            },
        }
    }
}

/// One package the artifact intentionally leaves to the consuming host.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRequirement {
    pub manager: HostPackageManager,
    pub name: String,
}

/// Filename of the runtime manifest within a runtime prefix.
pub const MANIFEST_FILE: &str = "runtime.json";

/// Validation status of an installed runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Validation {
    Passed,
    Failed,
    Pending,
}

impl Validation {
    pub fn as_str(self) -> &'static str {
        match self {
            Validation::Passed => "passed",
            Validation::Failed => "failed",
            Validation::Pending => "pending",
        }
    }
}

/// Where a runtime's artifacts came from (§ Phase 4b backend sources).
///
/// All sources resolve to the same shape (a real prefix + manifest), but they
/// differ in trust: `build`/`artifact` are reproducible/content-addressed,
/// `local` is *real but adopted* (an existing install we did not produce), and
/// `mock` is the placeholder layout the early backend materializes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeSource {
    /// Placeholder prefix layout, no real OpenUSD (the original backend).
    #[default]
    Mock,
    /// An existing USD install adopted in place (`--from-usd` / `OST_USD_ROOT`).
    Local,
    /// Built from source into the store (one-time, digested).
    Build,
    /// Fetched as a prebuilt, content-addressed artifact.
    Artifact,
}

impl RuntimeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeSource::Mock => "mock",
            RuntimeSource::Local => "local",
            RuntimeSource::Build => "build",
            RuntimeSource::Artifact => "artifact",
        }
    }

    /// A real runtime carries actual OpenUSD artifacts (anything but `mock`).
    pub fn is_real(self) -> bool {
        self != RuntimeSource::Mock
    }

    /// Reproducible/certified sources we produced ourselves or fetched by digest.
    /// An adopted `local` runtime is real but *not* reproducible.
    pub fn is_reproducible(self) -> bool {
        matches!(self, RuntimeSource::Build | RuntimeSource::Artifact)
    }
}

/// A resolved extension recorded in a runtime (provenance + identity).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionRecord {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub features: Vec<String>,
}

/// The canonical, digestable description of a runtime. Field order is fixed and
/// `BTreeMap`-free so the serialized form is stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Canonical {
    schema: u32,
    id: String,
    platform: String,
    profile: String,
    variant: Variant,
    python: String,
    capabilities: Vec<String>,
    layout: Vec<String>,
    extensions: Vec<ExtensionRecord>,
    host_requirements: Vec<HostRequirement>,
}

/// A written runtime manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeManifest {
    pub schema: u32,
    pub id: String,
    pub platform: String,
    pub profile: String,
    pub variant: Variant,
    /// Platform Python version, e.g. `3.13.x`.
    pub python: String,
    pub capabilities: Vec<String>,
    /// Subdirectories materialized under the prefix.
    pub layout: Vec<String>,
    /// Extensions this runtime resolves to (id/version/enabled features).
    #[serde(default)]
    pub extensions: Vec<ExtensionRecord>,
    /// `sha256:...` over the canonical fields (excludes `created_unix`).
    pub digest: String,
    pub validation: Validation,
    /// Seconds since the Unix epoch when this manifest was written (provenance).
    pub created_unix: u64,
    /// Where the runtime's artifacts came from. Provenance, not identity (not in
    /// the canonical digest), defaulting to `mock` for pre-4b manifests.
    #[serde(default)]
    pub source: RuntimeSource,
    /// For an adopted (`local`) runtime, the external root its real artifacts
    /// live under. `None` means the store prefix is the root (mock/build).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_prefix: Option<String>,
    /// Dependency prefixes a `build` runtime links against at runtime (e.g. the
    /// `--deps` of a CMake-direct build). Their lib dirs join the session env so
    /// the built USD can load external shared libraries. Empty when the build is
    /// self-contained (build_usd.py installs deps into the prefix).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_deps: Vec<String>,
    /// Native packages the artifact does not bundle but a consumer must have.
    /// This is compatibility identity, not incidental provenance: changing a
    /// requirement changes the runtime digest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host_requirements: Vec<HostRequirement>,
    /// For an `artifact`-sourced runtime, the registry digest (`sha256:<hex>`)
    /// of the artifact it was materialized from. Provenance, not identity (the
    /// canonical `digest` above still describes the runtime itself).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_digest: Option<String>,
}

// Bumped to 4 when host-side package requirements became compatibility
// identity. Older manifests deserialize with an empty list, but the schema gate
// still requires an explicit re-pull/repair before they can be republished.
const SCHEMA: u32 = 4;

impl RuntimeManifest {
    /// Build a manifest for a resolved runtime, computing the digest.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        runtime: &Runtime,
        python_version: &str,
        capabilities: Vec<String>,
        layout: Vec<String>,
        extensions: Vec<ExtensionRecord>,
        created_unix: u64,
        source: RuntimeSource,
    ) -> RuntimeManifest {
        let canonical = Canonical {
            schema: SCHEMA,
            id: runtime.id(),
            platform: runtime.platform.clone(),
            profile: runtime.profile.clone(),
            variant: runtime.variant.clone(),
            python: python_version.to_string(),
            capabilities,
            layout,
            extensions,
            host_requirements: Vec::new(),
        };
        // Serialization of a fixed-field struct is deterministic.
        let bytes = serde_json::to_vec(&canonical).expect("canonical serializes");
        let digest = digest::sha256_hex(&bytes);

        RuntimeManifest {
            schema: SCHEMA,
            id: canonical.id,
            platform: canonical.platform,
            profile: canonical.profile,
            variant: canonical.variant,
            python: canonical.python,
            capabilities: canonical.capabilities,
            layout: canonical.layout,
            extensions: canonical.extensions,
            digest,
            validation: Validation::Pending,
            created_unix,
            source,
            external_prefix: None,
            runtime_deps: Vec::new(),
            host_requirements: Vec::new(),
            artifact_digest: None,
        }
    }

    /// The effective root of the runtime's real artifacts: the adopted external
    /// prefix for a `local` runtime, otherwise the given store `prefix`.
    pub fn effective_prefix<'a>(&'a self, store_prefix: &'a Utf8Path) -> &'a Utf8Path {
        match &self.external_prefix {
            Some(p) => Utf8Path::new(p),
            None => store_prefix,
        }
    }

    /// The schema version this build of OpenStrata writes and expects.
    pub const SCHEMA_VERSION: u32 = SCHEMA;

    /// Recompute the canonical digest from the manifest's own fields. A correct
    /// manifest satisfies `compute_digest() == digest`.
    pub fn compute_digest(&self) -> String {
        let canonical = Canonical {
            schema: self.schema,
            id: self.id.clone(),
            platform: self.platform.clone(),
            profile: self.profile.clone(),
            variant: self.variant.clone(),
            python: self.python.clone(),
            capabilities: self.capabilities.clone(),
            layout: self.layout.clone(),
            extensions: self.extensions.clone(),
            host_requirements: self.host_requirements.clone(),
        };
        let bytes = serde_json::to_vec(&canonical).expect("canonical serializes");
        digest::sha256_hex(&bytes)
    }

    pub fn set_validation(&mut self, validation: Validation) {
        self.validation = validation;
    }

    /// Replace host requirements with a deterministic, duplicate-free set and
    /// refresh the runtime identity that includes them.
    pub fn set_host_requirements(&mut self, mut requirements: Vec<HostRequirement>) {
        requirements.sort();
        requirements.dedup();
        self.host_requirements = requirements;
        self.digest = self.compute_digest();
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(src: &str) -> Result<RuntimeManifest, serde_json::Error> {
        serde_json::from_str(src)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ost_core::host::{Arch, Os};
    use ost_core::Host;

    fn sample() -> RuntimeManifest {
        let host = Host {
            os: Os::Linux,
            arch: Arch::X86_64,
        };
        let rt = Runtime::resolve("cy2026", "usd", &host, "3.13.x");
        RuntimeManifest::build(
            &rt,
            "3.13.x",
            vec!["usd-stage-read".into()],
            vec!["bin".into(), "lib".into()],
            vec![ExtensionRecord {
                id: "openusd".into(),
                version: "25.05.01".into(),
                features: vec!["core".into()],
            }],
            1_700_000_000,
            RuntimeSource::Mock,
        )
    }

    #[test]
    fn digest_roundtrips() {
        let m = sample();
        assert_eq!(m.compute_digest(), m.digest);
    }

    #[test]
    fn validation_change_does_not_affect_digest() {
        let mut m = sample();
        let before = m.digest.clone();
        m.set_validation(Validation::Passed);
        assert_eq!(m.compute_digest(), before);
    }

    #[test]
    fn host_requirements_are_sorted_identity() {
        let mut m = sample();
        let before = m.digest.clone();
        m.set_host_requirements(vec![
            HostRequirement {
                manager: HostPackageManager::Apt,
                name: "libxt-dev".into(),
            },
            HostRequirement {
                manager: HostPackageManager::Apt,
                name: "libx11-dev".into(),
            },
            HostRequirement {
                manager: HostPackageManager::Apt,
                name: "libx11-dev".into(),
            },
        ]);

        assert_ne!(m.digest, before);
        assert_eq!(m.compute_digest(), m.digest);
        assert_eq!(m.host_requirements.len(), 2);
        assert_eq!(m.host_requirements[0].name, "libx11-dev");

        let json = m.to_json().unwrap();
        let roundtrip = RuntimeManifest::from_json(&json).unwrap();
        assert_eq!(roundtrip, m);
    }

    #[test]
    fn package_names_follow_the_native_manager_syntax() {
        assert!(HostPackageManager::Apt.accepts_name("libx11-dev"));
        assert!(!HostPackageManager::Apt.accepts_name("python@3.13"));
        assert!(HostPackageManager::Brew.accepts_name("python@3.13"));
        assert!(!HostPackageManager::Brew.accepts_name("python@@3.13"));
        assert!(!HostPackageManager::Brew.accepts_name("--formula"));
    }

    #[test]
    fn source_trust_tiers() {
        assert!(!RuntimeSource::Mock.is_real());
        assert!(RuntimeSource::Local.is_real());
        assert!(RuntimeSource::Build.is_real());
        assert!(RuntimeSource::Artifact.is_real());

        // Only sources we produced or fetched by digest are reproducible.
        assert!(!RuntimeSource::Mock.is_reproducible());
        assert!(!RuntimeSource::Local.is_reproducible());
        assert!(RuntimeSource::Build.is_reproducible());
        assert!(RuntimeSource::Artifact.is_reproducible());
    }

    #[test]
    fn effective_prefix_follows_external_root_for_local() {
        let store = Utf8Path::new("/store/runtimes/cy2026-usd");

        // No external_prefix (mock/build): the store prefix is the root.
        let m = sample();
        assert_eq!(m.effective_prefix(store), store);

        // Adopted local: the external root wins over the store prefix.
        let mut adopted = sample();
        adopted.external_prefix = Some("/opt/usd".into());
        assert_eq!(adopted.effective_prefix(store), Utf8Path::new("/opt/usd"));
    }

    #[test]
    fn source_is_not_part_of_digest() {
        // Provenance, not identity: changing only the source must not move the
        // digest (§23 — manifests are deterministic over their canonical form).
        let mock = sample();
        let mut local = sample();
        local.source = RuntimeSource::Local;
        local.external_prefix = Some("/opt/usd".into());
        // Recompute from the canonical form: source/external_prefix are excluded.
        assert_eq!(local.compute_digest(), mock.digest);
    }
}
