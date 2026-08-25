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
    /// Apple SDK/deployment boundary for macOS producer cells. Absent on
    /// Linux and Windows, where the native runtime provider carries the ABI
    /// floor instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macos: Option<OpenUsdMacos>,
    pub variants: Vec<OpenUsdVariant>,
}

/// Compatibility-critical macOS SDK and deployment boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenUsdMacos {
    pub sdk: OpenUsdProvider,
    pub deployment_target_from: String,
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

/// Canonical OpenUSD graphics variants plus the schema-1 legacy spellings.
///
/// New declarations and identities use `core`, `gl`, `vulkan`, or `metal`.
/// `headless` and `standard` remain readable so old runtime/artifact manifests
/// can be migrated without making up capabilities they did not record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenUsdVariantId {
    Core,
    Gl,
    Vulkan,
    Metal,
    #[doc(hidden)]
    Headless,
    #[doc(hidden)]
    Standard,
}

impl OpenUsdVariantId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Gl => "gl",
            Self::Vulkan => "vulkan",
            Self::Metal => "metal",
            Self::Headless => "headless",
            Self::Standard => "standard",
        }
    }

    /// Normalize a reader-facing legacy value using the capabilities recorded
    /// beside it. `standard` always meant the GL imaging cell. `headless` is
    /// only safe to call `core` when no imaging capability was recorded.
    pub fn canonical(self, capabilities: &[String]) -> Option<Self> {
        match self {
            Self::Core | Self::Gl | Self::Vulkan | Self::Metal => Some(self),
            Self::Standard => Some(Self::Gl),
            Self::Headless
                if capabilities.iter().all(|capability| {
                    !matches!(
                        capability.as_str(),
                        "imaging" | "opengl" | "vulkan" | "metal"
                    )
                }) =>
            {
                Some(Self::Core)
            }
            Self::Headless => None,
        }
    }

    pub fn is_canonical(self) -> bool {
        matches!(self, Self::Core | Self::Gl | Self::Vulkan | Self::Metal)
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
    /// Capability profile remains an identity axis separate from graphics.
    /// `None` on schema-1 records means legacy/unknown, never inferred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    pub os: Os,
    pub arch: Arch,
    pub toolchain: ResolvedOpenUsdToolchain,
    pub python: ResolvedOpenUsdProvider,
    pub tbb: ResolvedOpenUsdProvider,
    pub variant: OpenUsdVariantId,
    pub capabilities: Vec<String>,
    /// Exact producer OpenUSD release, populated after the built tree reports
    /// its version. Kept optional for migration-safe schema-1 reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer_openusd_version: Option<String>,
    /// Consumer-side release constraint used when selecting this producer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumer_openusd_constraint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macos: Option<ResolvedOpenUsdMacos>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedOpenUsdMacos {
    pub sdk: ResolvedOpenUsdProvider,
    pub deployment_target: String,
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
        let Some(required) = comparison_floor_version(required) else {
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

/// The numeric version a comparison constraint is measured against, with any
/// trailing wildcard dropped: `>=17.x` compares against `17`, so every 17
/// release satisfies it while 16.5 still does not. Without this, a constraint
/// that combines an operator with a wildcard could never be satisfied, because
/// `numeric_version` rejects the non-numeric component outright.
fn comparison_floor_version(value: &str) -> Option<Vec<u64>> {
    let base = value
        .trim()
        .split('.')
        .take_while(|part| !matches!(*part, "x" | "X" | "*"))
        .collect::<Vec<_>>()
        .join(".");
    numeric_version(&base)
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

fn runtime_floor_constraint(value: &str) -> String {
    let value = value.trim();
    if [">=", "<=", ">", "<", "="]
        .iter()
        .any(|operator| value.starts_with(operator))
    {
        value.to_string()
    } else {
        format!(">={value}")
    }
}

fn provider_is_verified(provider: &ResolvedOpenUsdProvider) -> bool {
    !provider.family.trim().is_empty()
        && !provider.provider.trim().is_empty()
        && !provider.version_constraint.trim().is_empty()
        && provider.version.as_ref().is_some_and(|version| {
            version_satisfies_constraint(version, &provider.version_constraint)
        })
}

/// Why `provider` fails verification, or `None` when it does not.
///
/// Every message ends in the condition that held, because the two point at
/// opposite owners: **unverified** is a predicate the *consumer* evaluates,
/// **contradictory** is a claim about the *bytes*. Saying only "unverified or
/// contradictory" sent a downstream consumer to republish a macOS runtime whose
/// libc++ constraint was `>=17.x` — the artifact was correct and the wildcard
/// comparison in one released `ost` was not (report 36 §7.1, §8).
fn provider_verification_failure(
    label: &str,
    provider: &ResolvedOpenUsdProvider,
) -> Option<String> {
    let name = provider.provider.trim();
    let named = if name.is_empty() {
        label.to_string()
    } else {
        format!("{label} provider '{name}'")
    };
    if provider.family.trim().is_empty() {
        return Some(format!("{named} declares no family (unverified)"));
    }
    if name.is_empty() {
        return Some(format!("{label} declares no provider (unverified)"));
    }
    if provider.version_constraint.trim().is_empty() {
        return Some(format!(
            "{named} declares no version constraint (unverified)"
        ));
    }
    let Some(version) = provider.version.as_deref() else {
        return Some(format!(
            "{named} records no observed version to check against its constraint '{}' (unverified)",
            provider.version_constraint
        ));
    };
    if !version_satisfies_constraint(version, &provider.version_constraint) {
        return Some(format!(
            "{named} records version '{version}', which does not satisfy its declared constraint '{}' (contradictory)",
            provider.version_constraint
        ));
    }
    None
}

impl ResolvedOpenUsdCompatibility {
    /// Provider facts shared by legacy schema-1 and canonical schema-2 cells.
    pub fn providers_are_verified(&self) -> bool {
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

    /// Why [`providers_are_verified`](Self::providers_are_verified) refuses this
    /// identity, or `None` when it does not — one named provider and which of
    /// the two conditions held. See [`provider_verification_failure`].
    pub fn providers_verification_failure(&self) -> Option<String> {
        if self.platform.trim().is_empty() {
            return Some(
                "the compatibility identity declares no platform (unverified)".to_string(),
            );
        }
        if self.toolchain.cxx_standard.trim().is_empty() {
            return Some("the toolchain declares no C++ standard (unverified)".to_string());
        }
        let toolchain = ResolvedOpenUsdProvider {
            family: self.toolchain.family.clone(),
            provider: self.toolchain.provider.clone(),
            version: self.toolchain.version.clone(),
            version_constraint: self.toolchain.version_constraint.clone(),
        };
        provider_verification_failure("toolchain", &toolchain)
            .or_else(|| provider_verification_failure("toolchain runtime", &self.toolchain.runtime))
            .or_else(|| provider_verification_failure("python", &self.python))
            .or_else(|| provider_verification_failure("tbb", &self.tbb))
    }

    /// Why [`is_verified`](Self::is_verified) refuses this identity, or `None`
    /// when it does not.
    pub fn verification_failure(&self) -> Option<String> {
        if self.profile.as_deref() != Some("usd") {
            return Some(format!(
                "the compatibility identity declares profile '{}' rather than 'usd' (unverified)",
                self.profile.as_deref().unwrap_or("<missing>")
            ));
        }
        if self
            .producer_openusd_version
            .as_deref()
            .is_none_or(|version| version.trim().is_empty())
        {
            return Some(
                "the compatibility identity records no producer OpenUSD version (unverified)"
                    .to_string(),
            );
        }
        if self
            .consumer_openusd_constraint
            .as_deref()
            .is_none_or(|constraint| constraint.trim().is_empty())
        {
            return Some(
                "the compatibility identity declares no consumer OpenUSD constraint (unverified)"
                    .to_string(),
            );
        }
        if self.variant.canonical(&self.capabilities).is_none() {
            return Some(format!(
                "variant '{}' is not canonical for the declared capabilities (contradictory)",
                self.variant.as_str()
            ));
        }
        if let Some(failure) = self.providers_verification_failure() {
            return Some(failure);
        }
        match (&self.macos, self.os) {
            (Some(macos), Os::Macos) => provider_verification_failure("macos sdk", &macos.sdk)
                .or_else(|| {
                    macos.deployment_target.trim().is_empty().then(|| {
                        "the macOS cell declares no deployment target (unverified)".to_string()
                    })
                }),
            (None, Os::Macos) => {
                Some("a macOS cell carries no 'macos' identity block (unverified)".to_string())
            }
            (None, _) => None,
            (Some(_), _) => Some(format!(
                "a '{}' cell carries a 'macos' identity block (contradictory)",
                self.os.as_str()
            )),
        }
    }

    /// Whether every compatibility-critical provider carries an observed exact
    /// version that satisfies its non-empty CY constraint.
    pub fn is_verified(&self) -> bool {
        !self.platform.trim().is_empty()
            && self.profile.as_deref() == Some("usd")
            && self
                .producer_openusd_version
                .as_deref()
                .is_some_and(|version| !version.trim().is_empty())
            && self
                .consumer_openusd_constraint
                .as_deref()
                .is_some_and(|constraint| !constraint.trim().is_empty())
            && self.variant.canonical(&self.capabilities).is_some()
            && self.providers_are_verified()
            && match (&self.macos, self.os) {
                (Some(macos), Os::Macos) => {
                    provider_is_verified(&macos.sdk) && !macos.deployment_target.trim().is_empty()
                }
                (None, Os::Macos) => false,
                (None, _) => true,
                (Some(_), _) => false,
            }
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
        if self.schema == 1 {
            return self.legacy_selector(artifact_target, openusd_version, source, dependencies);
        }
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
            profile: &'a str,
            producer_openusd_version: &'a str,
            consumer_openusd_constraint: &'a str,
            source: Option<&'a ResolvedSourceIdentity>,
            dependencies: Vec<&'a ResolvedDependencyIdentity>,
            toolchain: &'a ResolvedOpenUsdToolchain,
            python: &'a ResolvedOpenUsdProvider,
            tbb: &'a ResolvedOpenUsdProvider,
            variant: OpenUsdVariantId,
            macos: Option<&'a ResolvedOpenUsdMacos>,
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
        let canonical_variant = self.variant.canonical(&self.capabilities)?;
        let producer_version = self.producer_openusd_version.as_deref()?;
        if producer_version != openusd_version {
            return None;
        }
        let identity = SelectorIdentity {
            schema: self.schema,
            platform: &self.platform,
            os: self.os,
            arch: self.arch,
            artifact_target,
            profile: self.profile.as_deref()?,
            producer_openusd_version: producer_version,
            consumer_openusd_constraint: self.consumer_openusd_constraint.as_deref()?,
            source,
            dependencies,
            toolchain: &self.toolchain,
            python: &self.python,
            tbb: &self.tbb,
            variant: canonical_variant,
            macos: self.macos.as_ref(),
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
            canonical_variant.as_str()
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

    fn legacy_selector(
        &self,
        artifact_target: &str,
        openusd_version: &str,
        source: Option<&ResolvedSourceIdentity>,
        dependencies: &[ResolvedDependencyIdentity],
    ) -> Option<String> {
        if !self.providers_are_verified()
            || artifact_target.trim().is_empty()
            || openusd_version.trim().is_empty()
            || source.is_some_and(|value| !value.is_verified())
            || dependencies.iter().any(|value| !value.is_verified())
        {
            return None;
        }
        #[derive(Serialize)]
        struct LegacySelectorIdentity<'a> {
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
        let identity = LegacySelectorIdentity {
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
        let bytes = serde_json::to_vec(&identity).expect("legacy selector identity serializes");
        let digest = digest::sha256_hex(&bytes);
        let hash = digest
            .strip_prefix("sha256:")
            .expect("sha256 has algorithm");
        let readable = format!(
            "openusd-{}-{}-{}-{}",
            self.platform,
            self.os.as_str(),
            self.arch.as_str(),
            self.variant.as_str()
        );
        Some(format!("{readable}-{hash}"))
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
                profile: Some("usd".into()),
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
                    runtime: {
                        let mut runtime = provider(&cell.toolchain.runtime);
                        runtime.version_constraint =
                            runtime_floor_constraint(&runtime.version_constraint);
                        runtime
                    },
                },
                python: provider(&cell.python),
                tbb: provider(&cell.tbb),
                variant: variant_id,
                capabilities: variant.capabilities.clone(),
                producer_openusd_version: None,
                consumer_openusd_constraint: Some(">=26.05,<26.09".into()),
                macos: cell.macos.as_ref().map(|macos| ResolvedOpenUsdMacos {
                    sdk: provider(&macos.sdk),
                    deployment_target: self
                        .core
                        .get(&macos.deployment_target_from)
                        .cloned()
                        .unwrap_or_default(),
                }),
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
            schema: 2,
            platform: "cy2026".into(),
            profile: Some("usd".into()),
            os: Os::Linux,
            arch: Arch::X86_64,
            toolchain: ResolvedOpenUsdToolchain {
                family: "gcc".into(),
                provider: "system".into(),
                version: Some("14.2.0".into()),
                version_constraint: "14.2".into(),
                cxx_standard: "20".into(),
                runtime: provider("glibc", "system", "2.28", ">=2.28"),
            },
            python: provider("cpython", "platform", "3.13.7", "3.13.x"),
            tbb: provider("onetbb", "platform", "2022.1.0", "2022.x"),
            variant: OpenUsdVariantId::Vulkan,
            capabilities: vec!["usd-core".into(), "vulkan".into(), "opengl".into()],
            producer_openusd_version: Some("26.05".into()),
            consumer_openusd_constraint: Some(">=26.05,<26.09".into()),
            macos: None,
        }
    }

    /// `verification_failure` is the message half of `is_verified`, and a
    /// refusal with no reason (or a reason with no refusal) would be worse than
    /// the vague sentence it replaces. Every condition is exercised in both
    /// directions from one verified cell.
    #[test]
    fn every_verification_failure_agrees_with_the_predicate() {
        let verified = verified_compatibility();
        assert!(verified.is_verified());
        assert_eq!(verified.verification_failure(), None);

        type Mutation = (&'static str, Box<dyn Fn(&mut ResolvedOpenUsdCompatibility)>);
        let mutations: Vec<Mutation> = vec![
            ("platform", Box::new(|c| c.platform = String::new())),
            ("profile", Box::new(|c| c.profile = None)),
            (
                "producer version",
                Box::new(|c| c.producer_openusd_version = Some("  ".into())),
            ),
            (
                "consumer constraint",
                Box::new(|c| c.consumer_openusd_constraint = None),
            ),
            (
                "variant",
                Box::new(|c| c.variant = OpenUsdVariantId::Headless),
            ),
            (
                "toolchain c++ standard",
                Box::new(|c| c.toolchain.cxx_standard = String::new()),
            ),
            (
                "toolchain version",
                Box::new(|c| c.toolchain.version = None),
            ),
            (
                "toolchain runtime constraint",
                Box::new(|c| c.toolchain.runtime.version = Some("2.17".into())),
            ),
            (
                "python family",
                Box::new(|c| c.python.family = String::new()),
            ),
            (
                "tbb version",
                Box::new(|c| c.tbb.version = Some("2021.0.0".into())),
            ),
            (
                "macos block off macos",
                Box::new(|c| {
                    c.macos = Some(ResolvedOpenUsdMacos {
                        sdk: ResolvedOpenUsdProvider {
                            family: "macos-sdk".into(),
                            provider: "apple".into(),
                            version: Some("15.5".into()),
                            version_constraint: ">=15".into(),
                        },
                        deployment_target: "13.0".into(),
                    })
                }),
            ),
            ("macos block missing", Box::new(|c| c.os = Os::Macos)),
        ];

        for (label, mutate) in mutations {
            let mut broken = verified_compatibility();
            mutate(&mut broken);
            assert!(!broken.is_verified(), "{label} should not verify");
            let failure = broken
                .verification_failure()
                .unwrap_or_else(|| panic!("{label} refuses without saying why"));
            // Which of the two conditions held is the whole point: one points at
            // this consumer, the other at the bytes (report 36 §7.1).
            assert!(
                failure.ends_with("(unverified)") || failure.ends_with("(contradictory)"),
                "{label}: {failure}"
            );
        }
    }

    /// A wildcard floor constraint is satisfiable. `>=17.x` was not, and every
    /// macOS leaf carrying one failed its own cell inside `ost` 0.22.3 — read
    /// downstream as a bad artifact (report 36 §7.1, §8).
    #[test]
    fn a_wildcard_floor_constraint_is_satisfied_by_its_own_floor() {
        assert!(version_satisfies_constraint("17.0.0", ">=17.x"));
        assert!(!version_satisfies_constraint("16.0.0", ">=17.x"));
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
            // A floor built from a wildcard cell, e.g. CY2026's `libcxx: 17.x`
            // normalized to `>=17.x` by `runtime_floor_constraint`.
            ("17.0.0", ">=17.x"),
            ("18.1", ">=17.x"),
            ("17.0.0", "=17.x"),
            ("16.5", "<17.x"),
        ] {
            assert!(version_satisfies_constraint(observed, constraint));
        }
        for (observed, constraint) in [
            ("3.12.9", "3.13.x"),
            ("2.27", ">=2.28"),
            (" ", "3.13.x"),
            ("3.13.7", " "),
            ("3.13rc1", "3.13.x"),
            ("16.5", ">=17.x"),
            ("17.0.0", "<17.x"),
            ("17.0.0", ">=x"),
        ] {
            assert!(!version_satisfies_constraint(observed, constraint));
        }
    }
}
