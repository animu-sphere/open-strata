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

use ost_build::{pack_dir, stage_files};
use ost_core::host::Os;
use ost_core::paths::{find_project_root, STATE_DIR};
use ost_core::variant::{Abi, Variant};
use ost_core::{tools, Error, Host, Result};
use ost_plugin::{
    diagnose, run_levels, scaffold, session_env_with, usdview_check, Bundle, CxxAbi, DoctorReport,
    PluginKind, Probe, RuntimeContext, Session, Status, ToolOutput,
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
    /// Pack a built plugin bundle into a target-specific tar.zst artifact.
    Package {
        /// Path to the bundle directory.
        bundle: String,
        /// Platform target, e.g. `cy2026`. Defaults to the enclosing project's.
        #[arg(long)]
        target: Option<String>,
        /// Profile to package against. Defaults to the enclosing project's.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Run staged diagnostics (L0–L1) and write a report.
    Doctor {
        /// Path to the bundle directory.
        bundle: String,
        /// Additional plugin bundle(s) to include in the session env.
        #[arg(long = "with")]
        with: Vec<String>,
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
        /// Additional plugin bundle(s) to include in the session env.
        #[arg(long = "with")]
        with: Vec<String>,
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
        /// Additional plugin bundle(s) to include in the session env.
        #[arg(long = "with")]
        with: Vec<String>,
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
        /// Additional plugin bundle(s) to include in the session env.
        #[arg(long = "with")]
        with: Vec<String>,
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
        /// Additional plugin bundle(s) to include in the session env.
        #[arg(long = "with")]
        with: Vec<String>,
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
        PluginCmd::Package {
            bundle,
            target,
            profile,
        } => package(&bundle, target, profile, fmt),
        PluginCmd::Doctor {
            bundle,
            with,
            target,
            profile,
        } => doctor(&bundle, &with, target, profile, fmt),
        PluginCmd::Run {
            bundle,
            with,
            target,
            profile,
            command,
        } => run_session(&bundle, &with, target, profile, command, fmt),
        PluginCmd::Test {
            bundle,
            with,
            target,
            profile,
            up_to,
        } => test(&bundle, &with, target, profile, up_to, fmt),
        PluginCmd::View {
            bundle,
            with,
            fixture,
            target,
            profile,
        } => view(&bundle, &with, &fixture, target, profile),
        PluginCmd::TestView {
            bundle,
            with,
            fixture,
            target,
            profile,
        } => test_view(&bundle, &with, &fixture, target, profile, fmt),
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
        Error::usage(format!(
            "unknown plugin kind '{kind}' (expected one of: {})",
            kinds.join(", ")
        ))
    })?;

    let dest = Utf8PathBuf::from(dir.unwrap_or(name));
    let files = scaffold(kind, name, extension, &dest)?;

    if fmt.is_json() {
        output::success(&serde_json::json!({
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
        output::report(report.passed(), &ost_plugin::report_json(&bundle, &report));
    } else {
        print_report(&bundle, &report);
    }
    finish(&report)
}

fn doctor(
    bundle_path: &str,
    with_paths: &[String],
    target: Option<String>,
    profile: Option<String>,
    fmt: Format,
) -> Result<()> {
    let bundle = load_bundle(bundle_path)?;
    let with_bundles = load_with_bundles(with_paths)?;
    let host = Host::detect();

    // Resolve the runtime if we can (enclosing project or explicit flags). When
    // we can't, Level 1 honestly SKIPs rather than guessing.
    let resolved = resolve_runtime(target, profile)?;
    let ctx = resolved.as_ref().map(runtime_context).unwrap_or_default();

    // Compose the session env we *would* set (runtime env + bundle roots).
    let session = match &resolved {
        Some(r) => session_env_with(&r.env, &bundle, &with_bundles, host.os),
        None => standalone_session_env(&bundle, &with_bundles, host.os),
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
        output::report(report.passed(), &body);
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
        Error::usage(
            "no platform/profile: run inside an OpenStrata project or pass --target/--profile",
        )
    })?;
    let (tgt, r) = build_target(&platform, &profile)?;
    let id = tgt.id();

    // Compiler policy: CLI flags over the enclosing project's `[build]`, else host.
    let compiler = resolve_plugin_compiler(&bundle.root, &compiler_opts)?;

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
        // Ninja is single-config, so an unset CMAKE_BUILD_TYPE makes USD's
        // imported targets resolve to Debug — which links e.g. `tbb12_debug.lib`,
        // absent from a Release-only runtime (→ LNK1104). The runtimes OpenStrata
        // ships/adopts are Release, so default the build type to match.
        "-DCMAKE_BUILD_TYPE=Release".to_string(),
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
        if tgt.os() == Os::Windows && tools::which("cl").is_none() {
            println!("# (would auto-load the MSVC environment via vcvars64.bat)");
        }
        println!("cmake {}", configure_args.join(" "));
        println!("cmake {}", build_args.join(" "));
        return Ok(());
    }

    if !r.pulled {
        return Err(Error::coded(
            "RUNTIME_NOT_FOUND",
            ost_core::Category::Precondition,
            format!(
                "runtime '{}' not pulled — run `ost runtime pull {platform} --profile {profile}` first",
                tgt.runtime_id
            ),
        ));
    }
    let cmake = cmake.ok_or_else(|| {
        Error::coded(
            "REQUIRED_TOOL_MISSING",
            ost_core::Category::Precondition,
            "`cmake` not found on PATH",
        )
    })?;

    // If the compiler changed since the last build, the cached compiler/ABI in
    // build/<id> is stale — drop it so this configure is clean (mirrors the
    // project-level invalidation in `ost configure`).
    let lock_compiler = compiler::to_lock(&compiler, &r.artifact_prefix, tgt.os());
    invalidate_plugin_build_tree_if_compiler_changed(&bundle.root, &id, &lock_compiler);

    // On Windows the `host` compiler policy + Ninja needs cl.exe/link.exe (and
    // Ninja itself) on PATH. When they aren't — a plain shell rather than a VS
    // Developer Prompt — load the MSVC developer environment the same way
    // `ost build` does, so a plugin build need not be wrapped in a vcvars shell.
    let msvc_env = maybe_bootstrap_msvc(tgt.os());

    // A schema bundle's build runs `usdGenSchema` as a CMake step, which loads
    // `pxr` and resolves the base USD schemas (`@usd/schema.usda@`, where
    // `APISchemaBase` is defined) through the plugin registry. That needs the
    // runtime *session* env (`PXR_PLUGINPATH_NAME`, `PYTHONPATH`, the USD bin on
    // the loader path) — not just the MSVC delta a compile needs. Compose both for
    // a schema or schema-co-hosting build; a plain file-format build is unchanged.
    let build_env = if bundle.manifest.kind() == PluginKind::UsdSchema
        || !bundle.manifest.schema_provides().is_empty()
    {
        let session = session_env_with(&r.env, &bundle, &[], tgt.os());
        compose_build_env(&msvc_env, &session)
    } else {
        msvc_env
    };

    run_step(&cmake, &configure_args, &build_env)?;
    run_step(&cmake, &build_args, &build_env)?;

    // A co-hosting bundle (a non-schema kind that also declares `usd-schema:`)
    // regenerates its schema with usdGenSchema and *merges* the Types into its
    // existing plugInfo.json — usdGenSchema overwrites a whole file, so a co-host
    // must merge rather than clobber its SdfFileFormat entry. (A pure schema
    // bundle regenerates via its own CMakeLists usdGenSchema target above.)
    if bundle.manifest.kind() != PluginKind::UsdSchema
        && !bundle.manifest.schema_provides().is_empty()
    {
        regenerate_cohosted_schema(
            &bundle,
            &r.artifact_prefix,
            &target_dir.join("schema-gen"),
            &build_env,
        )?;
    }

    // Record the compiler so the next build can detect a change. The plugin
    // build writes no full target lock, so this lives beside the toolchain.
    let record = target_dir.join("compiler.lock.json");
    if let Ok(json) = serde_json::to_string_pretty(&lock_compiler) {
        let _ = std::fs::write(record.as_std_path(), json);
    }

    // plugInfo.json is shipped in the bundle (staged at scaffold time); confirm it.
    let plug_info = bundle.plug_info();
    if fmt.is_json() {
        output::success(&serde_json::json!({
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

fn package(
    bundle_path: &str,
    target: Option<String>,
    profile: Option<String>,
    fmt: Format,
) -> Result<()> {
    let bundle = load_bundle(bundle_path)?;
    let host = Host::detect();

    let (platform, profile) = selection(target, profile).ok_or_else(|| {
        Error::usage(
            "no platform/profile: run inside an OpenStrata project or pass --target/--profile",
        )
    })?;
    let (tgt, r) = build_target(&platform, &profile)?;
    let id = tgt.id();
    if !r.pulled {
        return Err(Error::coded(
            "RUNTIME_NOT_FOUND",
            ost_core::Category::Precondition,
            format!(
                "runtime '{}' not pulled — run `ost runtime pull {platform} --profile {profile}` first",
                tgt.runtime_id
            ),
        ));
    }

    let ctx = runtime_context(&r);
    // Validate the plugin *as authored* against the resolved runtime, so a
    // hand-authored ABI that conflicts with the target is reported rather than
    // silently rewritten. The emitted artifact then freezes the resolved ABI.
    let report = diagnose(&bundle, &ctx, 1);
    if !report.passed() {
        return Err(Error::validation(format!(
            "plugin '{}' did not pass static packaging validation",
            bundle.manifest.plugin.name
        ))
        .with_hint("run `ost plugin doctor` and fix the failing diagnostics before packaging"));
    }

    let mut packaged_manifest = bundle.manifest.clone();
    // The artifact targets exactly one variant, so freeze the one resolved ABI as
    // a scalar (collapsing any per-OS/`inherit` source declaration).
    packaged_manifest.runtime.cxx_abi = ctx.cxx_abi.clone().map(CxxAbi::Scalar);
    packaged_manifest.runtime.python_abi = ctx.python_abi.clone();
    let packaged_bundle = Bundle {
        root: bundle.root.clone(),
        manifest: packaged_manifest.clone(),
    };

    let session = session_env_with(&r.env, &packaged_bundle, &[], host.os);
    let stage = target_state_dir(&bundle.root, &id).join("package-stage");
    reset_dir(&stage)?;
    stage_plugin_bundle(&packaged_bundle, &stage)?;
    write_packaged_manifest(&stage.join(ost_plugin::PLUGIN_MANIFEST), &packaged_manifest)?;
    write_validation_files(&packaged_bundle, &report, &session, &stage)?;

    let staged = stage_files(&stage).map_err(|e| {
        if e.kind() == std::io::ErrorKind::InvalidData {
            Error::validation(e.to_string())
        } else {
            Error::io(stage.to_string(), e)
        }
    })?;

    let name = &packaged_manifest.plugin.name;
    let version = &packaged_manifest.plugin.version;
    let archive_name = plugin_archive_name(name, version, &id);
    let dist_dir = plugin_dist_dir(&bundle.root, name, version, &id);
    let archive_path = dist_dir.join(&archive_name);
    let packed = pack_dir(&stage, &archive_path, &staged)
        .map_err(|e| Error::io(archive_path.to_string(), e))?;

    let runtime_manifest = std::fs::read_to_string(r.prefix.join(MANIFEST_FILE).as_std_path())
        .ok()
        .and_then(|s| RuntimeManifest::from_json(&s).ok());
    let runtime_source = runtime_manifest
        .as_ref()
        .map(|m| m.source.as_str().to_string())
        .unwrap_or_else(|| "unknown".into());
    let runtime_validation = runtime_manifest
        .as_ref()
        .map(|m| m.validation.as_str().to_string())
        .unwrap_or_else(|| "unknown".into());

    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let files_json: Vec<_> = packed
        .files
        .iter()
        .map(|f| serde_json::json!({ "path": f.path, "sha256": f.sha256, "size": f.size }))
        .collect();
    let manifest = serde_json::json!({
        "schema": 1,
        "kind": "openstrata.plugin-bundle",
        "plugin": {
            "name": name,
            "version": version,
            "kind": packaged_manifest.kind().as_str(),
            "license": packaged_manifest.license,
        },
        "target": id,
        "archive": archive_name,
        "archive_digest": packed.archive_digest,
        "archive_size": packed.archive_size,
        "total_size": packed.total_size,
        "created_unix": created,
        "provenance": {
            "platform": tgt.platform,
            "profile": tgt.profile,
            "variant": tgt.variant.slug(),
            "cxx_abi": packaged_manifest.runtime.cxx_abi,
            "python_abi": packaged_manifest.runtime.python_abi,
            "runtime": {
                "id": tgt.runtime_id,
                "digest": tgt.runtime_digest,
                "source": runtime_source,
                "validation": runtime_validation,
            },
            "validation": {
                "passed": report.passed(),
                "report": "validation/report.json",
                "environment": "validation/environment.json",
            },
        },
        "files": files_json,
    });
    write_text(&dist_dir.join("manifest.json"), &pretty_json(&manifest)?)?;

    let bare = packed
        .archive_digest
        .strip_prefix("sha256:")
        .unwrap_or(&packed.archive_digest);
    write_text(
        &dist_dir.join("SHA256SUMS"),
        &format!("{bare}  {archive_name}"),
    )?;

    report_package(&id, &archive_path, &packed, fmt);
    Ok(())
}

/// `ost plugin run` — compose the runtime session and exec a command in it.
fn run_session(
    bundle_path: &str,
    with_paths: &[String],
    target: Option<String>,
    profile: Option<String>,
    command: Vec<String>,
    _fmt: Format,
) -> Result<()> {
    let bundle = load_bundle(bundle_path)?;
    let with_bundles = load_with_bundles(with_paths)?;
    let host = Host::detect();
    let r = require_real_runtime(target, profile)?;

    let session = session_env_with(&r.env, &bundle, &with_bundles, host.os);
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
    with_paths: &[String],
    target: Option<String>,
    profile: Option<String>,
    up_to: u8,
    fmt: Format,
) -> Result<()> {
    let bundle = load_bundle(bundle_path)?;
    let with_bundles = load_with_bundles(with_paths)?;
    let host = Host::detect();

    let resolved = resolve_runtime(target, profile)?;
    let ctx = resolved.as_ref().map(runtime_context).unwrap_or_default();
    let session = match &resolved {
        Some(r) => session_env_with(&r.env, &bundle, &with_bundles, host.os),
        None => standalone_session_env(&bundle, &with_bundles, host.os),
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
        output::report(report.passed(), &body);
    } else {
        print_report(&bundle, &report);
        println!("\nReport: {report_dir}");
    }
    finish(&report)
}

/// `ost plugin view` — open a fixture in usdview inside the runtime session.
fn view(
    bundle_path: &str,
    with_paths: &[String],
    fixture: &str,
    target: Option<String>,
    profile: Option<String>,
) -> Result<()> {
    let bundle = load_bundle(bundle_path)?;
    let with_bundles = load_with_bundles(with_paths)?;
    let host = Host::detect();
    let r = require_real_runtime(target, profile)?;

    let usdview = locate_runtime_tool(Some(&r), &["usdview.cmd", "usdview.exe", "usdview"])
        .ok_or_else(|| {
            Error::coded(
                "REQUIRED_TOOL_MISSING",
                ost_core::Category::Precondition,
                "usdview not found in the runtime (build/adopt one with usdview enabled)",
            )
        })?;
    let fixture_path = bundle.path(fixture); // absolute passes through; else under the bundle

    let session = session_env_with(&r.env, &bundle, &with_bundles, host.os);
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
    with_paths: &[String],
    fixture: &str,
    target: Option<String>,
    profile: Option<String>,
    fmt: Format,
) -> Result<()> {
    let bundle = load_bundle(bundle_path)?;
    let with_bundles = load_with_bundles(with_paths)?;
    let host = Host::detect();
    let r = require_real_runtime(target, profile)?;

    let session = session_env_with(&r.env, &bundle, &with_bundles, host.os);
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
        output::report(report.passed(), &body);
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

fn load_with_bundles(paths: &[String]) -> Result<Vec<Bundle>> {
    paths.iter().map(|path| load_bundle(path)).collect()
}

fn standalone_session_env(bundle: &Bundle, with: &[Bundle], os: Os) -> EnvSet {
    session_env_with(
        &EnvSet {
            sep: if os == Os::Windows { ';' } else { ':' },
            vars: Vec::new(),
        },
        bundle,
        with,
        os,
    )
}

/// Regenerate a co-hosting bundle's schema with usdGenSchema and merge the
/// generated `Types` into its existing `plugInfo.json`, copying the flattened
/// `generatedSchema.usda` beside it. Runs the runtime's `usdGenSchema` script via
/// `python` in the composed `build_env` (so `pxr` loads and the base USD schemas
/// resolve). A no-op — keeping the committed resources — when the bundle ships no
/// `schema.usda` or the runtime has no `usdGenSchema`.
fn regenerate_cohosted_schema(
    bundle: &Bundle,
    artifact_prefix: &Utf8Path,
    staging: &Utf8Path,
    build_env: &[(String, String)],
) -> Result<()> {
    let schema_src = bundle.path("schema.usda");
    if !schema_src.as_std_path().is_file() {
        return Ok(()); // no schema source to regenerate from
    }
    // The usdGenSchema *script* in the runtime bin, run via `python` so we need no
    // bare-name/`.cmd` resolution and stay cross-platform.
    let gen_script = artifact_prefix.join("bin/usdGenSchema");
    if !gen_script.as_std_path().is_file() {
        println!(
            "==> usdGenSchema not in the runtime; keeping the committed co-hosted schema resources"
        );
        return Ok(());
    }

    reset_dir(staging)?;
    std::fs::create_dir_all(staging.as_std_path())
        .map_err(|e| Error::io(staging.to_string(), e))?;
    println!("==> regenerating co-hosted schema with usdGenSchema");
    run_step(
        std::path::Path::new("python"),
        &[
            gen_script.to_string(),
            schema_src.to_string(),
            staging.to_string(),
        ],
        build_env,
    )?;

    // Merge the generated schema Types into the bundle's plugInfo.json.
    let target_pi = bundle.plug_info();
    let generated_pi = staging.join("plugInfo.json");
    let target_src = std::fs::read_to_string(target_pi.as_std_path())
        .map_err(|e| Error::io(target_pi.to_string(), e))?;
    let generated_src = std::fs::read_to_string(generated_pi.as_std_path())
        .map_err(|e| Error::io(generated_pi.to_string(), e))?;
    let merged = ost_plugin::merge_schema_types(&target_src, &generated_src)
        .map_err(|e| Error::Operation(format!("merging schema Types: {e}")))?;
    std::fs::write(target_pi.as_std_path(), merged)
        .map_err(|e| Error::io(target_pi.to_string(), e))?;

    // Copy the flattened schema definition beside the plugInfo (registration needs it).
    let generated_schema = staging.join("generatedSchema.usda");
    if generated_schema.as_std_path().is_file() {
        let dest = bundle.plug_info_root().join("generatedSchema.usda");
        std::fs::copy(generated_schema.as_std_path(), dest.as_std_path())
            .map_err(|e| Error::io(dest.to_string(), e))?;
    }
    println!("    merged schema Types into {target_pi}");
    Ok(())
}

fn reset_dir(dir: &Utf8Path) -> Result<()> {
    if dir.as_std_path().exists() {
        std::fs::remove_dir_all(dir.as_std_path()).map_err(|e| Error::io(dir.to_string(), e))?;
    }
    std::fs::create_dir_all(dir.as_std_path()).map_err(|e| Error::io(dir.to_string(), e))
}

fn stage_plugin_bundle(bundle: &Bundle, stage: &Utf8Path) -> Result<()> {
    copy_tree_if_exists(&bundle.plug_info_root(), &plug_info_root_rel(bundle), stage)?;
    copy_tree_if_exists(&bundle.lib_dir(), Utf8Path::new("lib"), stage)?;
    copy_tree_if_exists(&bundle.python_dir(), Utf8Path::new("python"), stage)?;
    for dir in &bundle.manifest.requires.runtime_libs {
        copy_tree_required(&bundle.path(dir), Utf8Path::new(dir), stage)?;
    }
    for fixture in bundle.manifest.all_fixtures() {
        copy_file_required(&bundle.path(fixture), Utf8Path::new(fixture), stage)?;
    }
    // Carry third-party notices into the package so it ships with attribution.
    for notice in bundle.notices() {
        copy_file_required(&bundle.path(notice), Utf8Path::new(notice), stage)?;
    }
    Ok(())
}

fn plug_info_root_rel(bundle: &Bundle) -> Utf8PathBuf {
    Utf8Path::new(&bundle.manifest.usd.plug_info)
        .parent()
        .map(Utf8Path::to_path_buf)
        .unwrap_or_default()
}

fn copy_tree_if_exists(src: &Utf8Path, rel: &Utf8Path, stage: &Utf8Path) -> Result<()> {
    if src.as_std_path().exists() {
        copy_tree_required(src, rel, stage)?;
    }
    Ok(())
}

fn copy_tree_required(src: &Utf8Path, rel: &Utf8Path, stage: &Utf8Path) -> Result<()> {
    let meta =
        std::fs::symlink_metadata(src.as_std_path()).map_err(|e| Error::io(src.to_string(), e))?;
    if meta.file_type().is_symlink() {
        return Err(Error::validation(format!(
            "symlink is not allowed in plugin package input: {src}"
        )));
    }
    if !meta.is_dir() {
        return Err(Error::validation(format!(
            "expected package input directory at {src}"
        )));
    }
    copy_tree_contents(src, rel, stage)
}

fn copy_tree_contents(src: &Utf8Path, rel: &Utf8Path, stage: &Utf8Path) -> Result<()> {
    for entry in std::fs::read_dir(src.as_std_path()).map_err(|e| Error::io(src.to_string(), e))? {
        let entry = entry.map_err(|e| Error::io(src.to_string(), e))?;
        let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|p| {
            Error::config(format!("non-UTF-8 path in plugin bundle: {}", p.display()))
        })?;
        let name = path.file_name().ok_or_else(|| {
            Error::config(format!(
                "cannot determine file name for package input: {path}"
            ))
        })?;
        let rel_path = rel.join(name);
        let ty = entry
            .file_type()
            .map_err(|e| Error::io(path.to_string(), e))?;
        if ty.is_symlink() {
            return Err(Error::validation(format!(
                "symlink is not allowed in plugin package input: {path}"
            )));
        } else if ty.is_dir() {
            copy_tree_contents(&path, &rel_path, stage)?;
        } else if ty.is_file() {
            copy_file_required(&path, &rel_path, stage)?;
        } else {
            return Err(Error::validation(format!(
                "special file is not allowed in plugin package input: {path}"
            )));
        }
    }
    Ok(())
}

fn copy_file_required(src: &Utf8Path, rel: &Utf8Path, stage: &Utf8Path) -> Result<()> {
    let meta =
        std::fs::symlink_metadata(src.as_std_path()).map_err(|e| Error::io(src.to_string(), e))?;
    if meta.file_type().is_symlink() {
        return Err(Error::validation(format!(
            "symlink is not allowed in plugin package input: {src}"
        )));
    }
    if !meta.is_file() {
        return Err(Error::validation(format!(
            "expected package input file at {src}"
        )));
    }
    let dest = stage.join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent.as_std_path())
            .map_err(|e| Error::io(parent.to_string(), e))?;
    }
    std::fs::copy(src.as_std_path(), dest.as_std_path())
        .map(|_| ())
        .map_err(|e| Error::io(format!("{src} -> {dest}"), e))
}

fn write_packaged_manifest(path: &Utf8Path, manifest: &ost_plugin::PluginManifest) -> Result<()> {
    let body = serde_yaml::to_string(manifest)
        .map_err(|e| Error::parse("openstrata.plugin.yaml", anyhow::Error::new(e)))?;
    write_text(path, body.trim_end())
}

fn write_validation_files(
    bundle: &Bundle,
    report: &DoctorReport,
    session: &EnvSet,
    stage: &Utf8Path,
) -> Result<()> {
    let validation = stage.join("validation");
    std::fs::create_dir_all(validation.as_std_path())
        .map_err(|e| Error::io(validation.to_string(), e))?;
    write_text(
        &validation.join("report.json"),
        &pretty_json(&ost_plugin::report_json(bundle, report))?,
    )?;
    write_text(
        &validation.join("environment.json"),
        &pretty_json(&ost_plugin::environment_json(session))?,
    )
}

fn plugin_dist_dir(bundle_root: &Utf8Path, name: &str, version: &str, id: &str) -> Utf8PathBuf {
    bundle_root
        .join("dist")
        .join("plugins")
        .join(name)
        .join(version)
        .join(id)
}

fn plugin_archive_name(name: &str, version: &str, id: &str) -> String {
    format!("{name}-{version}-{id}.tar.zst")
}

fn write_text(path: &Utf8Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent.as_std_path())
            .map_err(|e| Error::io(parent.to_string(), e))?;
    }
    std::fs::write(path.as_std_path(), format!("{contents}\n"))
        .map_err(|e| Error::io(path.to_string(), e))
}

fn pretty_json(value: &serde_json::Value) -> Result<String> {
    serde_json::to_string_pretty(value).map_err(|e| Error::parse("json", anyhow::Error::new(e)))
}

fn report_package(id: &str, archive: &Utf8Path, packed: &ost_build::PackResult, fmt: Format) {
    if fmt.is_json() {
        output::success(&serde_json::json!({
            "packaged": true,
            "target": id,
            "archive": archive.to_string(),
            "archive_digest": packed.archive_digest,
            "archive_size": packed.archive_size,
            "files": packed.files.len(),
        }));
        return;
    }
    println!("Packaged plugin target {id}");
    println!("  archive:  {archive}");
    println!("  digest:   {}", packed.archive_digest);
    println!(
        "  size:     {} bytes ({} file(s), {} uncompressed)",
        packed.archive_size,
        packed.files.len(),
        packed.total_size
    );
    println!("  manifest.json + SHA256SUMS written alongside the archive");
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
        Error::usage(
            "no platform/profile: run inside an OpenStrata project or pass --target/--profile",
        )
    })?;
    let r = resolve(&platform, &profile)?;
    if !r.pulled {
        return Err(Error::coded(
            "RUNTIME_NOT_FOUND",
            ost_core::Category::Precondition,
            format!(
                "runtime '{}' not pulled — adopt one with `ost runtime pull {platform} --profile {profile} --from-usd <path>`",
                r.runtime.id()
            ),
        ));
    }
    // Read the manifest to confirm the source is real (not mock).
    let manifest = std::fs::read_to_string(r.prefix.join(MANIFEST_FILE).as_std_path())
        .ok()
        .and_then(|s| RuntimeManifest::from_json(&s).ok());
    let real = manifest.map(|m| m.source.is_real()).unwrap_or(false);
    if !real {
        return Err(Error::coded(
            "REAL_RUNTIME_REQUIRED",
            ost_core::Category::Precondition,
            "runtime is mock — a real OpenUSD runtime is required (adopt with `--from-usd`)",
        ));
    }
    Ok(r)
}

