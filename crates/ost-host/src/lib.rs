// SPDX-License-Identifier: Apache-2.0
//! `ost-host` — third-party DCC hosts as discovered, validated, versioned records.
//!
//! OpenStrata's first-class surface is applications on the OpenStrata runtime.
//! Existing DCCs — Maya, Houdini — are supported as **third-party hosts**:
//! discovered, fingerprinted, and recorded, never installed, mutated, licensed,
//! or abstracted behind a fake cross-DCC API. A DCC's own bundled OpenUSD is,
//! in this tree's terms, an adopted runtime: real but not reproducible.
//!
//! Three things live here and nothing else does:
//!
//! * [`record`] — the versioned [`HostRecord`], its status model, its
//!   deterministic instance id, and its fingerprint.
//! * [`discovery`] — order-stable providers that *suggest* roots, bounded and
//!   declarative, plus the pass that validates them.
//! * [`validate`] — per-family validators that confirm a root is what it claims
//!   to be, or refuse it with a reason.
//!
//! Composing a host into a runnable environment is deliberately **not** here:
//! that is Formation's job, and a host launch must go through the same resolved,
//! conflict-checked environment model as everything else rather than growing a
//! parallel DCC-specific one.
//!
//! Note on the word "host": in this crate it means a DCC install on the
//! machine. The machine itself is [`ost_core::Host`], and appears here only as
//! [`record::HostPlatform`].

pub mod discovery;
pub mod inventory;
pub mod record;
pub mod validate;

pub use discovery::{discover, DiscoveryOutcome, DiscoveryRequest, DiscoveryWarning, SCAN_BUDGET};
pub use inventory::{
    project_inventory_path, select, user_cache_path, HostInventory, Selection,
    HOST_INVENTORY_SCHEMA,
};
pub use record::{
    canonical_root_key, fingerprint, normalize_path, DiscoveryEvidence, DiscoverySource,
    FingerprintMode, HostExecutable, HostFamily, HostFingerprint, HostIdentity, HostPlatform,
    HostPython, HostRecord, HostRejection, HostStatus, HostVersion, VersionSource,
    FINGERPRINT_INPUTS_VERSION, HOST_RECORD_SCHEMA,
};
pub use validate::{HostValidation, ValidateOptions, DEFAULT_PROBE_TIMEOUT};
