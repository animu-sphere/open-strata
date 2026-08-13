// SPDX-License-Identifier: Apache-2.0
//! The platform manifest data model.

use indexmap::IndexMap;
use ost_core::{
    digest,
    host::{Arch, Os},
};
use serde::{Deserialize, Serialize};

/// Provenance of a platform definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    VfxReferencePlatform,
    /// A studio- or user-authored platform that is not an upstream CY release.
    Custom,
}

/// Lifecycle status of a calendar-year definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Upstream draft, subject to change.
    Draft,
    /// Ratified final spec.
    Final,
    /// Superseded by a later year.
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub kind: SourceKind,
    #[serde(default = "default_status")]
    pub status: Status,
}

fn default_status() -> Status {
    Status::Draft
}

/// A VFX Reference Platform calendar-year definition (§4.1).
///
/// `core` is an ordered map of component → version constraint. Using an
/// [`IndexMap`] keeps the document order stable for deterministic display and
/// lets the set of components evolve year to year without a code change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Platform {
    /// Calendar-year id, e.g. `cy2026`.
    pub id: String,
    pub source: Source,
    /// Component → version constraint, e.g. `python: "3.13.x"`.
    pub core: IndexMap<String, String>,
    /// Approved OpenUSD build cells for this CY. Compatibility-critical
    /// versions are referenced from `core` instead of being copied into an
    /// imperative resolver, while the supported cell/variant matrix remains
    /// explicit data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openusd: Option<OpenUsdPolicy>,
    /// Optional free-form notes shown by `ost platform show`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Data-driven OpenUSD compatibility policy for one calendar year.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenUsdPolicy {
    pub schema: u32,
    pub cells: Vec<OpenUsdCell>,
}

/// One approved platform/architecture/provider cell. The full Cartesian
/// product is intentionally not implied: a missing cell is unsupported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenUsdCell {
    pub os: Os,
    pub arch: Arch,
    pub toolchain: OpenUsdToolchain,
    pub python: OpenUsdProvider,
    pub tbb: OpenUsdProvider,
    pub variants: Vec<OpenUsdVariant>,
}

/// A compatibility-critical component supplied by a named provider. Its exact
/// version is resolved through `version_from`, a key in the CY `core` map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenUsdProvider {
    pub family: String,
    pub provider: String,
    pub version_from: String,
}

/// Compiler and native runtime boundary for an approved build cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenUsdToolchain {
    pub family: String,
    pub provider: String,
    pub version_from: String,
    pub cxx_standard_from: String,
    pub runtime: OpenUsdProvider,
}

/// The constrained initial OpenUSD build variants from the v0.22.0 roadmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenUsdVariantId {
    Headless,
    Standard,
    Vulkan,
}

impl OpenUsdVariantId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Headless => "headless",
            Self::Standard => "standard",
            Self::Vulkan => "vulkan",
        }
    }
}

/// Declarative build inputs and resulting capabilities for one variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenUsdVariant {
    pub id: OpenUsdVariantId,
    #[serde(default)]
    pub builders: Vec<OpenUsdBuilder>,
    #[serde(default)]
    pub build_usd_args: Vec<String>,
    #[serde(default)]
    pub cmake_args: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OpenUsdBuilder {
    BuildUsd,
    Cmake,
}

/// Outcome of one independently evidenced OpenUSD verification stage.
///
/// `not-run` is deliberately distinct from failure and success. In particular,
/// a build runner without a graphics device must preserve that absence instead
/// of allowing a successful compile to imply device or render verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OpenUsdVerificationStatus {
    NotRun,
    Passed,
    Failed,
}

impl OpenUsdVerificationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRun => "not-run",
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }
}

/// Separately tracked verification stages for a produced OpenUSD runtime.
///
/// The stages are observations, not a ladder: importing or launching a runtime
/// may establish a later stage without rewriting what an earlier producer
/// actually recorded. Callers therefore must never infer one field from
/// another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenUsdVerification {
    pub schema: u32,
    pub compile: OpenUsdVerificationStatus,
    pub link: OpenUsdVerificationStatus,
    pub loader: OpenUsdVerificationStatus,
    pub physical_device: OpenUsdVerificationStatus,
    pub render: OpenUsdVerificationStatus,
}

