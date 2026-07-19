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

mod completion;
mod external;
pub mod glibc;
mod lease;
mod lock;
pub mod msvc;
pub mod package;
mod presets;
pub mod python;
mod target;
mod toolchain;

pub use completion::{
    BuildCompletion, BuildIntent, BuildOutput, BuildProjectIdentity, TestCompletion, TestTotals,
    BUILD_COMPLETION_FILE, BUILD_COMPLETION_SCHEMA, TEST_COMPLETION_FILE, TEST_COMPLETION_SCHEMA,
};
pub use external::{
    CMakeCache, ExternalBuildProvenance, ExternalRuntime, ExternalToolchain, ImportError,
    EXTERNAL_BUILD_FILE, EXTERNAL_BUILD_SCHEMA, IDENTITY_KEYS,
};
pub use glibc::{max_glibc_floor, GlibcVersion};
pub use lease::{
    LeaseMode, LeaseOwner, StaleReason, StaleTakeover, TargetLease, TARGET_BUSY_CODE,
    TARGET_LEASE_FILE, TARGET_LEASE_SCHEMA,
};
pub use lock::{LockCompiler, LockRuntime, TargetLock};
pub use package::{
    is_sdk_path, pack_dir, pack_dir_with, sdk_stage_files, source_date_epoch,
    source_date_epoch_opt, stage_files, FileEntry, PackOptions, PackProgress, PackResult,
    SdkStageFiles, ZSTD_LEVEL,
};
pub use presets::{
    ensure_includes, includes_of, is_managed_include, managed_include, remove_managed_includes,
    render_target_presets,
};
pub use python::{
    bundles_usdgenschema, module_present, provision_schema_gen_deps, relocate_baked_prefix,
    relocate_baked_python, resolve_for_runtime, resolve_python_hints, resolve_run_python,
    run_python_search_paths, usd_python_requirement, PythonHints, PythonSource, SchemaDepsOutcome,
    SCHEMA_GEN_MODULES, SCHEMA_GEN_PACKAGES,
};
pub use target::Target;
pub use toolchain::{render_toolchain, Compiler};

/// Normalize a path to forward slashes, which CMake accepts on every platform
/// and which keeps generated files identical across hosts.
pub(crate) fn cmake_path(path: &str) -> String {
    path.replace('\\', "/")
}
