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
//! against today's mock runtime backend. Levels 2+ (discovery, `usdcat`, Python
//! stage open, golden) require a *real* OpenUSD runtime and are reported as
//! `SKIP` — never a false `PASS` — until the artifact backend lands in 4b.

mod bundle;
mod doctor;
mod model;
mod report;
mod scaffold;
mod session;
mod version;

pub use bundle::Bundle;
pub use doctor::{diagnose, Diagnostic, DoctorReport, RuntimeContext, Status};
pub use model::{
    PluginIdentity, PluginKind, PluginManifest, Requires, RuntimeReq, Tests, UsdSection,
    PLUGIN_MANIFEST,
};
pub use report::{
    environment_json, report_json, summary_text, write_report, REPORT_SCHEMA,
};
pub use scaffold::scaffold;
pub use session::{bundle_vars, session_env};
pub use version::{satisfies, RangeError};
