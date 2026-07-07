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
    /// Publish a packaged plugin artifact into the local registry (by digest).
    Publish {
        /// Path to the bundle directory.
        bundle: String,
        /// Platform target, e.g. `cy2026`. Defaults to the enclosing project's.
        #[arg(long)]
        target: Option<String>,
        /// Profile the package was built against. Defaults to the project's.
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
        /// Path to the bundle directory (omit with --workspace).
        bundle: Option<String>,
        /// Discover and test every bundle in the workspace: immediate
        /// subdirectories and plugins/* holding an openstrata.plugin.yaml.
        #[arg(long)]
        workspace: bool,
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
    /// Manage a bundle's co-located USD schema.
    #[command(subcommand)]
    Schema(SchemaCmd),
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
        PluginCmd::Publish {
            bundle,
            target,
            profile,
        } => publish(&bundle, target, profile, fmt),
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
            workspace,
            with,
            target,
            profile,
            up_to,
        } => match (workspace, bundle) {
            (true, Some(_)) => Err(Error::usage(
                "--workspace discovers bundles itself — drop the bundle path",
            )),
            (true, None) => test_workspace(&with, target, profile, up_to, fmt),
            (false, Some(bundle)) => test(&bundle, &with, target, profile, up_to, fmt),
            (false, None) => Err(Error::usage(
                "missing bundle path (or pass --workspace to test every bundle)",
            )),
        },
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
        PluginCmd::Schema(SchemaCmd::Add {
            bundle,
            class,
            source,
            codeless,
        }) => schema_add(&bundle, &class, &source, codeless, fmt),
    }
}

#[derive(Debug, Subcommand)]
pub enum SchemaCmd {
    /// Add a co-located schema to an existing (non-schema) bundle: write a
    /// starter schema.usda and wire the manifest so the next `ost plugin
    /// build` generates + links it into the same plugin library.
    Add {
        /// Path to the bundle directory.
        bundle: String,
        /// Source class name; the public type is <PascalBundleName><CLASS>,
        /// e.g. bundle `toy` + class `MetadataAPI` -> `ToyMetadataAPI`.
        #[arg(long, default_value = "API")]
        class: String,
        /// Bundle-relative path for the schema source.
        #[arg(long, default_value = "schema/schema.usda")]
        source: String,
        /// Scaffold a codeless (skipCodeGeneration) schema: the build merges
        /// only the generated resources, adding no C++ to the library.
        #[arg(long)]
        codeless: bool,
    },
}

