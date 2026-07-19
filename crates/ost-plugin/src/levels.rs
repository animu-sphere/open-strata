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
//! An **asset resolver** uses L2 `resolver.registration` to resolve the smoke
//! fixture through its declared URI scheme, proving discovery, library loading,
//! resolver construction, and dispatch before the normal stage-open levels.
//!
//! A **package resolver** uses L2 `package_resolver.registration` to prove the
//! plug registry discovers the plugin and its library loads (`ArPackageResolver`
//! has no Python binding to dispatch through directly). Real extension dispatch
//! is exercised by the normal L3/L4 levels: the scaffolded smoke fixture
//! sublayers a packaged path (`basic.<ext>[content/inner.usda]`), so opening it
//! goes through the package resolver.
//!
//! All kinds share the format-agnostic upper levels:
//!
//! - **L5 `golden.roundtrip`** — `usdcat --flatten` output matches a committed
//!   golden, when one exists (else SKIP).
//! - **L6 `usdview.launch`** — `usdview --quitAfterStartup` opens the stage and
//!   exits cleanly (SKIP when usdview or a display is unavailable).
//!
//! Process execution is behind the [`Probe`] trait so the level logic is unit
//! testable without a real runtime: tests inject canned tool results.

use std::sync::atomic::{AtomicU64, Ordering};

use camino::{Utf8Path, Utf8PathBuf};

use crate::bundle::Bundle;
use crate::doctor::Diagnostic;
use crate::model::PluginKind;
use crate::verification::{adjacent_golden, PluginVerification, PLUGIN_VERIFICATION};

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

    /// Run a tool that supports `--out <path>` and return its output without
    /// routing the authored payload through stdout.
    ///
    /// Windows C/C++ tools commonly open stdout in text mode. Capturing that
    /// pipe can therefore translate every LF to CRLF, including newlines which
    /// are *inside* a triple-quoted USDA string and are semantic data. A real
    /// probe writes through the tool's file output instead. The default keeps
    /// fake probes source-compatible by materializing their canned stdout at
    /// `output` when the tool did not create a file itself.
    fn run_to_file(&self, program: &str, args: &[&str], output: &Utf8Path) -> ToolOutput {
        let mut owned = args
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>();
        owned.push("--out".into());
        owned.push(output.to_string());
        let borrowed = owned.iter().map(String::as_str).collect::<Vec<_>>();
        let result = self.run(program, &borrowed);
        if result.ok() && !output.as_std_path().is_file() {
            if let Err(error) = std::fs::write(output.as_std_path(), result.stdout.as_bytes()) {
                return ToolOutput {
                    code: None,
                    stdout: String::new(),
                    stderr: format!("could not materialize tool output at '{output}': {error}"),
                };
            }
        }
        result
    }
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
    if bundle.manifest.kind() == PluginKind::UsdAssetResolver {
        return run_asset_resolver_levels(bundle, session, up_to);
    }
    if bundle.manifest.kind() == PluginKind::UsdPackageResolver {
        return run_package_resolver_levels(bundle, session, up_to);
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

fn run_asset_resolver_levels(bundle: &Bundle, session: &Session, up_to: u8) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    if up_to >= 2 {
        diags.push(level2_resolver_registration(bundle, session));
    }
    if up_to >= 3 {
        diags.push(level3_usdcat(bundle, session));
    }
    if up_to >= 4 {
        diags.push(level4_stage_open(bundle, session));
    }
    if up_to >= 5 {
        diags.push(level5_golden(bundle, session));
    }
    if up_to >= 6 {
        diags.push(level6_usdview(bundle, session, None));
    }
    diags
}

