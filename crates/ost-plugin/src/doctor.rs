//! `ost plugin doctor` — staged diagnostics (harness §12).
//!
//! Each check has a **stable id**, a [`Status`] (PASS / FAIL / SKIP), the
//! observed fact, and machine-readable `suggested_actions`. This mirrors the
//! `Check`/`ValidationReport` pattern from `runtime validate`, extended with the
//! SKIP state the harness needs: levels that require a *real* OpenUSD runtime are
//! reported as `SKIP` with a reason — never a false `PASS` (harness §12, the 4a
//! definition of done).
//!
//! Levels 0–1 are static (manifest + filesystem + runtime manifest) and run on
//! today's mock backend. Levels 2+ (discovery, `usdcat`, Python stage open,
//! golden) are emitted as `SKIP` until the real runtime backend lands in 4b.

use indexmap::IndexMap;

use crate::bundle::Bundle;
use crate::version::{self, RangeError};

/// Outcome of a single staged check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pass,
    Fail,
    Skip,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Fail => "fail",
            Status::Skip => "skip",
        }
    }
}

/// One diagnostic: a stable id, the level it belongs to, its status, the fact we
/// observed, and any suggested actions.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub id: String,
    pub level: u8,
    pub status: Status,
    pub observed: String,
    pub suggested_actions: Vec<String>,
}

impl Diagnostic {
    fn pass(id: &str, level: u8, observed: impl Into<String>) -> Diagnostic {
        Diagnostic {
            id: id.into(),
            level,
            status: Status::Pass,
            observed: observed.into(),
            suggested_actions: Vec::new(),
        }
    }

    fn fail(id: &str, level: u8, observed: impl Into<String>, actions: Vec<String>) -> Diagnostic {
        Diagnostic {
            id: id.into(),
            level,
            status: Status::Fail,
            observed: observed.into(),
            suggested_actions: actions,
        }
    }

    fn skip(id: &str, level: u8, reason: impl Into<String>) -> Diagnostic {
        Diagnostic {
            id: id.into(),
            level,
            status: Status::Skip,
            observed: reason.into(),
            suggested_actions: Vec::new(),
        }
    }
}

/// The resolved runtime facts Level 1 checks against. The CLI populates this
/// from the pulled runtime manifest and the platform; the crate stays decoupled
/// from runtime resolution.
#[derive(Debug, Clone, Default)]
pub struct RuntimeContext {
    /// Whether a runtime has been pulled (its manifest exists on disk).
    pub pulled: bool,
    /// Backend source of the runtime (`mock`/`local`/`build`/`artifact`).
    pub source: Option<String>,
    /// Whether the source is reproducible (`build`/`artifact`) vs adopted/mock.
    pub reproducible: bool,
    /// Concrete OpenUSD version the runtime provides, e.g. `24.11`.
    pub openusd_version: Option<String>,
    /// C++ ABI tag the runtime/platform was built with.
    pub cxx_abi: Option<String>,
    /// Python ABI tag the runtime provides, e.g. `cp311`.
    pub python_abi: Option<String>,
    /// component id -> version the runtime provides (for `dependency.*`).
    pub components: IndexMap<String, String>,
}

/// A full staged report.
#[derive(Debug, Clone)]
pub struct DoctorReport {
    pub diagnostics: Vec<Diagnostic>,
}

impl DoctorReport {
    /// Whether the run passed: no diagnostic failed (SKIPs do not fail a run).
    pub fn passed(&self) -> bool {
        !self.diagnostics.iter().any(|d| d.status == Status::Fail)
    }

    pub fn count(&self, status: Status) -> usize {
        self.diagnostics.iter().filter(|d| d.status == status).count()
    }
}

/// Run the staged diagnostics for `bundle` against the runtime `ctx`.
///
/// `up_to_level` bounds which levels are evaluated (e.g. `inspect` runs L0 only).
/// Levels above the static ceiling are always emitted as `SKIP`.
pub fn diagnose(bundle: &Bundle, ctx: &RuntimeContext, up_to_level: u8) -> DoctorReport {
    let mut diags = Vec::new();

    // ---- Level 0: bundle structure (no runtime needed) ----
    diags.extend(level0(bundle));

    // ---- Level 1: runtime / ABI compatibility (reads the runtime manifest) ----
    if up_to_level >= 1 {
        diags.extend(level1(bundle, ctx));
    }

    // ---- Levels 2+: need a real OpenUSD runtime — SKIP, never false PASS ----
    if up_to_level >= 2 {
        const REASON: &str = "needs a real OpenUSD runtime (current backend is mock)";
        diags.push(Diagnostic::skip("plugin.discovery", 2, REASON));
        diags.push(Diagnostic::skip("usdcat.read", 3, REASON));
        diags.push(Diagnostic::skip("python.stage_open", 4, REASON));
        diags.push(Diagnostic::skip("golden.roundtrip", 5, REASON));
    }

    DoctorReport { diagnostics: diags }
}

