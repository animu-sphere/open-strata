//! Project manifest and lockfile models.
//!
//! * [`Project`] is the human-authored `openstrata.toml` at a project root.
//! * [`Lock`] is the generated `strata.lock` recording the resolved runtime,
//!   variant, extensions and validation status (§9.4). Phase 0 defines the
//!   shape; later phases populate it during resolution and build.

mod lock;
mod project;

pub use lock::{Lock, LockPython, LockRuntime};
pub use project::{Project, ProjectMeta, Requires};
