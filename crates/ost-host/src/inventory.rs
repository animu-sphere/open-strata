// SPDX-License-Identifier: Apache-2.0
//! Persisted host inventories: a reviewable project file and a fast user cache.
//!
//! An inventory is a *record of what was observed*, not a source of truth about
//! what is installed now. Reading one therefore re-checks the installs it names
//! before handing them back, so a host that was upgraded, moved, or removed
//! since the last scan is reported as such instead of being served stale.

use camino::{Utf8Path, Utf8PathBuf};
use ost_core::fs::write_atomic;
use ost_core::paths::{Store, STATE_DIR};
use ost_core::{Category, Error, Result};
use serde::{Deserialize, Serialize};

use crate::record::{
    canonical_root_key, HostFamily, HostRecord, HostStatus, FINGERPRINT_INPUTS_VERSION,
    HOST_RECORD_SCHEMA,
};

pub const HOST_INVENTORY_SCHEMA: &str = "openstrata.host-inventory/v1alpha1";

/// A set of host records, as persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostInventory {
    pub schema: String,
    /// Seconds since the Unix epoch. Cache metadata only: it deliberately does
    /// not participate in any record's identity or fingerprint, so re-running
    /// discovery on an unchanged machine still produces identical records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_at: Option<u64>,
    pub records: Vec<HostRecord>,
}

impl HostInventory {
    pub fn new(records: Vec<HostRecord>) -> HostInventory {
        HostInventory {
            schema: HOST_INVENTORY_SCHEMA.to_string(),
            cached_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|elapsed| elapsed.as_secs()),
            records,
        }
    }

    /// Read an inventory, or `None` when none has been written yet. A missing
    /// inventory is an ordinary state — nobody has run `discover` — not an error.
    pub fn load(path: &Utf8Path) -> Result<Option<HostInventory>> {
        let source = match std::fs::read_to_string(path.as_std_path()) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::io(path.to_string(), error)),
        };
        let inventory: HostInventory = serde_json::from_str(&source)
            .map_err(|error| Error::parse(path.to_string(), anyhow::Error::new(error)))?;
        if inventory.schema != HOST_INVENTORY_SCHEMA {
            return Err(Error::coded(
                "HOST_INVENTORY_SCHEMA_UNSUPPORTED",
                Category::Configuration,
                format!(
                    "host inventory '{path}' uses schema '{}' (expected '{HOST_INVENTORY_SCHEMA}')",
                    inventory.schema
                ),
            )
            .with_hint("re-run `ost host discover --refresh` to rewrite it"));
        }
        Ok(Some(inventory))
    }

    /// Write atomically, so a concurrent reader never sees a half-written file.
    pub fn save(&self, path: &Utf8Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent.as_std_path())
                .map_err(|error| Error::io(parent.to_string(), error))?;
        }
        let body = serde_json::to_string_pretty(self).map_err(|error| {
            Error::Operation(format!("cannot serialize the host inventory: {error}"))
        })?;
        write_atomic(path.as_std_path(), format!("{body}\n").as_bytes())
    }

    /// Re-check every validated record against the filesystem, downgrading any
    /// whose install has changed. Returns how many statuses moved.
    ///
    /// This is the whole reason a cache is safe to keep: a record is only
    /// served as usable after the bytes behind it have been confirmed to be the
    /// ones it was written from.
    pub fn refresh_statuses(&mut self) -> usize {
        let mut changed = 0;
        for record in &mut self.records {
            let refreshed = current_status(record);
            if refreshed != record.status {
                record.status = refreshed;
                changed += 1;
            }
        }
        changed
    }
}

/// What a previously validated record's status should be right now.
fn current_status(record: &HostRecord) -> HostStatus {
    if !record.status.is_usable() {
        // A rejection or a prior downgrade is a conclusion about this scan, not
        // something to re-derive from the filesystem.
        return record.status;
    }
    if record.schema_version != HOST_RECORD_SCHEMA {
        return HostStatus::Invalidated;
    }
    match &record.fingerprint {
        Some(fingerprint) if fingerprint.inputs_version != FINGERPRINT_INPUTS_VERSION => {
            return HostStatus::Invalidated;
        }
        None => return HostStatus::Candidate,
        _ => {}
    }
    if !record.root.as_std_path().is_dir() {
        return HostStatus::Unreachable;
    }
    for executable in record.executables.values() {
        let path = executable.absolute(&record.root);
        let Ok(metadata) = std::fs::metadata(path.as_std_path()) else {
            return HostStatus::Stale;
        };
        if metadata.len() != executable.size {
            return HostStatus::Stale;
        }
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok());
        // A platform that does not report mtime cannot contradict the record;
        // only a *different* reported mtime is evidence of a change.
        if let (Some(recorded), Some(observed)) = (executable.modified_unix, modified) {
            if recorded != observed {
                return HostStatus::Stale;
            }
        }
    }
    HostStatus::Validated
}

/// Where the shared, machine-wide discovery cache lives.
pub fn user_cache_path() -> Utf8PathBuf {
    Store::discover()
        .cache()
        .join("host-discovery")
        .join("inventory.json")
}

/// Where a project keeps its reviewable, checked-in-able inventory.
pub fn project_inventory_path(project_root: &Utf8Path) -> Utf8PathBuf {
    project_root
        .join(STATE_DIR)
        .join("hosts")
        .join("inventory.json")
}

/// The outcome of resolving a host selector against an inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection<'a> {
    One(&'a HostRecord),
    None,
    /// Matched several installs. The ids are returned so the caller can print
    /// them: an ambiguous selector must never be resolved by scan order.
    Ambiguous(Vec<String>),
}

/// Resolve a selector: an exact instance id, an install path, an id prefix, or
/// a family name — the last three only when they name exactly one install.
pub fn select<'a>(records: &'a [HostRecord], selector: &str) -> Selection<'a> {
    if let Some(record) = records.iter().find(|record| record.id == selector) {
        return Selection::One(record);
    }

    let key = canonical_root_key(Utf8Path::new(selector));
    let by_path: Vec<&HostRecord> = records
        .iter()
        .filter(|record| canonical_root_key(&record.root) == key)
        .collect();
    if let Some(selection) = unique(&by_path) {
        return selection;
    }

    let by_prefix: Vec<&HostRecord> = records
        .iter()
        .filter(|record| record.id.starts_with(selector))
        .collect();
    if let Some(selection) = unique(&by_prefix) {
        return selection;
    }

    if let Ok(family) = HostFamily::parse(selector) {
        // A family selector names an install to *use*, so it only considers
        // ones that are usable; a rejected record is not a candidate answer.
        let by_family: Vec<&HostRecord> = records
            .iter()
            .filter(|record| record.family == family && record.status.is_usable())
            .collect();
        if let Some(selection) = unique(&by_family) {
            return selection;
        }
    }
    Selection::None
}

fn unique<'a>(matches: &[&'a HostRecord]) -> Option<Selection<'a>> {
    match matches {
        [] => None,
        [single] => Some(Selection::One(single)),
        several => Some(Selection::Ambiguous(
            several.iter().map(|record| record.id.clone()).collect(),
        )),
    }
}