fn level0(bundle: &Bundle) -> Vec<Diagnostic> {
    let m = &bundle.manifest;
    let mut diags = Vec::new();

    // bundle.manifest — we parsed it; report identity as the observed fact.
    diags.push(Diagnostic::pass(
        "bundle.manifest",
        0,
        format!(
            "{} {} ({})",
            m.plugin.name,
            m.plugin.version,
            m.kind().as_str()
        ),
    ));

    // bundle.plug_info — present at the declared path and parses as JSON.
    let plug_info = bundle.plug_info();
    if !plug_info.as_std_path().is_file() {
        diags.push(Diagnostic::fail(
            "bundle.plug_info",
            0,
            format!("plugInfo.json not found at '{}'", m.usd.plug_info),
            vec![format!(
                "create the plugInfo.json at '{}', or fix `usd.plug_info`",
                m.usd.plug_info
            )],
        ));
    } else {
        match std::fs::read_to_string(plug_info.as_std_path()) {
            Ok(src) => match serde_json::from_str::<serde_json::Value>(&src) {
                Ok(_) => diags.push(Diagnostic::pass(
                    "bundle.plug_info",
                    0,
                    format!("valid JSON at '{}'", m.usd.plug_info),
                )),
                Err(e) => diags.push(Diagnostic::fail(
                    "bundle.plug_info",
                    0,
                    format!("plugInfo.json is not valid JSON: {e}"),
                    vec!["fix the JSON syntax in the plugInfo.json".into()],
                )),
            },
            Err(e) => diags.push(Diagnostic::fail(
                "bundle.plug_info",
                0,
                format!("cannot read plugInfo.json: {e}"),
                vec![],
            )),
        }
    }

    // plugin.shared_library — a built artifact in lib/.
    match find_shared_library(bundle) {
        Some(name) => diags.push(Diagnostic::pass(
            "plugin.shared_library",
            0,
            format!("found lib/{name}"),
        )),
        None => diags.push(Diagnostic::fail(
            "plugin.shared_library",
            0,
            "no shared library (.so/.dll/.dylib) in lib/",
            vec![format!("build it with `ost plugin build {}`", m.plugin.name)],
        )),
    }

    // bundle.fixtures — every referenced test fixture exists on disk.
    let missing: Vec<&str> = m
        .all_fixtures()
        .into_iter()
        .filter(|f| !bundle.path(f).as_std_path().is_file())
        .collect();
    if m.all_fixtures().is_empty() {
        diags.push(Diagnostic::skip(
            "bundle.fixtures",
            0,
            "no test fixtures declared",
        ));
    } else if missing.is_empty() {
        diags.push(Diagnostic::pass(
            "bundle.fixtures",
            0,
            format!("{} fixture(s) present", m.all_fixtures().len()),
        ));
    } else {
        diags.push(Diagnostic::fail(
            "bundle.fixtures",
            0,
            format!("missing fixtures: {}", missing.join(", ")),
            vec!["add the missing fixture files, or update `tests` in the manifest".into()],
        ));
    }

    // session.plugin_path — the discovery root the session would set is present.
    let pxr_root = bundle.plug_info_root();
    diags.push(Diagnostic::pass(
        "session.plugin_path",
        0,
        format!(
            "PXR_PLUGINPATH_NAME would include '{}'",
            pxr_root.to_string().replace('\\', "/")
        ),
    ));

    diags
}