impl Default for OpenUsdVerification {
    fn default() -> Self {
        Self {
            schema: 1,
            compile: OpenUsdVerificationStatus::NotRun,
            link: OpenUsdVerificationStatus::NotRun,
            loader: OpenUsdVerificationStatus::NotRun,
            physical_device: OpenUsdVerificationStatus::NotRun,
            render: OpenUsdVerificationStatus::NotRun,
        }
    }
}

impl OpenUsdVerification {
    /// Evidence established by a managed source build that returned success.
    pub fn managed_build_passed() -> Self {
        Self {
            compile: OpenUsdVerificationStatus::Passed,
            link: OpenUsdVerificationStatus::Passed,
            ..Self::default()
        }
    }

    pub fn is_supported(&self) -> bool {
        self.schema == 1
    }
}

/// Exact, normalized compatibility selection stored in runtime identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedOpenUsdCompatibility {
    pub schema: u32,
    pub platform: String,
    pub os: Os,
    pub arch: Arch,
    pub toolchain: ResolvedOpenUsdToolchain,
    pub python: ResolvedOpenUsdProvider,
    pub tbb: ResolvedOpenUsdProvider,
    pub variant: OpenUsdVariantId,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedOpenUsdProvider {
    pub family: String,
    pub provider: String,
    /// Exact version observed from the selected build input or produced tree.
    /// `None` exists only while a build cell is being prepared; a managed
    /// runtime must not persist the compatibility selection until this is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// CY constraint the observed version was checked against.
    pub version_constraint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedOpenUsdToolchain {
    pub family: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub version_constraint: String,
    pub cxx_standard: String,
    pub runtime: ResolvedOpenUsdProvider,
}

/// Exact upstream source revision that contributed artifact bytes.
///
/// This is kept separate from the compatibility cell because the cell is
/// selected before a build runs, while source identity is attached by a
/// producer when it exports the resulting artifact.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedSourceIdentity {
    pub repository: String,
    pub revision: String,
}

impl ResolvedSourceIdentity {
    pub fn is_verified(&self) -> bool {
        exact_identity_component(&self.repository) && exact_identity_component(&self.revision)
    }
}

/// One exact dependency selected by the artifact producer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedDependencyIdentity {
    pub name: String,
    pub version: String,
    pub source: ResolvedSourceIdentity,
    /// Digest of the exact source archive consumed by the producer, when the
    /// dependency was resolved from an archive rather than a content-addressed
    /// source repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_digest: Option<String>,
}

impl ResolvedDependencyIdentity {
    pub fn is_verified(&self) -> bool {
        exact_identity_component(&self.name)
            && exact_identity_component(&self.version)
            && self.source.is_verified()
            && self
                .archive_digest
                .as_deref()
                .is_none_or(is_canonical_sha256)
    }
}

fn is_canonical_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn exact_identity_component(value: &str) -> bool {
    !value.is_empty() && value == value.trim()
}

/// Whether one observed numeric version satisfies a CY compatibility
/// constraint. Plain dotted constraints are prefix matches (`14.2` accepts
/// `14.2.0`), `x`/`X`/`*` are trailing wildcards, and the usual comparison
/// prefixes are supported for runtime floors such as `>=2.28`.
pub fn version_satisfies_constraint(observed: &str, constraint: &str) -> bool {
    let Some(observed) = numeric_version(observed) else {
        return false;
    };
    let constraint = constraint.trim();
    if constraint.is_empty() {
        return false;
    }

    for operator in [">=", "<=", ">", "<", "="] {
        let Some(required) = constraint.strip_prefix(operator) else {
            continue;
        };
        let Some(required) = numeric_version(required) else {
            return false;
        };
        let ordering = compare_numeric_versions(&observed, &required);
        return match operator {
            ">=" => matches!(
                ordering,
                std::cmp::Ordering::Greater | std::cmp::Ordering::Equal
            ),
            "<=" => matches!(
                ordering,
                std::cmp::Ordering::Less | std::cmp::Ordering::Equal
            ),
            ">" => ordering == std::cmp::Ordering::Greater,
            "<" => ordering == std::cmp::Ordering::Less,
            "=" => ordering == std::cmp::Ordering::Equal,
            _ => unreachable!("the operator table is exhaustive"),
        };
    }

    let parts = constraint.split('.').collect::<Vec<_>>();
    if parts.is_empty() {
        return false;
    }
    for (index, required) in parts.iter().enumerate() {
        if matches!(*required, "x" | "X" | "*") {
            return parts[index..]
                .iter()
                .all(|part| matches!(*part, "x" | "X" | "*"));
        }
        let Ok(required) = required.parse::<u64>() else {
            return false;
        };
        if observed.get(index) != Some(&required) {
            return false;
        }
    }
    true
}