/// Build the Level 1 runtime context from a resolved runtime and its manifest.
fn runtime_context(r: &crate::commands::Resolved) -> RuntimeContext {
    let mut ctx = RuntimeContext {
        target_os: Some(r.runtime.variant.os),
        cxx_abi: Some(runtime_cxx_abi(&r.runtime.variant)),
        python_abi: Some(r.runtime.variant.python_abi()),
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

fn runtime_cxx_abi(variant: &Variant) -> String {
    match variant.os {
        Os::Linux => "libstdcxx".into(),
        Os::Macos => "libcxx".into(),
        Os::Windows => match &variant.abi {
            Abi::Msvc { toolset } => format!("msvc{toolset}"),
            _ => "msvc".into(),
        },
    }
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
    if let Some(license) = &m.license {
        println!("  license: {license}");
    }
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
        // The caller already emitted the report; a failing plugin check is a
        // validation mismatch (§14.4), so exit with that category code.
        std::process::exit(ost_core::Category::Validation.exit_code() as i32);
    }
}

/// Compose the build environment for a schema bundle: the MSVC delta (compiler
/// `PATH`/`LIB`/`INCLUDE`, when bootstrapped) plus the runtime *session* env that
/// `usdGenSchema` needs (`PXR_PLUGINPATH_NAME`, `PYTHONPATH`, USD on the loader
/// `PATH`). The session is resolved *over* a base carrying the MSVC delta, so its
/// `PATH` prepends USD's entries in front of the compiler's rather than dropping
/// them; case-variant keys (`Path` vs `PATH`) are folded so the original `PATH`
/// is not duplicated. The MSVC-only keys (`LIB`/`INCLUDE`/`LIBPATH`), which the
/// session does not carry, are kept by listing the delta first.
fn compose_build_env(msvc_env: &[(String, String)], session: &EnvSet) -> Vec<(String, String)> {
    let mut base: std::collections::HashMap<String, String> = std::env::vars().collect();
    for (k, v) in msvc_env {
        // Drop any case-variant of this key first so the case-folding lookup in
        // `resolve_over` is unambiguous (Windows spells the search path `Path`).
        base.retain(|bk, _| !bk.eq_ignore_ascii_case(k));
        base.insert(k.clone(), v.clone());
    }
    let mut env = msvc_env.to_vec();
    env.extend(session.resolve_over(&base));
    env
}

fn run_step(program: &std::path::Path, args: &[String], env: &[(String, String)]) -> Result<()> {
    println!("==> {} {}", program.display(), args.join(" "));
    let mut cmd = Command::new(program);
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v); // overlay the MSVC delta, no global mutation
    }
    let status = cmd
        .status()
        .map_err(|e| Error::io(format!("run {}", program.display()), e))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// Load the MSVC developer environment (cl/link/Ninja) as an env delta when the
