// SPDX-License-Identifier: Apache-2.0
//! `ost plugin` — OpenUSD plugin bundles.
//!
//! - `new`     scaffold a bundle from a template.
//! - `inspect` Level 0 bundle structure report (no runtime needed).
//! - `build`   build the shared library + stage plugInfo, reusing `ost-build`'s
//!   toolchain generation against the resolved runtime.
//! - `doctor`  static diagnostics (L0–L1) + a preview of the session env it
//!   *would* set; L2+ SKIP (run them with `test`).
//! - `run`     compose the runtime session and exec a command in it (needs a
//!   real runtime).
//! - `test`    orchestrate the verification pyramid L0..L6 — executing the
//!   runtime's tools for L2+ — and write a report under `.strata/reports/`.
//! - `view`    open a fixture in usdview inside the session (interactive, L6).
//! - `test-view` non-interactive usdview launch probe (L6) + report.
//!
//! The CLI stays thin: it resolves paths and the runtime, then calls into
//! `ost-plugin` for the model, checks, execution levels, and report shapes.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};
use clap::Subcommand;

use ost_core::host::Os;
use ost_core::paths::{find_project_root, STATE_DIR};
use ost_core::{tools, Error, Host, Result};
use ost_plugin::{
    diagnose, run_levels, scaffold, session_env, usdview_check, Bundle, DoctorReport, PluginKind,
    Probe, RuntimeContext, Session, Status, ToolOutput,
};
use ost_runtime::{EnvSet, RuntimeManifest, MANIFEST_FILE};

use crate::commands::compiler::{self, CompilerOpts};
use crate::commands::configure::{build_target, load_project};
use crate::commands::resolve;
use crate::output::{self, Format};

