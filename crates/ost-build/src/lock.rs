// SPDX-License-Identifier: Apache-2.0
//! `target.lock.json` — pins a configured target for reproducibility (§8.3).

use serde::{Deserialize, Serialize};

use ost_core::Variant;

use crate::target::Target;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockRuntime {
    pub id: String,
    pub digest: String,
}

/// The compiler a target was configured with, recorded for reproducibility.
///
/// `cc`/`cxx` are absolute paths when known (runtime/explicit policies); they
/// are `null` for the `host` policy, where CMake picks the compiler. `policy`
/// plus the paths form the fingerprint used to decide whether a build tree can
/// be reused (see `ost configure`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockCompiler {
    pub policy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cxx: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cc_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cxx_version: Option<String>,
}

impl LockCompiler {
    /// The reproducibility fingerprint: policy + the resolved compiler paths
    /// (versions are informational and excluded so they never force a rebuild).
    pub fn fingerprint(&self) -> (String, Option<String>, Option<String>) {
        (self.policy.clone(), self.cc.clone(), self.cxx.clone())
    }
}

impl Default for LockCompiler {
    /// The `host` policy — also the value used for locks written before the
    /// compiler field existed (see `#[serde(default)]` on `TargetLock`).
    fn default() -> Self {
        LockCompiler {
            policy: "host".to_string(),
            cc: None,
            cxx: None,
            cc_version: None,
            cxx_version: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetLock {
    pub lock_version: u32,
    pub target: String,
    pub platform: String,
    pub profile: String,
    pub variant: Variant,
    pub runtime: LockRuntime,
    pub python: String,
    pub cxx_standard: String,
    pub generator: String,
    /// Defaulted so locks written before the compiler policy existed (no
    /// `compiler` key) still deserialize — they read back as the `host` policy.
    #[serde(default)]
    pub compiler: LockCompiler,
    /// Path to the generated toolchain, relative to the project root.
    pub toolchain: String,
    pub created_unix: u64,
}

impl TargetLock {
    pub fn from_target(
        target: &Target,
        compiler: LockCompiler,
        toolchain_rel: &str,
        created_unix: u64,
    ) -> TargetLock {
        TargetLock {
            lock_version: 1,
            target: target.id(),
            platform: target.platform.clone(),
            profile: target.profile.clone(),
            variant: target.variant.clone(),
            runtime: LockRuntime {
                id: target.runtime_id.clone(),
                digest: target.runtime_digest.clone(),
            },
            python: target.python_version.clone(),
            cxx_standard: target.cxx_standard.clone(),
            generator: target.generator.clone(),
            compiler,
            toolchain: toolchain_rel.to_string(),
            created_unix,
        }
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lock written before the compiler policy existed has no `compiler` key;
    /// it must still deserialize (as the `host` policy) rather than fail and make
    /// the target look unconfigured.
    #[test]
    fn lock_without_compiler_field_defaults_to_host() {
        let json = serde_json::json!({
            "lock_version": 1,
            "target": "cy2026-linux-x86_64-py313-usd",
            "platform": "cy2026",
            "profile": "usd",
            "variant": { "os": "linux", "arch": "x86_64", "abi": { "glibc": { "version": "2.28" } }, "python": "313" },
            "runtime": { "id": "rt", "digest": "" },
            "python": "3.13.0",
            "cxx_standard": "17",
            "generator": "Ninja",
            "toolchain": ".strata/targets/x/toolchain.cmake",
            "created_unix": 0
        })
        .to_string();

        let lock: TargetLock = serde_json::from_str(&json).expect("legacy lock parses");
        assert_eq!(lock.compiler, LockCompiler::default());
        assert_eq!(lock.compiler.policy, "host");
    }
}
