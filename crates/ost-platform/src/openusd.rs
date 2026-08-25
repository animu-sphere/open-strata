// SPDX-License-Identifier: Apache-2.0
//! Canonical OpenUSD build and publication policy.

use ost_core::host::{Arch, Os};
use serde::{Deserialize, Serialize};

use crate::{OpenUsdBuilder, OpenUsdVariantId, ResolvedOpenUsdCompatibility};

pub const CANONICAL_OPENUSD_VERSIONS: &[&str] = &["26.05", "26.08"];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OpenUsdPlanError {
    #[error("unsupported canonical OpenUSD version '{0}' (expected 26.05 or 26.08)")]
    UnsupportedVersion(String),
    #[error("legacy OpenUSD variant '{0}' cannot be normalized from its recorded capabilities")]
    UnnormalizedVariant(String),
    #[error("OpenUSD variant '{variant}' is not canonical on {os}")]
    UnsupportedPlatform { variant: String, os: String },
}

/// One version-aware, compatibility-owned OpenUSD source build plan.
///
/// Producer scripts and CLI callers consume these arguments instead of
/// independently recreating compatibility-critical build_usd.py/CMake flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenUsdBuildPlan {
    pub source_version: String,
    pub platform_cell: String,
    pub profile: String,
    pub variant: OpenUsdVariantId,
    pub dependency_providers: Vec<String>,
    pub build_arguments: Vec<String>,
    pub cmake_cache_entries: Vec<String>,
    pub examples_required: bool,
}

impl OpenUsdBuildPlan {
    pub fn new(
        source_version: &str,
        compatibility: &ResolvedOpenUsdCompatibility,
        builder: OpenUsdBuilder,
    ) -> Result<Self, OpenUsdPlanError> {
        if !CANONICAL_OPENUSD_VERSIONS.contains(&source_version) {
            return Err(OpenUsdPlanError::UnsupportedVersion(source_version.into()));
        }
        let variant = compatibility
            .variant
            .canonical(&compatibility.capabilities)
            .ok_or_else(|| {
                OpenUsdPlanError::UnnormalizedVariant(compatibility.variant.as_str().into())
            })?;
        validate_variant_platform(variant, compatibility.os)?;

        let examples_required = variant != OpenUsdVariantId::Core;
        let mut build_arguments = match variant {
            OpenUsdVariantId::Core => vec!["--no-imaging".into(), "--no-examples".into()],
            OpenUsdVariantId::Gl | OpenUsdVariantId::Vulkan | OpenUsdVariantId::Metal => {
                vec!["--imaging".into(), "--examples".into()]
            }
            OpenUsdVariantId::Headless | OpenUsdVariantId::Standard => unreachable!(),
        };
        if source_version == "26.08" {
            build_arguments.push("--python-install-dir=lib/python".into());
        }
        if builder == OpenUsdBuilder::BuildUsd && compatibility.os == Os::Windows {
            // Upstream 26.05/26.08 otherwise select the Visual Studio 17
            // generator from the compiler version. CY2026 owns the MSVC 19.40
            // toolset, not the installed Visual Studio product generation;
            // Ninja consumes the already pinned vcvars environment directly.
            build_arguments.push("--generator=Ninja".into());
        }

        let mut cmake_cache_entries = vec![format!(
            "-DPXR_BUILD_IMAGING={}",
            if variant == OpenUsdVariantId::Core {
                "OFF"
            } else {
                "ON"
            }
        )];
        cmake_cache_entries.push(format!(
            "-DPXR_BUILD_USD_IMAGING={}",
            if variant == OpenUsdVariantId::Core {
                "OFF"
            } else {
                "ON"
            }
        ));
        cmake_cache_entries.push(format!(
            "-DPXR_BUILD_EXAMPLES={}",
            if examples_required { "ON" } else { "OFF" }
        ));
        cmake_cache_entries.push(format!(
            "-DPXR_ENABLE_VULKAN_SUPPORT={}",
            if variant == OpenUsdVariantId::Vulkan {
                "ON"
            } else {
                "OFF"
            }
        ));
        if compatibility.os == Os::Macos {
            cmake_cache_entries.push(format!(
                "-DPXR_ENABLE_METAL_SUPPORT={}",
                if variant == OpenUsdVariantId::Metal {
                    "ON"
                } else {
                    "OFF"
                }
            ));
        }

        if builder == OpenUsdBuilder::Cmake {
            build_arguments.clear();
        }
        let mut dependency_providers = vec![
            format!("python:{}", compatibility.python.provider),
            format!("tbb:{}", compatibility.tbb.provider),
            format!(
                "native-runtime:{}",
                compatibility.toolchain.runtime.provider
            ),
        ];
        if let Some(macos) = &compatibility.macos {
            dependency_providers.push(format!("macos-sdk:{}", macos.sdk.provider));
        }

        Ok(Self {
            source_version: source_version.into(),
            platform_cell: format!(
                "{}-{}-{}",
                compatibility.platform,
                compatibility.os.as_str(),
                compatibility.arch.as_str()
            ),
            profile: compatibility.profile.as_deref().unwrap_or("usd").into(),
            variant,
            dependency_providers,
            build_arguments,
            cmake_cache_entries,
            examples_required,
        })
    }
}

