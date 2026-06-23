// SPDX-License-Identifier: Apache-2.0
//! Structured diff between two platform years (`ost platform diff`).

use std::collections::BTreeSet;

use crate::model::Platform;

/// What happened to a single component between two platforms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentChange {
    /// Present in the newer platform only.
    Added { to: String },
    /// Present in the older platform only.
    Removed { from: String },
    /// Present in both with a different constraint.
    Changed { from: String, to: String },
}

/// The full diff: component name → change, ordered by name. Components that are
/// unchanged are omitted.
#[derive(Debug, Clone)]
pub struct PlatformDiff {
    pub from_id: String,
    pub to_id: String,
    pub changes: Vec<(String, ComponentChange)>,
}

impl PlatformDiff {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// Compute the diff `from -> to`, deterministically ordered by component name.
pub fn diff(from: &Platform, to: &Platform) -> PlatformDiff {
    let mut names: BTreeSet<&str> = BTreeSet::new();
    names.extend(from.core.keys().map(String::as_str));
    names.extend(to.core.keys().map(String::as_str));

    let mut changes = Vec::new();
    for name in names {
        match (from.component(name), to.component(name)) {
            (Some(a), Some(b)) if a != b => changes.push((
                name.to_string(),
                ComponentChange::Changed {
                    from: a.to_string(),
                    to: b.to_string(),
                },
            )),
            (Some(_), Some(_)) => {} // unchanged
            (None, Some(b)) => changes.push((
                name.to_string(),
                ComponentChange::Added { to: b.to_string() },
            )),
            (Some(a), None) => changes.push((
                name.to_string(),
                ComponentChange::Removed {
                    from: a.to_string(),
                },
            )),
            (None, None) => unreachable!("name came from the union of both maps"),
        }
    }

    PlatformDiff {
        from_id: from.id.clone(),
        to_id: to.id.clone(),
        changes,
    }
}
