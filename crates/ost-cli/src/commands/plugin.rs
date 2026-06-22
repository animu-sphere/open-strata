//! `ost plugin` — OpenUSD plugin bundles (Phase 4a surface).
//!
//! - `new`     scaffold a bundle from a template.
//! - `inspect` Level 0 bundle structure report (no runtime needed).
//! - `build`   build the shared library + stage plugInfo, reusing `ost-build`'s
//!             toolchain generation against the resolved runtime.
//! - `doctor`  staged diagnostics (L0–L1 now; L2+ SKIP until a real runtime),
//!             with a preview of the session env it *would* set, and a report
//!             written under `.strata/reports/`.
//!
//! The CLI stays thin: it resolves paths and the runtime, then calls into
//! `ost-plugin` for the model, checks, and report shapes.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use camino::Utf8PathBuf;
use clap::Subcommand;

use ost_core::host::Os;
use ost_core::paths::{find_project_root, STATE_DIR};
use ost_core::{tools, Error, Host, Result};
use ost_plugin::{
    diagnose, scaffold, session_env, Bundle, DoctorReport, PluginKind, RuntimeContext, Status,
};
use ost_runtime::{EnvSet, RuntimeManifest, MANIFEST_FILE};

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
        } => build(&bundle, target, profile, dry_run, ninja, fmt),
        PluginCmd::Doctor {
            bundle,
            target,
            profile,
        } => doctor(&bundle, target, profile, fmt),
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
    let ctx = resolved
        .as_ref()
        .map(runtime_context)
        .unwrap_or_default();

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

fn build(
    bundle_path: &str,
    target: Option<String>,
    profile: Option<String>,
    dry_run: bool,
    ninja: Option<String>,
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

    // Generate the toolchain that points CMake at the runtime (reusing ost-build).
    let target_dir = bundle.root.join(STATE_DIR).join("build");
    std::fs::create_dir_all(target_dir.as_std_path())
        .map_err(|e| Error::io(target_dir.to_string(), e))?;
    let toolchain = target_dir.join("toolchain.cmake");
    std::fs::write(
        toolchain.as_std_path(),
        format!("{}\n", ost_build::render_toolchain(&tgt, &r.artifact_prefix)),
    )
    .map_err(|e| Error::io(toolchain.to_string(), e))?;

    let build_dir = bundle.root.join("build");
    let cmake = tools::which("cmake");
    let ninja = ninja
        .map(PathBuf::from)
        .or_else(|| tools::which("ninja"));

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
    let build_args = vec!["--build".to_string(), build_dir.to_string().replace('\\', "/")];

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
    println!("\nBuilt {} against {}", bundle.manifest.plugin.name, tgt.runtime_id);
    println!("  lib:       {}", bundle.lib_dir());
    println!("  plugInfo:  {plug_info}");
    Ok(())
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