/// host build needs it: Windows, with `cl` not already on PATH. Mirrors
/// `ost build` so a plugin build need not run from a VS Developer Prompt. An
/// empty vec means "use the current environment" — non-Windows, `cl` already
/// present, or no Visual Studio found (a warning is printed in that last case).
fn maybe_bootstrap_msvc(os: Os) -> Vec<(String, String)> {
    if os != Os::Windows || tools::which("cl").is_some() {
        return Vec::new();
    }
    match ost_build::msvc::bootstrap() {
        Ok(Some(env)) => {
            println!(
                "==> loaded MSVC environment ({} vars) from {}",
                env.vars.len(),
                env.vcvars.display()
            );
            env.vars
        }
        Ok(None) => {
            eprintln!(
                "warning: MSVC not found; relying on the current environment (cl must be on PATH)"
            );
            Vec::new()
        }
        Err(e) => {
            eprintln!("warning: failed to load the MSVC environment: {e}");
            Vec::new()
        }
    }
}

/// Resolve the compiler policy for a plugin build: CLI flags over the enclosing
/// project's `[build]` table (if the bundle sits inside a project), else host.
///
/// The enclosing project is found from the *bundle's* location, not the current
/// working directory, so `ost plugin build path/to/bundle` honors that bundle's
/// project regardless of where it is invoked from.
fn resolve_plugin_compiler(
    bundle_root: &Utf8Path,
    opts: &CompilerOpts,
) -> Result<ost_build::Compiler> {
    let build = find_project_root(bundle_root.as_std_path())
        .and_then(|r| Utf8PathBuf::from_path_buf(r).ok())
        .and_then(|root| load_project(&root).ok())
        .and_then(|p| p.build);
    compiler::resolve(opts, build.as_ref())
}

