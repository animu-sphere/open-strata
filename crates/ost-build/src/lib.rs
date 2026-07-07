// SPDX-License-Identifier: Apache-2.0
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
pub mod python;
mod target;
mod toolchain;

pub use lock::{LockCompiler, TargetLock};
pub use package::{
    is_sdk_path, pack_dir, sdk_stage_files, stage_files, FileEntry, PackResult, SdkStageFiles,
};
pub use presets::{
    ensure_includes, includes_of, is_managed_include, managed_include, remove_managed_includes,
    render_target_presets,
};
pub use python::{
    relocate_baked_prefix, relocate_baked_python, resolve_for_runtime, resolve_python_hints,
    usd_python_requirement, PythonHints, PythonSource,
};
pub use target::Target;
pub use toolchain::{render_toolchain, Compiler};

/// Normalize a path to forward slashes, which CMake accepts on every platform
/// and which keeps generated files identical across hosts.
pub(crate) fn cmake_path(path: &str) -> String {
    path.replace('\\', "/")
}
