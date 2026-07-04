// SPDX-License-Identifier: Apache-2.0
//! `ost plugin doctor` — staged diagnostics (harness §12).
//!
//! Each check has a **stable id**, a [`Status`] (PASS / FAIL / SKIP), the
//! observed fact, and machine-readable `suggested_actions`. This mirrors the
//! `Check`/`ValidationReport` pattern from `runtime validate`, extended with the
//! SKIP state the harness needs: levels that require a *real* OpenUSD runtime are
//! reported as `SKIP` with a reason — never a false `PASS` (harness §12, the 4a
//! definition of done).
//!
//! `doctor` covers the static Levels 0–1 (manifest + filesystem + runtime
//! manifest); it emits `SKIP` for Levels 2+ and points at `ost plugin test`,
//! which executes those against a real runtime (see [`crate::run_levels`]).

use camino::{Utf8Path, Utf8PathBuf};
use indexmap::IndexMap;
use ost_core::host::Os;

use crate::bundle::Bundle;
use crate::model::CxxAbi;
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
    pub(crate) fn pass(id: &str, level: u8, observed: impl Into<String>) -> Diagnostic {
        Diagnostic {
            id: id.into(),
            level,
            status: Status::Pass,
            observed: observed.into(),
            suggested_actions: Vec::new(),
        }
    }

    pub(crate) fn fail(
        id: &str,
        level: u8,
        observed: impl Into<String>,
        actions: Vec<String>,
    ) -> Diagnostic {
        Diagnostic {
            id: id.into(),
            level,
            status: Status::Fail,
            observed: observed.into(),
            suggested_actions: actions,
        }
    }

    pub(crate) fn skip(id: &str, level: u8, reason: impl Into<String>) -> Diagnostic {
        Diagnostic {
            id: id.into(),
            level,
            status: Status::Skip,
            observed: reason.into(),
            suggested_actions: Vec::new(),
        }
    }

    pub(crate) fn skip_with_actions(
        id: &str,
        level: u8,
        reason: impl Into<String>,
        actions: Vec<String>,
    ) -> Diagnostic {
        Diagnostic {
            id: id.into(),
            level,
            status: Status::Skip,
            observed: reason.into(),
            suggested_actions: actions,
        }
    }
}

/// The resolved runtime facts Level 1 checks against. The CLI populates this
/// from the pulled runtime manifest and the platform; the crate stays decoupled
/// from runtime resolution.
#[derive(Debug, Clone, Default)]
pub struct RuntimeContext {
    /// Target operating system for target-specific bundle checks.
    pub target_os: Option<Os>,
    /// Whether a runtime has been pulled (its manifest exists on disk).
    pub pulled: bool,
    /// Backend source of the runtime (`mock`/`local`/`build`/`artifact`).
    pub source: Option<String>,
    /// Whether the source carries real OpenUSD artifacts (anything but `mock`).
    pub real: bool,
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
        self.diagnostics
            .iter()
            .filter(|d| d.status == status)
            .count()
    }
}

/// Run the staged diagnostics for `bundle` against the runtime `ctx`.
///
/// `up_to_level` bounds which levels are evaluated (e.g. `inspect` runs L0 only).
/// Levels above the static ceiling are always emitted as `SKIP`.
pub fn diagnose(bundle: &Bundle, ctx: &RuntimeContext, up_to_level: u8) -> DoctorReport {
    let mut diags = Vec::new();

    // ---- Level 0: bundle structure (no runtime needed) ----
    diags.extend(level0(bundle, ctx.target_os));

    // ---- Level 1: runtime / ABI compatibility (reads the runtime manifest) ----
    if up_to_level >= 1 {
        diags.extend(level1(bundle, ctx));
    }

    // ---- Levels 2+: executed by `ost plugin test`, not `doctor`. SKIP here,
    //      with a reason that depends on whether a real runtime is available. ----
    if up_to_level >= 2 {
        let reason = if ctx.real {
            "execute with `ost plugin test` (a real runtime is available)"
        } else {
            "needs a real OpenUSD runtime (current backend is mock or absent)"
        };
        // Mirror the ids `run_levels` would emit so doctor's SKIP placeholders and
        // an executed `ost plugin test` agree per kind. A schema bundle has no
        // file extension to discover or read, so it gets the schema contract ids.
        if bundle.manifest.kind() == crate::model::PluginKind::UsdSchema {
            diags.push(Diagnostic::skip("schema.registration", 2, reason));
            diags.push(Diagnostic::skip("schema.apply_roundtrip", 4, reason));
            diags.push(Diagnostic::skip("golden.roundtrip", 5, reason));
        } else {
            diags.push(Diagnostic::skip("plugin.discovery", 2, reason));
            diags.push(Diagnostic::skip("usdcat.read", 3, reason));
            diags.push(Diagnostic::skip("python.stage_open", 4, reason));
            // A co-hosted schema (another kind that also declares
            // `usd-schema:<Type>`) runs the schema contract too — mirror its ids.
            if !bundle.manifest.schema_provides().is_empty() {
                diags.push(Diagnostic::skip("schema.registration", 2, reason));
                diags.push(Diagnostic::skip("schema.apply_roundtrip", 4, reason));
            }
            diags.push(Diagnostic::skip("golden.roundtrip", 5, reason));
        }
    }

    DoctorReport { diagnostics: diags }
}