fn schema_add(bundle: &str, class: &str, source: &str, codeless: bool, fmt: Format) -> Result<()> {
    let root = Utf8PathBuf::from(bundle);
    let added = ost_plugin::add_cohosted_schema(&root, class, source, codeless)?;

    if fmt.is_json() {
        output::success(&serde_json::json!({
            "added": true,
            "schema_type": added.schema_type,
            "provides": format!("usd-schema:{}", added.schema_type),
            "source": added.source.to_string(),
            "codeless": added.codeless,
        }));
        return Ok(());
    }
    println!(
        "Added co-located schema {} ({})",
        added.schema_type,
        if added.codeless {
            "codeless"
        } else {
            "compiled"
        }
    );
    println!("  schema source:  {}", added.source);
    println!(
        "  manifest wired: provides usd-schema:{} + schema.source",
        added.schema_type
    );
    println!("\nNext steps:");
    println!("  1. edit {} (the real properties)", added.source);
    println!("  2. ost plugin build {bundle}   # usdGenSchema + Types merge + link");
    println!("  3. ost plugin test  {bundle}   # L2 registration / L4 apply round-trip");
    Ok(())
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
            "workspace_template": "usd-plugin-workspace",
            "workspace_command": "ost init --template usd-plugin-workspace",
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
    println!("  multi-bundle repo root: ost init --template usd-plugin-workspace");
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
    // Pin a host interpreter's Development artifacts so an adopted runtime's
    // pxrConfig (which bakes the export machine's Python paths) configures on
    // this host. `None` when none matches — the toolchain then falls back to
    // the runtime prefix, unchanged from before.
    let python = ost_build::resolve_for_runtime(&r.artifact_prefix, &tgt.python_version);
    std::fs::write(
        toolchain.as_std_path(),
        format!(
            "{}\n",
            ost_build::render_toolchain(&tgt, &r.artifact_prefix, &compiler, python.as_ref())
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
    let schema_sources_file = target_dir.join("schema-sources.cmake");
    configure_args.push(format!(
        "-DOPENSTRATA_SCHEMA_SOURCES_FILE={}",
        cmake_path(&schema_sources_file)
    ));
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

    // A non-schema bundle can co-host a schema by declaring `usd-schema:*` and
    // shipping schema.usda. Generate it before configure so any compiled C++ API
    // sources can be included in the same plugin library. The plugInfo merge is
    // delayed until after configure because the template regenerates
    // plugInfo.json from plugInfo.json.in during configure.
    let cohosted_schema = if bundle.manifest.kind() != PluginKind::UsdSchema
        && !bundle.manifest.schema_provides().is_empty()
    {
        prepare_cohosted_schema(
            &bundle,
            &r.artifact_prefix,
            &target_dir.join("schema-gen"),
            &schema_sources_dir(&target_dir),
            &schema_sources_file,
            &build_env,
        )?
    } else {
        clear_cohosted_schema_compile_state(
            &schema_sources_dir(&target_dir),
            &schema_sources_file,
        )?;
        None
    };

    run_step(&cmake, &configure_args, &build_env)?;
    run_step(&cmake, &build_args, &build_env)?;

    if let Some(schema) = &cohosted_schema {
        merge_cohosted_schema_resources(&bundle, schema)?;
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
    // Reruns must not fail on a stage the previous run left temporarily
    // undeletable (scanner-held handles, dogfooding report #9): stage into a
    // fresh sibling instead, and surface that as a warning.
    let preferred_stage = target_state_dir(&bundle.root, &id).join("package-stage");
    let stage = ost_core::fs::prepare_staging_dir(preferred_stage.as_std_path())?;
    let stage = Utf8PathBuf::from_path_buf(stage)
        .map_err(|p| Error::Operation(format!("non-UTF-8 staging path: {}", p.display())))?;
    let stage_warnings = if stage == preferred_stage {
        Vec::new()
    } else {
        vec![serde_json::json!({
            "code": "STAGE_FALLBACK",
            "message": format!(
                "previous package stage '{preferred_stage}' is held open by another \
                 process; staged into '{stage}' instead (a later run sweeps it)"
            ),
        })]
    };
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

    report_package(&id, &archive_path, &packed, &stage_warnings, fmt);
    Ok(())
}

/// `ost plugin publish` — enter a *packaged* plugin artifact into the local
/// registry, addressed by digest (Phase 6 publish MVP).
///
/// Publish consumes `ost plugin package` output; it never re-packages. Entry is
/// gated: the artifact must carry a passed static validation, complete runtime
/// provenance, a concrete (frozen) C++ ABI, an SPDX license, and every notices
/// file the bundle declares — an artifact CI pins by digest must not be missing
/// the facts CI branches on.
fn publish(
    bundle_path: &str,
    target: Option<String>,
    profile: Option<String>,
    fmt: Format,
) -> Result<()> {
    let bundle = load_bundle(bundle_path)?;
    let (platform, profile) = selection(target, profile).ok_or_else(|| {
        Error::usage(
            "no platform/profile: run inside an OpenStrata project or pass --target/--profile",
        )
    })?;
    let (tgt, _r) = build_target(&platform, &profile)?;
    let id = tgt.id();

    let name = &bundle.manifest.plugin.name;
    let version = &bundle.manifest.plugin.version;
    let dist_dir = plugin_dist_dir(&bundle.root, name, version, &id);
    let manifest_path = dist_dir.join("manifest.json");
    if !manifest_path.as_std_path().is_file() {
        return Err(Error::precondition(format!(
            "no packaged artifact for '{name}' {version} ({id}) — expected {manifest_path}"
        ))
        .with_hint("run `ost plugin package` first; publish consumes its output"));
    }
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(manifest_path.as_std_path())
            .map_err(|e| Error::io(manifest_path.to_string(), e))?,
    )
    .map_err(|e| Error::parse(manifest_path.to_string(), anyhow::Error::new(e)))?;

    check_publishable(&manifest, bundle.notices())?;

    let store = ost_artifact::ArtifactStore::discover();
    let out = store.import(&dist_dir, ost_artifact::ArtifactSource::Published)?;

    if fmt.is_json() {
        output::success(&serde_json::json!({
            "published": true,
            "already_present": out.already_present,
            "digest": out.record.digest,
            "artifact": serde_json::to_value(&out.record).unwrap_or_default(),
        }));
        return Ok(());
    }
    if out.already_present {
        println!(
            "Already published: {} {} {} is stored as {}",
            out.record.kind.as_str(),
            out.record.name,
            out.record.version,
            out.record.short_digest()
        );
    } else {
        println!(
            "Published {} {} for {}",
            out.record.name, out.record.version, out.record.target
        );
    }
    // The full reference is the line CI pins; print it unabbreviated.
    println!("  digest: {}", out.record.digest);
    println!("  pin it, e.g. `ost artifact show {}`", out.record.digest);
    Ok(())
}

/// The publish gates, over the packaged artifact's `manifest.json`.
///
/// Each refusal carries its own stable code so CI can branch on *why* an
/// artifact was rejected, and a hint naming the fix.
fn check_publishable(manifest: &serde_json::Value, notices: &[String]) -> Result<()> {
    if manifest.get("kind").and_then(|v| v.as_str()) != Some(ost_artifact::PLUGIN_BUNDLE_KIND) {
        return Err(Error::coded(
            "PUBLISH_NOT_A_PLUGIN_BUNDLE",
            ost_core::Category::Validation,
            "the packaged manifest is not a plugin-bundle artifact",
        )
        .with_hint("re-run `ost plugin package` to produce a current manifest"));
    }

    let provenance = manifest.get("provenance");
    let validation_passed = provenance
        .and_then(|p| p.get("validation"))
        .and_then(|v| v.get("passed"))
        .and_then(|b| b.as_bool());
    if validation_passed != Some(true) {
        return Err(Error::coded(
            "PUBLISH_VALIDATION_REQUIRED",
            ost_core::Category::Validation,
            "the packaged artifact does not record a passed validation",
        )
        .with_hint("fix `ost plugin doctor` findings, then re-run `ost plugin package`"));
    }

    let license = manifest
        .get("plugin")
        .and_then(|p| p.get("license"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if license.is_none() {
        return Err(Error::coded(
            "PUBLISH_LICENSE_REQUIRED",
            ost_core::Category::Validation,
            "the packaged artifact records no license",
        )
        .with_hint(
            "set `license: <SPDX id>` in openstrata.plugin.yaml and re-run `ost plugin package`",
        ));
    }

    let runtime = provenance.and_then(|p| p.get("runtime"));
    let runtime_complete = ["id", "digest"].iter().all(|k| {
        runtime
            .and_then(|r| r.get(*k))
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty())
    });
    if !runtime_complete {
        return Err(Error::coded(
            "PUBLISH_PROVENANCE_INCOMPLETE",
            ost_core::Category::Validation,
            "the packaged artifact does not record the runtime it was validated against",
        )
        .with_hint("re-run `ost plugin package` against a pulled runtime"));
    }

    // Package freezes `cxx_abi: inherit` / per-OS maps into one concrete tag;
    // an artifact that still defers its ABI cannot be a support-matrix cell.
    match provenance.and_then(|p| p.get("cxx_abi")) {
        Some(serde_json::Value::String(tag)) if tag != "inherit" && !tag.is_empty() => {}
        _ => {
            return Err(Error::coded(
                "PUBLISH_ABI_UNRESOLVED",
                ost_core::Category::Validation,
                "the packaged artifact does not freeze a concrete C++ ABI",
            )
            .with_hint(
                "re-run `ost plugin package` — it resolves `cxx_abi: inherit`/per-OS maps \
                 to the target's ABI",
            ));
        }
    }

    // Attribution is a release gate (§ Licensing): every notices file the
    // bundle declares must actually be inside the archive.
    let packed: Vec<&str> = manifest
        .get("files")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|f| f.get("path").and_then(|p| p.as_str()))
                .collect()
        })
        .unwrap_or_default();
    let missing: Vec<&String> = notices
        .iter()
        .filter(|n| !packed.contains(&normalize_slash(n).as_str()))
        .collect();
    if !missing.is_empty() {
        return Err(Error::coded(
            "PUBLISH_NOTICES_MISSING",
            ost_core::Category::Validation,
            format!(
                "declared notices file(s) missing from the packaged artifact: {}",
                missing
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
        .with_hint("re-run `ost plugin package` so the notices are staged into the archive"));
    }

    Ok(())
}

/// Manifest `files[]` paths are forward-slashed; compare notices the same way.
fn normalize_slash(path: &str) -> String {
    path.replace('\\', "/")
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

    let (report, report_dir) =
        test_bundle(&bundle, &with_bundles, resolved.as_ref(), &host, up_to)?;

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

/// Diagnose one bundle (L0..`up_to`) in the resolved session and write its
/// report — the shared core of `plugin test` and `plugin test --workspace`.
fn test_bundle(
    bundle: &Bundle,
    with_bundles: &[Bundle],
    resolved: Option<&crate::commands::Resolved>,
    host: &Host,
    up_to: u8,
) -> Result<(DoctorReport, Utf8PathBuf)> {
    let ctx = resolved.map(runtime_context).unwrap_or_default();
    let session = match resolved {
        Some(r) => session_env_with(&r.env, bundle, with_bundles, host.os),
        None => standalone_session_env(bundle, with_bundles, host.os),
    };

    // L0 + L1 are static. L2..up_to execute the runtime's tools — but only when a
    // real runtime is present; otherwise keep the honest SKIPs.
    let mut report = diagnose(bundle, &ctx, 1);
    if up_to >= 2 {
        if ctx.real {
            let probe = ProcessProbe::new(session.resolve());
            let tools = locate_tools(resolved, &probe);
            let sess = Session {
                probe: &probe,
                usdcat: tools.usdcat,
                python: tools.python,
                usdview: tools.usdview,
                has_display: has_display(host.os),
            };
            report
                .diagnostics
                .extend(run_levels(bundle, &sess, up_to.min(6)));
        } else {
            // Reuse diagnose's SKIP placeholders for the execution levels.
            let skips = diagnose(bundle, &ctx, up_to.min(5))
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
    let report_dir = ost_plugin::write_report(&reports_root, bundle, &report, &session, now)?;
    Ok((report, report_dir))
}

/// `ost plugin test --workspace` — discover the workspace's bundles and run
/// the verification pyramid on each, mirroring the `usd-plugin-workspace`
/// CMake discovery (immediate subdirectories and `plugins/*`).
fn test_workspace(
    with_paths: &[String],
    target: Option<String>,
    profile: Option<String>,
    up_to: u8,
    fmt: Format,
) -> Result<()> {
    let roots = discover_workspace_bundles(Utf8Path::new("."))?;
    if roots.is_empty() {
        return Err(Error::precondition(
            "no plugin bundles found in immediate subdirectories or plugins/*",
        )
        .with_hint("run from the workspace root, or pass a bundle path instead of --workspace"));
    }
    let with_bundles = load_with_bundles(with_paths)?;
    let host = Host::detect();
    // One resolution for the whole workspace: every bundle tests against the
    // same runtime session base.
    let resolved = resolve_runtime(target, profile)?;

    let mut results: Vec<(Bundle, DoctorReport, Utf8PathBuf)> = Vec::new();
    for root in &roots {
        let bundle = Bundle::load(root)?;
        let (report, report_dir) =
            test_bundle(&bundle, &with_bundles, resolved.as_ref(), &host, up_to)?;
        if !fmt.is_json() {
            println!("== {} ({root}) ==", bundle.manifest.plugin.name);
            print_report(&bundle, &report);
            println!("Report: {report_dir}\n");
        }
        results.push((bundle, report, report_dir));
    }

    let failed = results.iter().filter(|(_, r, _)| !r.passed()).count();
    let all_passed = failed == 0;
    if fmt.is_json() {
        let bundles: Vec<serde_json::Value> = results
            .iter()
            .map(|(bundle, report, dir)| {
                let mut body = ost_plugin::report_json(bundle, report);
                if let Some(obj) = body.as_object_mut() {
                    obj.insert(
                        "report_dir".into(),
                        serde_json::Value::String(dir.to_string()),
                    );
                }
                body
            })
            .collect();
        output::report(
            all_passed,
            &serde_json::json!({
                "workspace": true,
                "bundles": bundles,
                "total": results.len(),
                "failed": failed,
            }),
        );
    } else {
        println!("Workspace: {} bundle(s), {failed} failed", results.len());
    }
    if all_passed {
        Ok(())
    } else {
        // Reports were already emitted; aggregate like a single failing test.
        std::process::exit(ost_core::Category::Validation.exit_code() as i32);
    }
}

/// Bundle directories of a workspace: immediate subdirectories and
/// `plugins/*` entries holding an `openstrata.plugin.yaml`, sorted for
/// deterministic ordering.
fn discover_workspace_bundles(root: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    let mut found: Vec<Utf8PathBuf> = Vec::new();
    let scan = |dir: &Utf8Path, found: &mut Vec<Utf8PathBuf>| -> Result<()> {
        let Ok(entries) = std::fs::read_dir(dir.as_std_path()) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            let Ok(path) = Utf8PathBuf::from_path_buf(entry.path()) else {
                continue;
            };
            if path.is_dir() && path.join(ost_plugin::PLUGIN_MANIFEST).is_file() {
                found.push(path);
            }
        }
        Ok(())
    };
    scan(root, &mut found)?;
    scan(&root.join("plugins"), &mut found)?;
    found.sort();
    Ok(found)
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

#[derive(Debug)]
struct CohostedSchemaGeneration {
    generated_plug_info: Utf8PathBuf,
    generated_schema: Option<Utf8PathBuf>,
    compiled_sources: usize,
}

/// Regenerate a co-hosting bundle's schema with usdGenSchema. If usdGenSchema
/// emitted compiled C++ API files, stage those into `.strata/targets/<id>/` and
/// write a CMake fragment that the template can include into the plugin library.
/// The `plugInfo.json` merge happens after CMake configure, because configure may
/// regenerate the target plugInfo from `plugInfo.json.in`.
fn prepare_cohosted_schema(
    bundle: &Bundle,
    artifact_prefix: &Utf8Path,
    staging: &Utf8Path,
    compiled_dir: &Utf8Path,
    cmake_fragment: &Utf8Path,
    build_env: &[(String, String)],
) -> Result<Option<CohostedSchemaGeneration>> {
    let (schema_src, declared) = bundle.schema_source();
    if !schema_src.as_std_path().is_file() {
        // A manifest-declared source that is missing is a broken wiring the
        // user should hear about; the absent *conventional* file just means
        // this bundle keeps its committed resources.
        if declared {
            return Err(Error::config(format!(
                "schema.source declares '{schema_src}' but the file does not exist"
            ))
            .with_hint("create it (`ost plugin schema add` scaffolds one) or drop schema.source"));
        }
        clear_cohosted_schema_compile_state(compiled_dir, cmake_fragment)?;
        return Ok(None); // no schema source to regenerate from
    }
    // The usdGenSchema *script* in the runtime bin, run via `python` so we need no
    // bare-name/`.cmd` resolution and stay cross-platform.
    let gen_script = artifact_prefix.join("bin/usdGenSchema");
    if !gen_script.as_std_path().is_file() {
        clear_cohosted_schema_compile_state(compiled_dir, cmake_fragment)?;
        println!(
            "==> usdGenSchema not in the runtime; keeping the committed co-hosted schema resources"
        );
        return Ok(None);
    }

    reset_dir(staging)?; // also (re)creates the dir
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

    let generated_pi = staging.join("plugInfo.json");
    let generated_schema = staging.join("generatedSchema.usda");
    let compiled = stage_compiled_schema_sources(staging, compiled_dir, cmake_fragment)?;
    if compiled.sources > 0 {
        println!(
            "    staged compiled schema API ({} source file(s), {} header(s))",
            compiled.sources, compiled.headers
        );
    }

    Ok(Some(CohostedSchemaGeneration {
        generated_plug_info: generated_pi,
        generated_schema: generated_schema
            .as_std_path()
            .is_file()
            .then_some(generated_schema),
        compiled_sources: compiled.sources,
    }))
}

/// Merge a generated co-hosted schema's `Types` into the bundle's target
/// `plugInfo.json`, preserving any existing library plugin entry, and copy the
/// flattened `generatedSchema.usda` beside it.
fn merge_cohosted_schema_resources(
    bundle: &Bundle,
    generated: &CohostedSchemaGeneration,
) -> Result<()> {
    let target_pi = bundle.plug_info();
    let target_src = std::fs::read_to_string(target_pi.as_std_path())
        .map_err(|e| Error::io(target_pi.to_string(), e))?;
    let generated_src = std::fs::read_to_string(generated.generated_plug_info.as_std_path())
        .map_err(|e| Error::io(generated.generated_plug_info.to_string(), e))?;
    let merged = ost_plugin::merge_schema_types(&target_src, &generated_src)
        .map_err(|e| Error::Operation(format!("merging schema Types: {e}")))?;
    let target_library_names = ost_plugin::library_plugin_names(&target_src).unwrap_or_default();
    std::fs::write(target_pi.as_std_path(), merged)
        .map_err(|e| Error::io(target_pi.to_string(), e))?;
    let test_plug_infos = merge_schema_types_into_test_plug_infos(
        bundle,
        &target_pi,
        &target_library_names,
        &generated_src,
    )?;

    // Copy the flattened schema definition beside the plugInfo (registration needs it).
    if let Some(generated_schema) = &generated.generated_schema {
        let dest = bundle.plug_info_root().join("generatedSchema.usda");
        std::fs::copy(generated_schema.as_std_path(), dest.as_std_path())
            .map_err(|e| Error::io(dest.to_string(), e))?;
    }
    if generated.compiled_sources > 0 {
        println!("    merged schema Types into {target_pi} and linked compiled schema API");
    } else {
        println!("    merged schema Types into {target_pi}");
    }
    if test_plug_infos > 0 {
        println!("    merged schema Types into {test_plug_infos} test plugInfo.json file(s)");
    }
    Ok(())
}

fn reset_dir(dir: &Utf8Path) -> Result<()> {
    ost_core::fs::remove_dir_all_robust(dir.as_std_path())
        .map_err(|e| Error::io(dir.to_string(), e))?;
    std::fs::create_dir_all(dir.as_std_path()).map_err(|e| Error::io(dir.to_string(), e))
}

fn merge_schema_types_into_test_plug_infos(
    bundle: &Bundle,
    target_pi: &Utf8Path,
    target_library_names: &[String],
    generated_src: &str,
) -> Result<usize> {
    let tests_dir = bundle.path("tests");
    if !tests_dir.as_std_path().is_dir() || target_library_names.is_empty() {
        return Ok(0);
    }
    let mut plug_infos = Vec::new();
    collect_test_plug_infos(&tests_dir, &mut plug_infos)?;
    let mut merged_count = 0;
    for plug_info in plug_infos {
        if plug_info == target_pi || !is_known_test_registry_plug_info(bundle, &plug_info) {
            continue;
        }
        let target_src = std::fs::read_to_string(plug_info.as_std_path())
            .map_err(|e| Error::io(plug_info.to_string(), e))?;
        let candidate_library_names =
            ost_plugin::library_plugin_names(&target_src).map_err(|e| {
                Error::Operation(format!("reading library names from {plug_info}: {e}"))
            })?;
        if !library_names_overlap(target_library_names, &candidate_library_names) {
            continue;
        }
        let merged = ost_plugin::merge_schema_types(&target_src, generated_src)
            .map_err(|e| Error::Operation(format!("merging schema Types into {plug_info}: {e}")))?;
        std::fs::write(plug_info.as_std_path(), merged)
            .map_err(|e| Error::io(plug_info.to_string(), e))?;
        merged_count += 1;
    }
    Ok(merged_count)
}

fn is_known_test_registry_plug_info(bundle: &Bundle, plug_info: &Utf8Path) -> bool {
    plug_info.starts_with(bundle.path("tests/cmake"))
}

fn library_names_overlap(left: &[String], right: &[String]) -> bool {
    left.iter()
        .any(|name| right.iter().any(|other| other == name))
}

fn collect_test_plug_infos(dir: &Utf8Path, plug_infos: &mut Vec<Utf8PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir.as_std_path()).map_err(|e| Error::io(dir.to_string(), e))? {
        let entry = entry.map_err(|e| Error::io(dir.to_string(), e))?;
        let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|p| {
            Error::config(format!(
                "non-UTF-8 path in test plugInfo tree: {}",
                p.display()
            ))
        })?;
        let ty = entry
            .file_type()
            .map_err(|e| Error::io(path.to_string(), e))?;
        if ty.is_dir() {
            collect_test_plug_infos(&path, plug_infos)?;
        } else if ty.is_file() && path.file_name() == Some("plugInfo.json") {
            plug_infos.push(path);
        }
    }
    Ok(())
}

#[derive(Debug, Default)]
struct CompiledSchemaFiles {
    sources: usize,
    headers: usize,
}

fn stage_compiled_schema_sources(
    staging: &Utf8Path,
    compiled_dir: &Utf8Path,
    cmake_fragment: &Utf8Path,
) -> Result<CompiledSchemaFiles> {
    clear_cohosted_schema_compile_state(compiled_dir, cmake_fragment)?;

    let files = collect_compiled_schema_files(staging)?;
    if files.is_empty() {
        return Ok(CompiledSchemaFiles::default());
    }

    std::fs::create_dir_all(compiled_dir.as_std_path())
        .map_err(|e| Error::io(compiled_dir.to_string(), e))?;

    let mut staged = Vec::new();
    let mut counts = CompiledSchemaFiles::default();
    for rel in files {
        let src = staging.join(&rel);
        let dest = compiled_dir.join(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent.as_std_path())
                .map_err(|e| Error::io(parent.to_string(), e))?;
        }
        std::fs::copy(src.as_std_path(), dest.as_std_path())
            .map_err(|e| Error::io(format!("{src} -> {dest}"), e))?;
        if is_cxx_source(&dest) {
            counts.sources += 1;
        } else if is_cxx_header(&dest) {
            counts.headers += 1;
        }
        staged.push(dest);
    }

    write_schema_sources_fragment(compiled_dir, cmake_fragment, &staged)?;
    Ok(counts)
}

fn clear_cohosted_schema_compile_state(
    compiled_dir: &Utf8Path,
    cmake_fragment: &Utf8Path,
) -> Result<()> {
    if compiled_dir.as_std_path().exists() {
        std::fs::remove_dir_all(compiled_dir.as_std_path())
            .map_err(|e| Error::io(compiled_dir.to_string(), e))?;
    }
    if cmake_fragment.as_std_path().exists() {
        std::fs::remove_file(cmake_fragment.as_std_path())
            .map_err(|e| Error::io(cmake_fragment.to_string(), e))?;
    }
    Ok(())
}

fn collect_compiled_schema_files(root: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    let mut files = Vec::new();
    collect_compiled_schema_files_inner(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_compiled_schema_files_inner(
    root: &Utf8Path,
    dir: &Utf8Path,
    files: &mut Vec<Utf8PathBuf>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir.as_std_path()).map_err(|e| Error::io(dir.to_string(), e))? {
        let entry = entry.map_err(|e| Error::io(dir.to_string(), e))?;
        let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|p| {
            Error::config(format!(
                "non-UTF-8 path in generated schema output: {}",
                p.display()
            ))
        })?;
        let ty = entry
            .file_type()
            .map_err(|e| Error::io(path.to_string(), e))?;
        if ty.is_dir() {
            collect_compiled_schema_files_inner(root, &path, files)?;
        } else if ty.is_file() {
            let rel = path
                .strip_prefix(root)
                .map_err(|e| Error::Operation(format!("schema output path error: {e}")))?;
            if is_compiled_schema_file(root, rel) {
                files.push(rel.to_path_buf());
            }
        }
    }
    Ok(())
}

fn is_compiled_schema_file(root: &Utf8Path, rel: &Utf8Path) -> bool {
    let Some(name) = rel.file_name() else {
        return false;
    };
    if name == "module.cpp" || name == "generatedSchema.module.h" {
        return false;
    }
    if name.starts_with("wrap") && is_cxx_source(rel) && !has_matching_header(root, rel) {
        return false;
    }
    is_cxx_source(rel) || is_cxx_header(rel)
}

fn has_matching_header(root: &Utf8Path, rel: &Utf8Path) -> bool {
    ["h", "hpp", "hh"]
        .iter()
        .any(|ext| root.join(rel).with_extension(ext).as_std_path().is_file())
}

fn is_cxx_source(path: &Utf8Path) -> bool {
    matches!(path.extension(), Some("cpp" | "cxx" | "cc"))
}

fn is_cxx_header(path: &Utf8Path) -> bool {
    matches!(path.extension(), Some("h" | "hpp" | "hh"))
}

fn write_schema_sources_fragment(
    compiled_dir: &Utf8Path,
    cmake_fragment: &Utf8Path,
    staged: &[Utf8PathBuf],
) -> Result<()> {
    let source_files: Vec<&Utf8PathBuf> = staged.iter().filter(|p| is_cxx_source(p)).collect();
    let export_define = detect_schema_export_define(compiled_dir);

    let mut body = String::new();
    body.push_str("# Generated by `ost plugin build`; do not edit.\n");
    body.push_str("if(NOT DEFINED PLUGIN_NAME)\n");
    body.push_str("    message(FATAL_ERROR \"OPENSTRATA_SCHEMA_SOURCES_FILE requires PLUGIN_NAME to name the plugin target\")\n");
    body.push_str("endif()\n");
    body.push_str("target_include_directories(${PLUGIN_NAME} PRIVATE\n");
    body.push_str(&format!("    \"{}\"\n", cmake_path(compiled_dir)));
    body.push_str(")\n");
    if !source_files.is_empty() {
        body.push_str("target_sources(${PLUGIN_NAME} PRIVATE\n");
        for path in source_files {
            body.push_str(&format!("    \"{}\"\n", cmake_path(path)));
        }
        body.push_str(")\n");
    }
    if let Some(export_define) = export_define {
        body.push_str(&format!(
            "target_compile_definitions(${{PLUGIN_NAME}} PRIVATE {export_define})\n"
        ));
    }
    write_text(cmake_fragment, body.trim_end())
}

fn detect_schema_export_define(path: &Utf8Path) -> Option<String> {
    if path.as_std_path().is_dir() {
        for entry in std::fs::read_dir(path.as_std_path()).ok()? {
            let entry = entry.ok()?;
            let child = Utf8PathBuf::from_path_buf(entry.path()).ok()?;
            if let Some(found) = detect_schema_export_define(&child) {
                return Some(found);
            }
        }
        return None;
    }
    if path.file_name() != Some("api.h") {
        return None;
    }
    let src = std::fs::read_to_string(path.as_std_path()).ok()?;
    for marker in ["defined(", "#ifdef "] {
        let mut rest = src.as_str();
        while let Some(pos) = rest.find(marker) {
            let after = &rest[pos + marker.len()..];
            let candidate: String = after
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if candidate.ends_with("_EXPORTS") {
                return Some(candidate);
            }
            rest = after;
        }
    }
    None
}

fn schema_sources_dir(target_dir: &Utf8Path) -> Utf8PathBuf {
    target_dir.join("schema-sources")
}

fn cmake_path(path: &Utf8Path) -> String {
    path.to_string().replace('\\', "/")
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

fn report_package(
    id: &str,
    archive: &Utf8Path,
    packed: &ost_build::PackResult,
    warnings: &[serde_json::Value],
    fmt: Format,
) {
    if fmt.is_json() {
        output::report_with_warnings(
            true,
            &serde_json::json!({
                "packaged": true,
                "target": id,
                "archive": archive.to_string(),
                "archive_digest": packed.archive_digest,
                "archive_size": packed.archive_size,
                "files": packed.files.len(),
            }),
            warnings,
        );
        return;
    }
    for w in warnings {
        if let Some(msg) = w["message"].as_str() {
            eprintln!("warning: {msg}");
        }
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
        // The recorded OpenUSD version can be stale (a runtime adopted before the
        // version was derived from `pxr.h`), which makes the L1 range check pass
        // for the wrong reason. Prefer the install's actual `pxr.h` version when it
        // is present so the gate reflects the real runtime (dogfooding #1–#5).
        if let Some(real) = crate::commands::runtime::detect_openusd_version(&r.artifact_prefix) {
            ctx.openusd_version = Some(real);
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
///
/// `usdGenSchema` writes files through Python's text encoders, so force UTF-8 for
/// schema builds regardless of the host locale (notably Japanese Windows cp932).
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
    force_python_utf8(&mut env);
    env
}

fn force_python_utf8(env: &mut Vec<(String, String)>) {
    upsert_env(env, "PYTHONUTF8", "1");
    upsert_env(env, "PYTHONIOENCODING", "utf-8");
}

fn upsert_env(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some((existing_key, existing_value)) =
        env.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key))
    {
        existing_key.clear();
        existing_key.push_str(key);
        existing_value.clear();
        existing_value.push_str(value);
    } else {
        env.push((key.into(), value.into()));
    }
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

    /// A publishable plugin manifest, as `ost plugin package` writes it.
    fn publishable_manifest() -> serde_json::Value {
        serde_json::json!({
            "schema": 1,
            "kind": "openstrata.plugin-bundle",
            "plugin": { "name": "toy", "version": "0.1.0", "kind": "usd-fileformat", "license": "Apache-2.0" },
            "target": "cy2026-windows-x86_64-msvc143-py313-usd",
            "archive": "toy-0.1.0.tar.zst",
            "archive_digest": format!("sha256:{}", "ab".repeat(32)),
            "archive_size": 1,
            "total_size": 2,
            "provenance": {
                "profile": "usd",
                "cxx_abi": "msvc143",
                "runtime": { "id": "openstrata-cy2026-usd", "digest": "sha256:feed" },
                "validation": { "passed": true },
            },
            "files": [
                { "path": "NOTICE.md", "sha256": "sha256:aa", "size": 1 },
            ],
        })
    }

    #[test]
    fn publish_gates_accept_a_complete_artifact() {
        assert!(check_publishable(&publishable_manifest(), &["NOTICE.md".into()]).is_ok());
        // No declared notices is fine too — the gate is on *declared* files.
        assert!(check_publishable(&publishable_manifest(), &[]).is_ok());
    }

    #[test]
    fn publish_gates_refuse_incomplete_artifacts() {
        type Mutation = Box<dyn Fn(&mut serde_json::Value)>;
        let cases: Vec<(&str, Mutation)> = vec![
            (
                "PUBLISH_NOT_A_PLUGIN_BUNDLE",
                Box::new(|m| m["kind"] = "other".into()),
            ),
            (
                "PUBLISH_VALIDATION_REQUIRED",
                Box::new(|m| m["provenance"]["validation"]["passed"] = false.into()),
            ),
            (
                "PUBLISH_LICENSE_REQUIRED",
                Box::new(|m| m["plugin"]["license"] = serde_json::Value::Null),
            ),
            (
                "PUBLISH_PROVENANCE_INCOMPLETE",
                Box::new(|m| m["provenance"]["runtime"]["digest"] = "".into()),
            ),
            (
                "PUBLISH_ABI_UNRESOLVED",
                Box::new(|m| m["provenance"]["cxx_abi"] = "inherit".into()),
            ),
            (
                "PUBLISH_ABI_UNRESOLVED",
                Box::new(|m| {
                    m["provenance"]["cxx_abi"] = serde_json::json!({"windows": "msvc143"})
                }),
            ),
        ];
        for (code, mutate) in cases {
            let mut m = publishable_manifest();
            mutate(&mut m);
            let err = check_publishable(&m, &[]).expect_err(code);
            assert_eq!(err.code(), code);
            assert_eq!(err.category(), ost_core::Category::Validation);
        }

        // A declared notices file absent from the archive is refused.
        let err = check_publishable(&publishable_manifest(), &["THIRD_PARTY.md".into()])
            .expect_err("missing notices");
        assert_eq!(err.code(), "PUBLISH_NOTICES_MISSING");
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
    fn schema_build_env_forces_python_utf8() {
        let session = EnvSet {
            sep: ';',
            vars: Vec::new(),
        };
        let env = compose_build_env(
            &[
                ("PythonUtf8".into(), "0".into()),
                ("PYTHONIOENCODING".into(), "cp932".into()),
            ],
            &session,
        );

        assert_eq!(env_value(&env, "PYTHONUTF8"), Some("1"));
        assert_eq!(env_value(&env, "PYTHONIOENCODING"), Some("utf-8"));
        assert_eq!(
            env.iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("PYTHONUTF8"))
                .count(),
            1
        );
        assert_eq!(
            env.iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("PYTHONIOENCODING"))
                .count(),
            1
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

    #[test]
    fn compiled_schema_staging_keeps_typed_api_and_drops_python_helpers() {
        let root = unique_tmp("compiled-schema");
        let staging = root.join("raw");
        let compiled = root.join("compiled");
        let fragment = root.join("schema-sources.cmake");
        std::fs::create_dir_all(staging.as_std_path()).unwrap();

        write_test_file(
            &staging.join("api.h"),
            "#if defined(TOYSCHEMA_EXPORTS)\n#define TOY_API ARCH_EXPORT\n#endif\n",
        );
        write_test_file(&staging.join("tokens.cpp"), "void tokens() {}\n");
        write_test_file(&staging.join("tokens.h"), "#pragma once\n");
        write_test_file(&staging.join("ToyAPI.cpp"), "void api() {}\n");
        write_test_file(&staging.join("ToyAPI.h"), "#pragma once\n");
        write_test_file(&staging.join("wrapBehavior.cpp"), "void wrapped() {}\n");
        write_test_file(&staging.join("wrapBehavior.h"), "#pragma once\n");
        write_test_file(&staging.join("wrapToyAPI.cpp"), "void py() {}\n");
        write_test_file(&staging.join("module.cpp"), "void module() {}\n");
        write_test_file(&staging.join("generatedSchema.module.h"), "#pragma once\n");
        write_test_file(&staging.join("plugInfo.json"), "{}\n");
        write_test_file(&staging.join("generatedSchema.usda"), "#usda 1.0\n");

        let counts = stage_compiled_schema_sources(&staging, &compiled, &fragment).expect("stages");

        assert_eq!(counts.sources, 3);
        assert_eq!(counts.headers, 4);
        assert!(compiled.join("tokens.cpp").as_std_path().is_file());
        assert!(compiled.join("ToyAPI.cpp").as_std_path().is_file());
        assert!(compiled.join("wrapBehavior.cpp").as_std_path().is_file());
        assert!(!compiled.join("wrapToyAPI.cpp").as_std_path().exists());
        assert!(!compiled.join("module.cpp").as_std_path().exists());
        assert!(!compiled
            .join("generatedSchema.module.h")
            .as_std_path()
            .exists());

        let cmake = std::fs::read_to_string(fragment.as_std_path()).unwrap();
        assert!(cmake.contains("target_sources(${PLUGIN_NAME} PRIVATE"));
        assert!(cmake.contains("tokens.cpp"));
        assert!(cmake.contains("ToyAPI.cpp"));
        assert!(cmake.contains("wrapBehavior.cpp"));
        assert!(!cmake.contains("wrapToyAPI.cpp"));
        assert!(!cmake.contains("module.cpp"));
        assert!(
            cmake.contains("target_compile_definitions(${PLUGIN_NAME} PRIVATE TOYSCHEMA_EXPORTS)")
        );

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn codeless_schema_output_clears_stale_compiled_fragment() {
        let root = unique_tmp("codeless-schema");
        let staging = root.join("raw");
        let compiled = root.join("compiled");
        let fragment = root.join("schema-sources.cmake");
        std::fs::create_dir_all(staging.as_std_path()).unwrap();
        std::fs::create_dir_all(compiled.as_std_path()).unwrap();
        write_test_file(&compiled.join("stale.cpp"), "void stale() {}\n");
        write_test_file(
            &fragment,
            "target_sources(${PLUGIN_NAME} PRIVATE stale.cpp)\n",
        );
        write_test_file(&staging.join("plugInfo.json"), "{}\n");
        write_test_file(&staging.join("generatedSchema.usda"), "#usda 1.0\n");

        let counts = stage_compiled_schema_sources(&staging, &compiled, &fragment).expect("stages");

        assert_eq!(counts.sources, 0);
        assert_eq!(counts.headers, 0);
        assert!(!compiled.as_std_path().exists());
        assert!(!fragment.as_std_path().exists());

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn schema_resource_merge_updates_only_matching_test_registries() {
        let root = unique_tmp("schema-resource-merge");
        write_test_file(
            &root.join("openstrata.plugin.yaml"),
            "plugin:\n  name: toy\n  version: 0.1.0\n  kind: usd-fileformat\n\
             runtime:\n  openusd: \">=25.05,<27.0\"\n\
             provides:\n  - usd-fileformat:toy\n  - usd-schema:ToyAPI\n\
             usd:\n  plug_info: plugin/resources/toy/plugInfo.json\n",
        );
        let target_plug_info = r#"{
            "Plugins": [
                { "Type": "library", "Name": "toy",
                  "Info": { "Types": { "ToyFileFormat": { "bases": ["SdfFileFormat"] } } } }
            ]
        }"#;
        write_test_file(
            &root.join("plugin/resources/toy/plugInfo.json"),
            target_plug_info,
        );
        write_test_file(&root.join("tests/cmake/plugInfo.json"), target_plug_info);
        write_test_file(&root.join("tests/fixtures/plugInfo.json"), target_plug_info);
        write_test_file(
            &root.join("tests/cmake/secondary/plugInfo.json"),
            r#"{
                "Plugins": [
                    { "Type": "library", "Name": "other",
                      "Info": { "Types": { "OtherFileFormat": { "bases": ["SdfFileFormat"] } } } }
                ]
            }"#,
        );

        let generated_plug_info = root.join("raw/plugInfo.json");
        let generated_schema = root.join("raw/generatedSchema.usda");
        write_test_file(
            &generated_plug_info,
            r#"{
                "Plugins": [
                    { "Info": { "Types": {
                        "ToyAPI": {
                            "schemaIdentifier": "API",
                            "schemaKind": "singleApplyAPI",
                            "bases": ["UsdAPISchemaBase"]
                        }
                    } } }
                ]
            }"#,
        );
        write_test_file(&generated_schema, "#usda 1.0\n");

        let bundle = Bundle::load(&root).expect("bundle loads");
        let generated = CohostedSchemaGeneration {
            generated_plug_info,
            generated_schema: Some(generated_schema),
            compiled_sources: 1,
        };

        merge_cohosted_schema_resources(&bundle, &generated).expect("merges resources");

        let target = std::fs::read_to_string(bundle.plug_info().as_std_path()).unwrap();
        let test =
            std::fs::read_to_string(root.join("tests/cmake/plugInfo.json").as_std_path()).unwrap();
        for src in [target, test] {
            let value: serde_json::Value = serde_json::from_str(&src).unwrap();
            let types = value["Plugins"][0]["Info"]["Types"].as_object().unwrap();
            assert!(types.contains_key("ToyFileFormat"));
            assert!(types.contains_key("ToyAPI"));
        }
        for path in [
            root.join("tests/fixtures/plugInfo.json"),
            root.join("tests/cmake/secondary/plugInfo.json"),
        ] {
            let src = std::fs::read_to_string(path.as_std_path()).unwrap();
            let value: serde_json::Value = serde_json::from_str(&src).unwrap();
            let types = value["Plugins"][0]["Info"]["Types"].as_object().unwrap();
            assert!(!types.contains_key("ToyAPI"));
        }
        assert!(bundle
            .plug_info_root()
            .join("generatedSchema.usda")
            .as_std_path()
            .is_file());

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    fn env_value<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
        env.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    fn write_test_file(path: &Utf8Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent.as_std_path()).unwrap();
        }
        std::fs::write(path.as_std_path(), contents).unwrap();
    }

    fn unique_tmp(tag: &str) -> Utf8PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut dir = Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
        dir.push(format!("ost-cli-{tag}-{}-{nanos}", std::process::id()));
        dir
    }

    /// A `package` rerun resets the previous stage. On Windows the staged
    /// copies keep the source's read-only attribute, which used to fail the
    /// reset with access-denied (dogfooding report #8); `reset_dir` must clear
    /// it and proceed.
    #[test]
    fn reset_dir_survives_readonly_stage_entries() {
        let stage = unique_tmp("stage");
        let file = stage.join("resources").join("plugInfo.json");
        std::fs::create_dir_all(file.parent().unwrap().as_std_path()).unwrap();
        std::fs::write(file.as_std_path(), "{}").unwrap();
        let mut perms = std::fs::metadata(file.as_std_path()).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(file.as_std_path(), perms).unwrap();

        reset_dir(&stage).expect("reset over a read-only staged file");
        assert!(stage.as_std_path().is_dir(), "stage was recreated");
        assert!(!file.as_std_path().exists(), "old contents were removed");

        std::fs::remove_dir_all(stage.as_std_path()).unwrap();
    }
}