fn numeric_version(value: &str) -> Option<Vec<u64>> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    value
        .split('.')
        .map(|part| {
            if !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()) {
                part.parse::<u64>().ok()
            } else {
                None
            }
        })
        .collect()
}

fn compare_numeric_versions(left: &[u64], right: &[u64]) -> std::cmp::Ordering {
    let length = left.len().max(right.len());
    (0..length)
        .map(|index| {
            left.get(index)
                .copied()
                .unwrap_or_default()
                .cmp(&right.get(index).copied().unwrap_or_default())
        })
        .find(|ordering| *ordering != std::cmp::Ordering::Equal)
        .unwrap_or(std::cmp::Ordering::Equal)
}

fn provider_is_verified(provider: &ResolvedOpenUsdProvider) -> bool {
    !provider.family.trim().is_empty()
        && !provider.provider.trim().is_empty()
        && !provider.version_constraint.trim().is_empty()
        && provider.version.as_ref().is_some_and(|version| {
            version_satisfies_constraint(version, &provider.version_constraint)
        })
}

impl ResolvedOpenUsdCompatibility {
    /// Whether every compatibility-critical provider carries an observed exact
    /// version that satisfies its non-empty CY constraint.
    pub fn is_verified(&self) -> bool {
        !self.platform.trim().is_empty()
            && !self.toolchain.family.trim().is_empty()
            && !self.toolchain.provider.trim().is_empty()
            && !self.toolchain.version_constraint.trim().is_empty()
            && !self.toolchain.cxx_standard.trim().is_empty()
            && self.toolchain.version.as_ref().is_some_and(|version| {
                version_satisfies_constraint(version, &self.toolchain.version_constraint)
            })
            && provider_is_verified(&self.toolchain.runtime)
            && provider_is_verified(&self.python)
            && provider_is_verified(&self.tbb)
    }

    /// A deterministic, OCI-tag-safe selector for this exact compatibility
    /// identity.
    ///
    /// The readable prefix is deliberately short; the full SHA-256 suffix is
    /// over every compatibility-critical field (including providers, exact
    /// versions, the OpenUSD release, exact source/dependency revisions, C++
    /// standard, and sorted capability/dependency sets). This keeps the
    /// selector within OCI's 128-character tag limit without dropping identity
    /// dimensions from the comparison contract.
    pub fn selector(
        &self,
        artifact_target: &str,
        openusd_version: &str,
        source: Option<&ResolvedSourceIdentity>,
        dependencies: &[ResolvedDependencyIdentity],
    ) -> Option<String> {
        if !self.is_verified()
            || artifact_target.trim().is_empty()
            || openusd_version.trim().is_empty()
            || source.is_some_and(|value| !value.is_verified())
            || dependencies.iter().any(|value| !value.is_verified())
        {
            return None;
        }

        #[derive(Serialize)]
        struct SelectorIdentity<'a> {
            schema: u32,
            platform: &'a str,
            os: Os,
            arch: Arch,
            artifact_target: &'a str,
            openusd_version: &'a str,
            source: Option<&'a ResolvedSourceIdentity>,
            dependencies: Vec<&'a ResolvedDependencyIdentity>,
            toolchain: &'a ResolvedOpenUsdToolchain,
            python: &'a ResolvedOpenUsdProvider,
            tbb: &'a ResolvedOpenUsdProvider,
            variant: OpenUsdVariantId,
            capabilities: Vec<&'a str>,
        }

        let mut capabilities = self
            .capabilities
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        capabilities.sort_unstable();
        capabilities.dedup();
        let mut dependencies = dependencies.iter().collect::<Vec<_>>();
        dependencies.sort_unstable();
        dependencies.dedup();
        let identity = SelectorIdentity {
            schema: self.schema,
            platform: &self.platform,
            os: self.os,
            arch: self.arch,
            artifact_target,
            openusd_version,
            source,
            dependencies,
            toolchain: &self.toolchain,
            python: &self.python,
            tbb: &self.tbb,
            variant: self.variant,
            capabilities,
        };
        let bytes = serde_json::to_vec(&identity).expect("selector identity serializes");
        let digest = digest::sha256_hex(&bytes);
        let hash = digest
            .strip_prefix("sha256:")
            .expect("sha256 renderer includes its algorithm");