fn level0(bundle: &Bundle, target_os: Option<Os>) -> Vec<Diagnostic> {
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
            Ok(src) => match crate::plug_info::parse_plug_info(&src) {
                Ok(json) => {
                    diags.push(Diagnostic::pass(
                        "bundle.plug_info",
                        0,
                        format!("valid JSON at '{}'", m.usd.plug_info),
                    ));
                    // A codeless schema is resource-only: it declares schema
                    // `Types` and ships no shared library, so the library-path
                    // checks would hard-fail a perfectly valid bundle. Validate
                    // the `Types` block instead.
                    if m.is_codeless_schema() {
                        diags.push(check_plug_info_schema_types(&json));
                    } else {
                        diags.push(check_plug_info_library_paths(bundle, &json, target_os));
                    }
                }
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

    // plugin.shared_library — a built artifact in lib/. A codeless schema ships
    // no library at all, so this check does not apply to it.
    if m.is_codeless_schema() {
        diags.push(Diagnostic::skip(
            "plugin.shared_library",
            0,
            "codeless schema: resource-only, no shared library",
        ));
    } else {
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
                vec![format!(
                    "build it with `ost plugin build {}`",
                    m.plugin.name
                )],
            )),
        }
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

    // bundle.runtime_libs — every loader-path directory declared by the bundle exists.
    diags.push(check_runtime_lib_dirs(bundle));

    // schema.library_prefix — non-failing guidance for a usdGenSchema naming
    // footgun: the generated C++/TfType name composes libraryPrefix + class name.
    if let Some(hint) = check_schema_library_prefix(bundle) {
        diags.push(hint);
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

    // runtime.source — surface the backend source as an observed fact. Three
    // tiers: a `mock` runtime carries no real OpenUSD; an adopted (`local`)
    // runtime is real but not reproducible; only `build`/`artifact` are
    // reproducible. None are failures, but the tier is recorded so reports
    // never imply certification.
    if let Some(src) = &ctx.source {
        let observed = if !ctx.real {
            format!("runtime source is '{src}' (mock — no real OpenUSD)")
        } else if ctx.reproducible {
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
                vec!["pull a runtime within the plugin's range, or widen `runtime.openusd`".into()],
            )),
            Err(e) => diags.push(range_error_diag(
                "runtime.openusd.version",
                &m.runtime.openusd,
                e,
            )),
        },
        None => diags.push(Diagnostic::skip(
            "runtime.openusd.version",
            1,
            "runtime does not record an OpenUSD version",
        )),
    }

    // runtime.cxx_abi — the plugin's declared ABI matches the runtime's. The
    // declared side may be a scalar, an `inherit` sentinel (defer to the runtime),
    // or a per-OS map resolved against the target.
    diags.push(match &m.runtime.cxx_abi {
        None => Diagnostic::skip("runtime.cxx_abi", 1, "plugin declares no C++ ABI"),
        Some(abi) if abi.is_inherit() => Diagnostic::skip(
            "runtime.cxx_abi",
            1,
            "plugin defers its C++ ABI to the runtime (inherit)",
        ),
        Some(abi) => match abi.tag_for(ctx.target_os) {
            Some(declared) => match_cxx_abi(abi, declared, ctx),
            None => Diagnostic::skip_with_actions(
                "runtime.cxx_abi",
                1,
                "plugin declares no C++ ABI for the resolved target OS",
                missing_target_cxx_abi_actions(ctx),
            ),
        },
    });

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
            vec![format!(
                "rebuild the plugin against the runtime's {label} ('{r}')"
            )],
        ),
        (None, _) => Diagnostic::skip(id, 1, format!("plugin declares no {label}")),
        (Some(_), None) => Diagnostic::skip(id, 1, format!("runtime records no {label}")),
    }
}

fn match_cxx_abi(abi: &CxxAbi, declared: &str, ctx: &RuntimeContext) -> Diagnostic {
    const ID: &str = "runtime.cxx_abi";
    match ctx.cxx_abi.as_deref() {
        Some(runtime) if declared == runtime => {
            Diagnostic::pass(ID, 1, format!("C++ ABI {declared} matches runtime"))
        }
        Some(runtime) => {
            let target = ctx
                .target_os
                .map(|os| format!(" for target {}", os.as_str()))
                .unwrap_or_default();
            Diagnostic::fail(
                ID,
                1,
                format!("plugin C++ ABI '{declared}' != runtime '{runtime}'{target}"),
                cxx_abi_mismatch_actions(abi, ctx.target_os, runtime),
            )
        }
        None => Diagnostic::skip(ID, 1, "runtime records no C++ ABI"),
    }
}

