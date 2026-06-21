//! The generated lockfile: `strata.lock` (§9.4).
//!
//! The lock pins everything needed to reproduce a session: the resolved
//! runtime digest, the concrete variant, the Python ABI and the validation
//! status. Phase 0 defines the shape and a serializer; resolution and build
//! phases fill it in.

use serde::{Deserialize, Serialize};

use ost_core::variant::Variant;

/// Validation outcome recorded against a resolved runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Validation {
    Passed,
    Failed,
    /// Not yet validated (freshly resolved).
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockRuntime {
    /// e.g. `openstrata-cy2026-linux-x86_64-py313-usd`.
    pub id: String,
    pub platform: String,
    pub profile: String,
    pub variant: Variant,
    /// Content-addressed digest, e.g. `sha256:...` (empty until built).
    #[serde(default)]
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockPython {
    pub version: String,
    /// Full ABI tag, e.g. `cpython-313-x86_64-linux-gnu`.
    pub abi: String,
    #[serde(default = "default_manager")]
    pub manager: String,
    /// Hash of the companion `uv.lock`, if one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uv_lock_hash: Option<String>,
}

fn default_manager() -> String {
    "uv".into()
}

/// The whole `strata.lock`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lock {
    /// Schema version of the lockfile format itself.
    #[serde(default = "default_lock_version")]
    pub lock_version: u32,
    pub runtime: LockRuntime,
    pub python: LockPython,
    pub validation: Validation,
}

fn default_lock_version() -> u32 {
    1
}

impl Lock {
    /// Pretty-printed, deterministic JSON (§23: manifests must be deterministic).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(src: &str) -> Result<Lock, serde_json::Error> {
        serde_json::from_str(src)
    }
}
