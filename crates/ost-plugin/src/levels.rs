// SPDX-License-Identifier: Apache-2.0
//! Execution levels 2–6 (harness §11), run against a *real* OpenUSD runtime.
//!
//! Unlike levels 0–1 (static manifest + filesystem checks), these run the
//! runtime's tools inside the composed session env and interpret the result.
//!
//! The contract depends on the plugin kind. A **file-format** plugin:
//!
//! - **L2 `plugin.discovery`** — USD's plug registry sees the format
//!   (`Sdf.FileFormat.FindByExtension`), proving `PXR_PLUGINPATH_NAME` and the
//!   `plugInfo.json` line up and the library loads.
//! - **L3 `usdcat.read`** — `usdcat` opens a smoke fixture and emits USDA.
//! - **L4 `python.stage_open`** — `Usd.Stage.Open()` opens the fixture.
//!
//! A **schema** plugin (codeless or compiled) has its own analogue — there is no
//! file extension to discover, so L2/L4 verify the *schema* contract instead:
//!
//! - **L2 `schema.registration`** — the declared schema types are known to
//!   `Usd.SchemaRegistry` (the plugin registered them).
//! - **L4 `schema.apply_roundtrip`** — the smoke fixture applies one of the
//!   `*API` schemas to a prim, and its authored attributes survive a flatten
//!   round-trip unchanged (the analogue of `python.stage_open`).
//!
//! Both kinds share the format-agnostic upper levels:
//!
//! - **L5 `golden.roundtrip`** — `usdcat --flatten` output matches a committed
//!   golden, when one exists (else SKIP).
//! - **L6 `usdview.launch`** — `usdview --quitAfterStartup` opens the stage and
//!   exits cleanly (SKIP when usdview or a display is unavailable).
//!
//! Process execution is behind the [`Probe`] trait so the level logic is unit
//! testable without a real runtime: tests inject canned tool results.

use camino::Utf8PathBuf;

use crate::bundle::Bundle;
use crate::doctor::Diagnostic;
use crate::model::PluginKind;

/// The captured result of running one tool.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    /// Process exit code, or `None` if the tool could not be spawned.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl ToolOutput {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
    /// True when the tool could not be launched at all (not found / not runnable).
    pub fn unspawned(&self) -> bool {
        self.code.is_none()
    }
}

/// Runs tools for the level checks. The real implementation spawns processes
/// with the session env; tests substitute a fake.
pub trait Probe {
    /// Run `program` with `args`, returning its captured output.
    fn run(&self, program: &str, args: &[&str]) -> ToolOutput;
}

/// What tool executables to invoke and where the session points. The CLI builds
/// this from the resolved runtime; tools are `None` when not found.
pub struct Session<'a> {
    pub probe: &'a dyn Probe,
    /// `usdcat` executable (absolute path or bare name), if located.
    pub usdcat: Option<String>,
    /// Python interpreter that can `import pxr`, if located.
    pub python: Option<String>,
    /// `usdview` executable (the `.cmd` wrapper on Windows), if located.
    pub usdview: Option<String>,
    /// Whether a display is available for GUI tools (Level 6). The CLI sets this
    /// false on headless Linux so `usdview` is SKIPped, not falsely FAILed.
    pub has_display: bool,
}

/// Run execution levels 2..=`up_to` for `bundle` against `session`.
///
/// A schema bundle runs the schema contract (registration + apply round-trip) in
/// place of the file-format discovery/read levels; both share the upper
/// (format-agnostic) golden and usdview levels.
pub fn run_levels(bundle: &Bundle, session: &Session, up_to: u8) -> Vec<Diagnostic> {
    if bundle.manifest.kind() == PluginKind::UsdSchema {
        return run_schema_levels(bundle, session, up_to);
    }

    let mut diags = Vec::new();
    if up_to >= 2 {
        diags.push(level2_discovery(bundle, session));
    }
    if up_to >= 3 {
        diags.push(level3_usdcat(bundle, session));
    }
    if up_to >= 4 {
        diags.push(level4_stage_open(bundle, session));
    }
    // A bundle of another kind (e.g. a file-format plugin) may *co-host* a schema
    // — declare `usd-schema:<Type>` and register schema types from the same
    // plugInfo (USD allows one plugInfo to provide both an SdfFileFormat and
    // schema types). Gate on the explicit `provides` declaration (not inferred
    // plugInfo Types, which would catch the file-format's own type), then run the
    // schema contract alongside the primary-kind levels.
    if !bundle.manifest.schema_provides().is_empty() {
        if up_to >= 2 {
            diags.push(level2_schema_registration(bundle, session));
        }
        if up_to >= 4 {
            diags.push(level4_schema_apply_roundtrip(bundle, session));
        }
    }
    if up_to >= 5 {
        diags.push(level5_golden(bundle, session));
    }
    if up_to >= 6 {
        diags.push(level6_usdview(bundle, session, None));
    }
    diags
}

