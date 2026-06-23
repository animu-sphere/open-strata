// SPDX-License-Identifier: Apache-2.0
//! The platform manifest data model.

use indexmap::IndexMap;
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
    /// Optional free-form notes shown by `ost platform show`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl Platform {
    /// Look up a single component's version constraint.
    pub fn component(&self, name: &str) -> Option<&str> {
        self.core.get(name).map(String::as_str)
    }
}

impl ost_core::catalog::Identified for Platform {
    fn id(&self) -> &str {
        &self.id
    }
}