/// Execution levels for a package resolver: L2 proves plug-registry discovery
/// and library load; the shared L3/L4 levels then open the smoke fixture, which
/// sublayers a packaged path and so exercises real extension dispatch.
fn run_package_resolver_levels(bundle: &Bundle, session: &Session, up_to: u8) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    if up_to >= 2 {
        diags.push(level2_package_resolver_registration(bundle, session));
    }
    if up_to >= 3 {
        diags.push(level3_usdcat(bundle, session));
    }
    if up_to >= 4 {
        diags.push(level4_stage_open(bundle, session));
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

/// The fixtures L5 should flatten. Every explicitly declared round-trip fixture
/// is a verification claim; the single smoke fallback exists only for legacy
/// manifests that predate `tests.roundtrip`.
fn roundtrip_fixtures(bundle: &Bundle) -> Vec<(&str, Utf8PathBuf)> {
    if bundle.manifest.tests.roundtrip.is_empty() {
        return bundle
            .manifest
            .tests
            .smoke
            .first()
            .map(String::as_str)
            .or_else(|| bundle.manifest.all_fixtures().first().copied())
            .map(|rel| vec![(rel, bundle.path(rel))])
            .unwrap_or_default();
    }

    let mut seen = Vec::new();
    let mut fixtures = Vec::new();
    for rel in &bundle.manifest.tests.roundtrip {
        if seen.contains(&rel.as_str()) {
            continue;
        }
        seen.push(rel.as_str());
        fixtures.push((rel.as_str(), bundle.path(rel)));
    }
    fixtures
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
    let Ok(json) = crate::plug_info::parse_plug_info(&src) else {
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

fn resolver_uri_schemes(bundle: &Bundle) -> Vec<String> {
    let Ok(src) = std::fs::read_to_string(bundle.plug_info().as_std_path()) else {
        return Vec::new();
    };
    let Ok(json) = crate::plug_info::parse_plug_info(&src) else {
        return Vec::new();
    };
    let Some(plugins) = json.get("Plugins").and_then(|value| value.as_array()) else {
        return Vec::new();
    };
    let mut schemes = Vec::new();
    for plugin in plugins {
        let Some(types) = plugin
            .get("Info")
            .and_then(|info| info.get("Types"))
            .and_then(|types| types.as_object())
        else {
            continue;
        };
        for metadata in types.values() {
            if metadata
                .get("bases")
                .and_then(|bases| bases.as_array())
                .is_some_and(|bases| bases.iter().any(|base| base.as_str() == Some("ArResolver")))
            {
                if let Some(uri_schemes) = metadata
                    .get("uriSchemes")
                    .and_then(|value| value.as_array())
                {
                    schemes.extend(
                        uri_schemes
                            .iter()
                            .filter_map(|value| value.as_str().map(str::to_string)),
                    );
                }
            }
        }
    }
    schemes.sort();
    schemes.dedup();
    schemes
}

fn level2_resolver_registration(bundle: &Bundle, session: &Session) -> Diagnostic {
    const ID: &str = "resolver.registration";
    let schemes = resolver_uri_schemes(bundle);
    let Some(scheme) = schemes.first() else {
        return Diagnostic::fail(
            ID,
            2,
            "plugInfo.json declares no ArResolver uriSchemes",
            vec!["declare a resolver type with bases: [ArResolver] and uriSchemes".into()],
        );
    };
    let Some(python) = &session.python else {
        return Diagnostic::skip(ID, 2, "no python interpreter on the session PATH");
    };
    let Some(fixture) = smoke_fixture(bundle) else {
        return Diagnostic::skip(ID, 2, "no smoke fixture declared");
    };
    if !fixture.as_std_path().is_file() {
        return Diagnostic::fail(ID, 2, format!("fixture '{fixture}' is missing"), vec![]);
    }

    let path = fixture.to_string().replace('\\', "/");
    let uri = format!("{scheme}:{path}");
    let uri_literal = serde_json::to_string(&uri).unwrap_or_else(|_| "\"\"".into());
    let script = format!(
        "import sys\nfrom pxr import Ar\np = Ar.GetResolver().Resolve({uri_literal})\nsys.exit(0 if p else 7)"
    );
    let out = session
        .probe
        .run(python, &["-c", &with_dll_preamble(&script)]);
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
            format!("USD dispatched '{scheme}:' to the resolver and resolved the fixture"),
        )
    } else {
        Diagnostic::fail(
            ID,
            2,
            format!(
                "resolver registration or dispatch failed: {}",
                tail(&out.stderr)
            ),
            vec![
                "check PXR_PLUGINPATH_NAME points at the bundle's plugInfo root".into(),
                "verify uriSchemes, LibraryPath, and AR_DEFINE_RESOLVER agree".into(),
            ],
        )
    }
}

/// The plugin `Name`s in the bundle's `plugInfo.json` that declare a type
/// based on `ArPackageResolver`, with the package extensions they claim.
/// Empty when the file is absent/unreadable or declares no package resolver.
fn package_resolver_registrations(bundle: &Bundle) -> Vec<(String, Vec<String>)> {
    let Ok(src) = std::fs::read_to_string(bundle.plug_info().as_std_path()) else {
        return Vec::new();
    };
    let Ok(json) = crate::plug_info::parse_plug_info(&src) else {
        return Vec::new();
    };
    let Some(plugins) = json.get("Plugins").and_then(|value| value.as_array()) else {
        return Vec::new();
    };
    let mut registrations = Vec::new();
    for plugin in plugins {
        let Some(types) = plugin
            .get("Info")
            .and_then(|info| info.get("Types"))
            .and_then(|types| types.as_object())
        else {
            continue;
        };
        let mut extensions = Vec::new();
        for metadata in types.values() {
            if metadata
                .get("bases")
                .and_then(|bases| bases.as_array())
                .is_some_and(|bases| {
                    bases
                        .iter()
                        .any(|base| base.as_str() == Some("ArPackageResolver"))
                })
            {
                if let Some(declared) = metadata
                    .get("extensions")
                    .and_then(|value| value.as_array())
                {
                    extensions.extend(
                        declared
                            .iter()
                            .filter_map(|value| value.as_str().map(str::to_string)),
                    );
                }
            }
        }
        if extensions.is_empty() {
            continue;
        }
        let Some(name) = plugin.get("Name").and_then(|value| value.as_str()) else {
            continue;
        };
        extensions.sort();
        extensions.dedup();
        registrations.push((name.to_string(), extensions));
    }
    registrations
}

fn level2_package_resolver_registration(bundle: &Bundle, session: &Session) -> Diagnostic {
    const ID: &str = "package_resolver.registration";
    let registrations = package_resolver_registrations(bundle);
    let Some((plugin_name, extensions)) = registrations.first() else {
        return Diagnostic::fail(
            ID,
            2,
            "plugInfo.json declares no ArPackageResolver package extensions",
            vec!["declare a resolver type with bases: [ArPackageResolver] and extensions".into()],
        );
    };
    let Some(python) = &session.python else {
        return Diagnostic::skip(ID, 2, "no python interpreter on the session PATH");
    };

    // `ArPackageResolver` is not bound to Python, so registration is proven at
    // the plug-registry level: the plugin is discovered by name and its library
    // loads. Dispatch through the extension is covered by L3/L4 opening the
    // smoke fixture, which sublayers a packaged path.
    // Bind the name once from its JSON-escaped literal (valid Python too), then
    // reference the `name` variable everywhere so a plugInfo `Name` containing a
    // quote or newline can never break — or inject into — the probe script.
    let name_literal = serde_json::to_string(plugin_name).unwrap_or_else(|_| "\"\"".into());
    let script = format!(
        r#"import sys
from pxr import Plug
name = {name_literal}
p = Plug.Registry().GetPluginWithName(name)
if not p:
    sys.stderr.write('plugin %s not found in the plug registry' % name)
    sys.exit(7)
if not p.Load():
    sys.stderr.write('plugin %s found but its library failed to load' % name)
    sys.exit(8)
sys.exit(0)
"#
    );
    let out = session
        .probe
        .run(python, &["-c", &with_dll_preamble(&script)]);
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
            format!(
                "plug registry discovered '{plugin_name}' and loaded its library for .{} packages",
                extensions.join("/.")
            ),
        )
    } else {
        Diagnostic::fail(
            ID,
            2,
            format!(
                "package resolver registration failed: {}",
                tail(&out.stderr)
            ),
            vec![
                "check PXR_PLUGINPATH_NAME points at the bundle's plugInfo root".into(),
                "verify Name, LibraryPath, and AR_DEFINE_PACKAGE_RESOLVER agree".into(),
            ],
        )
    }
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
    let out = session
        .probe
        .run(python, &["-c", &with_dll_preamble(&script)]);
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
    let out = session
        .probe
        .run(python, &["-c", &with_dll_preamble(&script)]);
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

