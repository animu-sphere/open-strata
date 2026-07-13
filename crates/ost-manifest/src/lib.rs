// SPDX-License-Identifier: Apache-2.0
//! Project, renderer, and lockfile models.
//!
//! * [`Project`] is the human-authored `openstrata.toml` at a project root.
//! * [`RendererManifest`] records renderer composition and validation intent.
//! * [`RendererReport`] is deterministic renderer validation evidence.
//! * [`Lock`] is the generated `strata.lock` recording the resolved runtime,
//!   variant, extensions and validation status (§9.4). Phase 0 defines the
//!   shape; later phases populate it during resolution and build.

mod lock;
mod project;
mod renderer;

pub use lock::{Lock, LockExtension, LockPython, LockRuntime, Validation};
pub use project::{add_extension, set_version_file, BuildConfig, Project, ProjectMeta, Requires};
pub use renderer::{
    FrameContract, RenderProducts, RendererCheck, RendererCheckStatus, RendererComposition,
    RendererDevice, RendererIdentity, RendererManifest, RendererReport, RendererReportIdentity,
    RendererValidation, RENDERER_MANIFEST, RENDERER_REPORT_FILE, RENDERER_REPORT_SCHEMA,
    RENDERER_SCHEMA,
};