/// Execution levels for a schema bundle. There is no file extension to discover
/// and no custom `Read()` to exercise, so L2/L4 verify the schema contract; L3
/// (`usdcat.read`) has no schema analogue and is omitted.
fn run_schema_levels(bundle: &Bundle, session: &Session, up_to: u8) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    if up_to >= 2 {
        diags.push(level2_schema_registration(bundle, session));
    }
    if up_to >= 4 {
        diags.push(level4_schema_apply_roundtrip(bundle, session));
    }
    if up_to >= 5 {
        diags.push(level5_golden(bundle, session));
    }
    if up_to >= 6 {
        diags.push(level6_usdview(bundle, session, None));
    }
    diags
}

/// Run only the Level 6 `usdview` check against an explicit `fixture` (or the
/// smoke fixture when `None`). Used by `ost plugin test-view`.
pub fn usdview_check(bundle: &Bundle, session: &Session, fixture: Option<&str>) -> Diagnostic {
    level6_usdview(bundle, session, fixture)
}

/// The file extension this fileformat plugin registers, from `provides`
/// (`usd-fileformat:<ext>`) with the first declared fixture as a fallback.
fn fileformat_ext(bundle: &Bundle) -> Option<String> {
    bundle
        .manifest
        .provides
        .iter()
        .find_map(|p| p.strip_prefix("usd-fileformat:").map(str::to_string))
        .or_else(|| {
            bundle
                .manifest
                .all_fixtures()
                .first()
                .and_then(|f| Utf8PathBuf::from(f).extension().map(str::to_string))
        })
}

/// The first smoke fixture (or any declared fixture) as a path under the bundle.
fn smoke_fixture(bundle: &Bundle) -> Option<Utf8PathBuf> {
    let rel = bundle
        .manifest
        .tests
        .smoke
        .first()
        .map(String::as_str)
        .or_else(|| bundle.manifest.all_fixtures().first().copied())?;
    Some(bundle.path(rel))
}

/// The schema type names this bundle registers. Primary source is `provides`
/// (`usd-schema:<TypeName>`), e.g. `VrmHumanoidAPI`. When `provides` declares
/// none — a bundle whose types live only in the generated `plugInfo.json` — fall
/// back to the `Info.Types` keys so L2/L4 still verify them instead of SKIPping
/// green (the L0 `bundle.plug_info.schema_types` check reads the same block).
fn schema_type_names(bundle: &Bundle) -> Vec<String> {
    let from_provides: Vec<String> = bundle
        .manifest
        .provides
        .iter()
        .filter_map(|p| p.strip_prefix("usd-schema:").map(str::to_string))
        .collect();
    if !from_provides.is_empty() {
        return from_provides;
    }
    schema_types_from_plug_info(bundle)
}

/// The schema type names declared under every plugin's `Info.Types` in the
/// bundle's `plugInfo.json`. Empty when the file is absent/unreadable or carries
/// no `Types` block.
fn schema_types_from_plug_info(bundle: &Bundle) -> Vec<String> {
    let Ok(src) = std::fs::read_to_string(bundle.plug_info().as_std_path()) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&src) else {
        return Vec::new();
    };
    let Some(plugins) = json.get("Plugins").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for plugin in plugins {
        if let Some(types) = plugin
            .get("Info")
            .and_then(|info| info.get("Types"))
            .and_then(|t| t.as_object())
        {
            names.extend(types.keys().cloned());
        }
    }
    names
}

/// Render schema type names as a Python list literal, e.g. `['A', 'B']`.
fn py_name_list(names: &[String]) -> String {
    let items: Vec<String> = names.iter().map(|n| format!("'{n}'")).collect();
    format!("[{}]", items.join(", "))
}