fn level1(bundle: &Bundle, ctx: &RuntimeContext) -> Vec<Diagnostic> {
    let m = &bundle.manifest;
    let mut diags = Vec::new();

    if !ctx.pulled {
        let reason = "runtime not pulled (run `ost runtime pull <platform> --profile <profile>`)";
        diags.push(Diagnostic::skip("runtime.openusd.version", 1, reason));
        diags.push(Diagnostic::skip("runtime.cxx_abi", 1, reason));
        diags.push(Diagnostic::skip("runtime.python_abi", 1, reason));
        for comp in m.requires.components.keys() {
            diags.push(Diagnostic::skip(&format!("dependency.{comp}"), 1, reason));
        }
        return diags;
    }

    // runtime.source — surface the backend source as an observed fact. An
    // adopted (`local`) or mock runtime is real-but-not-reproducible; that is
    // not a failure, but it is recorded so reports never imply certification.
    if let Some(src) = &ctx.source {
        let observed = if ctx.reproducible {
            format!("runtime source is '{src}' (reproducible)")
        } else {
            format!("runtime source is '{src}' (real but not reproducible/certified)")
        };
        diags.push(Diagnostic::pass("runtime.source", 1, observed));
    }

    // runtime.openusd.version — concrete runtime version satisfies the range.
    match &ctx.openusd_version {
        Some(have) => match version::satisfies(have, &m.runtime.openusd) {
            Ok(true) => diags.push(Diagnostic::pass(
                "runtime.openusd.version",
                1,
                format!("runtime OpenUSD {have} satisfies '{}'", m.runtime.openusd),
            )),
            Ok(false) => diags.push(Diagnostic::fail(
                "runtime.openusd.version",
                1,
                format!(
                    "runtime OpenUSD {have} does not satisfy '{}'",
                    m.runtime.openusd
                ),
                vec![
                    "pull a runtime within the plugin's range, or widen `runtime.openusd`".into(),
                ],
            )),
            Err(e) => diags.push(range_error_diag("runtime.openusd.version", &m.runtime.openusd, e)),
        },
        None => diags.push(Diagnostic::skip(
            "runtime.openusd.version",
            1,
            "runtime does not record an OpenUSD version",
        )),
    }

    // runtime.cxx_abi — declared plugin ABI matches the runtime ABI.
    diags.push(match_tag(
        "runtime.cxx_abi",
        m.runtime.cxx_abi.as_deref(),
        ctx.cxx_abi.as_deref(),
        "C++ ABI",
    ));

    // runtime.python_abi — declared plugin Python ABI matches the runtime.
    diags.push(match_tag(
        "runtime.python_abi",
        m.runtime.python_abi.as_deref(),
        ctx.python_abi.as_deref(),
        "Python ABI",
    ));

    // dependency.<component> — required components present + within range.
    for (comp, range) in &m.requires.components {
        let id = format!("dependency.{comp}");
        match ctx.components.get(comp) {
            Some(have) => match version::satisfies(have, range) {
                Ok(true) => diags.push(Diagnostic::pass(
                    &id,
                    1,
                    format!("runtime provides {comp} {have} (satisfies '{range}')"),
                )),
                Ok(false) => diags.push(Diagnostic::fail(
                    &id,
                    1,
                    format!("runtime provides {comp} {have}, which does not satisfy '{range}'"),
                    vec![format!("use a runtime providing {comp} '{range}'")],
                )),
                Err(e) => diags.push(range_error_diag(&id, range, e)),
            },
            None => diags.push(Diagnostic::fail(
                &id,
                1,
                format!("runtime does not provide required component '{comp}'"),
                vec![format!(
                    "use a profile/runtime that provides {comp}, or drop it from `requires.components`"
                )],
            )),
        }
    }

    diags
}

/// Compare a plugin-declared tag against the runtime's. Missing either side is a
/// SKIP (we cannot assert), matching/mismatch is PASS/FAIL.
fn match_tag(id: &str, declared: Option<&str>, runtime: Option<&str>, label: &str) -> Diagnostic {
    match (declared, runtime) {
        (Some(d), Some(r)) if d == r => {
            Diagnostic::pass(id, 1, format!("{label} {d} matches runtime"))
        }
        (Some(d), Some(r)) => Diagnostic::fail(
            id,
            1,
            format!("plugin {label} '{d}' != runtime '{r}'"),
            vec![format!("rebuild the plugin against the runtime's {label} ('{r}')")],
        ),
        (None, _) => Diagnostic::skip(id, 1, format!("plugin declares no {label}")),
        (Some(_), None) => Diagnostic::skip(id, 1, format!("runtime records no {label}")),
    }
}

fn range_error_diag(id: &str, range: &str, e: RangeError) -> Diagnostic {
    Diagnostic::fail(
        id,
        1,
        format!("cannot evaluate range '{range}': {e}"),
        vec!["fix the version range to a comma-separated comparator list, e.g. `>=24.11,<25.0`".into()],
    )
}

/// Find a built shared library in the bundle's `lib/` directory, if any.
fn find_shared_library(bundle: &Bundle) -> Option<String> {
    let lib = bundle.lib_dir();
    let entries = std::fs::read_dir(lib.as_std_path()).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if matches!(ext, "so" | "dll" | "dylib") {
            return path.file_name().and_then(|n| n.to_str()).map(String::from);
        }
    }
    None
}