/// Prepend a Python preamble that makes USD's extension-module DLLs loadable
/// on Windows before importing `pxr`.
///
/// Since Python 3.8 the interpreter no longer searches `PATH` for an extension
/// module's dependent DLLs, so `import pxr` fails with "DLL load failed while
/// importing _tf" even though the runtime's `lib/` (holding `usd_*.dll`) is on
/// `PATH` — the exact failure a clean CI runner hits with an adopted runtime.
/// Registering each session `PATH` directory via `os.add_dll_directory`
/// restores discovery. A no-op off Windows (where `add_dll_directory` is
/// absent and `LD_LIBRARY_PATH`/`DYLD_LIBRARY_PATH` already cover it).
fn with_dll_preamble(script: &str) -> String {
    let preamble = [
        "import os",
        "if hasattr(os, 'add_dll_directory'):",
        "    for _ostdir in os.environ.get('PATH', '').split(os.pathsep):",
        "        if _ostdir and os.path.isdir(_ostdir):",
        "            try:",
        "                os.add_dll_directory(_ostdir)",
        "            except OSError:",
        "                pass",
        "",
    ]
    .join("\n");
    format!("{preamble}{script}")
}

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
    let out = session
        .probe
        .run(python, &["-c", &with_dll_preamble(&script)]);
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
    let out = session
        .probe
        .run(python, &["-c", &with_dll_preamble(&script)]);
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
    let fixtures = roundtrip_fixtures(bundle);
    if fixtures.is_empty() {
        return Diagnostic::skip(ID, 5, "no roundtrip fixture declared");
    }

    // A packaged verification contract turns an adjacent golden from optional
    // source content into a digest-bound claim. Missing or modified declared
    // content must fail, rather than looking like a source bundle that never
    // opted into L5.
    let contract = match PluginVerification::load(&bundle.root) {
        Ok(contract) => contract,
        Err(error) => {
            return Diagnostic::fail(
                ID,
                5,
                format!("invalid packaged verification contract: {error}"),
                vec![format!(
                    "re-run `ost plugin package`; inspect {PLUGIN_VERIFICATION} if the failure persists"
                )],
            );
        }
    };

    let mut results = fixtures
        .into_iter()
        .map(|(fixture_rel, fixture)| {
            let diagnostic =
                level5_golden_fixture(bundle, session, contract.as_ref(), fixture_rel, &fixture);
            (fixture_rel, diagnostic)
        })
        .collect::<Vec<_>>();
    if results.len() == 1 {
        return results.pop().expect("one L5 result").1;
    }

    let total = results.len();
    let failures = results
        .iter()
        .filter(|(_, diagnostic)| diagnostic.status == crate::doctor::Status::Fail)
        .collect::<Vec<_>>();
    if !failures.is_empty() {
        let mut actions = Vec::new();
        for (_, diagnostic) in &failures {
            for action in &diagnostic.suggested_actions {
                if !actions.contains(action) {
                    actions.push(action.clone());
                }
            }
        }
        let observed = failures
            .iter()
            .map(|(fixture, diagnostic)| format!("'{fixture}': {}", diagnostic.observed))
            .collect::<Vec<_>>()
            .join("; ");
        return Diagnostic::fail(
            ID,
            5,
            format!(
                "{} of {total} roundtrip fixtures failed: {observed}",
                failures.len()
            ),
            actions,
        );
    }

    let passed = results
        .iter()
        .filter(|(_, diagnostic)| diagnostic.status == crate::doctor::Status::Pass)
        .count();
    let skipped = total - passed;
    if passed > 0 {
        let detail = if skipped == 0 {
            format!("all {passed} roundtrip fixtures match their goldens")
        } else {
            format!(
                "{passed} roundtrip fixture(s) match their goldens; {skipped} fixture(s) have no golden"
            )
        };
        return Diagnostic::pass(ID, 5, detail);
    }

    let mut actions = Vec::new();
    for (_, diagnostic) in &results {
        for action in &diagnostic.suggested_actions {
            if !actions.contains(action) {
                actions.push(action.clone());
            }
        }
    }
    Diagnostic::skip_with_actions(
        ID,
        5,
        format!("none of the {total} roundtrip fixtures has a golden file"),
        actions,
    )
}

