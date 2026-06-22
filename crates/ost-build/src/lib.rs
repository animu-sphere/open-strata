//! `ost-build` — build target model and CMake generation (§8).
//!
//! OpenStrata does not replace CMake; it decides *which* target/runtime/ABI to
//! build against and emits the files CMake needs:
//!
//! * a `toolchain.cmake` pointing at the resolved runtime,
//! * a per-target `CMakePresets.json` (included from the project root),
//! * a `target.lock.json` pinning the target for reproducibility.
//!
//! This crate renders those artifacts as strings/values; the CLI owns the I/O.

mod lock;
pub mod msvc;
pub mod package;
mod presets;
mod target;
mod toolchain;

pub use lock::TargetLock;
pub use package::{pack_dir, FileEntry, PackResult};
pub use presets::{render_target_presets, root_presets_with_include};
pub use target::Target;
pub use toolchain::render_toolchain;

/// Normalize a path to forward slashes, which CMake accepts on every platform
/// and which keeps generated files identical across hosts.
pub(crate) fn cmake_path(path: &str) -> String {
    path.replace('\\', "/")
}
