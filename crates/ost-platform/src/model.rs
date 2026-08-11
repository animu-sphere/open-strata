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

impl ResolvedOpenUsdCompatibility {
    /// Whether every compatibility-critical provider carries a non-empty,
    /// observed exact version rather than only its CY constraint.
    pub fn is_verified(&self) -> bool {
        let exact = |version: &Option<String>| version.as_ref().is_some_and(|v| !v.is_empty());
        exact(&self.toolchain.version)
            && exact(&self.toolchain.runtime.version)
            && exact(&self.python.version)
            && exact(&self.tbb.version)
    }

    /// A deterministic, OCI-tag-safe selector for this exact compatibility
    /// identity.
    ///
    /// The readable prefix is deliberately short; the full SHA-256 suffix is
    /// over every compatibility-critical field (including providers, exact
    /// versions, C++ standard, and a sorted capability set). This keeps the
    /// selector within OCI's 128-character tag limit without dropping identity
    /// dimensions from the comparison contract.
    pub fn selector(&self) -> Option<String> {
        if !self.is_verified() {
            return None;
        }

        #[derive(Serialize)]
        struct SelectorIdentity<'a> {
            schema: u32,
            platform: &'a str,
            os: Os,
            arch: Arch,
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
        let identity = SelectorIdentity {
            schema: self.schema,
            platform: &self.platform,
            os: self.os,
            arch: self.arch,
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
                runtime: provider("glibc", "system", "2.39", "2.28"),
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
        let selector = compatibility.selector().unwrap();
        assert!(selector.starts_with("openusd-cy2026-linux-x86_64-vulkan-"));
        assert!(selector.len() <= 128);
        assert!(selector
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)));
        assert_eq!(compatibility.selector().as_deref(), Some(selector.as_str()));
    }

    #[test]
    fn selector_normalizes_capability_order_but_captures_providers() {
        let compatibility = verified_compatibility();
        let mut reordered = compatibility.clone();
        reordered.capabilities.reverse();
        reordered.capabilities.push("vulkan".into());
        assert_eq!(compatibility.selector(), reordered.selector());

        let mut other_provider = compatibility.clone();
        other_provider.python.provider = "host".into();
        assert_ne!(compatibility.selector(), other_provider.selector());
    }

    #[test]
    fn unresolved_identity_has_no_selector() {
        let mut compatibility = verified_compatibility();
        compatibility.tbb.version = None;
        assert_eq!(compatibility.selector(), None);
    }
}