fn level5_golden_fixture(
    bundle: &Bundle,
    session: &Session,
    contract: Option<&PluginVerification>,
    fixture_rel: &str,
    fixture: &Utf8Path,
) -> Diagnostic {
    const ID: &str = "golden.roundtrip";
    let declared = contract.and_then(|contract| contract.oracle_for(fixture_rel));
    if let Some(entry) = declared {
        if let Err(error) = entry.verify(&bundle.root) {
            return Diagnostic::fail(
                ID,
                5,
                error.to_string(),
                vec!["re-run `ost plugin package` from the source bundle so its verification content is complete".into()],
            );
        }
    }

    // Golden convention: `<fixture>.golden.usda` sits next to the fixture — the
    // fixture *filename* is retained, so a `minimal.vrm` fixture pairs with
    // `minimal.vrm.golden.usda`, not `minimal.golden.usda`.
    let golden = declared
        .map(|entry| bundle.path(&entry.oracle))
        .unwrap_or_else(|| bundle.path(&adjacent_golden(fixture_rel)));
    if !golden.as_std_path().is_file() {
        // A bare "no golden file" leaves the author guessing the exact name, that
        // it must be the *flattened* stage, and how to produce it. Name all three.
        let golden_name = golden.file_name().unwrap_or(golden.as_str());
        let fixture_name = fixture.file_name().unwrap_or(fixture.as_str());
        let recipe = format!(
            "generate it: ost plugin run {} -- usdcat --flatten {} --out {}",
            bundle.root, fixture, golden,
        );
        return Diagnostic::skip_with_actions(
            ID,
            5,
            format!(
                "no golden file: expected '{golden_name}' next to the fixture — \
                 the flattened stage of '{fixture_name}'"
            ),
            vec![recipe],
        );
    }
    let Some(usdcat) = &session.usdcat else {
        return Diagnostic::fail(ID, 5, "usdcat not found in the runtime", vec![]);
    };

    let output = flatten_capture_path();
    let out = session
        .probe
        .run_to_file(usdcat, &["--flatten", fixture.as_str()], &output);
    if out.unspawned() {
        let _ = std::fs::remove_file(output.as_std_path());
        return Diagnostic::fail(ID, 5, format!("could not run usdcat ({usdcat})"), vec![]);
    }
    if !out.ok() {
        let _ = std::fs::remove_file(output.as_std_path());
        return Diagnostic::fail(
            ID,
            5,
            format!("usdcat --flatten failed: {}", tail(&out.stderr)),
            vec![],
        );
    }
    let flattened = match std::fs::read(output.as_std_path()) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(error) => {
            let _ = std::fs::remove_file(output.as_std_path());
            return Diagnostic::fail(
                ID,
                5,
                format!("could not read usdcat --out result: {error}"),
                vec![],
            );
        }
    };
    let _ = std::fs::remove_file(output.as_std_path());
    let expected = std::fs::read_to_string(golden.as_std_path()).unwrap_or_default();
    let actual = normalize(&flattened);
    let expected = normalize(&expected);
    if actual == expected {
        Diagnostic::pass(ID, 5, "flattened output matches the golden")
    } else {
        let crlf_only = normalize(&flattened.replace("\r\n", "\n"))
            == normalize(&expected.replace("\r\n", "\n"));
        let observed = if crlf_only {
            format!(
                "flattened output differs only by CRLF embedded in a USDA string value; {}",
                bounded_diff(&expected, &actual)
            )
        } else {
            format!(
                "flattened output differs from the golden; {}",
                bounded_diff(&expected, &actual)
            )
        };
        let mut actions =
            vec!["review the diff; update the golden if the change is intended".into()];
        if crlf_only {
            actions.push(
                "the flatten payload was captured through `usdcat --out`, so the CR is not stdout translation; inspect the authored fixture and generated values, pin checkout line endings when appropriate, and do not normalize semantic CRLF during comparison"
                    .into(),
            );
        }
        Diagnostic::fail(ID, 5, observed, actions)
    }
}

/// A unique temporary destination for `usdcat --out`. The process id keeps
/// independent OST invocations apart and the counter keeps concurrent L5
/// checks in one process apart. The caller removes the file on every path.
fn flatten_capture_path() -> Utf8PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let name = format!("openstrata-usdcat-{}-{sequence}.usda", std::process::id());
    Utf8PathBuf::from_path_buf(std::env::temp_dir().join(name))
        .unwrap_or_else(|path| Utf8PathBuf::from(path.to_string_lossy().into_owned()))
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

