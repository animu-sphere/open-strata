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
use ost_platform::{
    OpenUsdVerification, ResolvedDependencyIdentity, ResolvedOpenUsdCompatibility,
    ResolvedSourceIdentity,
};

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
    openusd_compatibility: Option<ResolvedOpenUsdCompatibility>,
    openusd_verification: OpenUsdVerification,
    build_source: Option<ResolvedSourceIdentity>,
    build_dependencies: Vec<ResolvedDependencyIdentity>,
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
    /// Exact CY cell and constrained OpenUSD build variant selected for a
    /// managed source build. Absent for legacy, mock, and adopted runtimes,
    /// where OpenStrata cannot honestly claim those inputs controlled the
    /// produced bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openusd_compatibility: Option<ResolvedOpenUsdCompatibility>,
    /// Independent compile/link/loader/device/render evidence for OpenUSD.
    /// A successful managed build establishes only compile and link; later
    /// stages remain `not-run` until a command actually observes them.
    #[serde(default)]
    pub openusd_verification: OpenUsdVerification,
    /// Exact source checkout used by a managed OpenUSD build. This is captured
    /// from Git rather than inferred later from the machine exporting it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_source: Option<ResolvedSourceIdentity>,
    /// Exact source archives that `build_usd.py` selected and installed. The
    /// sorted closure is runtime identity and is forwarded into artifact build
    /// metadata, selector hashing, provenance, and the SPDX SBOM.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub build_dependencies: Vec<ResolvedDependencyIdentity>,
    /// For an `artifact`-sourced runtime, the registry digest (`sha256:<hex>`)
    /// of the artifact it was materialized from. Provenance, not identity (the
    /// canonical `digest` above still describes the runtime itself).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_digest: Option<String>,
}

