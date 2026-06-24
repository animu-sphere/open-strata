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