fn cxx_abi_mismatch_actions(abi: &CxxAbi, target_os: Option<Os>, runtime: &str) -> Vec<String> {
    let target = target_os
        .map(|os| os.as_str().to_string())
        .unwrap_or_else(|| "the resolved target".into());
    match abi {
        CxxAbi::Scalar(_) => vec![
            "for source bundles, prefer `runtime.cxx_abi: inherit` or a per-OS map such as `runtime.cxx_abi: { windows: msvc143, linux: libstdcxx, macos: libcxx }`".into(),
            format!(
                "for a target-specific artifact, rebuild for {target} and record `runtime.cxx_abi: {runtime}`"
            ),
        ],
        CxxAbi::PerOs(_) => {
            let key = target_os.map(os_key).unwrap_or("<target>");
            vec![format!(
                "set `runtime.cxx_abi.{key}` to `{runtime}`, or rebuild the plugin for {target}"
            )]
        }
    }
}

fn missing_target_cxx_abi_actions(ctx: &RuntimeContext) -> Vec<String> {
    let mut actions = vec![
        "use `runtime.cxx_abi: inherit` for a source bundle that defers to each runtime".into(),
    ];
    if let (Some(os), Some(runtime)) = (ctx.target_os, ctx.cxx_abi.as_deref()) {
        actions.push(format!(
            "or add `runtime.cxx_abi.{}` with `{runtime}` for this target",
            os_key(os)
        ));
    }
    actions
}

fn os_key(os: Os) -> &'static str {
    match os {
        Os::Linux => "linux",
        Os::Macos => "macos",
        Os::Windows => "windows",
    }
}

fn range_error_diag(id: &str, range: &str, e: RangeError) -> Diagnostic {
    Diagnostic::fail(
        id,
        1,
        format!("cannot evaluate range '{range}': {e}"),
        vec![
            "fix the version range to a comma-separated comparator list, e.g. `>=24.11,<25.0`"
                .into(),
        ],
    )
}

fn check_plug_info_library_paths(
    bundle: &Bundle,
    json: &serde_json::Value,
    target_os: Option<Os>,
) -> Diagnostic {
    const ID: &str = "bundle.plug_info.library_path";

    let Some(plugins) = json.get("Plugins").and_then(|v| v.as_array()) else {
        return Diagnostic::fail(
            ID,
            0,
            "plugInfo.json has no `Plugins` array",
            vec!["add a USD `Plugins` array with a library plugin entry".into()],
        );
    };

    let mut checked = Vec::new();
    let existing_libs = find_shared_libraries(bundle);
    let lib_dir = normalize_path(bundle.lib_dir());

    for plugin in plugins {
        if plugin.get("Type").and_then(|v| v.as_str()) != Some("library") {
            continue;
        }

        let name = plugin
            .get("Name")
            .and_then(|v| v.as_str())
            .unwrap_or("<unnamed>");
        let Some(library_path) = plugin.get("LibraryPath").and_then(|v| v.as_str()) else {
            return Diagnostic::fail(
                ID,
                0,
                format!("library plugin '{name}' has no `LibraryPath`"),
                vec![
                    "set `LibraryPath` to the built shared library under the bundle's lib/".into(),
                ],
            );
        };
        if library_path.trim().is_empty() {
            return Diagnostic::fail(
                ID,
                0,
                format!("library plugin '{name}' has an empty `LibraryPath`"),
                vec![
                    "set `LibraryPath` to the built shared library under the bundle's lib/".into(),
                ],
            );
        }
        if contains_template_token(library_path) {
            return Diagnostic::fail(
                ID,
                0,
                format!("library plugin '{name}' has unresolved LibraryPath '{library_path}'"),
                vec!["generate plugInfo.json from plugInfo.json.in during CMake configure".into()],
            );
        }
        if let Err(why) = check_portable_relative_path(library_path) {
            return Diagnostic::fail(
                ID,
                0,
                format!("library plugin '{name}' has unsafe LibraryPath '{library_path}': {why}"),
                vec![
                    "make `LibraryPath` relative to the plugInfo Root and inside the bundle".into(),
                ],
            );
        }

        if let Some(os) = target_os {
            let expected = shared_library_suffix(os);
            if !library_path.ends_with(expected) {
                return Diagnostic::fail(
                    ID,
                    0,
                    format!(
                        "library plugin '{name}' uses LibraryPath '{library_path}', expected a {expected} library for {}",
                        os.as_str()
                    ),
                    vec![
                        format!(
                            "regenerate plugInfo.json for the {} target so `LibraryPath` ends in {expected}",
                            os.as_str()
                        ),
                        "for source bundles, keep `plugInfo.json.in` and configure `LibraryPath` with `@OPENSTRATA_PLUGIN_LIBRARY_PREFIX@` plus `@CMAKE_SHARED_LIBRARY_SUFFIX@`".into(),
                    ],
                );
            }
        }

        let root = plugin.get("Root").and_then(|v| v.as_str()).unwrap_or(".");
        if contains_template_token(root) {
            return Diagnostic::fail(
                ID,
                0,
                format!("library plugin '{name}' has unresolved Root '{root}'"),
                vec!["generate plugInfo.json from plugInfo.json.in during CMake configure".into()],
            );
        }
        if let Err(why) = check_portable_relative_path(root) {
            return Diagnostic::fail(
                ID,
                0,
                format!("library plugin '{name}' has unsafe Root '{root}': {why}"),
                vec!["keep `Root` relative to the plugInfo.json directory".into()],
            );
        }

        let resolved = normalize_path(bundle.plug_info_root().join(root).join(library_path));
        if !resolved.starts_with(&lib_dir) {
            return Diagnostic::fail(
                ID,
                0,
                format!(
                    "library plugin '{name}' LibraryPath resolves to '{}', outside bundle lib/",
                    resolved.to_string().replace('\\', "/")
                ),
                vec!["point `LibraryPath` at the shared library staged under bundle lib/".into()],
            );
        }

        if !existing_libs.is_empty() && !resolved.as_std_path().is_file() {
            return Diagnostic::fail(
                ID,
                0,
                format!(
                    "LibraryPath '{}' does not match a built library (lib/ contains {})",
                    library_path,
                    existing_libs.join(", ")
                ),
                vec![
                    "regenerate plugInfo.json for the target, or rebuild the plugin library".into(),
                    "if the bundle is cross-platform source, generate `LibraryPath` per target from `plugInfo.json.in`".into(),
                ],
            );
        }

        checked.push(library_path.to_string());
    }

    if checked.is_empty() {
        return Diagnostic::fail(
            ID,
            0,
            "plugInfo.json has no library plugin entry",
            vec!["add a `Type: library` plugin entry with a concrete `LibraryPath`".into()],
        );
    }

    let target_note = target_os
        .map(|os| format!(" (target {})", os.as_str()))
        .unwrap_or_else(|| " (target suffix not checked)".into());
    let (subject, verb) = if checked.len() == 1 {
        ("entry", "points")
    } else {
        ("entries", "point")
    };
    Diagnostic::pass(
        ID,
        0,
        format!(
            "{} LibraryPath {subject} {verb} under bundle lib/{target_note}",
            checked.len()
        ),
    )
}

