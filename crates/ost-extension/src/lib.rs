// SPDX-License-Identifier: Apache-2.0
//! `ost-extension` — controlled VFX-adjacent components (§4.4, §5, §6).
//!
//! OpenStrata does not allow arbitrary dependencies. Extensions (OpenUSD,
//! MaterialX, …) are *certified* components that declare the capabilities they
//! provide, the features/packages those capabilities require, and the runtimes
//! they are compatible with. The [`resolve`] step turns a profile's requested
//! capabilities into a concrete set of extensions, enabled features, and — for
//! OpenUSD — a certified build point within an allowed version range (§5.3).

mod loader;
mod model;
mod resolve;

pub use loader::{load_all, Catalog};
pub use model::{Certified, Extension, Feature, Provide};
pub use resolve::{resolve, why, ProviderEdge, RequirementReason, Resolution, ResolvedExtension};