fn level2_schema_registration(bundle: &Bundle, session: &Session) -> Diagnostic {
    const ID: &str = "schema.registration";
    let names = schema_type_names(bundle);
    if names.is_empty() {
        return Diagnostic::skip(
            ID,
            2,
            "no schema types declared (set `provides: usd-schema:<TypeName>`)",
        );
    }
    let Some(python) = &session.python else {
        return Diagnostic::skip(ID, 2, "no python interpreter on the session PATH");
    };

    // Ask USD's schema registry whether each declared type is known. Codeless
    // schemas register here too (that is the point — no C++ required).
    let script = format!(
        r#"import sys
from pxr import Usd
names = {names}
def known(n):
    try:
        return Usd.SchemaRegistry.GetSchemaKind(n) != Usd.SchemaKind.Invalid
    except Exception:
        return False
missing = [n for n in names if not known(n)]
if missing:
    sys.stderr.write('unregistered schema types: ' + ', '.join(missing))
sys.exit(0 if not missing else 7)
"#,
        names = py_name_list(&names)
    );
    let out = session.probe.run(python, &["-c", &script]);
    if out.unspawned() {
        return Diagnostic::fail(
            ID,
            2,
            format!("could not run python ({python})"),
            vec!["ensure the runtime python is on PATH".into()],
        );
    }
    if out.ok() {
        Diagnostic::pass(
            ID,
            2,
            format!("USD schema registry knows {}", names.join(", ")),
        )
    } else {
        Diagnostic::fail(
            ID,
            2,
            format!("schema types not registered: {}", tail(&out.stderr)),
            vec![
                "check PXR_PLUGINPATH_NAME points at the bundle's plugInfo root".into(),
                "verify plugInfo.json declares the schema `Types` (run `usdGenSchema`)".into(),
            ],
        )
    }
}

fn level4_schema_apply_roundtrip(bundle: &Bundle, session: &Session) -> Diagnostic {
    const ID: &str = "schema.apply_roundtrip";
    let names = schema_type_names(bundle);
    if names.is_empty() {
        return Diagnostic::skip(ID, 4, "no schema types declared (set `provides`)");
    }
    let Some(python) = &session.python else {
        return Diagnostic::skip(ID, 4, "no python interpreter on the session PATH");
    };
    let Some(fixture) = smoke_fixture(bundle) else {
        return Diagnostic::skip(ID, 4, "no smoke fixture declared");
    };
    if !fixture.as_std_path().is_file() {
        return Diagnostic::fail(ID, 4, format!("fixture '{fixture}' is missing"), vec![]);
    }

    // Open the fixture, find a prim with one of the schema APIs applied, snapshot
    // its authored attributes, flatten the stage, re-open, and assert the API is
    // still applied and the attribute values are unchanged. `__NAMES__`/
    // `__FIXTURE__` are substituted (not `format!`-interpolated) so the script's
    // Python dict/set literals keep their braces.
    let path = fixture.to_string().replace('\\', "/");
    let script = SCHEMA_ROUNDTRIP_PY
        .replace("__NAMES__", &py_name_list(&names))
        .replace("__FIXTURE__", &format!("'{path}'"));
    let out = session.probe.run(python, &["-c", &script]);
    if out.unspawned() {
        return Diagnostic::fail(ID, 4, format!("could not run python ({python})"), vec![]);
    }
    if out.ok() {
        Diagnostic::pass(
            ID,
            4,
            "schema applies to a prim and authored attributes survive a flatten round-trip",
        )
    } else {
        Diagnostic::fail(
            ID,
            4,
            format!("schema apply/round-trip failed: {}", tail(&out.stderr)),
            vec![
                "confirm the fixture's `apiSchemas` names a registered schema (Level 2)".into(),
                "check the applied attributes are declared by the schema".into(),
            ],
        )
    }
}

/// Python for L4: apply-and-round-trip. Markers are substituted before running.
const SCHEMA_ROUNDTRIP_PY: &str = r#"
import sys
from pxr import Usd
names = __NAMES__
stage = Usd.Stage.Open(__FIXTURE__)
if not stage:
    sys.stderr.write('could not open the fixture stage')
    sys.exit(8)
target = None
for prim in stage.Traverse():
    if any(n in prim.GetAppliedSchemas() for n in names):
        target = prim
        break
if target is None:
    sys.stderr.write('no prim applies any of: ' + ', '.join(names))
    sys.exit(9)