/// Validate the `Types` block of a *codeless* schema's `plugInfo.json` — the
/// resource-only analogue of [`check_plug_info_library_paths`].
///
/// A codeless schema registers its classes entirely through `plugInfo.json`:
/// each plugin entry carries an `Info.Types` map (the schema class names) and,
/// being resource-only, no `LibraryPath`. We assert that at least one schema
/// type is declared and that the bundle does not also point at a library it does
/// not ship.
fn check_plug_info_schema_types(json: &serde_json::Value) -> Diagnostic {
    const ID: &str = "bundle.plug_info.schema_types";

    let Some(plugins) = json.get("Plugins").and_then(|v| v.as_array()) else {
        return Diagnostic::fail(
            ID,
            0,
            "plugInfo.json has no `Plugins` array",
            vec!["add a USD `Plugins` entry declaring the schema `Info.Types`".into()],
        );
    };

    let mut type_names = Vec::new();
    let mut has_library_path = false;
    for plugin in plugins {
        if plugin
            .get("LibraryPath")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.trim().is_empty())
        {
            has_library_path = true;
        }
        if let Some(types) = plugin
            .get("Info")
            .and_then(|info| info.get("Types"))
            .and_then(|t| t.as_object())
        {
            type_names.extend(types.keys().cloned());
        }
    }

    if type_names.is_empty() {
        return Diagnostic::fail(
            ID,
            0,
            "codeless schema declares no types under any plugin's `Info.Types`",
            vec![
                "run `usdGenSchema schema.usda .` to populate the `Types` block".into(),
                "or set `schema.codeless: false` if this schema ships a compiled library".into(),
            ],
        );
    }

    if has_library_path {
        return Diagnostic::fail(
            ID,
            0,
            format!(
                "codeless schema declares {} type(s) but also a `LibraryPath` (codeless schemas ship no library)",
                type_names.len()
            ),
            vec![
                "drop the `LibraryPath` — a codeless schema is resource-only".into(),
                "or set `schema.codeless: false` if it really ships a compiled library".into(),
            ],
        );
    }

    Diagnostic::pass(
        ID,
        0,
        format!(
            "codeless schema registers {} type(s): {}",
            type_names.len(),
            type_names.join(", ")
        ),
    )
}

