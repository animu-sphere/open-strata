// SPDX-License-Identifier: Apache-2.0
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

/// An extension pinned in the lock (id/version/enabled features).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockExtension {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub features: Vec<String>,
}

/// The whole `strata.lock`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lock {
    /// Schema version of the lockfile format itself.
    #[serde(default = "default_lock_version")]
    pub lock_version: u32,
    pub runtime: LockRuntime,
    pub python: LockPython,
    /// Extensions the runtime resolves to (§9.4).
    #[serde(default)]
    pub extensions: Vec<LockExtension>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use ost_core::host::{Arch, Host, Os};
    use ost_core::variant::{Abi, Variant};

    fn sample_lock() -> Lock {
        let variant = Variant::new(
            &Host {
                os: Os::Linux,
                arch: Arch::X86_64,
            },
            Abi::default_for(Os::Linux),
            "313",
        );
        Lock {
            lock_version: 1,
            runtime: LockRuntime {
                id: "openstrata-cy2026-linux-x86_64-py313-usd".into(),
                platform: "cy2026".into(),
                profile: "usd".into(),
                variant,
                digest: "sha256:abc".into(),
            },
            python: LockPython {
                version: "3.13.1".into(),
                abi: "cpython-313-x86_64-linux-gnu".into(),
                manager: "uv".into(),
                uv_lock_hash: Some("deadbeef".into()),
            },
            extensions: vec![
                LockExtension {
                    id: "openusd".into(),
                    version: "24.08".into(),
                    features: vec!["materialx".into()],
                },
                LockExtension {
                    id: "ptex".into(),
                    version: "2.4".into(),
                    features: vec![],
                },
            ],
            validation: Validation::Passed,
        }
    }

    #[test]
    fn round_trips_through_json() {
        let lock = sample_lock();
        let restored = Lock::from_json(&lock.to_json().unwrap()).unwrap();
        assert_eq!(lock, restored);
    }

    #[test]
    fn serialization_is_deterministic() {
        let lock = sample_lock();
        assert_eq!(lock.to_json().unwrap(), lock.to_json().unwrap());
    }

    #[test]
    fn extensions_default_when_absent() {
        // A lock written before `extensions` existed must still parse. Build the
        // JSON from a real lock, then drop the `extensions` key to mimic a legacy
        // file rather than hand-writing the variant's serde shape.
        let mut value: serde_json::Value =
            serde_json::from_str(&sample_lock().to_json().unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("extensions");

        let lock = Lock::from_json(&value.to_string()).unwrap();
        assert!(lock.extensions.is_empty());
        assert_eq!(lock.python.manager, "uv");
    }
}