        let readable = format!(
            "openusd-{}-{}-{}-{}",
            self.platform,
            self.os.as_str(),
            self.arch.as_str(),
            self.variant.as_str()
        );
        let mut prefix = String::with_capacity(readable.len());
        let mut previous_separator = false;
        for ch in readable.chars() {
            let normalized = if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch.to_ascii_lowercase()
            } else {
                '-'
            };
            if normalized == '-' && previous_separator {
                continue;
            }
            previous_separator = normalized == '-';
            prefix.push(normalized);
        }
        let prefix = prefix.trim_matches(['.', '_', '-']);
        let mut prefix = if prefix.is_empty() {
            "openusd".to_string()
        } else {
            prefix.to_string()
        };
        // 63 readable bytes + '-' + 64 hex bytes = OCI's 128-byte tag limit.
        prefix.truncate(prefix.len().min(63));
        while prefix.ends_with(['.', '_', '-']) {
            prefix.pop();
        }
        Some(format!("{prefix}-{hash}"))
    }
}

impl Platform {
    /// Look up a single component's version constraint.
    pub fn component(&self, name: &str) -> Option<&str> {
        self.core.get(name).map(String::as_str)
    }

    /// Resolve one explicitly approved OpenUSD cell and build variant.
    pub fn resolve_openusd(
        &self,
        os: Os,
        arch: Arch,
        variant_id: OpenUsdVariantId,
    ) -> Option<(ResolvedOpenUsdCompatibility, &OpenUsdVariant)> {
        let policy = self.openusd.as_ref()?;
        let cell = policy
            .cells
            .iter()
            .find(|cell| cell.os == os && cell.arch == arch)?;
        let variant = cell
            .variants
            .iter()
            .find(|variant| variant.id == variant_id)?;
        let provider = |source: &OpenUsdProvider| ResolvedOpenUsdProvider {
            family: source.family.clone(),
            provider: source.provider.clone(),
            version: None,
            version_constraint: self
                .core
                .get(&source.version_from)
                .cloned()
                .unwrap_or_default(),
        };
        Some((
            ResolvedOpenUsdCompatibility {
                schema: policy.schema,
                platform: self.id.clone(),
                os,
                arch,
                toolchain: ResolvedOpenUsdToolchain {
                    family: cell.toolchain.family.clone(),
                    provider: cell.toolchain.provider.clone(),
                    version: None,
                    version_constraint: self
                        .core
                        .get(&cell.toolchain.version_from)
                        .cloned()
                        .unwrap_or_default(),
                    cxx_standard: self
                        .core
                        .get(&cell.toolchain.cxx_standard_from)
                        .cloned()
                        .unwrap_or_default(),
                    runtime: provider(&cell.toolchain.runtime),
                },
                python: provider(&cell.python),
                tbb: provider(&cell.tbb),
                variant: variant_id,
                capabilities: variant.capabilities.clone(),
            },
            variant,
        ))
    }
}