fn check_runtime_lib_dirs(bundle: &Bundle) -> Diagnostic {
    const ID: &str = "bundle.runtime_libs";

    let dirs = &bundle.manifest.requires.runtime_libs;
    if dirs.is_empty() {
        return Diagnostic::skip(ID, 0, "no runtime library dirs declared");
    }

    let mut missing = Vec::new();
    let mut not_dirs = Vec::new();
    for dir in dirs {
        let path = bundle.path(dir);
        let std_path = path.as_std_path();
        if !std_path.exists() {
            missing.push(dir.as_str());
        } else if !std_path.is_dir() {
            not_dirs.push(dir.as_str());
        }
    }

    if missing.is_empty() && not_dirs.is_empty() {
        return Diagnostic::pass(
            ID,
            0,
            format!("{} runtime library dir(s) present", dirs.len()),
        );
    }

    let mut parts = Vec::new();
    if !missing.is_empty() {
        parts.push(format!("missing: {}", missing.join(", ")));
    }
    if !not_dirs.is_empty() {
        parts.push(format!("not directories: {}", not_dirs.join(", ")));
    }
    Diagnostic::fail(
        ID,
        0,
        parts.join("; "),
        vec![
            "create the declared runtime library directories, or update `requires.runtime_libs`"
                .into(),
        ],
    )
}

fn check_schema_library_prefix(bundle: &Bundle) -> Option<Diagnostic> {
    const ID: &str = "schema.library_prefix";

    // The declared schema.source (or the conventional root schema.usda).
    let (schema, _) = bundle.schema_source();
    let src = std::fs::read_to_string(schema.as_std_path()).ok()?;
    let prefix = extract_usda_string_assignment(&src, "libraryPrefix")?;
    if prefix.trim().is_empty() {
        return None;
    }

    let prefixed_classes: Vec<String> = extract_usda_class_names(&src)
        .into_iter()
        .filter(|name| name.starts_with(&prefix))
        .collect();
    if prefixed_classes.is_empty() {
        return None;
    }

    Some(Diagnostic::skip_with_actions(
        ID,
        0,
        format!(
            "schema.usda libraryPrefix '{prefix}' also prefixes class name(s): {}",
            prefixed_classes.join(", ")
        ),
        vec![
            "usdGenSchema composes generated C++/TfType names as `libraryPrefix + class`; avoid repeating the same leading token".into(),
            "use a shorter/distinct `libraryPrefix`, or remove the prefix from the schema class name".into(),
        ],
    ))
}

fn extract_usda_string_assignment(src: &str, key: &str) -> Option<String> {
    for line in src.lines() {
        let line = strip_usda_comment(line).trim();
        let Some(key_pos) = line.find(key) else {
            continue;
        };
        let rest = line[key_pos + key.len()..].trim_start();
        if !rest.starts_with('=') {
            continue;
        }
        return extract_quoted(rest[1..].trim_start());
    }
    None
}

fn extract_usda_class_names(src: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in src.lines() {
        let line = strip_usda_comment(line).trim_start();
        let Some(rest) = line.strip_prefix("class") else {
            continue;
        };
        if let Some(name) = extract_quoted(rest.trim_start()) {
            names.push(name);
        }
    }
    names
}

fn extract_quoted(s: &str) -> Option<String> {
    let start = s.find('"')?;
    let rest = &s[start + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn strip_usda_comment(line: &str) -> &str {
    line.split_once('#').map(|(head, _)| head).unwrap_or(line)
}

/// Find a built shared library in the bundle's `lib/` directory, if any.
fn find_shared_library(bundle: &Bundle) -> Option<String> {
    find_shared_libraries(bundle).into_iter().next()
}

fn find_shared_libraries(bundle: &Bundle) -> Vec<String> {
    let lib = bundle.lib_dir();
    let entries = match std::fs::read_dir(lib.as_std_path()) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let mut libs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if matches!(ext, "so" | "dll" | "dylib") {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                libs.push(name.to_string());
            }
        }
    }
    libs.sort();
    libs
}

fn shared_library_suffix(os: Os) -> &'static str {
    match os {
        Os::Linux => ".so",
        Os::Macos => ".dylib",
        Os::Windows => ".dll",
    }
}

fn contains_template_token(s: &str) -> bool {
    s.contains('@') || s.contains("{{") || s.contains("}}")
}

fn check_portable_relative_path(path: &str) -> std::result::Result<(), &'static str> {
    let b = path.as_bytes();
    if path.is_empty() {
        return Err("it is empty");
    }
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return Err("it looks like a drive path");
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err("it is absolute");
    }
    Ok(())
}

