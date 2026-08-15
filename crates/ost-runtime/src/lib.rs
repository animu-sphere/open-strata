// SPDX-License-Identifier: Apache-2.0
//! `ost-runtime` — runtime identity, profiles, and environment generation.
//!
//! This crate turns a platform + profile selection into a concrete [`Runtime`]
//! identity and the [`EnvSet`] needed to use it. The first vertical slice does
//! not pull real artifacts: it resolves the runtime *prefix* under the user
//! store and generates the environment that would activate it, so
//! `ost env <platform> --profile <p>` produces correct shell output today.

mod env;
mod graphics;
mod manifest;
mod profile;
mod runtime;
mod validate;

pub use env::{usd_python_dir, usd_python_dir_for, EnvOp, EnvSet, EnvVar, Shell};
pub use graphics::{
    graphics_device_status, graphics_loader_probes_supported, graphics_loader_status,
    probe_graphics_devices, probe_graphics_loaders, GraphicsApi, GraphicsDeviceProbe,
    GraphicsLoaderProbe,
};
pub use manifest::{
    ExtensionRecord, HostPackageManager, HostRequirement, RuntimeManifest, RuntimeSource,
    Validation, MANIFEST_FILE,
};
pub use ost_platform::{
    OpenUsdBuilder, OpenUsdVariantId, OpenUsdVerification, OpenUsdVerificationStatus,
    ResolvedDependencyIdentity, ResolvedOpenUsdCompatibility, ResolvedSourceIdentity,
};
pub use profile::{Profile, ProfileCatalog, Requires};
pub use runtime::{python_abi_tag, python_minor, Runtime};
pub use validate::{validate, Check, ValidationReport};