fn validate_variant_platform(variant: OpenUsdVariantId, os: Os) -> Result<(), OpenUsdPlanError> {
    let supported = match os {
        Os::Linux | Os::Windows => variant != OpenUsdVariantId::Metal,
        // macOS publishes `core` and `metal` only. Vulkan would mean shipping a
        // translation layer, and OpenStrata observes no physical OpenGL device on
        // macOS, so a `gl` leaf could never carry the device and render evidence
        // `check_exportable` requires of an imaging cell.
        Os::Macos => !matches!(variant, OpenUsdVariantId::Vulkan | OpenUsdVariantId::Gl),
    };
    if supported {
        Ok(())
    } else {
        Err(OpenUsdPlanError::UnsupportedPlatform {
            variant: variant.as_str().into(),
            os: os.as_str().into(),
        })
    }
}

/// Format the one canonical human-facing leaf tag. This is deliberately not a
/// compatibility selector or immutable digest.
pub fn canonical_openusd_leaf_tag(
    openusd_version: &str,
    variant: OpenUsdVariantId,
    os: Os,
    arch: Arch,
) -> Result<String, OpenUsdPlanError> {
    if !CANONICAL_OPENUSD_VERSIONS.contains(&openusd_version) {
        return Err(OpenUsdPlanError::UnsupportedVersion(openusd_version.into()));
    }
    if !variant.is_canonical() {
        return Err(OpenUsdPlanError::UnnormalizedVariant(
            variant.as_str().into(),
        ));
    }
    validate_variant_platform(variant, os)?;
    Ok(format!(
        "{openusd_version}-{}-{}-{}",
        variant.as_str(),
        os.as_str(),
        arch.as_str()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ResolvedOpenUsdProvider, ResolvedOpenUsdToolchain};

    fn compatibility(os: Os, variant: OpenUsdVariantId) -> ResolvedOpenUsdCompatibility {
        let provider = |family: &str| ResolvedOpenUsdProvider {
            family: family.into(),
            provider: "platform".into(),
            version: None,
            version_constraint: "1.x".into(),
        };
        ResolvedOpenUsdCompatibility {
            schema: 2,
            platform: "cy2026".into(),
            profile: Some("usd".into()),
            os,
            arch: if os == Os::Macos {
                Arch::Arm64
            } else {
                Arch::X86_64
            },
            toolchain: ResolvedOpenUsdToolchain {
                family: "compiler".into(),
                provider: "platform".into(),
                version: None,
                version_constraint: "1.x".into(),
                cxx_standard: "20".into(),
                runtime: provider("runtime"),
            },
            python: provider("cpython"),
            tbb: provider("onetbb"),
            variant,
            capabilities: match variant {
                OpenUsdVariantId::Core => vec!["usd-core".into()],
                _ => vec!["usd-core".into(), "imaging".into()],
            },
            producer_openusd_version: None,
            consumer_openusd_constraint: Some(">=26.05,<26.09".into()),
            macos: None,
        }
    }

    #[test]
    fn imaging_plans_always_build_examples() {
        for variant in [OpenUsdVariantId::Gl, OpenUsdVariantId::Vulkan] {
            let plan = OpenUsdBuildPlan::new(
                "26.08",
                &compatibility(Os::Linux, variant),
                OpenUsdBuilder::BuildUsd,
            )
            .unwrap();
            assert!(plan.examples_required);
            assert!(plan.build_arguments.iter().any(|arg| arg == "--examples"));
        }
    }

    #[test]
    fn windows_source_plans_use_ninja_with_the_pinned_msvc_environment() {
        let plan = OpenUsdBuildPlan::new(
            "26.05",
            &compatibility(Os::Windows, OpenUsdVariantId::Core),
            OpenUsdBuilder::BuildUsd,
        )
        .unwrap();

        assert!(plan
            .build_arguments
            .iter()
            .any(|argument| argument == "--generator=Ninja"));
    }

    #[test]
    fn leaf_tags_reject_legacy_and_cross_platform_variants() {
        assert_eq!(
            canonical_openusd_leaf_tag("26.08", OpenUsdVariantId::Metal, Os::Macos, Arch::Arm64)
                .unwrap(),
            "26.08-metal-macos-arm64"
        );
        assert!(canonical_openusd_leaf_tag(
            "26.08",
            OpenUsdVariantId::Standard,
            Os::Linux,
            Arch::X86_64
        )
        .is_err());
        assert!(canonical_openusd_leaf_tag(
            "26.08",
            OpenUsdVariantId::Vulkan,
            Os::Macos,
            Arch::Arm64
        )
        .is_err());
    }

    /// macOS publishes `core` and `metal`. `gl` is refused at plan time for the
    /// same reason `vulkan` is: the platform cannot produce the evidence an
    /// imaging leaf has to carry, so the failure belongs before the build.
    #[test]
    fn macos_refuses_both_non_canonical_graphics_variants() {
        for variant in [OpenUsdVariantId::Vulkan, OpenUsdVariantId::Gl] {
            assert!(matches!(
                validate_variant_platform(variant, Os::Macos),
                Err(OpenUsdPlanError::UnsupportedPlatform { .. })
            ));
            assert!(validate_variant_platform(variant, Os::Linux).is_ok());
        }
        assert!(validate_variant_platform(OpenUsdVariantId::Metal, Os::Macos).is_ok());
        assert!(validate_variant_platform(OpenUsdVariantId::Core, Os::Macos).is_ok());
    }
}
