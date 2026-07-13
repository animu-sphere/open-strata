// SPDX-License-Identifier: Apache-2.0
//! `ost-plugin` — OpenUSD plugin bundles (Phase 4).
//!
//! A plugin is a *self-describing bundle* (`openstrata.plugin.yaml` + sources +
//! `plugInfo.json` + fixtures), not a bare shared library. This crate models the
//! bundle, scaffolds new ones, and runs the **static** half of the verification
//! pyramid (harness §11):
//!
//! * Level 0 — bundle structure (manifest, `plugInfo.json`, shared library,
//!   fixtures), and
//! * Level 1 — runtime / ABI compatibility (OpenUSD range, C++/Python ABI,
//!   required components),
//!
//! against the mock runtime backend. Levels 2–5 (discovery, `usdcat`, Python
//! stage open, golden) execute the runtime's tools and so require a *real*
//! OpenUSD runtime (Phase 4b `local`/`build`/`artifact` source); they run via
//! [`run_levels`] and are orchestrated by `ost plugin test`.

mod bundle;
mod doctor;
mod levels;
mod model;
mod plug_info;
mod report;
mod scaffold;
mod session;
mod version;
mod workspace;

pub use bundle::Bundle;
pub use doctor::{diagnose, Diagnostic, DoctorReport, RuntimeContext, Status};
pub use levels::{run_levels, usdview_check, Probe, Session, ToolOutput};
pub use model::{
    BundleDependency, CxxAbi, PluginIdentity, PluginKind, PluginManifest, PluginManifestHeader,
    Requires, RuntimeReq, SchemaSection, Tests, UsdSection, PLUGIN_MANIFEST, PLUGIN_SCHEMA,
};
pub use plug_info::{
    contains_template_token, library_plugin_names, library_plugin_paths, merge_schema_types,
    shared_library_suffix, MergeError,
};
pub use report::{
    ci_evidence_from_env, environment_json, report_json, summary_text, write_report, REPORT_SCHEMA,
};
pub use scaffold::{
    add_cohosted_schema, default_template_id, scaffold, scaffold_with_template,
    scaffold_with_template_inputs, template_ids, AddedSchema, ExecTemplateInputs,
};
pub use session::{bundle_vars, session_env, session_env_from, session_env_with};
pub use version::{satisfies, RangeError};
pub use workspace::{
    validate_workspace, WorkspaceEdge, WorkspaceIssue, WorkspaceNode, WorkspaceValidation,
};
