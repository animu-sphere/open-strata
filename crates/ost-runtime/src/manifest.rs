//! The per-runtime manifest written into the store on `pull` (§4.2, §10.2).
//!
//! This is the identity record for an installed runtime: what it is, what it
//! provides, and its digest. The digest is computed over the *canonical* fields
//! only (not the creation time), so the same runtime always digests identically
//! — satisfying the "manifests must be deterministic" bar (§23) while still
//! recording provenance.

use serde::{Deserialize, Serialize};

use ost_core::{digest, Variant};

use crate::runtime::Runtime;

/// Filename of the runtime manifest within a runtime prefix.
pub const MANIFEST_FILE: &str = "runtime.json";

/// Validation status of an installed runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Validation {
    Passed,
    Failed,
    Pending,
}

/// The canonical, digestable description of a runtime. Field order is fixed and
/// `BTreeMap`-free so the serialized form is stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Canonical {
    schema: u32,
    id: String,
    platform: String,
    profile: String,
    variant: Variant,
    python: String,
    capabilities: Vec<String>,
    layout: Vec<String>,
}

/// A written runtime manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeManifest {
    pub schema: u32,
    pub id: String,
    pub platform: String,
    pub profile: String,
    pub variant: Variant,
    /// Platform Python version, e.g. `3.13.x`.
    pub python: String,
    pub capabilities: Vec<String>,
    /// Subdirectories materialized under the prefix.
    pub layout: Vec<String>,
    /// `sha256:...` over the canonical fields (excludes `created_unix`).
    pub digest: String,
    pub validation: Validation,
    /// Seconds since the Unix epoch when this manifest was written (provenance).
    pub created_unix: u64,
    /// True when the runtime was materialized by the local/mock backend rather
    /// than pulled from real artifacts.
    pub mock: bool,
}

const SCHEMA: u32 = 1;

impl RuntimeManifest {
    /// Build a manifest for a resolved runtime, computing the digest.
    pub fn build(
        runtime: &Runtime,
        python_version: &str,
        capabilities: Vec<String>,
        layout: Vec<String>,
        created_unix: u64,
        mock: bool,
    ) -> RuntimeManifest {
        let canonical = Canonical {
            schema: SCHEMA,
            id: runtime.id(),
            platform: runtime.platform.clone(),
            profile: runtime.profile.clone(),
            variant: runtime.variant.clone(),
            python: python_version.to_string(),
            capabilities: capabilities.clone(),
            layout: layout.clone(),
        };
        // Serialization of a fixed-field struct is deterministic.
        let bytes = serde_json::to_vec(&canonical).expect("canonical serializes");
        let digest = digest::sha256_hex(&bytes);

        RuntimeManifest {
            schema: SCHEMA,
            id: canonical.id,
            platform: canonical.platform,
            profile: canonical.profile,
            variant: canonical.variant,
            python: canonical.python,
            capabilities: canonical.capabilities,
            layout: canonical.layout,
            digest,
            validation: Validation::Pending,
            created_unix,
            mock,
        }
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(src: &str) -> Result<RuntimeManifest, serde_json::Error> {
        serde_json::from_str(src)
    }
}