/// Remove the bundle's `build/<id>` when the compiler differs from the last
/// build. Mirrors `ost configure`'s invalidation: CMake caches the compiler and
/// its ABI on first configure, and reusing that cache with a different compiler
/// produces incoherent builds (or a hard `CMAKE_*_COMPILER changed` error). The
/// previous compiler is read from `compiler.lock.json` beside the toolchain; a
/// missing/unreadable record means nothing to invalidate.
fn invalidate_plugin_build_tree_if_compiler_changed(
    bundle_root: &Utf8Path,
    id: &str,
    next: &ost_build::LockCompiler,
) {
    let record = target_state_dir(bundle_root, id).join("compiler.lock.json");
    let previous = std::fs::read_to_string(record.as_std_path())
        .ok()
        .and_then(|s| serde_json::from_str::<ost_build::LockCompiler>(&s).ok());

    if let Some(prev) = previous {
        if prev.fingerprint() != next.fingerprint() {
            let build_dir = target_build_dir(bundle_root, id);
            if build_dir.as_std_path().exists() {
                let _ = std::fs::remove_dir_all(build_dir.as_std_path());
            }
        }
    }
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
    use ost_core::host::Arch;

    fn variant(os: Os, abi: Abi) -> Variant {
        Variant {
            os,
            arch: Arch::X86_64,
            abi,
            python: "313".into(),
        }
    }

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

    #[test]
    fn runtime_cxx_abi_is_target_aware() {
        assert_eq!(
            runtime_cxx_abi(&variant(
                Os::Linux,
                Abi::Glibc {
                    version: "2.28".into()
                }
            )),
            "libstdcxx"
        );
        assert_eq!(runtime_cxx_abi(&variant(Os::Macos, Abi::Native)), "libcxx");
        assert_eq!(
            runtime_cxx_abi(&variant(
                Os::Windows,
                Abi::Msvc {
                    toolset: "143".into()
                }
            )),
            "msvc143"
        );
    }

    #[test]
    fn plugin_package_paths_are_target_keyed() {
        let root = Utf8PathBuf::from("/bundle");
        let id = "cy2026-linux-x86_64-py313-usd";
        assert_eq!(
            plugin_archive_name("toy", "0.1.0", id),
            "toy-0.1.0-cy2026-linux-x86_64-py313-usd.tar.zst"
        );
        assert_eq!(
            plugin_dist_dir(&root, "toy", "0.1.0", id),
            root.join("dist/plugins/toy/0.1.0").join(id)
        );
    }
}