before = {a.GetName(): a.Get() for a in target.GetAttributes() if a.HasAuthoredValue()}
flat = stage.Flatten()
restage = Usd.Stage.Open(flat)
reprim = restage.GetPrimAtPath(target.GetPath())
if not reprim or not any(n in reprim.GetAppliedSchemas() for n in names):
    sys.stderr.write('API schema not applied after flatten round-trip')
    sys.exit(10)
after = {a.GetName(): a.Get() for a in reprim.GetAttributes() if a.HasAuthoredValue()}
if before != after:
    sys.stderr.write('attribute values changed across round-trip: %r -> %r' % (before, after))
    sys.exit(11)
sys.exit(0)
"#;

fn level2_discovery(bundle: &Bundle, session: &Session) -> Diagnostic {
    const ID: &str = "plugin.discovery";
    if bundle.manifest.kind() != PluginKind::UsdFileformat {
        return Diagnostic::skip(ID, 2, "discovery check implemented for usd-fileformat only");
    }
    let Some(ext) = fileformat_ext(bundle) else {
        return Diagnostic::skip(ID, 2, "no extension to look up (set `provides`)");
    };
    let Some(python) = &session.python else {
        return Diagnostic::skip(ID, 2, "no python interpreter on the session PATH");
    };

    // Ask USD's registry whether the extension resolves to a file format.
    let script = format!(
        "import sys\nfrom pxr import Sdf\nsys.exit(0 if Sdf.FileFormat.FindByExtension('{ext}') else 7)"
    );
    let out = session.probe.run(python, &["-c", &script]);
    if out.unspawned() {
        return Diagnostic::fail(
            ID,
            2,
            format!("could not run python ({python})"),
            vec!["ensure the runtime python is on PATH".into()],
        );
    }
    if out.ok() {
        Diagnostic::pass(
            ID,
            2,
            format!("USD registry resolves '.{ext}' to a file format"),
        )
    } else {
        Diagnostic::fail(
            ID,
            2,
            format!(
                "USD does not recognize '.{ext}' (discovery failed): {}",
                tail(&out.stderr)
            ),
            vec![
                "check PXR_PLUGINPATH_NAME points at the bundle's plugInfo root".into(),
                "verify plugInfo.json LibraryPath resolves and the library loads".into(),
            ],
        )
    }
}

fn level3_usdcat(bundle: &Bundle, session: &Session) -> Diagnostic {
    const ID: &str = "usdcat.read";
    let Some(usdcat) = &session.usdcat else {
        return Diagnostic::fail(
            ID,
            3,
            "usdcat not found in the runtime",
            vec!["adopt/build a runtime whose bin/ contains usdcat".into()],
        );
    };
    let Some(fixture) = smoke_fixture(bundle) else {
        return Diagnostic::skip(ID, 3, "no smoke fixture declared");
    };
    if !fixture.as_std_path().is_file() {
        return Diagnostic::fail(
            ID,
            3,
            format!("fixture '{fixture}' is missing"),
            vec!["add the fixture or fix `tests.smoke`".into()],
        );
    }

    let out = session.probe.run(usdcat, &[fixture.as_str()]);
    if out.unspawned() {
        return Diagnostic::fail(ID, 3, format!("could not run usdcat ({usdcat})"), vec![]);
    }
    if out.ok() && !out.stdout.trim().is_empty() {
        Diagnostic::pass(
            ID,
            3,
            format!(
                "usdcat read '{}' and emitted USDA",
                fixture.file_name().unwrap_or("")
            ),
        )
    } else {
        Diagnostic::fail(
            ID,
            3,
            format!("usdcat could not read the fixture: {}", tail(&out.stderr)),
            vec!["confirm the plugin is discovered (Level 2) and CanRead accepts the file".into()],
        )
    }
}

fn level4_stage_open(bundle: &Bundle, session: &Session) -> Diagnostic {
    const ID: &str = "python.stage_open";
    let Some(python) = &session.python else {
        return Diagnostic::skip(ID, 4, "no python interpreter on the session PATH");
    };
    let Some(fixture) = smoke_fixture(bundle) else {
        return Diagnostic::skip(ID, 4, "no smoke fixture declared");
    };
    if !fixture.as_std_path().is_file() {
        return Diagnostic::fail(ID, 4, format!("fixture '{fixture}' is missing"), vec![]);
    }

    // Forward-slash the path so it embeds cleanly in the Python string literal.
    let path = fixture.to_string().replace('\\', "/");
    let script =
        format!("import sys\nfrom pxr import Usd\nsys.exit(0 if Usd.Stage.Open('{path}') else 8)");
    let out = session.probe.run(python, &["-c", &script]);
    if out.unspawned() {
        return Diagnostic::fail(ID, 4, format!("could not run python ({python})"), vec![]);
    }
    if out.ok() {
        Diagnostic::pass(ID, 4, "Usd.Stage.Open() opened the fixture")
    } else {
        Diagnostic::fail(
            ID,
            4,
            format!("Usd.Stage.Open() failed: {}", tail(&out.stderr)),
            vec!["check the plugin's Read() authors a valid layer".into()],
        )
    }
}