/// Normalize USDA text for comparison: trim trailing whitespace and normalize
/// physical line endings *outside string values*, ignore leading/trailing blank
/// lines, and canonicalize host-specific content so a golden is portable across
/// machines. Literal CRLF inside a USDA string is authored data and is retained.
///
/// `usdcat --flatten` stamps the *absolute* root-layer path into the flattened
/// stage's `doc` ("Generated from Composed Stage of root layer <path>"). That
/// path is the checkout location — `C:\dev\…` on one host, `D:\a\…` on a CI
/// runner — so a committed golden would never match anywhere but the machine
/// that produced it. Collapse that line to a path-free form on both sides.
fn normalize(s: &str) -> String {
    const FLATTEN_DOC: &str = "Generated from Composed Stage of root layer ";
    let mut portable = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(index) = rest.find(FLATTEN_DOC) {
        let keep = index + FLATTEN_DOC.len();
        portable.push_str(&rest[..keep]);
        rest = &rest[keep..];
        let end = rest.find(['\r', '\n']).unwrap_or(rest.len());
        // If the doc string closes on the same line as the path, keep the
        // closing quotes: dropping them would leave the string-aware layout
        // pass below inside an unterminated triple string for the rest of
        // the document.
        if rest[..end].trim_end().ends_with("\"\"\"") {
            portable.push_str("\"\"\"");
        }
        rest = &rest[end..];
    }
    portable.push_str(rest);
    normalize_usda_layout(&portable)
}

fn normalize_usda_layout(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    let mut in_string = false;
    let mut in_triple = false;
    let mut in_comment = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        let triple_quote = index + 2 < bytes.len()
            && bytes[index] == b'"'
            && bytes[index + 1] == b'"'
            && bytes[index + 2] == b'"';

        if in_triple && triple_quote && !escaped {
            out.extend_from_slice(b"\"\"\"");
            in_triple = false;
            index += 3;
            continue;
        }
        if !in_string && !in_triple && !in_comment && triple_quote {
            out.extend_from_slice(b"\"\"\"");
            in_triple = true;
            index += 3;
            continue;
        }
        if !in_triple && !in_comment && byte == b'"' && !escaped {
            in_string = !in_string;
            out.push(byte);
            index += 1;
            continue;
        }
        if !in_string && !in_triple && byte == b'#' {
            in_comment = true;
        }

        let newline = byte == b'\n' || (byte == b'\r' && bytes.get(index + 1) == Some(&b'\n'));
        if newline {
            if in_string || in_triple {
                if byte == b'\r' {
                    out.extend_from_slice(b"\r\n");
                    index += 2;
                } else {
                    out.push(b'\n');
                    index += 1;
                }
            } else {
                while matches!(out.last(), Some(b' ' | b'\t')) {
                    out.pop();
                }
                out.push(b'\n');
                index += if byte == b'\r' { 2 } else { 1 };
                in_comment = false;
            }
            escaped = false;
            continue;
        }

        out.push(byte);
        escaped = (in_string || in_triple) && byte == b'\\' && !escaped;
        if byte != b'\\' {
            escaped = false;
        }
        index += 1;
    }
    String::from_utf8(out)
        .expect("normalization preserves UTF-8")
        .trim_matches([' ', '\t', '\r', '\n'])
        .to_string()
}

