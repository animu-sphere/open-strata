//! `ost-core` — shared primitives for the OpenStrata toolchain.
//!
//! This crate intentionally holds no domain logic. It defines the vocabulary
//! used across the workspace: where things live on disk ([`paths`]), how we
//! describe the machine we are running on ([`host`]), and how a build/runtime
//! variant is identified ([`variant`]).

pub mod catalog;
pub mod digest;
pub mod error;
pub mod host;
pub mod paths;
pub mod tools;
pub mod variant;

pub use error::{Error, Result};
pub use host::Host;
pub use variant::Variant;