impl ost_core::catalog::Identified for Platform {
    fn id(&self) -> &str {
        &self.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verified_compatibility() -> ResolvedOpenUsdCompatibility {
        let provider = |family: &str, provider: &str, version: &str, constraint: &str| {
            ResolvedOpenUsdProvider {
                family: family.into(),
                provider: provider.into(),
                version: Some(version.into()),
                version_constraint: constraint.into(),
            }
        };
        ResolvedOpenUsdCompatibility {
            schema: 1,
            platform: "cy2026".into(),
            os: Os::Linux,
            arch: Arch::X86_64,
            toolchain: ResolvedOpenUsdToolchain {
                family: "gcc".into(),
                provider: "system".into(),
                version: Some("14.2.0".into()),
                version_constraint: "14.2".into(),
                cxx_standard: "20".into(),
                runtime: provider("glibc", "system", "2.28", "2.28"),
            },
            python: provider("cpython", "platform", "3.13.7", "3.13.x"),
            tbb: provider("onetbb", "platform", "2022.1.0", "2022.x"),
            variant: OpenUsdVariantId::Vulkan,
            capabilities: vec!["usd-core".into(), "vulkan".into(), "opengl".into()],
        }
    }

    #[test]
    fn selector_is_deterministic_oci_tag_identity() {
        let compatibility = verified_compatibility();
        let target = "linux-x86_64-glibc228-py313";
        let selector = compatibility.selector(target, "26.05", None, &[]).unwrap();
        assert!(selector.starts_with("openusd-cy2026-linux-x86_64-vulkan-"));
        assert!(selector.len() <= 128);
        assert!(selector
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)));
        assert_eq!(
            compatibility
                .selector(target, "26.05", None, &[])
                .as_deref(),
            Some(selector.as_str())
        );
    }

    #[test]
    fn selector_normalizes_capability_order_but_captures_providers() {
        let compatibility = verified_compatibility();
        let mut reordered = compatibility.clone();
        reordered.capabilities.reverse();
        reordered.capabilities.push("vulkan".into());
        let target = "linux-x86_64-glibc228-py313";
        assert_eq!(
            compatibility.selector(target, "26.05", None, &[]),
            reordered.selector(target, "26.05", None, &[])
        );

        let mut other_provider = compatibility.clone();
        other_provider.python.provider = "host".into();
        assert_ne!(
            compatibility.selector(target, "26.05", None, &[]),
            other_provider.selector(target, "26.05", None, &[])
        );

        assert_ne!(
            compatibility.selector(target, "26.05", None, &[]),
            compatibility.selector("linux-x86_64-glibc234-py313", "26.05", None, &[])
        );
        assert_ne!(
            compatibility.selector(target, "26.05", None, &[]),
            compatibility.selector(target, "25.11", None, &[])
        );

        let source = ResolvedSourceIdentity {
            repository: "github.com/PixarAnimationStudios/OpenUSD".into(),
            revision: "v26.05".into(),
        };
        let mut other_source = source.clone();
        other_source.revision = "v26.08".into();
        assert_ne!(
            compatibility.selector(target, "26.05", Some(&source), &[]),
            compatibility.selector(target, "26.05", Some(&other_source), &[])
        );

        let dependency = ResolvedDependencyIdentity {
            name: "onetbb".into(),
            version: "2022.1.0".into(),
            source: ResolvedSourceIdentity {
                repository: "github.com/uxlfoundation/oneTBB".into(),
                revision: "v2022.1.0".into(),
            },
            archive_digest: None,
        };
        let mut other_dependency = dependency.clone();
        other_dependency.source.revision = "v2022.2.0".into();
        assert_ne!(
            compatibility.selector(
                target,
                "26.05",
                Some(&source),
                std::slice::from_ref(&dependency),
            ),
            compatibility.selector(
                target,
                "26.05",
                Some(&source),
                std::slice::from_ref(&other_dependency),
            )
        );

        let mut archived_dependency = dependency.clone();
        archived_dependency.archive_digest = Some(format!("sha256:{}", "ab".repeat(32)));
        assert_ne!(
            compatibility.selector(
                target,
                "26.05",
                Some(&source),
                std::slice::from_ref(&dependency),
            ),
            compatibility.selector(
                target,
                "26.05",
                Some(&source),
                std::slice::from_ref(&archived_dependency),
            )
        );
        let mut malformed_dependency = archived_dependency;
        malformed_dependency.archive_digest = Some("sha256:not-a-digest".into());
        assert!(!malformed_dependency.is_verified());
    }

    #[test]
    fn unresolved_identity_has_no_selector() {
        let mut compatibility = verified_compatibility();
        compatibility.tbb.version = None;
        assert_eq!(
            compatibility.selector("linux-x86_64-glibc228-py313", "26.05", None, &[]),
            None
        );
    }

    #[test]
    fn exact_versions_must_satisfy_their_constraints() {
        let mut compatibility = verified_compatibility();
        compatibility.python.version = Some("3.12.9".into());
        assert!(!compatibility.is_verified());
        assert_eq!(
            compatibility.selector("linux-x86_64-glibc228-py313", "26.05", None, &[]),
            None
        );

        compatibility.python.version = Some("   ".into());
        assert!(!compatibility.is_verified());
    }

    #[test]
    fn numeric_constraints_support_prefix_wildcard_and_floor_forms() {
        for (observed, constraint) in [
            ("14.2.0", "14.2"),
            ("3.13.7", "3.13.x"),
            ("2022.1.0", "2022.*"),
            ("2.39", ">=2.28"),
            ("2.28.0", "=2.28"),
            ("2.27", "<2.28"),
        ] {
            assert!(version_satisfies_constraint(observed, constraint));
        }
        for (observed, constraint) in [
            ("3.12.9", "3.13.x"),
            ("2.27", ">=2.28"),
            (" ", "3.13.x"),
            ("3.13.7", " "),
            ("3.13rc1", "3.13.x"),
        ] {
            assert!(!version_satisfies_constraint(observed, constraint));
        }
    }
}
