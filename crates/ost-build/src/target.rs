// SPDX-License-Identifier: Apache-2.0
//! The build target (§4.6): platform + OS + arch + Python ABI + profile.

use ost_core::host::Os;
use ost_core::Variant;

/// Stable lock/evidence identity for a build that intentionally consumes no
/// OpenStrata runtime. This is distinct from an ordinary runtime whose digest
/// is temporarily empty because it has not been pulled yet.
pub const NO_RUNTIME_ID: &str = "none";

/// A fully-described build target. Holds everything the generators need so they
/// stay pure (no catalog or filesystem access).
#[derive(Debug, Clone)]
pub struct Target {
    pub platform: String,
    pub profile: String,
    pub variant: Variant,
    /// Resolved runtime id this target builds against.
    pub runtime_id: String,
    /// Runtime digest, or empty if the runtime is not yet pulled.
    pub runtime_digest: String,
    /// Platform Python version, e.g. `3.13.x`.
    pub python_version: String,
    /// C++ standard from the platform, e.g. `20`.
    pub cxx_standard: String,
    /// Capabilities provided by the profile (drives OpenUSD/MaterialX roots).
    pub capabilities: Vec<String>,
    /// CMake generator, e.g. `Ninja`.
    pub generator: String,
}

impl Target {
    /// Target id, e.g. `cy2026-linux-x86_64-py313-usd` (§4.6).
    pub fn id(&self) -> String {
        if !self.uses_runtime() {
            return format!(
                "{}-{}-runtime-free",
                self.platform,
                self.variant.short_slug(),
            );
        }
        format!(
            "{}-{}-{}",
            self.platform,
            self.variant.short_slug(),
            self.profile
        )
    }

    pub fn os(&self) -> Os {
        self.variant.os
    }

    pub fn uses_runtime(&self) -> bool {
        self.runtime_id != NO_RUNTIME_ID
    }

    pub fn has_usd(&self) -> bool {
        self.capabilities.iter().any(|c| c.starts_with("usd"))
    }

    pub fn has_materialx(&self) -> bool {
        self.capabilities.iter().any(|c| c == "usd-materialx")
    }
}