/// A bounded, JSON-safe first-difference summary. Reports carry this in the
/// diagnostic's `observed` field instead of embedding an unbounded golden.
fn bounded_diff(expected: &str, actual: &str) -> String {
    const LIMIT: usize = 180;
    let expected_lines = expected.split('\n').collect::<Vec<_>>();
    let actual_lines = actual.split('\n').collect::<Vec<_>>();
    let line = (0..expected_lines.len().max(actual_lines.len()))
        .find(|index| expected_lines.get(*index) != actual_lines.get(*index))
        .unwrap_or(0);
    let clip = |value: Option<&&str>| {
        let value = value.copied().unwrap_or("<missing>");
        let mut clipped = value.chars().take(LIMIT).collect::<String>();
        if value.chars().count() > LIMIT {
            clipped.push('…');
        }
        format!("{clipped:?}")
    };
    format!(
        "first difference at line {}: expected {}, actual {}",
        line + 1,
        clip(expected_lines.get(line)),
        clip(actual_lines.get(line))
    )
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
        fn run(&self, program: &str, args: &[&str]) -> ToolOutput {
            self.calls
                .borrow_mut()
                .push(format!("{program} {}", args.join(" ")));
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

    fn resolver_bundle_with_fixture() -> (tempdir_like::Dir, Bundle) {
        let dir = tempdir_like::Dir::new("levels-resolver");
        std::fs::create_dir_all(dir.path.join("tests/fixtures").as_std_path()).unwrap();
        std::fs::create_dir_all(dir.path.join("plugin/resources/assets").as_std_path()).unwrap();
        std::fs::write(
            dir.path.join("tests/fixtures/basic.usda").as_std_path(),
            "#usda 1.0\n",
        )
        .unwrap();
        std::fs::write(
            dir.path
                .join("plugin/resources/assets/plugInfo.json")
                .as_std_path(),
            r#"{
  "Plugins": [{
    "Info": {"Types": {"AssetsResolver": {
      "bases": ["ArResolver"], "uriSchemes": ["assets"]
    }}}
  }]
}"#,
        )
        .unwrap();
        let manifest = PluginManifest::parse(
            r#"
plugin: { name: assets, version: 0.1.0, kind: usd-asset-resolver }
runtime: { openusd: ">=25.05,<27.0" }
provides: [usd-asset-resolver]
usd: { plug_info: plugin/resources/assets/plugInfo.json }
tests: { smoke: [tests/fixtures/basic.usda] }
"#,
        )
        .unwrap();
        let bundle = Bundle {
            root: dir.path.clone(),
            manifest,
        };
        (dir, bundle)
    }

    fn package_resolver_bundle_with_fixture() -> (tempdir_like::Dir, Bundle) {
        let dir = tempdir_like::Dir::new("levels-pkg-resolver");
        std::fs::create_dir_all(dir.path.join("tests/fixtures").as_std_path()).unwrap();
        std::fs::create_dir_all(dir.path.join("plugin/resources/shot-pack").as_std_path()).unwrap();
        std::fs::write(
            dir.path.join("tests/fixtures/basic.usda").as_std_path(),
            "#usda 1.0\n(\n    subLayers = [@./basic.pack[content/inner.usda]@]\n)\n",
        )
        .unwrap();
        std::fs::write(
            dir.path
                .join("plugin/resources/shot-pack/plugInfo.json")
                .as_std_path(),
            r#"{
  "Plugins": [{
    "Name": "ShotPackPackageResolver",
    "Info": {"Types": {"ShotPackPackageResolver": {
      "bases": ["ArPackageResolver"], "extensions": ["pack"]
    }}}
  }]
}"#,
        )
        .unwrap();
        let manifest = PluginManifest::parse(
            r#"
plugin: { name: shot-pack, version: 0.1.0, kind: usd-package-resolver }
runtime: { openusd: ">=25.05,<27.0" }
provides: ["usd-package-resolver:pack"]
usd: { plug_info: plugin/resources/shot-pack/plugInfo.json }
tests: { smoke: [tests/fixtures/basic.usda] }
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
    fn package_resolver_registration_discovers_and_loads_the_plugin() {
        let (_d, bundle) = package_resolver_bundle_with_fixture();
        assert_eq!(
            package_resolver_registrations(&bundle),
            vec![(
                "ShotPackPackageResolver".to_string(),
                vec!["pack".to_string()]
            )]
        );
        let probe = FakeProbe::new().on("python", Some(0), "", "");
        let session = Session {
            probe: &probe,
            usdcat: None,
            python: Some("python".into()),
            usdview: None,
            has_display: false,
        };
        let diagnostic = &run_levels(&bundle, &session, 2)[0];
        assert_eq!(diagnostic.id, "package_resolver.registration");
        assert_eq!(diagnostic.status, Status::Pass);
    }

    #[test]
    fn package_resolver_registration_fails_without_extension_metadata() {
        let (_d, bundle) = package_resolver_bundle_with_fixture();
        let plug_info = bundle.plug_info();
        let source = std::fs::read_to_string(plug_info.as_std_path()).unwrap();
        std::fs::write(
            plug_info.as_std_path(),
            source.replace(", \"extensions\": [\"pack\"]", ""),
        )
        .unwrap();
        let probe = FakeProbe::new();
        let session = Session {
            probe: &probe,
            usdcat: None,
            python: Some("python".into()),
            usdview: None,
            has_display: false,
        };
        let diagnostic = &run_levels(&bundle, &session, 2)[0];
        assert_eq!(diagnostic.id, "package_resolver.registration");
        assert_eq!(diagnostic.status, Status::Fail);
    }

    #[test]
    fn package_resolver_runs_the_shared_read_levels() {
        // Dispatch is proven by the shared L3/L4 levels reading the smoke
        // fixture (it sublayers a packaged path), so they must run for this
        // kind rather than the schema replacements.
        let (_d, bundle) = package_resolver_bundle_with_fixture();
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
        assert_eq!(
            ids,
            vec![
                "package_resolver.registration",
                "usdcat.read",
                "python.stage_open"
            ]
        );
    }

    #[test]
    fn resolver_registration_dispatches_a_declared_scheme() {
        let (_d, bundle) = resolver_bundle_with_fixture();
        assert_eq!(resolver_uri_schemes(&bundle), vec!["assets"]);
        let probe = FakeProbe::new().on("python", Some(0), "", "");
        let session = Session {
            probe: &probe,
            usdcat: None,
            python: Some("python".into()),
            usdview: None,
            has_display: false,
        };
        let diagnostic = &run_levels(&bundle, &session, 2)[0];
        assert_eq!(diagnostic.id, "resolver.registration");
        assert_eq!(diagnostic.status, Status::Pass);
    }

    #[test]
    fn resolver_registration_fails_without_uri_scheme_metadata() {
        let (_d, bundle) = resolver_bundle_with_fixture();
        let plug_info = bundle.plug_info();
        let source = std::fs::read_to_string(plug_info.as_std_path()).unwrap();
        std::fs::write(
            plug_info.as_std_path(),
            source.replace(", \"uriSchemes\": [\"assets\"]", ""),
        )
        .unwrap();
        let probe = FakeProbe::new();
        let session = Session {
            probe: &probe,
            usdcat: None,
            python: Some("python".into()),
            usdview: None,
            has_display: false,
        };
        let diagnostic = &run_levels(&bundle, &session, 2)[0];
        assert_eq!(diagnostic.id, "resolver.registration");
        assert_eq!(diagnostic.status, Status::Fail);
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
        // The skip must name the exact expected file (fixture filename retained),
        // say it is the flattened stage, and carry the generation recipe.
        assert!(
            golden.observed.contains("basic.toy.golden.usda"),
            "skip should name the concrete golden file: {}",
            golden.observed
        );
        assert!(
            golden.observed.contains("flattened"),
            "skip should say the golden is the flattened stage: {}",
            golden.observed
        );
        let recipe = golden.suggested_actions.join("\n");
        assert!(
            recipe.contains("usdcat --flatten") && recipe.contains("--out"),
            "skip should print the generation recipe: {recipe}"
        );
    }

    #[test]
    fn golden_uses_file_transport_for_multiline_usda_values() {
        let (_d, bundle) = bundle_with_fixture();
        let flattened = "#usda 1.0\n(\n    comment = \"\"\"authored\nvalue\"\"\"\n    doc = \"\"\"Generated from Composed Stage of root layer C:\\work\\basic.toy\ngenerated value\"\"\"\n)\n";
        let golden = bundle.path("tests/fixtures/basic.toy.golden.usda");
        std::fs::write(golden.as_std_path(), flattened).unwrap();
        let probe = FakeProbe::new().on("usdcat", Some(0), flattened, "");
        let session = Session {
            probe: &probe,
            usdcat: Some("usdcat".into()),
            python: None,
            usdview: None,
            has_display: false,
        };

        let diagnostic = level5_golden(&bundle, &session);

        assert_eq!(diagnostic.status, Status::Pass);
        let calls = probe.calls.borrow().join("\n");
        assert!(calls.contains("--flatten"), "{calls}");
        assert!(calls.contains("--out"), "L5 must bypass stdout: {calls}");
    }

    #[test]
    fn packaged_golden_passes_from_an_extracted_tree_without_source_paths() {
        let (_source_dir, mut source) = bundle_with_fixture();
        source
            .manifest
            .tests
            .roundtrip
            .push("tests/fixtures/basic.toy".into());
        let flattened = "#usda 1.0\ndef Xform \"Root\" {}\n";
        std::fs::write(
            source
                .path("tests/fixtures/basic.toy.golden.usda")
                .as_std_path(),
            flattened,
        )
        .unwrap();
        let contract = PluginVerification::from_bundle(&source).unwrap();

        let extracted_dir = tempdir_like::Dir::new("levels-packaged-golden");
        std::fs::create_dir_all(extracted_dir.path.join("tests/fixtures").as_std_path()).unwrap();
        for relative in [
            "tests/fixtures/basic.toy",
            "tests/fixtures/basic.toy.golden.usda",
        ] {
            std::fs::copy(
                source.path(relative).as_std_path(),
                extracted_dir.path.join(relative).as_std_path(),
            )
            .unwrap();
        }
        std::fs::write(
            extracted_dir.path.join(PLUGIN_VERIFICATION).as_std_path(),
            serde_json::to_vec_pretty(&contract).unwrap(),
        )
        .unwrap();
        let extracted = Bundle {
            root: extracted_dir.path.clone(),
            manifest: source.manifest.clone(),
        };
        let probe = FakeProbe::new().on("usdcat", Some(0), flattened, "");
        let session = Session {
            probe: &probe,
            usdcat: Some("usdcat".into()),
            python: None,
            usdview: None,
            has_display: false,
        };

        let diagnostic = level5_golden(&extracted, &session);

        assert_eq!(diagnostic.status, Status::Pass, "{diagnostic:?}");
        let calls = probe.calls.borrow().join("\n");
        assert!(calls.contains(extracted.root.as_str()), "{calls}");
        assert!(!calls.contains(source.root.as_str()), "{calls}");
    }

    #[test]
    fn packaged_golden_claim_fails_when_the_oracle_was_omitted() {
        let (_dir, mut bundle) = bundle_with_fixture();
        bundle
            .manifest
            .tests
            .roundtrip
            .push("tests/fixtures/basic.toy".into());
        let oracle = bundle.path("tests/fixtures/basic.toy.golden.usda");
        std::fs::write(oracle.as_std_path(), "#usda 1.0\n").unwrap();
        let contract = PluginVerification::from_bundle(&bundle).unwrap();
        std::fs::write(
            bundle.root.join(PLUGIN_VERIFICATION).as_std_path(),
            serde_json::to_vec_pretty(&contract).unwrap(),
        )
        .unwrap();
        std::fs::remove_file(oracle.as_std_path()).unwrap();
        let probe = FakeProbe::new().on("usdcat", Some(0), "#usda 1.0\n", "");
        let session = Session {
            probe: &probe,
            usdcat: Some("usdcat".into()),
            python: None,
            usdview: None,
            has_display: false,
        };

        let diagnostic = level5_golden(&bundle, &session);

        assert_eq!(diagnostic.status, Status::Fail);
        assert!(diagnostic.observed.contains("oracle"));
        assert!(diagnostic.observed.contains("missing"));
        assert!(probe.calls.borrow().is_empty(), "usdcat must not run");
    }

    #[test]
    fn packaged_golden_verifies_every_declared_roundtrip_fixture() {
        let (_dir, mut bundle) = bundle_with_fixture();
        let first = "tests/fixtures/basic.toy";
        let second = "tests/fixtures/secondary.toy";
        bundle.manifest.tests.roundtrip = vec![first.into(), second.into()];
        std::fs::write(bundle.path(second).as_std_path(), "toy 2.0\n").unwrap();
        for fixture in [first, second] {
            std::fs::write(
                bundle.path(&adjacent_golden(fixture)).as_std_path(),
                "#usda 1.0\n",
            )
            .unwrap();
        }
        let contract = PluginVerification::from_bundle(&bundle).unwrap();
        assert_eq!(contract.roundtrip.len(), 2);
        std::fs::write(
            bundle.root.join(PLUGIN_VERIFICATION).as_std_path(),
            serde_json::to_vec_pretty(&contract).unwrap(),
        )
        .unwrap();
        std::fs::remove_file(bundle.path(&adjacent_golden(second)).as_std_path()).unwrap();
        let probe = FakeProbe::new().on("usdcat", Some(0), "#usda 1.0\n", "");
        let session = Session {
            probe: &probe,
            usdcat: Some("usdcat".into()),
            python: None,
            usdview: None,
            has_display: false,
        };

        let diagnostic = level5_golden(&bundle, &session);

        assert_eq!(diagnostic.status, Status::Fail, "{diagnostic:?}");
        assert!(diagnostic.observed.contains(second), "{diagnostic:?}");
        assert!(diagnostic.observed.contains("missing"), "{diagnostic:?}");
        assert_eq!(
            probe.calls.borrow().len(),
            1,
            "the first fixture should run before the later contract failure"
        );
    }

    #[test]
    fn golden_file_transport_still_preserves_authored_cr() {
        let (_d, bundle) = bundle_with_fixture();
        let golden_text = "#usda 1.0\n(\n    comment = \"\"\"authored\nvalue\"\"\"\n)\n";
        let actual = golden_text.replacen("authored\nvalue", "authored\r\nvalue", 1);
        let golden = bundle.path("tests/fixtures/basic.toy.golden.usda");
        std::fs::write(golden.as_std_path(), golden_text).unwrap();
        let probe = FakeProbe::new().on("usdcat", Some(0), &actual, "");
        let session = Session {
            probe: &probe,
            usdcat: Some("usdcat".into()),
            python: None,
            usdview: None,
            has_display: false,
        };

        let diagnostic = level5_golden(&bundle, &session);

        assert_eq!(diagnostic.status, Status::Fail);
        assert!(diagnostic.observed.contains("CRLF embedded"));
        assert!(
            diagnostic
                .suggested_actions
                .iter()
                .any(|action| action.contains("not stdout translation")),
            "{:?}",
            diagnostic.suggested_actions
        );
    }

    #[test]
    fn normalize_makes_flatten_doc_host_independent() {
        // Same flattened stage produced on two hosts differs only in the
        // absolute root-layer path usdcat stamps into `doc`.
        let on_windows = "#usda 1.0\n(\n    doc = \"\"\"Generated from Composed Stage of root layer C:\\dev\\_ost_runner_test\\plugins\\toy\\tests\\fixtures\\basic.toy\n\"\"\"\n)\n";
        let on_runner = "#usda 1.0\n(\n    doc = \"\"\"Generated from Composed Stage of root layer D:\\a\\_ost_runner_test\\_ost_runner_test\\plugins\\toy\\tests\\fixtures\\basic.toy\n\"\"\"\n)\n";
        assert_eq!(normalize(on_windows), normalize(on_runner));
        // The path is dropped, but the surrounding structure is preserved.
        assert!(normalize(on_windows).contains("Generated from Composed Stage of root layer"));
        assert!(!normalize(on_windows).contains("basic.toy"));
    }

    #[test]
    fn normalize_keeps_layout_portable_after_a_single_line_flatten_doc() {
        // A doc string that closes on the same line as the stamped path must
        // not leave the layout pass inside an unterminated triple string —
        // physical line endings after it would silently stop normalizing.
        let lf = "#usda 1.0\n(\n    doc = \"\"\"Generated from Composed Stage of root layer C:\\dev\\x\\basic.toy\"\"\"\n)\ndef X \"Root\"\n{\n}\n";
        let crlf = lf.replace('\n', "\r\n");
        assert_eq!(normalize(lf), normalize(&crlf));
        assert!(!normalize(lf).contains("basic.toy"));
        assert!(normalize(lf).contains("\"\"\""));
    }

    #[test]
    fn normalize_does_not_close_a_triple_string_on_an_escaped_quote() {
        // `\"""` inside a triple string is an escaped quote followed by two
        // literal quotes, not a terminator; the CRLF after it is still
        // authored string data.
        let text = "#usda 1.0\nx = \"\"\"a\\\"\"\"b\r\nc\"\"\"\n";
        assert!(normalize(text).contains("b\r\nc"), "{:?}", normalize(text));
    }

    #[test]
    fn normalize_preserves_semantic_crlf_inside_usda_strings() {
        let lf_file = "#usda 1.0\n(\n)\ndef X \"Root\"\n{\n}\n";
        let crlf_file = lf_file.replace('\n', "\r\n");
        assert_eq!(
            normalize(lf_file),
            normalize(&crlf_file),
            "physical file line endings outside strings are portable"
        );

        let authored_lf = "#usda 1.0\n(\n    customData = { string note = \"\"\"a\nb\"\"\" }\n)\n";
        let authored_crlf =
            "#usda 1.0\n(\n    customData = { string note = \"\"\"a\r\nb\"\"\" }\n)\n";
        assert_ne!(
            normalize(authored_lf),
            normalize(authored_crlf),
            "CRLF inside a multiline USDA string is authored data"
        );
        assert_eq!(
            normalize(&authored_lf.replace("\r\n", "\n")),
            normalize(&authored_crlf.replace("\r\n", "\n")),
            "the mismatch is identifiable as CRLF-only for the actionable hint"
        );
        let diff = bounded_diff(&normalize(authored_lf), &normalize(authored_crlf));
        assert!(diff.contains("first difference at line"), "{diff}");
        assert!(diff.len() < 512, "bounded diff must stay report-sized");
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
