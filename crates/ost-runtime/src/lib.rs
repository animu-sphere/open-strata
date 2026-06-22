//! `ost-runtime` — runtime identity, profiles, and environment generation.
//!
//! This crate turns a platform + profile selection into a concrete [`Runtime`]
//! identity and the [`EnvSet`] needed to use it. The first vertical slice does
//! not pull real artifacts: it resolves the runtime *prefix* under the user
//! store and generates the environment that would activate it, so
//! `ost env <platform> --profile <p>` produces correct shell output today.

mod env;
mod manifest;
mod profile;
mod runtime;
mod validate;

pub use env::{EnvOp, EnvSet, EnvVar, Shell};
pub use manifest::{ExtensionRecord, RuntimeManifest, RuntimeSource, Validation, MANIFEST_FILE};
pub use profile::{Profile, ProfileCatalog, Requires};
pub use runtime::{python_abi_tag, python_minor, Runtime};
pub use validate::{validate, Check, ValidationReport};