fn level5_golden(bundle: &Bundle, session: &Session) -> Diagnostic {
    const ID: &str = "golden.roundtrip";
    let Some(fixture) = smoke_fixture(bundle) else {
        return Diagnostic::skip(ID, 5, "no smoke fixture declared");
    };
    // Golden convention: `<fixture>.golden.usda` sits next to the fixture.
    let golden = Utf8PathBuf::from(format!("{fixture}.golden.usda"));
    if !golden.as_std_path().is_file() {
        return Diagnostic::skip(ID, 5, "no golden file (expected <fixture>.golden.usda)");
    }
    let Some(usdcat) = &session.usdcat else {
        return Diagnostic::fail(ID, 5, "usdcat not found in the runtime", vec![]);
    };

    let out = session.probe.run(usdcat, &["--flatten", fixture.as_str()]);
    if out.unspawned() {
        return Diagnostic::fail(ID, 5, format!("could not run usdcat ({usdcat})"), vec![]);
    }
    if !out.ok() {
        return Diagnostic::fail(
            ID,
            5,
            format!("usdcat --flatten failed: {}", tail(&out.stderr)),
            vec![],
        );
    }
    let expected = std::fs::read_to_string(golden.as_std_path()).unwrap_or_default();
    if normalize(&out.stdout) == normalize(&expected) {
        Diagnostic::pass(ID, 5, "flattened output matches the golden")
    } else {
        Diagnostic::fail(
            ID,
            5,
            "flattened output differs from the golden",
            vec!["review the diff; update the golden if the change is intended".into()],
        )
    }
}

fn level6_usdview(bundle: &Bundle, session: &Session, fixture: Option<&str>) -> Diagnostic {
    const ID: &str = "usdview.launch";
    let Some(usdview) = &session.usdview else {
        return Diagnostic::skip(
            ID,
            6,
            "usdview not in the runtime (build with usdview enabled)",
        );
    };
    if !session.has_display {
        return Diagnostic::skip(ID, 6, "no display available for usdview");
    };
    let path = match fixture {
        Some(f) => bundle.path(f),
        None => match smoke_fixture(bundle) {
            Some(p) => p,
            None => return Diagnostic::skip(ID, 6, "no fixture to open"),
        },
    };
    if !path.as_std_path().is_file() {
        return Diagnostic::fail(ID, 6, format!("fixture '{path}' is missing"), vec![]);
    }

    // `--quitAfterStartup` opens the stage in usdview then exits: a non-interactive
    // launch probe. The exit code is the signal — usdview prints many benign
    // warnings (e.g. no numpy) on stderr even on a clean startup.
    let out = session
        .probe
        .run(usdview, &[path.as_str(), "--quitAfterStartup"]);
    if out.unspawned() {
        return Diagnostic::fail(ID, 6, format!("could not run usdview ({usdview})"), vec![]);
    }
    if out.ok() {
        Diagnostic::pass(
            ID,
            6,
            format!(
                "usdview opened '{}' and exited cleanly",
                path.file_name().unwrap_or("")
            ),
        )
    } else {
        Diagnostic::fail(
            ID,
            6,
            format!(
                "usdview failed to launch/open the stage: {}",
                tail(&out.stderr)
            ),
            vec!["run `ost plugin view` to see the full usdview output".into()],
        )
    }
}