// Bumped to 7 when independent OpenUSD verification stages became runtime
// identity. Older manifests deserialize with every stage `not-run`, but the
// schema gate still requires an explicit rebuild before publication as a
// normalized v0.22 artifact.
const SCHEMA: u32 = 7;

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
            openusd_compatibility: None,
            openusd_verification: OpenUsdVerification::default(),
            build_source: None,
            build_dependencies: Vec::new(),
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
            openusd_compatibility: None,
            openusd_verification: OpenUsdVerification::default(),
            build_source: None,
            build_dependencies: Vec::new(),
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
            openusd_compatibility: self.openusd_compatibility.clone(),
            openusd_verification: self.openusd_verification.clone(),
            build_source: self.build_source.clone(),
            build_dependencies: self.build_dependencies.clone(),
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

    /// Bind a resolved OpenUSD cell/variant to runtime identity.
    pub fn set_openusd_compatibility(
        &mut self,
        compatibility: Option<ResolvedOpenUsdCompatibility>,
    ) -> ost_core::Result<()> {
        if compatibility
            .as_ref()
            .is_some_and(|value| !value.is_verified())
        {
            return Err(ost_core::Error::InvalidManifest(
                "OpenUSD compatibility identity has unverified or contradictory provider versions"
                    .to_string(),
            ));
        }
        self.openusd_compatibility = compatibility;
        self.digest = self.compute_digest();
        Ok(())
    }

    /// Replace the independently observed OpenUSD verification stages.
    pub fn set_openusd_verification(
        &mut self,
        verification: OpenUsdVerification,
    ) -> ost_core::Result<()> {
        if !verification.is_supported() {
            return Err(ost_core::Error::InvalidManifest(format!(
                "unsupported OpenUSD verification schema {} (expected 1)",
                verification.schema
            )));
        }
        self.openusd_verification = verification;
        self.digest = self.compute_digest();
        Ok(())
    }

    /// Bind the exact managed source and dependency closure to runtime
    /// identity. Dependency order follows names, not download order, so two
    /// equivalent builds serialize identically.
    pub fn set_build_identities(
        &mut self,
        source: Option<ResolvedSourceIdentity>,
        mut dependencies: Vec<ResolvedDependencyIdentity>,
    ) -> ost_core::Result<()> {
        if source.as_ref().is_some_and(|value| !value.is_verified())
            || dependencies.iter().any(|value| !value.is_verified())
        {
            return Err(ost_core::Error::InvalidManifest(
                "managed build source or dependency identity is incomplete".to_string(),
            ));
        }
        dependencies.sort_by(|left, right| left.name.cmp(&right.name));
        if dependencies
            .windows(2)
            .any(|pair| pair[0].name == pair[1].name)
        {
            return Err(ost_core::Error::InvalidManifest(
                "managed build dependency names must be unique".to_string(),
            ));
        }
        self.build_source = source;
        self.build_dependencies = dependencies;
        self.digest = self.compute_digest();
        Ok(())
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
    fn openusd_verification_is_split_and_digest_significant() {
        let mut manifest = sample();
        assert_eq!(
            manifest.openusd_verification,
            OpenUsdVerification::default()
        );
        let before = manifest.digest.clone();
        manifest
            .set_openusd_verification(OpenUsdVerification::managed_build_passed())
            .unwrap();

        assert_ne!(manifest.digest, before);
        assert_eq!(manifest.compute_digest(), manifest.digest);
        assert_eq!(
            manifest.openusd_verification.compile,
            ost_platform::OpenUsdVerificationStatus::Passed
        );
        assert_eq!(
            manifest.openusd_verification.link,
            ost_platform::OpenUsdVerificationStatus::Passed
        );
        assert_eq!(
            manifest.openusd_verification.loader,
            ost_platform::OpenUsdVerificationStatus::NotRun
        );
        assert_eq!(
            manifest.openusd_verification.physical_device,
            ost_platform::OpenUsdVerificationStatus::NotRun
        );
        assert_eq!(
            manifest.openusd_verification.render,
            ost_platform::OpenUsdVerificationStatus::NotRun
        );

        let roundtrip = RuntimeManifest::from_json(&manifest.to_json().unwrap()).unwrap();
        assert_eq!(
            roundtrip.openusd_verification,
            manifest.openusd_verification
        );
    }

    #[test]
    fn unsupported_openusd_verification_schema_cannot_be_stamped() {
        let mut manifest = sample();
        let mut verification = OpenUsdVerification::managed_build_passed();
        verification.schema = 2;
        let error = manifest.set_openusd_verification(verification).unwrap_err();
        assert!(error
            .to_string()
            .contains("unsupported OpenUSD verification schema 2"));
        assert_eq!(
            manifest.openusd_verification,
            OpenUsdVerification::default()
        );
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
    fn openusd_compatibility_is_digest_significant() {
        let platform = ost_platform::load_one("cy2026").unwrap();
        let (compatibility, _) = platform
            .resolve_openusd(
                Os::Linux,
                Arch::X86_64,
                ost_platform::OpenUsdVariantId::Headless,
            )
            .unwrap();
        let mut manifest = sample();
        let before = manifest.digest.clone();
        let mut compatibility = compatibility;
        compatibility.toolchain.version = Some("14.2.0".into());
        compatibility.toolchain.runtime.version = Some("2.28".into());
        compatibility.python.version = Some("3.13.7".into());
        compatibility.tbb.version = Some("2022.1.0".into());
        manifest
            .set_openusd_compatibility(Some(compatibility))
            .unwrap();
        assert_ne!(manifest.digest, before);
        assert_eq!(manifest.compute_digest(), manifest.digest);
        assert_eq!(
            manifest.openusd_compatibility.as_ref().unwrap().variant,
            ost_platform::OpenUsdVariantId::Headless
        );
    }

    #[test]
    fn unresolved_openusd_compatibility_cannot_be_stamped() {
        let platform = ost_platform::load_one("cy2026").unwrap();
        let (compatibility, _) = platform
            .resolve_openusd(
                Os::Linux,
                Arch::X86_64,
                ost_platform::OpenUsdVariantId::Standard,
            )
            .unwrap();
        let mut manifest = sample();
        let error = manifest
            .set_openusd_compatibility(Some(compatibility))
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("unverified or contradictory provider versions"));
        assert!(manifest.openusd_compatibility.is_none());
    }

    #[test]
    fn managed_build_identities_are_sorted_and_digest_significant() {
        let mut manifest = sample();
        let before = manifest.digest.clone();
        let source = ResolvedSourceIdentity {
            repository: "https://github.com/PixarAnimationStudios/OpenUSD".into(),
            revision: "2095fafafd033fa23386d7ec6d58c7cc33974518".into(),
        };
        manifest
            .set_build_identities(
                Some(source.clone()),
                vec![
                    ResolvedDependencyIdentity {
                        name: "oneTBB".into(),
                        version: "2022.1.0".into(),
                        source: ResolvedSourceIdentity {
                            repository: "https://github.com/uxlfoundation/oneTBB".into(),
                            revision: "v2022.1.0".into(),
                        },
                        archive_digest: None,
                    },
                    ResolvedDependencyIdentity {
                        name: "MaterialX".into(),
                        version: "1.39.5".into(),
                        source: ResolvedSourceIdentity {
                            repository: "https://github.com/AcademySoftwareFoundation/MaterialX"
                                .into(),
                            revision: "v1.39.5".into(),
                        },
                        archive_digest: None,
                    },
                ],
            )
            .unwrap();

        assert_ne!(manifest.digest, before);
        assert_eq!(manifest.build_source, Some(source));
        assert_eq!(manifest.build_dependencies[0].name, "MaterialX");
        assert_eq!(manifest.compute_digest(), manifest.digest);
        assert_eq!(
            RuntimeManifest::from_json(&manifest.to_json().unwrap()).unwrap(),
            manifest
        );
    }

    #[test]
    fn managed_build_identities_reject_duplicates_and_blanks() {
        let dependency = ResolvedDependencyIdentity {
            name: "oneTBB".into(),
            version: "2022.1.0".into(),
            source: ResolvedSourceIdentity {
                repository: "https://github.com/uxlfoundation/oneTBB".into(),
                revision: "v2022.1.0".into(),
            },
            archive_digest: None,
        };
        let mut manifest = sample();
        assert!(manifest
            .set_build_identities(None, vec![dependency.clone(), dependency])
            .is_err());
        assert!(manifest
            .set_build_identities(
                Some(ResolvedSourceIdentity {
                    repository: "".into(),
                    revision: "revision".into(),
                }),
                Vec::new(),
            )
            .is_err());
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
