// SPDX-License-Identifier: Apache-2.0
//! The extension manifest data model (§4.4, §5.2).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// A controlled extension definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extension {
    pub id: String,
    /// Component kind, e.g. `solution.openusd` or `library.materialx`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    pub version: String,

    /// Allowed version range for resolution (§5.3), e.g. `>=25.05,<26.08`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_range: Option<String>,

    /// capability → how this extension provides it.
    #[serde(default)]
    pub provides: IndexMap<String, Provide>,

    /// Named feature sets and what each requires (§5.2).
    #[serde(default)]
    pub features: IndexMap<String, Feature>,

    /// Certified build points actually built + validated (§5.3).
    #[serde(default)]
    pub certified: Vec<Certified>,

    /// Validation suites this extension ships with.
    #[serde(default)]
    pub validation: Vec<String>,
}

/// How an extension provides a capability.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Provide {
    /// Feature that must be enabled for this capability (None = always on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature: Option<String>,
}

/// A feature set's requirements.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Feature {
    /// Platform/runtime packages this feature needs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_packages: Vec<String>,
    /// Other extensions this feature pulls in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_extensions: Vec<String>,
}

/// A certified, built-and-validated version + feature combination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Certified {
    pub version: String,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<String>,
}

impl Extension {
    /// The extensions a feature pulls in, if the feature is defined.
    pub fn feature(&self, name: &str) -> Option<&Feature> {
        self.features.get(name)
    }
}

impl ost_core::catalog::Identified for Extension {
    fn id(&self) -> &str {
        &self.id
    }
}