/// Normalize USDA text for comparison: trim trailing whitespace per line and
/// ignore leading/trailing blank lines and line-ending differences.
fn normalize(s: &str) -> String {
    s.replace("\r\n", "\n")
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// The last non-empty line of tool stderr, for a compact failure summary.
fn tail(stderr: &str) -> String {
    stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::Status;
    use crate::model::PluginManifest;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// A fake probe: maps a program name to a canned [`ToolOutput`], and records
    /// the calls it received.
    struct FakeProbe {
        responses: HashMap<String, ToolOutput>,
        calls: RefCell<Vec<String>>,
    }

    impl FakeProbe {
        fn new() -> Self {
            FakeProbe {
                responses: HashMap::new(),
                calls: RefCell::new(Vec::new()),
            }
        }
        fn on(mut self, program: &str, code: Option<i32>, stdout: &str, stderr: &str) -> Self {
            self.responses.insert(
                program.to_string(),
                ToolOutput {
                    code,
                    stdout: stdout.into(),
                    stderr: stderr.into(),
                },
            );
            self
        }
    }

    impl Probe for FakeProbe {
        fn run(&self, program: &str, _args: &[&str]) -> ToolOutput {
            self.calls.borrow_mut().push(program.to_string());
            self.responses.get(program).cloned().unwrap_or(ToolOutput {
                code: None,
                stdout: String::new(),
                stderr: "not found".into(),
            })
        }
    }

    fn bundle_with_fixture() -> (tempdir_like::Dir, Bundle) {
        let dir = tempdir_like::Dir::new("levels");
        std::fs::create_dir_all(dir.path.join("tests/fixtures").as_std_path()).unwrap();
        std::fs::write(
            dir.path.join("tests/fixtures/basic.toy").as_std_path(),
            "toy 1.0\n",
        )
        .unwrap();
        let manifest = PluginManifest::parse(
            r#"
plugin: { name: toy, version: 0.1.0, kind: usd-fileformat }
runtime: { openusd: ">=25.05,<26.0" }
provides: ["usd-fileformat:toy"]
usd: { plug_info: plugin/resources/toy/plugInfo.json }
tests: { smoke: ["tests/fixtures/basic.toy"] }
"#,
        )
        .unwrap();
        let bundle = Bundle {
            root: dir.path.clone(),
            manifest,
        };
        (dir, bundle)
    }

    fn schema_bundle_with_fixture() -> (tempdir_like::Dir, Bundle) {
        let dir = tempdir_like::Dir::new("levels-schema");
        std::fs::create_dir_all(dir.path.join("tests/fixtures").as_std_path()).unwrap();
        std::fs::write(
            dir.path.join("tests/fixtures/basic.usda").as_std_path(),
            "#usda 1.0\ndef Xform \"Root\" (prepend apiSchemas = [\"VrmSchemaAPI\"]) {}\n",
        )
        .unwrap();
        let manifest = PluginManifest::parse(
            r#"
plugin: { name: vrm-schema, version: 0.1.0, kind: usd-schema }
runtime: { openusd: ">=25.05,<27.0" }
schema: { codeless: true }
provides: ["usd-schema:VrmSchemaAPI"]
usd: { plug_info: plugin/resources/vrm-schema/plugInfo.json }
tests: { smoke: ["tests/fixtures/basic.usda"] }
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
    fn schema_levels_replace_fileformat_levels_and_pass() {
        let (_d, bundle) = schema_bundle_with_fixture();
        // Both the registration and round-trip scripts run `python` and succeed.
        let probe = FakeProbe::new().on("python", Some(0), "", "");
        let session = Session {
            probe: &probe,
            usdcat: None,
            python: Some("python".into()),
            usdview: None,
            has_display: false,
        };
        let diags = run_levels(&bundle, &session, 4);
        let ids: Vec<&str> = diags.iter().map(|d| d.id.as_str()).collect();
        // The schema contract runs in place of the file-format discovery/read levels.
        assert!(ids.contains(&"schema.registration"));
        assert!(ids.contains(&"schema.apply_roundtrip"));
        assert!(!ids.contains(&"plugin.discovery"));
        assert!(!ids.contains(&"usdcat.read"));
        let by_id = |id: &str| diags.iter().find(|d| d.id == id).unwrap().status;
        assert_eq!(by_id("schema.registration"), Status::Pass);
        assert_eq!(by_id("schema.apply_roundtrip"), Status::Pass);
    }

    #[test]
    fn schema_registration_fails_when_type_unregistered() {
        let (_d, bundle) = schema_bundle_with_fixture();
        let probe = FakeProbe::new().on(
            "python",
            Some(7),
            "",
            "unregistered schema types: VrmSchemaAPI",
        );
        let session = Session {
            probe: &probe,
            usdcat: None,
            python: Some("python".into()),
            usdview: None,
            has_display: false,
        };
        let d = &run_levels(&bundle, &session, 2)[0];
        assert_eq!(d.id, "schema.registration");
        assert_eq!(d.status, Status::Fail);
        assert!(!d.suggested_actions.is_empty());
    }

    #[test]
    fn schema_levels_skip_without_python() {
        let (_d, bundle) = schema_bundle_with_fixture();
        let probe = FakeProbe::new();
        let session = Session {
            probe: &probe,
            usdcat: None,
            python: None,
            usdview: None,
            has_display: false,
        };
        let diags = run_levels(&bundle, &session, 4);
        let by_id = |id: &str| diags.iter().find(|d| d.id == id).unwrap().status;
        assert_eq!(by_id("schema.registration"), Status::Skip);
        assert_eq!(by_id("schema.apply_roundtrip"), Status::Skip);
    }

    #[test]
    fn schema_type_names_fall_back_to_plug_info_when_provides_is_empty() {
        // A schema bundle that forgot `provides` but whose types live in the
        // generated plugInfo.json must still be verified (not SKIP green).
        let dir = tempdir_like::Dir::new("levels-schema-fallback");
        std::fs::create_dir_all(dir.path.join("plugin/resources/vrm").as_std_path()).unwrap();
        std::fs::write(
            dir.path
                .join("plugin/resources/vrm/plugInfo.json")
                .as_std_path(),
            r#"{ "Plugins": [ { "Info": { "Types": { "VrmHumanoidAPI": { "bases": [] } } } } ] }"#,
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
        // `provides` is empty, so the names come from the plugInfo `Info.Types`.
        assert!(bundle.manifest.provides.is_empty());
        assert_eq!(
            schema_type_names(&bundle),
            vec!["VrmHumanoidAPI".to_string()]
        );
    }

    #[test]
    fn fileformat_co_hosting_a_schema_runs_both_contracts() {
        // A file-format bundle that also declares `usd-schema:<Type>` in provides
        // runs the schema levels alongside the file-format ones.
        let dir = tempdir_like::Dir::new("levels-cohost");
        std::fs::create_dir_all(dir.path.join("tests/fixtures").as_std_path()).unwrap();
        std::fs::write(dir.path.join("tests/fixtures/basic.toy").as_std_path(), "x").unwrap();
        let manifest = PluginManifest::parse(
            r#"
plugin: { name: toy, version: 0.1.0, kind: usd-fileformat }
runtime: { openusd: ">=25.05,<27.0" }
provides: ["usd-fileformat:toy", "usd-schema:ToyAPI"]
usd: { plug_info: plugin/resources/toy/plugInfo.json }
tests: { smoke: ["tests/fixtures/basic.toy"] }
"#,
        )
        .unwrap();
        let bundle = Bundle {
            root: dir.path.clone(),
            manifest,
        };
        let probe =
            FakeProbe::new()
                .on("python", Some(0), "", "")
                .on("usdcat", Some(0), "#usda 1.0\n", "");
        let session = Session {
            probe: &probe,
            usdcat: Some("usdcat".into()),
            python: Some("python".into()),
            usdview: None,
            has_display: false,
        };
        let ids: Vec<String> = run_levels(&bundle, &session, 4)
            .into_iter()
            .map(|d| d.id)
            .collect();
        // Both the file-format and the co-hosted schema contracts ran.
        assert!(ids.iter().any(|i| i == "plugin.discovery"));
        assert!(ids.iter().any(|i| i == "python.stage_open"));
        assert!(ids.iter().any(|i| i == "schema.registration"));
        assert!(ids.iter().any(|i| i == "schema.apply_roundtrip"));
    }

    #[test]
    fn plain_fileformat_does_not_run_schema_levels() {
        // A file-format bundle with no `usd-schema:` in provides must NOT trigger
        // the schema contract — its own plugInfo `Info.Types` is not a schema.
        let (_d, bundle) = bundle_with_fixture();
        let probe = FakeProbe::new()
            .on("python", Some(0), "", "")
            .on("usdcat", Some(0), "x", "");
        let session = Session {
            probe: &probe,
            usdcat: Some("usdcat".into()),
            python: Some("python".into()),
            usdview: None,
            has_display: false,
        };
        let ids: Vec<String> = run_levels(&bundle, &session, 4)
            .into_iter()
            .map(|d| d.id)
            .collect();
        assert!(ids.iter().all(|i| i != "schema.registration"));
        assert!(ids.iter().all(|i| i != "schema.apply_roundtrip"));
    }

    #[test]
    fn discovery_and_read_pass_when_tools_succeed() {
        let (_d, bundle) = bundle_with_fixture();
        let probe =
            FakeProbe::new()
                .on("python", Some(0), "", "")
                .on("usdcat", Some(0), "#usda 1.0\n", "");
        let session = Session {
            probe: &probe,
            usdcat: Some("usdcat".into()),
            python: Some("python".into()),
            usdview: None,
            has_display: false,
        };
        let diags = run_levels(&bundle, &session, 4);
        let by_id = |id: &str| diags.iter().find(|d| d.id == id).unwrap().status;
        assert_eq!(by_id("plugin.discovery"), Status::Pass);
        assert_eq!(by_id("usdcat.read"), Status::Pass);
        assert_eq!(by_id("python.stage_open"), Status::Pass);
    }

    #[test]
    fn discovery_fails_when_registry_rejects_extension() {
        let (_d, bundle) = bundle_with_fixture();
        let probe = FakeProbe::new().on("python", Some(7), "", "unknown extension");
        let session = Session {
            probe: &probe,
            usdcat: None,
            python: Some("python".into()),
            usdview: None,
            has_display: false,
        };
        let d = &run_levels(&bundle, &session, 2)[0];
        assert_eq!(d.status, Status::Fail);
        assert_eq!(d.id, "plugin.discovery");
        assert!(!d.suggested_actions.is_empty());
    }

    #[test]
    fn usdcat_missing_is_a_fail_not_a_skip() {
        let (_d, bundle) = bundle_with_fixture();
        let probe = FakeProbe::new();
        let session = Session {
            probe: &probe,
            usdcat: None,
            python: None,
            usdview: None,
            has_display: false,
        };
        let d = &run_levels(&bundle, &session, 3)[1];
        assert_eq!(d.id, "usdcat.read");
        assert_eq!(d.status, Status::Fail);
    }

    #[test]
    fn golden_skips_when_absent() {
        let (_d, bundle) = bundle_with_fixture();
        let probe = FakeProbe::new().on("usdcat", Some(0), "x", "");
        let session = Session {
            probe: &probe,
            usdcat: Some("usdcat".into()),
            python: None,
            usdview: None,
            has_display: false,
        };
        let diags = run_levels(&bundle, &session, 5);
        let golden = diags.iter().find(|d| d.id == "golden.roundtrip").unwrap();
        assert_eq!(golden.status, Status::Skip);
    }

    #[test]
    fn usdview_level_passes_skips_and_reports() {
        let (_d, bundle) = bundle_with_fixture();

        // Clean exit (even with benign stderr) -> PASS.
        let probe = FakeProbe::new().on("usdview", Some(0), "", "no numpy; harmless");
        let pass = usdview_check(
            &bundle,
            &Session {
                probe: &probe,
                usdcat: None,
                python: None,
                usdview: Some("usdview".into()),
                has_display: true,
            },
            None,
        );
        assert_eq!(pass.id, "usdview.launch");
        assert_eq!(pass.status, Status::Pass);

        // No display -> SKIP (not a false FAIL on headless CI).
        let skip = usdview_check(
            &bundle,
            &Session {
                probe: &probe,
                usdcat: None,
                python: None,
                usdview: Some("usdview".into()),
                has_display: false,
            },
            None,
        );
        assert_eq!(skip.status, Status::Skip);

        // usdview missing -> SKIP.
        let none = usdview_check(
            &bundle,
            &Session {
                probe: &probe,
                usdcat: None,
                python: None,
                usdview: None,
                has_display: true,
            },
            None,
        );
        assert_eq!(none.status, Status::Skip);
    }

    /// Minimal scoped temp directory helper (no external dev-deps).
    mod tempdir_like {
        use camino::Utf8PathBuf;
        pub struct Dir {
            pub path: Utf8PathBuf,
        }
        impl Dir {
            pub fn new(tag: &str) -> Dir {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let mut p = Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
                p.push(format!("ost-levels-{tag}-{}-{nanos}", std::process::id()));
                std::fs::create_dir_all(p.as_std_path()).unwrap();
                Dir { path: p }
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(self.path.as_std_path());
            }
        }
    }
}