fn normalize_path(path: Utf8PathBuf) -> Utf8PathBuf {
    let mut normalized = Utf8PathBuf::new();
    for component in path.components() {
        match component {
            camino::Utf8Component::CurDir => {}
            camino::Utf8Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(Utf8Path::new(other.as_str())),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PluginManifest;

    fn library_path_diag(report: &DoctorReport) -> &Diagnostic {
        report
            .diagnostics
            .iter()
            .find(|d| d.id == "bundle.plug_info.library_path")
            .expect("library path diagnostic exists")
    }

    fn runtime_libs_diag(report: &DoctorReport) -> &Diagnostic {
        report
            .diagnostics
            .iter()
            .find(|d| d.id == "bundle.runtime_libs")
            .expect("runtime libs diagnostic exists")
    }

    fn schema_prefix_diag(report: &DoctorReport) -> &Diagnostic {
        report
            .diagnostics
            .iter()
            .find(|d| d.id == "schema.library_prefix")
            .expect("schema libraryPrefix diagnostic exists")
    }

    fn bundle_with_plug_info(library_path: &str, built_lib: Option<&str>) -> (TempDir, Bundle) {
        let dir = TempDir::new("doctor");
        std::fs::create_dir_all(dir.path.join("plugin/resources/toy").as_std_path()).unwrap();
        std::fs::create_dir_all(dir.path.join("lib").as_std_path()).unwrap();
        if let Some(name) = built_lib {
            std::fs::write(dir.path.join("lib").join(name).as_std_path(), "").unwrap();
        }
        let plug_info = format!(
            r#"{{
    "Plugins": [
        {{
            "Type": "library",
            "Name": "ToyFileFormat",
            "Root": ".",
            "LibraryPath": "{library_path}",
            "ResourcePath": ".",
            "Info": {{}}
        }}
    ]
}}"#
        );
        std::fs::write(
            dir.path
                .join("plugin/resources/toy/plugInfo.json")
                .as_std_path(),
            plug_info,
        )
        .unwrap();
        let manifest = PluginManifest::parse(
            r#"
plugin: { name: toy, version: 0.1.0, kind: usd-fileformat }
runtime: { openusd: ">=25.05,<26.0" }
usd: { plug_info: plugin/resources/toy/plugInfo.json }
"#,
        )
        .unwrap();
        let bundle = Bundle {
            root: dir.path.clone(),
            manifest,
        };
        (dir, bundle)
    }

    #[test]
    fn plug_info_library_path_accepts_target_suffix_and_lib_layout() {
        let (_dir, bundle) = bundle_with_plug_info(
            "../../../lib/libToyFileFormat.so",
            Some("libToyFileFormat.so"),
        );
        let report = diagnose(
            &bundle,
            &RuntimeContext {
                target_os: Some(Os::Linux),
                ..RuntimeContext::default()
            },
            0,
        );
        assert_eq!(library_path_diag(&report).status, Status::Pass);
    }

    #[test]
    fn plug_info_library_path_rejects_windows_dll_for_linux_target() {
        let (_dir, bundle) = bundle_with_plug_info(
            "../../../lib/libToyFileFormat.dll",
            Some("libToyFileFormat.dll"),
        );
        let report = diagnose(
            &bundle,
            &RuntimeContext {
                target_os: Some(Os::Linux),
                ..RuntimeContext::default()
            },
            0,
        );
        let diag = library_path_diag(&report);
        assert_eq!(diag.status, Status::Fail);
        assert!(diag.observed.contains("expected a .so library"));
        assert!(diag
            .suggested_actions
            .iter()
            .any(|a| a.contains("plugInfo.json.in")));
    }

    #[test]
    fn plug_info_library_path_rejects_paths_outside_bundle_lib() {
        let (_dir, bundle) = bundle_with_plug_info("../libToyFileFormat.so", None);
        let report = diagnose(
            &bundle,
            &RuntimeContext {
                target_os: Some(Os::Linux),
                ..RuntimeContext::default()
            },
            0,
        );
        let diag = library_path_diag(&report);
        assert_eq!(diag.status, Status::Fail);
        assert!(diag.observed.contains("outside bundle lib/"));
    }

    #[test]
    fn plug_info_library_path_rejects_mismatched_built_library() {
        let (_dir, bundle) =
            bundle_with_plug_info("../../../lib/libWrong.so", Some("libToyFileFormat.so"));
        let report = diagnose(
            &bundle,
            &RuntimeContext {
                target_os: Some(Os::Linux),
                ..RuntimeContext::default()
            },
            0,
        );
        let diag = library_path_diag(&report);
        assert_eq!(diag.status, Status::Fail);
        assert!(diag.observed.contains("does not match a built library"));
    }

    #[test]
    fn runtime_libs_pass_when_declared_dirs_exist() {
        let (dir, mut bundle) = bundle_with_plug_info(
            "../../../lib/libToyFileFormat.so",
            Some("libToyFileFormat.so"),
        );
        std::fs::create_dir_all(dir.path.join("third_party/zlib/bin").as_std_path()).unwrap();
        bundle.manifest.requires.runtime_libs = vec!["third_party/zlib/bin".into()];

        let report = diagnose(&bundle, &RuntimeContext::default(), 0);

        assert_eq!(runtime_libs_diag(&report).status, Status::Pass);
    }

    #[test]
    fn runtime_libs_fail_when_declared_dir_is_missing() {
        let (_dir, mut bundle) = bundle_with_plug_info(
            "../../../lib/libToyFileFormat.so",
            Some("libToyFileFormat.so"),
        );
        bundle.manifest.requires.runtime_libs = vec!["third_party/zlib/bin".into()];

        let report = diagnose(&bundle, &RuntimeContext::default(), 0);
        let diag = runtime_libs_diag(&report);

        assert_eq!(diag.status, Status::Fail);
        assert!(diag.observed.contains("missing: third_party/zlib/bin"));
    }

    #[test]
    fn runtime_libs_fail_when_declared_path_is_a_file() {
        let (dir, mut bundle) = bundle_with_plug_info(
            "../../../lib/libToyFileFormat.so",
            Some("libToyFileFormat.so"),
        );
        std::fs::create_dir_all(dir.path.join("third_party/zlib").as_std_path()).unwrap();
        std::fs::write(dir.path.join("third_party/zlib/bin").as_std_path(), "").unwrap();
        bundle.manifest.requires.runtime_libs = vec!["third_party/zlib/bin".into()];

        let report = diagnose(&bundle, &RuntimeContext::default(), 0);
        let diag = runtime_libs_diag(&report);

        assert_eq!(diag.status, Status::Fail);
        assert!(diag
            .observed
            .contains("not directories: third_party/zlib/bin"));
    }

    fn diag<'a>(report: &'a DoctorReport, id: &str) -> &'a Diagnostic {
        report
            .diagnostics
            .iter()
            .find(|d| d.id == id)
            .unwrap_or_else(|| panic!("diagnostic '{id}' exists"))
    }

    /// Build a codeless-schema bundle whose plugInfo.json carries the given
    /// `Info.Types` JSON object body and, optionally, a `LibraryPath`.
    fn codeless_schema_bundle(types_body: &str, library_path: Option<&str>) -> (TempDir, Bundle) {
        let dir = TempDir::new("doctor-schema");
        std::fs::create_dir_all(dir.path.join("plugin/resources/vrm").as_std_path()).unwrap();
        let lib_line = library_path
            .map(|p| format!(",\n            \"LibraryPath\": \"{p}\""))
            .unwrap_or_default();
        let plug_info = format!(
            r#"{{
    "Plugins": [
        {{
            "Info": {{ "Types": {types_body} }},
            "Name": "vrm",
            "ResourcePath": "resources",
            "Root": ".",
            "Type": "resource"{lib_line}
        }}
    ]
}}"#
        );
        std::fs::write(
            dir.path
                .join("plugin/resources/vrm/plugInfo.json")
                .as_std_path(),
            plug_info,
        )
        .unwrap();
        let manifest = PluginManifest::parse(
            r#"
plugin: { name: vrm, version: 0.1.0, kind: usd-schema }
runtime: { openusd: ">=25.05,<27.0" }
schema: { codeless: true }
usd: { plug_info: plugin/resources/vrm/plugInfo.json }
"#,
        )
        .unwrap();
        let bundle = Bundle {
            root: dir.path.clone(),
            manifest,
        };
        (dir, bundle)
    }

    #[test]
    fn codeless_schema_skips_library_and_validates_types() {
        let (_dir, bundle) =
            codeless_schema_bundle(r#"{ "VrmHumanoidAPI": { "bases": [] } }"#, None);
        let report = diagnose(&bundle, &RuntimeContext::default(), 0);

        // No shared library is required, and the library-path check is replaced
        // by the schema-types check — which passes.
        assert_eq!(diag(&report, "plugin.shared_library").status, Status::Skip);
        let types = diag(&report, "bundle.plug_info.schema_types");
        assert_eq!(types.status, Status::Pass);
        assert!(types.observed.contains("VrmHumanoidAPI"));
        // The library-path check does not run for a codeless schema.
        assert!(report
            .diagnostics
            .iter()
            .all(|d| d.id != "bundle.plug_info.library_path"));
    }

    #[test]
    fn codeless_schema_fails_when_no_types_declared() {
        let (_dir, bundle) = codeless_schema_bundle("{}", None);
        let report = diagnose(&bundle, &RuntimeContext::default(), 0);
        let types = diag(&report, "bundle.plug_info.schema_types");
        assert_eq!(types.status, Status::Fail);
        assert!(types.observed.contains("no types"));
    }

    #[test]
    fn codeless_schema_fails_when_it_also_declares_a_library() {
        let (_dir, bundle) = codeless_schema_bundle(
            r#"{ "VrmHumanoidAPI": { "bases": [] } }"#,
            Some("../../../lib/libVrm.so"),
        );
        let report = diagnose(&bundle, &RuntimeContext::default(), 0);
        let types = diag(&report, "bundle.plug_info.schema_types");
        assert_eq!(types.status, Status::Fail);
        assert!(types.observed.contains("LibraryPath"));
    }

    #[test]
    fn schema_library_prefix_hint_detects_double_prefix_risk() {
        let (dir, bundle) = codeless_schema_bundle(r#"{ "FooBarAPI": { "bases": [] } }"#, None);
        std::fs::write(
            dir.path.join("schema.usda").as_std_path(),
            r#"
over "GLOBAL" (
    customData = {
        string libraryPrefix = "Foo"
    }
)
{
}

class "FooBarAPI" (
    inherits = </APISchemaBase>
)
{
}
"#,
        )
        .unwrap();

        let report = diagnose(&bundle, &RuntimeContext::default(), 0);
        let hint = schema_prefix_diag(&report);
        assert_eq!(hint.status, Status::Skip);
        assert!(hint.observed.contains("FooBarAPI"));
        assert!(hint
            .suggested_actions
            .iter()
            .any(|a| a.contains("libraryPrefix + class")));
    }

    #[test]
    fn schema_bundle_skips_l2plus_with_schema_level_ids() {
        let (_dir, bundle) = codeless_schema_bundle(r#"{ "VrmSchemaAPI": { "bases": [] } }"#, None);
        // up_to 2 triggers the L2+ SKIP placeholders.
        let report = diagnose(&bundle, &RuntimeContext::default(), 2);
        assert_eq!(diag(&report, "schema.registration").status, Status::Skip);
        assert_eq!(diag(&report, "schema.apply_roundtrip").status, Status::Skip);
        // The file-format placeholder ids are not emitted for a schema bundle, so
        // doctor's SKIPs match the ids an executed `ost plugin test` would report.
        assert!(report
            .diagnostics
            .iter()
            .all(|d| d.id != "plugin.discovery"));
        assert!(report.diagnostics.iter().all(|d| d.id != "usdcat.read"));
    }

    #[test]
    fn cxx_abi_resolves_per_os_against_the_target() {
        use crate::model::CxxAbi;
        let (_dir, mut bundle) = bundle_with_plug_info(
            "../../../lib/libToyFileFormat.so",
            Some("libToyFileFormat.so"),
        );
        let mut map = indexmap::IndexMap::new();
        map.insert("windows".to_string(), "msvc143".to_string());
        map.insert("linux".to_string(), "libstdcxx".to_string());
        bundle.manifest.runtime.cxx_abi = Some(CxxAbi::PerOs(map));

        let ctx_for = |os: Os, runtime_abi: &str| RuntimeContext {
            target_os: Some(os),
            pulled: true,
            cxx_abi: Some(runtime_abi.into()),
            ..RuntimeContext::default()
        };

        // Matching per-OS entry vs the runtime ABI -> PASS.
        let report = diagnose(&bundle, &ctx_for(Os::Linux, "libstdcxx"), 1);
        assert_eq!(diag(&report, "runtime.cxx_abi").status, Status::Pass);

        // Per-OS entry mismatching the runtime ABI -> FAIL.
        let report = diagnose(&bundle, &ctx_for(Os::Windows, "msvc142"), 1);
        let d = diag(&report, "runtime.cxx_abi");
        assert_eq!(d.status, Status::Fail);
        assert!(d
            .suggested_actions
            .iter()
            .any(|a| a.contains("runtime.cxx_abi.windows")));

        // A target OS not listed in the map -> SKIP (nothing declared for it).
        let report = diagnose(&bundle, &ctx_for(Os::Macos, "libcxx"), 1);
        let d = diag(&report, "runtime.cxx_abi");
        assert_eq!(d.status, Status::Skip);
        assert!(d
            .suggested_actions
            .iter()
            .any(|a| a.contains("runtime.cxx_abi.macos")));
    }

    #[test]
    fn cxx_abi_inherit_defers_to_runtime() {
        use crate::model::CxxAbi;
        let (_dir, mut bundle) = bundle_with_plug_info(
            "../../../lib/libToyFileFormat.so",
            Some("libToyFileFormat.so"),
        );
        bundle.manifest.runtime.cxx_abi = Some(CxxAbi::Scalar("inherit".into()));
        let ctx = RuntimeContext {
            target_os: Some(Os::Windows),
            pulled: true,
            cxx_abi: Some("msvc143".into()),
            ..RuntimeContext::default()
        };
        let report = diagnose(&bundle, &ctx, 1);
        let d = diag(&report, "runtime.cxx_abi");
        assert_eq!(d.status, Status::Skip);
        assert!(d.observed.contains("inherit"));
    }

    #[test]
    fn cxx_abi_scalar_mismatch_suggests_inherit_or_per_os_map() {
        use crate::model::CxxAbi;
        let (_dir, mut bundle) = bundle_with_plug_info(
            "../../../lib/libToyFileFormat.dylib",
            Some("libToyFileFormat.dylib"),
        );
        bundle.manifest.runtime.cxx_abi = Some(CxxAbi::Scalar("msvc143".into()));
        let ctx = RuntimeContext {
            target_os: Some(Os::Macos),
            pulled: true,
            cxx_abi: Some("libcxx".into()),
            ..RuntimeContext::default()
        };

        let report = diagnose(&bundle, &ctx, 1);
        let d = diag(&report, "runtime.cxx_abi");
        assert_eq!(d.status, Status::Fail);
        assert!(d.suggested_actions.iter().any(|a| a.contains("inherit")));
        assert!(d
            .suggested_actions
            .iter()
            .any(|a| a.contains("windows: msvc143")));
    }

    struct TempDir {
        path: Utf8PathBuf,
    }

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let mut path = Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
            path.push(format!("ost-plugin-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(path.as_std_path()).unwrap();
            TempDir { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.path.as_std_path());
        }
    }
}
