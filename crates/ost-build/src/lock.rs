//! `target.lock.json` — pins a configured target for reproducibility (§8.3).

use serde::{Deserialize, Serialize};

use ost_core::Variant;

use crate::target::Target;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockRuntime {
    pub id: String,
    pub digest: String,
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
    /// Path to the generated toolchain, relative to the project root.
    pub toolchain: String,
    pub created_unix: u64,
}

impl TargetLock {
    pub fn from_target(target: &Target, toolchain_rel: &str, created_unix: u64) -> TargetLock {
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
            toolchain: toolchain_rel.to_string(),
            created_unix,
        }
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}
