// SPDX-License-Identifier: Apache-2.0
//! VFX Reference Platform calendar-year manifests (§4.1).
//!
//! A [`Platform`] is a machine-readable snapshot of a VFX Reference Platform
//! calendar year — the *target constraints* OpenStrata turns into certified
//! runtimes. This crate models that document, loads built-in and user-provided
//! manifests, and computes structured diffs between two years.

mod diff;
mod loader;
mod model;

pub use diff::{diff, ComponentChange, PlatformDiff};
pub use loader::{load_all, load_one, Catalog};
pub use model::{Platform, Source, SourceKind, Status};