#[derive(Debug, Subcommand)]
pub enum PluginCmd {
    /// Scaffold a new plugin bundle from a template.
    New {
        /// Plugin kind: usd-fileformat | usd-asset-resolver | usd-schema.
        kind: String,
        /// Plugin name (becomes the bundle directory), e.g. `toy`.
        name: String,
        /// File extension the plugin reads (required for usd-fileformat).
        #[arg(long)]
        extension: Option<String>,
        /// Destination directory. Defaults to ./<name>.
        #[arg(long)]
        dir: Option<String>,
    },
    /// Report a bundle's Level 0 structure.
    Inspect {
        /// Path to the bundle directory (containing openstrata.plugin.yaml).
        bundle: String,
    },
    /// Build the plugin's shared library against the resolved runtime.
    Build {
        /// Path to the bundle directory.
        bundle: String,
        /// Platform target, e.g. `cy2026`. Defaults to the enclosing project's.
        #[arg(long)]
        target: Option<String>,
        /// Profile to build against. Defaults to the enclosing project's.
        #[arg(long)]
        profile: Option<String>,
        /// Print the commands that would run, without executing them.
        #[arg(long)]
        dry_run: bool,
        /// Path to the ninja executable if it is not on PATH.
        #[arg(long)]
        ninja: Option<String>,
        #[command(flatten)]
        compiler: CompilerOpts,
    },
    /// Run staged diagnostics (L0–L1) and write a report.
    Doctor {
        /// Path to the bundle directory.
        bundle: String,
        /// Platform target, e.g. `cy2026`. Defaults to the enclosing project's.
        #[arg(long)]
        target: Option<String>,
        /// Profile to check against. Defaults to the enclosing project's.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Launch a command inside the plugin's runtime session (needs a real runtime).
    Run {
        /// Path to the bundle directory.
        bundle: String,
        /// Platform target, e.g. `cy2026`. Defaults to the enclosing project's.
        #[arg(long)]
        target: Option<String>,
        /// Profile to activate. Defaults to the enclosing project's.
        #[arg(long)]
        profile: Option<String>,
        /// Command to execute after `--`, e.g. `-- usdcat fixture.toy`.
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// Orchestrate the verification pyramid (L0..L6) and write a report.
    Test {
        /// Path to the bundle directory.
        bundle: String,
        /// Platform target, e.g. `cy2026`. Defaults to the enclosing project's.
        #[arg(long)]
        target: Option<String>,
        /// Profile to test against. Defaults to the enclosing project's.
        #[arg(long)]
        profile: Option<String>,
        /// Highest verification level to run (0..=6). Default 5; 6 adds usdview.
        #[arg(long, default_value_t = 5)]
        up_to: u8,
    },
    /// Open a fixture in usdview inside the plugin's runtime session (Level 6).
    View {
        /// Path to the bundle directory.
        bundle: String,
        /// Fixture to open (relative to the bundle, or an absolute path).
        fixture: String,
        /// Platform target, e.g. `cy2026`. Defaults to the enclosing project's.
        #[arg(long)]
        target: Option<String>,
        /// Profile to activate. Defaults to the enclosing project's.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Verify usdview launches on a fixture (Level 6) and write a report.
    TestView {
        /// Path to the bundle directory.
        bundle: String,
        /// Fixture to open (relative to the bundle, or an absolute path).
        fixture: String,
        /// Platform target, e.g. `cy2026`. Defaults to the enclosing project's.
        #[arg(long)]
        target: Option<String>,
        /// Profile to test against. Defaults to the enclosing project's.
        #[arg(long)]
        profile: Option<String>,
    },
}

pub fn run(cmd: PluginCmd, fmt: Format) -> Result<()> {
    match cmd {
        PluginCmd::New {
            kind,
            name,
            extension,
            dir,
        } => new(&kind, &name, extension.as_deref(), dir.as_deref(), fmt),
        PluginCmd::Inspect { bundle } => inspect(&bundle, fmt),
        PluginCmd::Build {
            bundle,
            target,
            profile,
            dry_run,
            ninja,
            compiler,
        } => build(&bundle, target, profile, dry_run, ninja, compiler, fmt),
        PluginCmd::Doctor {
            bundle,
            target,
            profile,
        } => doctor(&bundle, target, profile, fmt),
        PluginCmd::Run {
            bundle,
            target,
            profile,
            command,
        } => run_session(&bundle, target, profile, command, fmt),
        PluginCmd::Test {
            bundle,
            target,
            profile,
            up_to,
        } => test(&bundle, target, profile, up_to, fmt),
        PluginCmd::View {
            bundle,
            fixture,
            target,
            profile,
        } => view(&bundle, &fixture, target, profile),
        PluginCmd::TestView {
            bundle,
            fixture,
            target,
            profile,
        } => test_view(&bundle, &fixture, target, profile, fmt),
    }
}

fn new(
    kind: &str,
    name: &str,
    extension: Option<&str>,
    dir: Option<&str>,
    fmt: Format,
) -> Result<()> {
    let kind = PluginKind::from_tag(kind).ok_or_else(|| {
        let kinds: Vec<&str> = PluginKind::ALL.iter().map(|k| k.as_str()).collect();
        Error::Operation(format!(
            "unknown plugin kind '{kind}' (expected one of: {})",
            kinds.join(", ")
        ))
    })?;

    let dest = Utf8PathBuf::from(dir.unwrap_or(name));
    let files = scaffold(kind, name, extension, &dest)?;

    if fmt.is_json() {
        output::json(&serde_json::json!({
            "created": true,
            "kind": kind.as_str(),
            "name": name,
            "dir": dest.to_string(),
            "files": files.iter().map(|f| f.to_string()).collect::<Vec<_>>(),
        }));
        return Ok(());
    }

    println!("Created {} plugin '{name}' in {dest}/", kind.as_str());
    for f in &files {
        println!("  {f}");
    }
    println!("\nNext:");
    println!("  ost plugin inspect {dest}");
    println!("  ost plugin doctor {dest}");
    Ok(())
}

fn inspect(bundle_path: &str, fmt: Format) -> Result<()> {
    let bundle = load_bundle(bundle_path)?;
    // Level 0 only: bundle structure, no runtime resolution.
    let report = diagnose(&bundle, &RuntimeContext::default(), 0);

    if fmt.is_json() {
        output::json(&ost_plugin::report_json(&bundle, &report));
    } else {
        print_report(&bundle, &report);
    }
    finish(&report)
}

fn doctor(
    bundle_path: &str,
    target: Option<String>,
    profile: Option<String>,
    fmt: Format,
) -> Result<()> {
    let bundle = load_bundle(bundle_path)?;
    let host = Host::detect();

    // Resolve the runtime if we can (enclosing project or explicit flags). When
    // we can't, Level 1 honestly SKIPs rather than guessing.
    let resolved = resolve_runtime(target, profile)?;
    let ctx = resolved.as_ref().map(runtime_context).unwrap_or_default();

    // Compose the session env we *would* set (runtime env + bundle roots).
    let session = match &resolved {
        Some(r) => session_env(&r.env, &bundle, host.os),
        None => EnvSet {
            sep: if host.os == Os::Windows { ';' } else { ':' },
            vars: ost_plugin::bundle_vars(&bundle, host.os),
        },
    };

    // Levels 0–1 run; 2+ are emitted as SKIP (need a real runtime).
    let report = diagnose(&bundle, &ctx, 5);

    // Write the report under the bundle's .strata/reports/.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let reports_root = bundle.root.join(STATE_DIR).join("reports");
    let report_dir = ost_plugin::write_report(&reports_root, &bundle, &report, &session, now)?;

    if fmt.is_json() {
        let mut body = ost_plugin::report_json(&bundle, &report);
        if let Some(obj) = body.as_object_mut() {
            obj.insert(
                "report_dir".into(),
                serde_json::Value::String(report_dir.to_string()),
            );
            obj.insert("environment".into(), ost_plugin::environment_json(&session));
        }
        output::json(&body);
    } else {
        print_report(&bundle, &report);
        println!("\nSession env preview (PXR_PLUGINPATH_NAME / lib / PYTHONPATH):");
        for (k, v) in session.pairs() {
            println!("  {k} += {v}");
        }
        println!("\nReport: {report_dir}");
    }
    finish(&report)
}

#[allow(clippy::too_many_arguments)]
fn build(
    bundle_path: &str,
    target: Option<String>,
    profile: Option<String>,
    dry_run: bool,
    ninja: Option<String>,
    compiler_opts: CompilerOpts,
    fmt: Format,
) -> Result<()> {
    let bundle = load_bundle(bundle_path)?;

    // A build needs a concrete runtime to compile against.
    let (platform, profile) = selection(target, profile).ok_or_else(|| {
        Error::Operation(
            "no platform/profile: run inside an OpenStrata project or pass --target/--profile"
                .into(),
        )
    })?;
    let (tgt, r) = build_target(&platform, &profile)?;
    let id = tgt.id();

    // Compiler policy: CLI flags over the enclosing project's `[build]`, else host.
    let compiler = resolve_plugin_compiler(&compiler_opts)?;

    // Generate the toolchain that points CMake at the runtime (reusing ost-build).
    // Both the toolchain and the build tree are keyed by target id, so switching
    // platform/profile/runtime never reuses (and corrupts) another target's
    // CMake cache — mirroring the project-level `build/<id>` layout.
    let target_dir = target_state_dir(&bundle.root, &id);
    std::fs::create_dir_all(target_dir.as_std_path())
        .map_err(|e| Error::io(target_dir.to_string(), e))?;
    let toolchain = target_dir.join("toolchain.cmake");
    std::fs::write(
        toolchain.as_std_path(),
        format!(
            "{}\n",
            ost_build::render_toolchain(&tgt, &r.artifact_prefix, &compiler)
        ),
    )
    .map_err(|e| Error::io(toolchain.to_string(), e))?;

    let build_dir = target_build_dir(&bundle.root, &id);
    let cmake = tools::which("cmake");
    let ninja = ninja.map(PathBuf::from).or_else(|| tools::which("ninja"));

    let toolchain_arg = toolchain.to_string().replace('\\', "/");
    let mut configure_args = vec![
        "-S".to_string(),
        bundle.root.to_string().replace('\\', "/"),
        "-B".to_string(),
        build_dir.to_string().replace('\\', "/"),
        "-G".to_string(),
        "Ninja".to_string(),
        format!("-DCMAKE_TOOLCHAIN_FILE={toolchain_arg}"),
    ];
    if let Some(n) = &ninja {
        configure_args.push(format!(
            "-DCMAKE_MAKE_PROGRAM={}",
            n.display().to_string().replace('\\', "/")
        ));
    }
    let build_args = vec![
        "--build".to_string(),
        build_dir.to_string().replace('\\', "/"),
    ];

    if dry_run {
        println!("# dry run — would generate {toolchain} then:");
        println!("cmake {}", configure_args.join(" "));
        println!("cmake {}", build_args.join(" "));
        return Ok(());
    }

    if !r.pulled {
        return Err(Error::Operation(format!(
            "runtime '{}' not pulled — run `ost runtime pull {platform} --profile {profile}` first",
            tgt.runtime_id
        )));
    }
    let cmake = cmake.ok_or_else(|| Error::Operation("`cmake` not found on PATH".into()))?;

    run_step(&cmake, &configure_args)?;
    run_step(&cmake, &build_args)?;

    // plugInfo.json is shipped in the bundle (staged at scaffold time); confirm it.
    let plug_info = bundle.plug_info();
    if fmt.is_json() {
        output::json(&serde_json::json!({
            "built": true,
            "plugin": bundle.manifest.plugin.name,
            "runtime": tgt.runtime_id,
            "build_dir": build_dir.to_string(),
            "lib_dir": bundle.lib_dir().to_string(),
            "plug_info": plug_info.to_string(),
        }));
        return Ok(());
    }
    println!(
        "\nBuilt {} against {}",
        bundle.manifest.plugin.name, tgt.runtime_id
    );
    println!("  lib:       {}", bundle.lib_dir());
    println!("  plugInfo:  {plug_info}");
    Ok(())
}

/// `ost plugin run` — compose the runtime session and exec a command in it.
fn run_session(
    bundle_path: &str,
    target: Option<String>,
    profile: Option<String>,
    command: Vec<String>,
    _fmt: Format,
) -> Result<()> {
    let bundle = load_bundle(bundle_path)?;
    let host = Host::detect();
    let r = require_real_runtime(target, profile)?;

    let session = session_env(&r.env, &bundle, host.os);
    let (prog, rest) = command.split_first().expect("clap requires >=1 arg");

    let mut cmd = Command::new(prog);
    cmd.args(rest);
    session.apply(&mut cmd); // overlay the resolved session env, no global mutation
    let status = cmd
        .status()
        .map_err(|e| Error::io(format!("run {prog}"), e))?;
    // Propagate the child's exit code for CI.
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// `ost plugin test` — run the verification pyramid L0..=`up_to` and write a report.
fn test(
    bundle_path: &str,
    target: Option<String>,
    profile: Option<String>,
    up_to: u8,
    fmt: Format,
) -> Result<()> {
    let bundle = load_bundle(bundle_path)?;
    let host = Host::detect();

    let resolved = resolve_runtime(target, profile)?;
    let ctx = resolved.as_ref().map(runtime_context).unwrap_or_default();
    let session = match &resolved {
        Some(r) => session_env(&r.env, &bundle, host.os),
        None => EnvSet {
            sep: if host.os == Os::Windows { ';' } else { ':' },
            vars: ost_plugin::bundle_vars(&bundle, host.os),
        },
    };

    // L0 + L1 are static. L2..up_to execute the runtime's tools — but only when a
    // real runtime is present; otherwise keep the honest SKIPs.
    let mut report = diagnose(&bundle, &ctx, 1);
    if up_to >= 2 {
        if ctx.real {
            let probe = ProcessProbe::new(session.resolve());
            let tools = locate_tools(resolved.as_ref(), &probe);
            let sess = Session {
                probe: &probe,
                usdcat: tools.usdcat,
                python: tools.python,
                usdview: tools.usdview,
                has_display: has_display(host.os),
            };
            report
                .diagnostics
                .extend(run_levels(&bundle, &sess, up_to.min(6)));
        } else {
            // Reuse diagnose's SKIP placeholders for the execution levels.
            let skips = diagnose(&bundle, &ctx, up_to.min(5))
                .diagnostics
                .into_iter()
                .filter(|d| d.level >= 2);
            report.diagnostics.extend(skips);
        }
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let reports_root = bundle.root.join(STATE_DIR).join("reports");
    let report_dir = ost_plugin::write_report(&reports_root, &bundle, &report, &session, now)?;

    if fmt.is_json() {
        let mut body = ost_plugin::report_json(&bundle, &report);
        if let Some(obj) = body.as_object_mut() {
            obj.insert(
                "report_dir".into(),
                serde_json::Value::String(report_dir.to_string()),
            );
        }
        output::json(&body);
    } else {
        print_report(&bundle, &report);
        println!("\nReport: {report_dir}");
    }
    finish(&report)
}

/// `ost plugin view` — open a fixture in usdview inside the runtime session.
fn view(
    bundle_path: &str,
    fixture: &str,
    target: Option<String>,
    profile: Option<String>,
) -> Result<()> {
    let bundle = load_bundle(bundle_path)?;
    let host = Host::detect();
    let r = require_real_runtime(target, profile)?;

    let usdview = locate_runtime_tool(Some(&r), &["usdview.cmd", "usdview.exe", "usdview"])
        .ok_or_else(|| {
            Error::Operation(
                "usdview not found in the runtime (build/adopt one with usdview enabled)".into(),
            )
        })?;
    let fixture_path = bundle.path(fixture); // absolute passes through; else under the bundle

    let session = session_env(&r.env, &bundle, host.os);
    let mut cmd = Command::new(&usdview);
    cmd.arg(fixture_path.as_str());
    session.apply(&mut cmd); // overlay the session env, no global mutation
    let status = cmd
        .status()
        .map_err(|e| Error::io(format!("run {usdview}"), e))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// `ost plugin test-view` — run the Level 6 usdview check on a fixture + report.
fn test_view(
    bundle_path: &str,
    fixture: &str,
    target: Option<String>,
    profile: Option<String>,
    fmt: Format,
) -> Result<()> {
    let bundle = load_bundle(bundle_path)?;
    let host = Host::detect();
    let r = require_real_runtime(target, profile)?;

    let session = session_env(&r.env, &bundle, host.os);
    let probe = ProcessProbe::new(session.resolve());
    let usdview = locate_runtime_tool(Some(&r), &["usdview.cmd", "usdview.exe", "usdview"]);
    let sess = Session {
        probe: &probe,
        usdcat: None,
        python: None,
        usdview,
        has_display: has_display(host.os),
    };

    let report = DoctorReport {
        diagnostics: vec![usdview_check(&bundle, &sess, Some(fixture))],
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let reports_root = bundle.root.join(STATE_DIR).join("reports");
    let report_dir = ost_plugin::write_report(&reports_root, &bundle, &report, &session, now)?;

    if fmt.is_json() {
        let mut body = ost_plugin::report_json(&bundle, &report);
        if let Some(obj) = body.as_object_mut() {
            obj.insert(
                "report_dir".into(),
                serde_json::Value::String(report_dir.to_string()),
            );
        }
        output::json(&body);
    } else {
        print_report(&bundle, &report);
        println!("\nReport: {report_dir}");
    }
    finish(&report)
}

/// A [`Probe`] that spawns real processes with the resolved session env applied
/// on top of the current environment (no global mutation).
struct ProcessProbe {
    env: Vec<(String, String)>,
}

impl ProcessProbe {
    fn new(env: Vec<(String, String)>) -> ProcessProbe {
        ProcessProbe { env }
    }
}

impl Probe for ProcessProbe {
    fn run(&self, program: &str, args: &[&str]) -> ToolOutput {
        let mut cmd = Command::new(program);
        cmd.args(args);
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        match cmd.output() {
            Ok(o) => ToolOutput {
                code: o.status.code(),
                stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            },
            Err(_) => ToolOutput {
                code: None,
                stdout: String::new(),
                stderr: format!("could not spawn {program}"),
            },
        }
    }
}

struct Tools {
    usdcat: Option<String>,
    python: Option<String>,
    usdview: Option<String>,
}

/// Find a runtime tool in `<artifact_prefix>/bin` by trying each candidate name.
fn locate_runtime_tool(
    resolved: Option<&crate::commands::Resolved>,
    names: &[&str],
) -> Option<String> {
    let r = resolved?;
    let bin = r.artifact_prefix.join("bin");
    names.iter().find_map(|name| {
        let p = bin.join(name);
        p.as_std_path().is_file().then(|| p.to_string())
    })
}

/// Locate the tools the execution levels need, using the session env: `usdcat`
/// and `usdview` from the runtime's `bin/`, and a python interpreter that can
/// import `pxr`.
fn locate_tools(resolved: Option<&crate::commands::Resolved>, probe: &ProcessProbe) -> Tools {
    let usdcat = locate_runtime_tool(resolved, &["usdcat", "usdcat.exe"]);
    // usdview is a wrapper: `usdview.cmd` on Windows, a bare `usdview` elsewhere.
    let usdview = locate_runtime_tool(resolved, &["usdview.cmd", "usdview.exe", "usdview"]);
    // Probe for a python interpreter on the session PATH (cheap `--version`).
    let python = ["python", "python3"]
        .into_iter()
        .find(|p| probe.run(p, &["--version"]).code.is_some())
        .map(str::to_string);
    Tools {
        usdcat,
        python,
        usdview,
    }
}

/// Whether a display is available for GUI tools (Level 6). Headless Linux/CI has
/// no `DISPLAY`/`WAYLAND_DISPLAY`; Windows and macOS always have one.
fn has_display(os: Os) -> bool {
    match os {
        Os::Linux => {
            std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
        }
        Os::Windows | Os::Macos => true,
    }
}

// ---- helpers ----

/// Load a bundle from a path, with an actionable error if it is not a bundle.
fn load_bundle(path: &str) -> Result<Bundle> {
    let root = Utf8PathBuf::from(path);
    Bundle::load(&root)
}

/// Determine platform+profile from explicit flags or the enclosing project.
/// Returns `None` when neither is available.
fn selection(target: Option<String>, profile: Option<String>) -> Option<(String, String)> {
    if let Some(t) = target {
        return Some((t, profile.unwrap_or_else(|| "core".to_string())));
    }
    let cwd = std::env::current_dir().ok()?;
    let root = find_project_root(&cwd)?;
    let root = Utf8PathBuf::from_path_buf(root).ok()?;
    let project = load_project(&root).ok()?;
    Some((
        project.requires.platform,
        profile.unwrap_or(project.requires.profile),
    ))
}

/// Resolve the runtime for L1/session preview, if a selection is available.
fn resolve_runtime(
    target: Option<String>,
    profile: Option<String>,
) -> Result<Option<crate::commands::Resolved>> {
    match selection(target, profile) {
        Some((platform, profile)) => Ok(Some(resolve(&platform, &profile)?)),
        None => Ok(None),
    }
}

/// Resolve a runtime that must be pulled and carry real OpenUSD artifacts.
/// `ost plugin run` and the execution levels cannot work against mock/absent.
fn require_real_runtime(
    target: Option<String>,
    profile: Option<String>,
) -> Result<crate::commands::Resolved> {
    let (platform, profile) = selection(target, profile).ok_or_else(|| {
        Error::Operation(
            "no platform/profile: run inside an OpenStrata project or pass --target/--profile"
                .into(),
        )
    })?;
    let r = resolve(&platform, &profile)?;
    if !r.pulled {
        return Err(Error::Operation(format!(
            "runtime '{}' not pulled — adopt one with `ost runtime pull {platform} --profile {profile} --from-usd <path>`",
            r.runtime.id()
        )));
    }
    // Read the manifest to confirm the source is real (not mock).
    let manifest = std::fs::read_to_string(r.prefix.join(MANIFEST_FILE).as_std_path())
        .ok()
        .and_then(|s| RuntimeManifest::from_json(&s).ok());
    let real = manifest.map(|m| m.source.is_real()).unwrap_or(false);
    if !real {
        return Err(Error::Operation(
            "runtime is mock — a real OpenUSD runtime is required (adopt with `--from-usd`)".into(),
        ));
    }
    Ok(r)
}

/// Build the Level 1 runtime context from a resolved runtime and its manifest.
fn runtime_context(r: &crate::commands::Resolved) -> RuntimeContext {
    let mut ctx = RuntimeContext {
        pulled: r.pulled,
        ..RuntimeContext::default()
    };
    if r.pulled {
        let manifest_path = r.prefix.join(MANIFEST_FILE);
        if let Ok(src) = std::fs::read_to_string(manifest_path.as_std_path()) {
            if let Ok(m) = RuntimeManifest::from_json(&src) {
                ctx.source = Some(m.source.as_str().to_string());
                ctx.real = m.source.is_real();
                ctx.reproducible = m.source.is_reproducible();
                for ext in &m.extensions {
                    ctx.components.insert(ext.id.clone(), ext.version.clone());
                    if ext.id == "openusd" {
                        ctx.openusd_version = Some(ext.version.clone());
                    }
                }
            }
        }
    }
    ctx
}

fn print_report(bundle: &Bundle, report: &DoctorReport) {
    let m = &bundle.manifest;
    println!(
        "Plugin {} {} ({})  —  {}",
        m.plugin.name,
        m.plugin.version,
        m.kind().as_str(),
        bundle.root
    );
    for d in &report.diagnostics {
        let mark = match d.status {
            Status::Pass => "PASS",
            Status::Fail => "FAIL",
            Status::Skip => "SKIP",
        };
        println!("  [{mark}] L{} {:<26} {}", d.level, d.id, d.observed);
        for action in &d.suggested_actions {
            println!("         ↳ {action}");
        }
    }
    println!(
        "\nResult: {} ({} pass, {} fail, {} skip)",
        if report.passed() { "OK" } else { "FAILED" },
        report.count(Status::Pass),
        report.count(Status::Fail),
        report.count(Status::Skip),
    );
}

/// Map a report's pass/fail onto a deterministic process exit (§13.2).
fn finish(report: &DoctorReport) -> Result<()> {
    if report.passed() {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

fn run_step(program: &std::path::Path, args: &[String]) -> Result<()> {
    println!("==> {} {}", program.display(), args.join(" "));
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|e| Error::io(format!("run {}", program.display()), e))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// Resolve the compiler policy for a plugin build: CLI flags over the enclosing
/// project's `[build]` table (if the bundle sits inside a project), else host.
fn resolve_plugin_compiler(opts: &CompilerOpts) -> Result<ost_build::Compiler> {
    let build = std::env::current_dir()
        .ok()
        .and_then(|cwd| find_project_root(&cwd))
        .and_then(|r| Utf8PathBuf::from_path_buf(r).ok())
        .and_then(|root| load_project(&root).ok())
        .and_then(|p| p.build);
    compiler::resolve(opts, build.as_ref())
}

/// Per-target toolchain/state directory inside a bundle: `.strata/targets/<id>/`.
/// Keyed by target id so each platform/profile/runtime keeps its own toolchain.
fn target_state_dir(root: &Utf8Path, id: &str) -> Utf8PathBuf {
    root.join(STATE_DIR).join("targets").join(id)
}

/// Per-target CMake build tree inside a bundle: `build/<id>`. Keeping the build
/// tree under the target id prevents one target reusing another's CMake cache.
fn target_build_dir(root: &Utf8Path, id: &str) -> Utf8PathBuf {
    root.join("build").join(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_paths_are_keyed_by_target_id() {
        let root = Utf8PathBuf::from("/bundle");
        let id = "cy2026-linux-x86_64-py311-usd";

        let bd = target_build_dir(&root, id);
        assert_eq!(bd.file_name(), Some(id));
        assert_eq!(bd.parent().unwrap().file_name(), Some("build"));

        let sd = target_state_dir(&root, id);
        assert_eq!(sd.file_name(), Some(id));
        assert_eq!(sd.parent().unwrap().file_name(), Some("targets"));

        // Different targets never share a build tree (no CMake-cache mixing).
        assert_ne!(bd, target_build_dir(&root, "cy2027-linux-x86_64-py313-usd"));
    }
}
