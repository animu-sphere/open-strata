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

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};
use clap::Subcommand;
use serde::Deserialize;

use ost_build::{
    pack_dir_with, stage_files, BuildCompletion, BuildIntent, BuildOutput, BuildProjectIdentity,
    CMakeCacheEntry, LeaseMode, PackOptions, TargetLease, TargetLock, BUILD_COMPLETION_FILE,
    TARGET_BUSY_CODE, TARGET_LEASE_FILE,
};
use ost_core::fs::write_atomic;
use ost_core::host::Os;
use ost_core::paths::{find_project_root, PROJECT_MANIFEST, STATE_DIR};
use ost_core::template::SCAFFOLD_PROVENANCE;
use ost_core::variant::{Abi, Variant};
use ost_core::{tools, Category, Error, Host, Result};
use ost_plugin::{
    adjacent_golden, default_template_id, diagnose, fixture_identifier, run_levels,
    scaffold_with_template_inputs, session_env_with, usdview_check, Bundle, CxxAbi, DoctorReport,
    ExecTemplateInputs, Library, PluginKind, PluginVerification, Probe, RuntimeContext, Session,
    Status, ToolOutput, PLUGIN_VERIFICATION, PLUGIN_VERIFICATION_SCHEMA,
};
use ost_runtime::{EnvSet, ProfileCatalog, RuntimeManifest, MANIFEST_FILE};

use crate::commands::compiler::{self, CompilerOpts};
use crate::commands::configure::{build_target, load_project};
use crate::commands::resolve;
use crate::output::{self, Format};

#[derive(Debug, Subcommand)]
pub enum PluginCmd {
    /// Scaffold a new plugin bundle from a template.
    New {
        /// Plugin kind: usd-fileformat | usd-asset-resolver |
        /// usd-package-resolver | usd-exec | usd-schema.
        kind: String,
        /// Plugin name (becomes the bundle directory), e.g. `toy`.
        name: String,
        /// File extension the plugin handles (required for usd-fileformat and
        /// usd-package-resolver).
        #[arg(long)]
        extension: Option<String>,
        /// URI scheme the resolver handles (required for usd-asset-resolver).
        #[arg(long)]
        scheme: Option<String>,
        /// Public schema bundle whose contract the OpenExec plugin consumes
        /// (required for usd-exec).
        #[arg(long)]
        schema_bundle: Option<String>,
        /// C++ schema type used by EXEC_REGISTER_COMPUTATIONS_FOR_SCHEMA
        /// (required for usd-exec), e.g. VrmSchemaContractAPI.
        #[arg(long)]
        schema_type: Option<String>,
        /// Catalog template id. usd-schema defaults to usd-schema-codeless;
        /// use usd-schema-cpp for the experimental compiled skeleton.
        #[arg(long)]
        template: Option<String>,
        /// Destination directory. Defaults to ./<name>.
        #[arg(long)]
        dir: Option<String>,
    },
    /// Report a bundle's Level 0 structure.
    Inspect {
        /// Path to the bundle directory (containing openstrata.plugin.yaml).
        bundle: String,
        /// Fail unless the manifest declares exactly this plugin version
        /// (generated release gates pin the tag's version here).
        #[arg(long)]
        expect_version: Option<String>,
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
        /// Path to the bundle directory (omit with --workspace).
        bundle: Option<String>,
        /// Package every discovered bundle, in dependency order, using the same
        /// validated graph `plugin test --workspace` checks.
        #[arg(long)]
        workspace: bool,
        /// Also emit one aggregate product artifact containing the exact member
        /// archives, manifests, checksums and evidence in dependency order.
        /// Requires --workspace.
        #[arg(long)]
        product: bool,
        /// Platform target, e.g. `cy2026`. Defaults to the enclosing project's.
        #[arg(long)]
        target: Option<String>,
        /// Profile to package against. Defaults to the enclosing project's.
        #[arg(long)]
        profile: Option<String>,
        /// Reclaim the stable package stage harder and sweep stale fallback
        /// stages a previous locked run left behind, instead of quietly staging
        /// into another sibling. Use once the holding process has exited.
        #[arg(long)]
        clean_stage: bool,
        /// Ship debug symbols (`.pdb`, `.dwo`) *inside* the main package instead
        /// of the default lean package. By default the main archive is lean and
        /// any debug symbols are split into a sibling `*-debug` package.
        #[arg(long)]
        with_debug: bool,
        /// Package outputs that no longer match the last `ost plugin build`,
        /// recording the package origin as an explicit unmanaged override.
        /// Without this flag, a managed-output mismatch fails closed.
        #[arg(long)]
        allow_unmanaged_output: bool,
    },
    /// Verify or install an aggregate plugin product artifact.
    #[command(subcommand)]
    Product(ProductCmd),
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
        /// External installed/extracted plugin tree(s) to put on the discovery
        /// path — an extracted package root (holds `openstrata.plugin.yaml`), not
        /// the source bundle. Use with `--no-inject` to run a clean-install /
        /// discovery test against the shipped layout rather than the build tree.
        #[arg(long = "plugin-path")]
        plugin_path: Vec<String>,
        /// Do not inject the source bundle's own build-tree plugInfo/lib/python
        /// paths. The session becomes the bare runtime env plus any
        /// `--plugin-path` / `--with` trees — an `ost runtime run`-style USD-only
        /// session. The bundle argument then only selects the runtime.
        #[arg(long)]
        no_inject: bool,
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
        /// Validate the dependency graph, then test every discovered bundle:
        /// immediate subdirectories and plugins/* with a plugin manifest.
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
        /// Test the *packaged* artifact, not the build tree: extract the
        /// already-built `ost plugin package` output to a clean directory and run
        /// discovery / open / validate against it. Catches a build-tree path
        /// baked into `plugInfo`/`LibraryPath` that source-tree discovery cannot
        /// see. Requires a prior `ost plugin package`. Composes with
        /// `--workspace`, which extracts every bundle and tests each against its
        /// dependencies' *extracted* trees rather than their source directories.
        #[arg(long)]
        from_package: bool,
        /// Validate the workspace dependency graph and stop, exiting on that
        /// result alone. Needs no build, no runtime, and no packaged artifact,
        /// so it runs as an early PR gate in milliseconds. Requires --workspace.
        ///
        /// The flags that select or extend what gets *tested* have nothing to
        /// act on here, so naming one is a usage error rather than a silent
        /// no-op.
        #[arg(
            long,
            requires = "workspace",
            conflicts_with_all = ["from_package", "with", "target", "profile", "up_to"]
        )]
        graph_only: bool,
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

#[derive(Debug, Subcommand)]
pub enum ProductCmd {
    /// Verify the product archive and every member archive/checksum.
    Verify {
        /// Product dist directory, manifest.json, or product .tar.zst archive.
        product: String,
        /// Require the outer product archive to have this full sha256 digest.
        #[arg(long)]
        expect_digest: Option<String>,
    },
    /// Verify and install every member in dependency order.
    Install {
        /// Product dist directory, manifest.json, or product .tar.zst archive.
        product: String,
        /// New installation root. The command refuses to overwrite it.
        #[arg(long)]
        prefix: String,
        /// Require the outer product archive to have this full sha256 digest.
        #[arg(long)]
        expect_digest: Option<String>,
    },
}

pub fn run(cmd: PluginCmd, fmt: Format) -> Result<()> {
    match cmd {
        PluginCmd::New {
            kind,
            name,
            extension,
            scheme,
            schema_bundle,
            schema_type,
            template,
            dir,
        } => new(
            NewPluginArgs {
                kind: &kind,
                name: &name,
                extension: extension.as_deref(),
                scheme: scheme.as_deref(),
                schema_bundle: schema_bundle.as_deref(),
                schema_type: schema_type.as_deref(),
                template: template.as_deref(),
                dir: dir.as_deref(),
            },
            fmt,
        ),
        PluginCmd::Inspect {
            bundle,
            expect_version,
        } => inspect(&bundle, expect_version.as_deref(), fmt),
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
            workspace,
            product,
            target,
            profile,
            clean_stage,
            with_debug,
            allow_unmanaged_output,
        } => match (workspace, product, bundle) {
            (true, _, Some(_)) => Err(Error::usage(
                "--workspace discovers bundles itself — drop the bundle path",
            )),
            (true, product, None) => package_workspace(
                target,
                profile,
                clean_stage,
                with_debug,
                allow_unmanaged_output,
                product,
                fmt,
            ),
            (false, true, _) => Err(Error::usage(
                "--product aggregates a workspace — pass it together with --workspace",
            )),
            (false, false, Some(bundle)) => package(
                &bundle,
                target,
                profile,
                clean_stage,
                with_debug,
                allow_unmanaged_output,
                fmt,
            ),
            (false, false, None) => Err(Error::usage(
                "missing bundle path (or pass --workspace to package every bundle)",
            )),
        },
        PluginCmd::Product(command) => product(command, fmt),
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
            plugin_path,
            no_inject,
            target,
            profile,
            command,
        } => run_session(
            &bundle,
            &with,
            &plugin_path,
            no_inject,
            target,
            profile,
            command,
            fmt,
        ),
        PluginCmd::Test {
            bundle,
            workspace,
            with,
            target,
            profile,
            up_to,
            from_package,
            graph_only,
        } => match (workspace, bundle) {
            (true, Some(_)) => Err(Error::usage(
                "--workspace discovers bundles itself — drop the bundle path",
            )),
            (true, None) if graph_only => validate_workspace_graph(fmt),
            (true, None) if from_package => {
                test_workspace_from_package(&with, target, profile, up_to, fmt)
            }
            (true, None) => test_workspace(&with, target, profile, up_to, fmt),
            (false, Some(bundle)) if from_package => {
                test_from_package(&bundle, &with, target, profile, up_to, fmt)
            }
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

struct NewPluginArgs<'a> {
    kind: &'a str,
    name: &'a str,
    extension: Option<&'a str>,
    scheme: Option<&'a str>,
    schema_bundle: Option<&'a str>,
    schema_type: Option<&'a str>,
    template: Option<&'a str>,
    dir: Option<&'a str>,
}

fn new(args: NewPluginArgs<'_>, fmt: Format) -> Result<()> {
    let NewPluginArgs {
        kind,
        name,
        extension,
        scheme,
        schema_bundle,
        schema_type,
        template,
        dir,
    } = args;
    let kind = PluginKind::from_tag(kind).ok_or_else(|| {
        let kinds: Vec<&str> = PluginKind::ALL.iter().map(|k| k.as_str()).collect();
        Error::usage(format!(
            "unknown plugin kind '{kind}' (expected one of: {})",
            kinds.join(", ")
        ))
    })?;

    let dest = Utf8PathBuf::from(dir.unwrap_or(name));
    let template_id = template.unwrap_or_else(|| default_template_id(kind));
    let exec = match (schema_bundle, schema_type) {
        (Some(schema_bundle), Some(schema_type)) => Some(ExecTemplateInputs {
            schema_bundle,
            schema_type,
        }),
        (None, None) => None,
        _ => {
            return Err(Error::usage(
                "--schema-bundle and --schema-type must be provided together",
            ))
        }
    };
    let files =
        scaffold_with_template_inputs(kind, name, extension, scheme, template, exec, &dest)?;

    if fmt.is_json() {
        output::success(&serde_json::json!({
            "created": true,
            "kind": kind.as_str(),
            "name": name,
            "template": template_id,
            "dir": dest.to_string(),
            "workspace_template": "usd-plugin-workspace",
            "workspace_command": "ost init --template usd-plugin-workspace",
            "files": files.iter().map(|f| f.to_string()).collect::<Vec<_>>(),
        }));
        return Ok(());
    }

    println!(
        "Created {} plugin '{name}' from {template_id} in {dest}/",
        kind.as_str()
    );
    for f in &files {
        println!("  {f}");
    }
    println!("\nNext:");
    println!("  ost plugin inspect {dest}");
    println!("  ost plugin doctor {dest}");
    println!("  multi-bundle repo root: ost init --template usd-plugin-workspace");
    Ok(())
}

fn inspect(bundle_path: &str, expect_version: Option<&str>, fmt: Format) -> Result<()> {
    let bundle = load_bundle(bundle_path)?;
    // A typed release gate: generated trusted-release workflows compare the
    // tag's version against the manifest here instead of scraping JSON output.
    if let Some(expected) = expect_version {
        let declared = &bundle.manifest.plugin.version;
        if declared != expected {
            return Err(Error::coded(
                "PLUGIN_VERSION_MISMATCH",
                Category::Validation,
                format!("bundle declares version '{declared}', expected '{expected}'"),
            ));
        }
    }
    // Level 0 only: bundle structure, no runtime resolution.
    let report = diagnose(&bundle, &RuntimeContext::default(), 0);
    let libraries = selected_workspace_library_evidence(&bundle, None)?;

    if fmt.is_json() {
        let mut body = ost_plugin::report_json(&bundle, &report);
        if !libraries.is_empty() {
            body["libraries"] = serde_json::Value::Array(libraries);
        }
        output::report(report.passed(), &body);
    } else {
        print_report(&bundle, &report);
        for library in &libraries {
            println!(
                "  library: {} {} -> {}",
                library["id"].as_str().unwrap_or("?"),
                library["version"].as_str().unwrap_or("?"),
                library["cmake_target"].as_str().unwrap_or("?")
            );
        }
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
    let dependencies = selected_workspace_dependencies(&bundle)?;
    let explicit = load_with_bundles(with_paths)?;
    let with_bundles = merge_composed_bundles(&bundle, dependencies, explicit)?;
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
    let primary = load_bundle(bundle_path)?;
    let Some(workspace) = source_workspace_for(&primary)? else {
        return build_one(
            bundle_path,
            target,
            profile,
            dry_run,
            ninja,
            compiler_opts,
            None,
            false,
            true,
            fmt,
        );
    };
    let dependencies = dependencies_from_workspace(&primary, &workspace)?;
    let libraries = libraries_from_workspace(&primary, &workspace)?;
    if dependencies.is_empty() && libraries.is_empty() {
        return build_one(
            bundle_path,
            target,
            profile,
            dry_run,
            ninja,
            compiler_opts,
            None,
            false,
            true,
            fmt,
        );
    }

    let (platform, selected_profile) =
        selection(target.clone(), profile.clone()).ok_or_else(|| {
            Error::usage(
                "no platform/profile: run inside an OpenStrata project or pass --target/--profile",
            )
        })?;
    let (tgt, _) = build_target(&platform, &selected_profile)?;
    let prefix = workspace
        .root
        .join(STATE_DIR)
        .join("targets")
        .join(tgt.id())
        .join("workspace-prefix");
    if !dry_run {
        if prefix.as_std_path().exists() {
            std::fs::remove_dir_all(prefix.as_std_path())
                .map_err(|e| Error::io(prefix.to_string(), e))?;
        }
        std::fs::create_dir_all(prefix.as_std_path())
            .map_err(|e| Error::io(prefix.to_string(), e))?;
    }

    if !fmt.is_json() {
        println!(
            "Workspace composition: {} bundle dependenc{}, {} librar{} -> {}",
            dependencies.len(),
            if dependencies.len() == 1 { "y" } else { "ies" },
            libraries.len(),
            if libraries.len() == 1 { "y" } else { "ies" },
            prefix
        );
    }
    for library in &libraries {
        if !fmt.is_json() {
            println!("\n== build library {} ==", library.id());
        }
        build_library_one(
            library,
            target.clone(),
            profile.clone(),
            dry_run,
            ninja.clone(),
            compiler_opts.clone(),
            &prefix,
        )?;
    }
    for dependency in &dependencies {
        if !fmt.is_json() {
            println!("\n== build dependency {} ==", dependency.manifest.name());
        }
        build_one(
            dependency.root.as_str(),
            target.clone(),
            profile.clone(),
            dry_run,
            ninja.clone(),
            compiler_opts.clone(),
            Some(&prefix),
            true,
            false,
            fmt,
        )?;
    }
    if !fmt.is_json() {
        println!("\n== build primary {} ==", primary.manifest.name());
    }
    build_one(
        primary.root.as_str(),
        target,
        profile,
        dry_run,
        ninja,
        compiler_opts,
        Some(&prefix),
        false,
        true,
        fmt,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_library_one(
    library: &Library,
    target: Option<String>,
    profile: Option<String>,
    dry_run: bool,
    ninja: Option<String>,
    compiler_opts: CompilerOpts,
    workspace_prefix: &Utf8Path,
) -> Result<()> {
    let (platform, profile) = selection(target, profile).ok_or_else(|| {
        Error::usage(
            "no platform/profile: run inside an OpenStrata project or pass --target/--profile",
        )
    })?;
    let (tgt, r) = build_target(&platform, &profile)?;
    let id = tgt.id();
    let compiler = resolve_plugin_compiler(&library.root, &compiler_opts)?;
    let target_dir = target_state_dir(&library.root, &id);
    let toolchain = target_dir.join("toolchain.cmake");
    let python = ost_build::resolve_for_runtime(&r.artifact_prefix, &tgt.python_version);
    let build_dir = target_build_dir(&library.root, &id);
    let cmake = tools::which("cmake");
    let ninja = ninja.map(PathBuf::from).or_else(|| tools::which("ninja"));
    let mut configure_args = vec![
        "-S".to_string(),
        cmake_path(&library.root),
        "-B".to_string(),
        cmake_path(&build_dir),
        "-G".to_string(),
        "Ninja".to_string(),
        format!("-DCMAKE_TOOLCHAIN_FILE={}", cmake_path(&toolchain)),
        "-DCMAKE_BUILD_TYPE=Release".to_string(),
        format!("-DCMAKE_INSTALL_PREFIX={}", cmake_path(workspace_prefix)),
    ];
    if let Some(ninja) = &ninja {
        configure_args.push(format!(
            "-DCMAKE_MAKE_PROGRAM={}",
            ninja.display().to_string().replace('\\', "/")
        ));
    }
    let build_args = vec!["--build".to_string(), cmake_path(&build_dir)];
    let install_args = vec![
        "--install".to_string(),
        cmake_path(&build_dir),
        "--prefix".to_string(),
        cmake_path(workspace_prefix),
        "--config".to_string(),
        "Release".to_string(),
    ];
    if dry_run {
        println!("# dry run — would generate {toolchain} then:");
        if tgt.os() == Os::Windows && tools::which("cl").is_none() {
            println!("# (would auto-load the MSVC environment via vcvars64.bat)");
        }
        println!("cmake {}", configure_args.join(" "));
        println!("cmake {}", build_args.join(" "));
        println!("cmake {}", install_args.join(" "));
        return Ok(());
    }
    std::fs::create_dir_all(target_dir.as_std_path())
        .map_err(|error| Error::io(target_dir.to_string(), error))?;
    crate::commands::relocate_baked_python_if_stale(&r.artifact_prefix, python.as_ref());
    let mut toolchain_text =
        ost_build::render_toolchain(&tgt, &r.artifact_prefix, &compiler, python.as_ref());
    toolchain_text.push_str(&format!(
        "\n# Source-workspace library install prefix.\nlist(PREPEND CMAKE_PREFIX_PATH \"{}\")\n",
        cmake_path(workspace_prefix)
    ));
    std::fs::write(toolchain.as_std_path(), format!("{toolchain_text}\n"))
        .map_err(|error| Error::io(toolchain.to_string(), error))?;
    if !r.pulled {
        return Err(Error::coded(
            "RUNTIME_NOT_FOUND",
            Category::Precondition,
            format!(
                "runtime '{}' not pulled — run `ost runtime pull {platform} --profile {profile}` first",
                tgt.runtime_id
            ),
        ));
    }
    let cmake = cmake.ok_or_else(|| {
        Error::coded(
            "REQUIRED_TOOL_MISSING",
            Category::Precondition,
            "`cmake` not found on PATH",
        )
    })?;
    let lock_compiler = compiler::to_lock(&compiler, &r.artifact_prefix, tgt.os());
    invalidate_plugin_build_tree_if_compiler_changed(&library.root, &id, &lock_compiler);
    let build_env = maybe_bootstrap_msvc(tgt.os());
    run_step(PHASE_CONFIGURE, &cmake, &configure_args, &build_env)?;
    run_step(PHASE_COMPILE_LINK, &cmake, &build_args, &build_env)?;
    run_step("workspace-install", &cmake, &install_args, &build_env)?;
    let record = target_dir.join("compiler.lock.json");
    if let Ok(json) = serde_json::to_string_pretty(&lock_compiler) {
        let _ = std::fs::write(record.as_std_path(), json);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_one(
    bundle_path: &str,
    target: Option<String>,
    profile: Option<String>,
    dry_run: bool,
    ninja: Option<String>,
    compiler_opts: CompilerOpts,
    workspace_prefix: Option<&Utf8Path>,
    install_to_workspace: bool,
    emit_result: bool,
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
    // A plugin build publishes the same authoritative completion evidence as a
    // project build, so it must obey the same single-writer rule. Hold the
    // target lease from the first generated target file through completion
    // publication; otherwise two plugin builds can hash outputs from different
    // invocations into one apparently managed record.
    let lease = TargetLease::acquire(
        &target_dir.join(TARGET_LEASE_FILE),
        &id,
        "ost plugin build",
        LeaseMode::Fail,
    )
    .map_err(|error| {
        if error.code() == TARGET_BUSY_CODE {
            error
                .with_hint("wait for the in-flight plugin build to finish, then retry this command")
        } else {
            error
        }
    })?;
    std::fs::create_dir_all(target_dir.as_std_path())
        .map_err(|e| Error::io(target_dir.to_string(), e))?;
    let toolchain = target_dir.join("toolchain.cmake");
    // Pin a host interpreter's Development artifacts so an adopted runtime's
    // pxrConfig (which bakes the export machine's Python paths) configures on
    // this host. `None` when none matches — the toolchain then falls back to
    // the runtime prefix, unchanged from before.
    let python = ost_build::resolve_for_runtime(&r.artifact_prefix, &tgt.python_version);
    crate::commands::relocate_baked_python_if_stale(&r.artifact_prefix, python.as_ref());
    let mut toolchain_text =
        ost_build::render_toolchain(&tgt, &r.artifact_prefix, &compiler, python.as_ref());
    if let Some(prefix) = workspace_prefix {
        toolchain_text.push_str(&format!(
            "\n# Source-workspace dependency install prefix.\nlist(PREPEND CMAKE_PREFIX_PATH \"{}\")\n",
            cmake_path(prefix)
        ));
    }
    std::fs::write(toolchain.as_std_path(), format!("{toolchain_text}\n"))
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
    // Only a dependency being installed into the workspace prefix configures
    // with it; the primary consumes the prefix (via CMAKE_PREFIX_PATH in the
    // toolchain) but keeps its own install destination untouched.
    let install_prefix = workspace_prefix.filter(|_| install_to_workspace);
    if let Some(prefix) = install_prefix {
        configure_args.push(format!("-DCMAKE_INSTALL_PREFIX={}", cmake_path(prefix)));
    }
    let schema_sources_file = target_dir.join("schema-sources.cmake");
    let cohosts_schema = bundle.manifest.kind() != PluginKind::UsdSchema
        && !bundle.manifest.schema_provides().is_empty();
    if cohosts_schema {
        configure_args.push(format!(
            "-DOPENSTRATA_SCHEMA_SOURCES_FILE={}",
            cmake_path(&schema_sources_file)
        ));
    } else if !dry_run {
        // A bundle may have changed shape since the last configure. Remove both
        // the generated fragment and sources so a stale CMake cache cannot keep
        // compiling a schema the manifest no longer owns/co-hosts.
        clear_cohosted_schema_compile_state(
            &schema_sources_dir(&target_dir),
            &schema_sources_file,
        )?;
    }
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
    let install_args = install_prefix.map(|prefix| {
        vec![
            "--install".to_string(),
            build_dir.to_string().replace('\\', "/"),
            "--prefix".to_string(),
            cmake_path(prefix),
            "--config".to_string(),
            "Release".to_string(),
        ]
    });

    if dry_run {
        println!("# dry run — would generate {toolchain} then:");
        if tgt.os() == Os::Windows && tools::which("cl").is_none() {
            println!("# (would auto-load the MSVC environment via vcvars64.bat)");
        }
        println!("cmake {}", configure_args.join(" "));
        println!("cmake {}", build_args.join(" "));
        if let Some(args) = &install_args {
            println!("cmake {}", args.join(" "));
        }
        lease.release();
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
    let is_schema_build = bundle.manifest.kind() == PluginKind::UsdSchema || cohosts_schema;
    let build_env = if is_schema_build {
        let session = session_env_with(&r.env, &bundle, &[], tgt.os());
        compose_build_env(&msvc_env, &session)
    } else {
        msvc_env.clone()
    };

    // The schema-generation step (`usdGenSchema`) must resolve the *base* USD
    // schemas through the runtime's plugin registry but must NOT discover the
    // bundle's own plugInfo — that would make USD try to load the plugin library
    // this build has not produced yet (or an old one with the wrong platform
    // suffix) and fail. So compose its env from the runtime session alone, with
    // the bundle's `PXR_PLUGINPATH_NAME`/lib entries scoped out.
    let schema_gen_env = compose_build_env(&msvc_env, &r.env);

    // A non-schema bundle can co-host a schema by declaring `usd-schema:*` and
    // shipping schema.usda. Generate it before configure so any compiled C++ API
    // sources can be included in the same plugin library. The plugInfo merge is
    // delayed until after configure because the template regenerates
    // plugInfo.json from plugInfo.json.in during configure.
    let cohosted_schema = if cohosts_schema {
        prepare_cohosted_schema(
            &bundle,
            &r.artifact_prefix,
            &tgt.python_version,
            &target_dir.join("schema-gen"),
            &schema_sources_dir(&target_dir),
            &schema_sources_file,
            &schema_gen_env,
        )
        .map_err(|e| in_phase(PHASE_SCHEMA_GENERATE, e))?
    } else {
        None
    };

    run_step(PHASE_CONFIGURE, &cmake, &configure_args, &build_env)?;
    run_step(PHASE_COMPILE_LINK, &cmake, &build_args, &build_env)?;
    if let Some(args) = &install_args {
        run_step("workspace-install", &cmake, args, &build_env)?;
    }

    if let Some(schema) = &cohosted_schema {
        merge_cohosted_schema_resources(&bundle, schema)
            .map_err(|e| in_phase(PHASE_SCHEMA_MERGE, e))?;
    }

    // The plugInfo the runtime will dlopen at registration/test time must name a
    // library with *this* platform's suffix. A committed plugInfo carrying
    // another platform's suffix (a `.dll` on macOS) would otherwise fail later
    // with USD's opaque loader error; fail here with the exact fix instead. A
    // source bundle shipping `plugInfo.json.in` has already had this regenerated
    // per target by configure, so its concrete path is correct by construction.
    verify_target_library_suffix(&bundle, tgt.os())?;

    // Keep the compiler fingerprint beside the toolchain so the next invocation
    // can invalidate CMake's compiler-cached build tree before configuring.
    let record = target_dir.join("compiler.lock.json");
    if let Ok(json) = serde_json::to_string_pretty(&lock_compiler) {
        let _ = std::fs::write(record.as_std_path(), json);
    }
    let completion = write_plugin_build_completion(
        &bundle,
        &tgt,
        &lock_compiler,
        &toolchain,
        &build_dir,
        lease.invocation(),
    )?;
    lease.release();

    if !emit_result {
        return Ok(());
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
            "workspace_prefix": workspace_prefix.map(ToString::to_string),
            "build_completion": build_dir.join(BUILD_COMPLETION_FILE).to_string(),
            "build_fingerprint": completion.fingerprint(),
            "managed_outputs": completion.outputs.len(),
        }));
        return Ok(());
    }
    println!(
        "\nBuilt {} against {}",
        bundle.manifest.plugin.name, tgt.runtime_id
    );
    println!("  lib:       {}", bundle.lib_dir());
    println!("  plugInfo:  {plug_info}");
    println!(
        "  provenance: {} managed output(s)",
        completion.outputs.len()
    );
    Ok(())
}

/// Publish one managed completion only after configure, compile/link, optional
/// install, schema merge, and output validation have all succeeded. The
/// configuration fingerprint and package-relevant byte digests travel together
/// so `plugin package` can detect a later plain-CMake overwrite.
fn write_plugin_build_completion(
    bundle: &Bundle,
    target: &ost_build::Target,
    compiler: &ost_build::LockCompiler,
    toolchain: &Utf8Path,
    build_dir: &Utf8Path,
    invocation: Option<&str>,
) -> Result<BuildCompletion> {
    let completed_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let toolchain_rel = toolchain.strip_prefix(&bundle.root).map_err(|error| {
        Error::Operation(format!(
            "plugin toolchain '{toolchain}' is outside bundle '{}': {error}",
            bundle.root
        ))
    })?;
    let build_rel = build_dir.strip_prefix(&bundle.root).map_err(|error| {
        Error::Operation(format!(
            "plugin build directory '{build_dir}' is outside bundle '{}': {error}",
            bundle.root
        ))
    })?;
    let lock = TargetLock::from_target(
        target,
        compiler.clone(),
        &portable(toolchain_rel),
        completed_unix,
    );
    let mut intent = BuildIntent::default();
    intent.cache.insert(
        "CMAKE_BUILD_TYPE".into(),
        CMakeCacheEntry::string("Release"),
    );
    let mut completion = BuildCompletion::from_lock(
        &lock,
        BuildProjectIdentity {
            name: bundle.manifest.plugin.name.clone(),
            version: bundle.manifest.plugin.version.clone(),
        },
        portable(build_rel),
        intent,
        completed_unix,
    )
    .with_outputs(collect_plugin_managed_outputs(bundle)?);
    if let Some(invocation) = invocation {
        completion = completion.with_invocation(invocation);
    }

    let lock_body = lock
        .to_json()
        .map_err(|error| Error::parse("target.lock.json", anyhow::Error::new(error)))?;
    write_atomic(
        target_state_dir(&bundle.root, &target.id())
            .join("target.lock.json")
            .as_std_path(),
        format!("{lock_body}\n").as_bytes(),
    )?;
    let completion_body = completion
        .to_json()
        .map_err(|error| Error::parse(BUILD_COMPLETION_FILE, anyhow::Error::new(error)))?;
    write_atomic(
        build_dir.join(BUILD_COMPLETION_FILE).as_std_path(),
        format!("{completion_body}\n").as_bytes(),
    )?;
    Ok(completion)
}

/// Hash the primary bundle outputs that packaging treats as managed: the USD
/// registration tree, built libraries, and Python modules. Fixtures, notices,
/// activation files, dependency closure, and validation reports are package
/// inputs generated or copied later and therefore are not attributed to the
/// native build.
fn collect_plugin_managed_outputs(bundle: &Bundle) -> Result<Vec<BuildOutput>> {
    let mut outputs = BTreeMap::<String, BuildOutput>::new();
    for root in [
        bundle.plug_info_root(),
        bundle.lib_dir(),
        bundle.python_dir(),
    ] {
        match std::fs::symlink_metadata(root.as_std_path()) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::validation(format!(
                    "managed plugin output root is a symlink: {root}"
                )));
            }
            Ok(metadata) if metadata.is_dir() => {
                collect_plugin_managed_outputs_from(bundle, &root, &mut outputs)?;
            }
            Ok(_) => {
                return Err(Error::validation(format!(
                    "managed plugin output root is not a directory: {root}"
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::io(root.to_string(), error)),
        }
    }
    Ok(outputs.into_values().collect())
}

fn collect_plugin_managed_outputs_from(
    bundle: &Bundle,
    directory: &Utf8Path,
    outputs: &mut BTreeMap<String, BuildOutput>,
) -> Result<()> {
    for entry in std::fs::read_dir(directory.as_std_path())
        .map_err(|error| Error::io(directory.to_string(), error))?
    {
        let entry = entry.map_err(|error| Error::io(directory.to_string(), error))?;
        let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|path| {
            Error::config(format!(
                "non-UTF-8 path in managed plugin outputs: {}",
                path.display()
            ))
        })?;
        let file_type = entry
            .file_type()
            .map_err(|error| Error::io(path.to_string(), error))?;
        if file_type.is_symlink() {
            return Err(Error::validation(format!(
                "symlink is not allowed in managed plugin outputs: {path}"
            )));
        }
        if file_type.is_dir() {
            collect_plugin_managed_outputs_from(bundle, &path, outputs)?;
            continue;
        }
        if !file_type.is_file() {
            return Err(Error::validation(format!(
                "special file is not allowed in managed plugin outputs: {path}"
            )));
        }
        let relative = path.strip_prefix(&bundle.root).map_err(|error| {
            Error::Operation(format!(
                "managed plugin output '{path}' is outside bundle '{}': {error}",
                bundle.root
            ))
        })?;
        let bytes = std::fs::read(path.as_std_path())
            .map_err(|error| Error::io(path.to_string(), error))?;
        let relative = portable(relative);
        outputs.insert(
            relative.clone(),
            BuildOutput {
                path: relative,
                sha256: ost_core::digest::sha256_hex(&bytes),
                size: bytes.len() as u64,
            },
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluginBuildProvenanceStatus {
    Matched,
    Untracked,
    Mismatched,
}

impl PluginBuildProvenanceStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Matched => "matched",
            Self::Untracked => "untracked",
            Self::Mismatched => "mismatched",
        }
    }
}

#[derive(Debug, Clone)]
struct ManagedOutputDifference {
    path: String,
    kind: &'static str,
    expected: Option<String>,
    observed: Option<String>,
}

impl ManagedOutputDifference {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.path,
            "kind": self.kind,
            "expected": self.expected,
            "observed": self.observed,
        })
    }
}

#[derive(Debug, Clone)]
struct PluginBuildProvenance {
    status: PluginBuildProvenanceStatus,
    origin: &'static str,
    build_fingerprint: Option<String>,
    invocation: Option<String>,
    completed_unix: Option<u64>,
    expected_outputs: usize,
    observed_outputs: usize,
    differences: Vec<ManagedOutputDifference>,
    detail: String,
    rebuild_command: Option<String>,
    override_accepted: bool,
}

impl PluginBuildProvenance {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "status": self.status.as_str(),
            "origin": self.origin,
            "build": self.build_fingerprint.as_ref().map(|fingerprint| serde_json::json!({
                "fingerprint": fingerprint,
                "invocation": self.invocation,
                "completed_unix": self.completed_unix,
            })),
            "outputs": {
                "expected": self.expected_outputs,
                "observed": self.observed_outputs,
                "differences": self.differences.iter().map(ManagedOutputDifference::json).collect::<Vec<_>>(),
            },
            "detail": self.detail,
            "override_accepted": self.override_accepted,
        })
    }

    fn warning(&self) -> Option<serde_json::Value> {
        match self.status {
            PluginBuildProvenanceStatus::Matched => None,
            PluginBuildProvenanceStatus::Untracked => Some(serde_json::json!({
                "code": "PLUGIN_PACKAGE_OUTPUT_UNTRACKED",
                "message": self.detail,
            })),
            PluginBuildProvenanceStatus::Mismatched if self.override_accepted => {
                Some(serde_json::json!({
                    "code": "PLUGIN_PACKAGE_OUTPUT_MISMATCH_OVERRIDDEN",
                    "message": format!("{}; recording explicit unmanaged output origin", self.detail),
                }))
            }
            PluginBuildProvenanceStatus::Mismatched => None,
        }
    }

    fn accept_unmanaged_override(&mut self) {
        self.origin = "external-or-unmanaged-override";
        self.override_accepted = true;
    }

    fn mismatch_message(&self) -> String {
        let first = self.differences.first().map(|difference| {
            format!(
                "{} '{}' (expected {}, observed {})",
                difference.kind,
                difference.path,
                difference.expected.as_deref().unwrap_or("absent"),
                difference.observed.as_deref().unwrap_or("absent")
            )
        });
        match (first, self.build_fingerprint.as_deref()) {
            (Some(first), Some(fingerprint)) => {
                format!("{first}; last managed build {fingerprint}")
            }
            (Some(first), None) => first,
            (None, _) => self.detail.clone(),
        }
    }

    fn rebuild_command(&self) -> &str {
        self.rebuild_command
            .as_deref()
            .unwrap_or("ost plugin build")
    }
}

#[cfg(test)]
fn assess_plugin_build_provenance(
    bundle: &Bundle,
    target: &ost_build::Target,
) -> Result<PluginBuildProvenance> {
    let observed = collect_plugin_managed_outputs(bundle)?;
    assess_plugin_build_provenance_for_outputs(bundle, target, observed)
}

/// Compare managed-build evidence with a caller-supplied snapshot. Packaging
/// supplies outputs collected from its completed stage, so the provenance in
/// the producer manifest describes the bytes that will actually be archived,
/// not an earlier read of mutable source outputs.
fn assess_plugin_build_provenance_for_outputs(
    bundle: &Bundle,
    target: &ost_build::Target,
    observed: Vec<BuildOutput>,
) -> Result<PluginBuildProvenance> {
    let local = assess_bundle_local_build_provenance(bundle, target, observed.clone())?;
    let Some(root) = assess_root_bundle_build_provenance(bundle, target, &observed)? else {
        return Ok(local);
    };

    // A matching producer is stronger evidence than a stale completion from a
    // different build path. This is the ordinary root-build-after-bundle-build
    // case the workspace contract needs to support. If neither producer matches,
    // prefer the more recently completed managed build for the diagnostic.
    Ok(match (local.status, root.status) {
        (PluginBuildProvenanceStatus::Matched, _) => local,
        (_, PluginBuildProvenanceStatus::Matched) => root,
        (PluginBuildProvenanceStatus::Untracked, _) => root,
        (_, PluginBuildProvenanceStatus::Untracked) => local,
        (PluginBuildProvenanceStatus::Mismatched, PluginBuildProvenanceStatus::Mismatched) => {
            if root.completed_unix.unwrap_or(0) >= local.completed_unix.unwrap_or(0) {
                root
            } else {
                local
            }
        }
    })
}

fn assess_bundle_local_build_provenance(
    bundle: &Bundle,
    target: &ost_build::Target,
    observed: Vec<BuildOutput>,
) -> Result<PluginBuildProvenance> {
    let id = target.id();
    let completion_path = target_build_dir(&bundle.root, &id).join(BUILD_COMPLETION_FILE);
    let lock_path = target_state_dir(&bundle.root, &id).join("target.lock.json");
    let completion_exists = completion_path.as_std_path().is_file();
    let lock_exists = lock_path.as_std_path().is_file();
    if !completion_exists && !lock_exists {
        return Ok(PluginBuildProvenance {
            status: PluginBuildProvenanceStatus::Untracked,
            origin: "external-or-unmanaged",
            build_fingerprint: None,
            invocation: None,
            completed_unix: None,
            expected_outputs: 0,
            observed_outputs: observed.len(),
            differences: Vec::new(),
            detail: format!(
                "{} package-relevant output(s) have no `ost plugin build` completion; treating their origin as external or unmanaged",
                observed.len()
            ),
            rebuild_command: None,
            override_accepted: false,
        });
    }

    let evidence_error = |detail: String| PluginBuildProvenance {
        status: PluginBuildProvenanceStatus::Mismatched,
        origin: "ost-managed-diverged",
        build_fingerprint: None,
        invocation: None,
        completed_unix: None,
        expected_outputs: 0,
        observed_outputs: observed.len(),
        differences: Vec::new(),
        detail,
        rebuild_command: Some("ost plugin build".into()),
        override_accepted: false,
    };
    if !completion_exists || !lock_exists {
        return Ok(evidence_error(format!(
            "managed build evidence is incomplete: completion present={completion_exists}, target lock present={lock_exists}"
        )));
    }
    let lock: TargetLock = match read_plugin_build_json(&lock_path) {
        Ok(lock) => lock,
        Err(detail) => return Ok(evidence_error(detail)),
    };
    let completion: BuildCompletion = match read_plugin_build_json(&completion_path) {
        Ok(completion) => completion,
        Err(detail) => return Ok(evidence_error(detail)),
    };
    let build_fingerprint = completion.fingerprint();
    let build_identity = || {
        (
            Some(build_fingerprint.clone()),
            completion.invocation.clone(),
            Some(completion.completed_unix),
        )
    };
    let identity_error = if lock.target != id
        || lock.platform != target.platform
        || lock.profile != target.profile
        || lock.variant != target.variant
        || lock.runtime.id != target.runtime_id
        || lock.runtime.digest != target.runtime_digest
        || lock.generator != target.generator
    {
        Some(format!(
            "last managed build target/runtime identity does not match selected target '{id}' runtime '{}@{}'",
            target.runtime_id, target.runtime_digest
        ))
    } else {
        let build_rel = Utf8PathBuf::from(format!("build/{id}"));
        completion
            .validate_against(
                &lock,
                &bundle.manifest.plugin.name,
                &bundle.manifest.plugin.version,
                &build_rel,
            )
            .err()
            .map(|detail| format!("last managed build completion is incompatible: {detail}"))
    };
    if let Some(detail) = identity_error {
        let (build_fingerprint, invocation, completed_unix) = build_identity();
        return Ok(PluginBuildProvenance {
            status: PluginBuildProvenanceStatus::Mismatched,
            origin: "ost-managed-diverged",
            build_fingerprint,
            invocation,
            completed_unix,
            expected_outputs: completion.outputs.len(),
            observed_outputs: observed.len(),
            differences: Vec::new(),
            detail,
            rebuild_command: Some("ost plugin build".into()),
            override_accepted: false,
        });
    }
    if completion.outputs.is_empty() {
        let (build_fingerprint, invocation, completed_unix) = build_identity();
        return Ok(PluginBuildProvenance {
            status: PluginBuildProvenanceStatus::Untracked,
            origin: "external-or-unmanaged",
            build_fingerprint,
            invocation,
            completed_unix,
            expected_outputs: 0,
            observed_outputs: observed.len(),
            differences: Vec::new(),
            detail:
                "the build completion predates managed output digests; package output is untracked"
                    .into(),
            rebuild_command: None,
            override_accepted: false,
        });
    }

    let expected_by_path = completion
        .outputs
        .iter()
        .map(|output| (output.path.as_str(), output))
        .collect::<BTreeMap<_, _>>();
    let observed_by_path = observed
        .iter()
        .map(|output| (output.path.as_str(), output))
        .collect::<BTreeMap<_, _>>();
    let mut differences = Vec::new();
    for (path, expected) in &expected_by_path {
        match observed_by_path.get(path) {
            None => differences.push(ManagedOutputDifference {
                path: (*path).to_string(),
                kind: "missing",
                expected: Some(expected.sha256.clone()),
                observed: None,
            }),
            Some(observed)
                if observed.sha256 != expected.sha256 || observed.size != expected.size =>
            {
                differences.push(ManagedOutputDifference {
                    path: (*path).to_string(),
                    kind: "digest-mismatch",
                    expected: Some(expected.sha256.clone()),
                    observed: Some(observed.sha256.clone()),
                });
            }
            Some(_) => {}
        }
    }
    for (path, output) in &observed_by_path {
        if !expected_by_path.contains_key(path) {
            differences.push(ManagedOutputDifference {
                path: (*path).to_string(),
                kind: "untracked",
                expected: None,
                observed: Some(output.sha256.clone()),
            });
        }
    }
    let (build_fingerprint, invocation, completed_unix) = build_identity();
    let status = if differences.is_empty() {
        PluginBuildProvenanceStatus::Matched
    } else {
        PluginBuildProvenanceStatus::Mismatched
    };
    Ok(PluginBuildProvenance {
        status,
        origin: if status == PluginBuildProvenanceStatus::Matched {
            "ost-managed"
        } else {
            "ost-managed-diverged"
        },
        build_fingerprint,
        invocation,
        completed_unix,
        expected_outputs: completion.outputs.len(),
        observed_outputs: observed.len(),
        detail: if status == PluginBuildProvenanceStatus::Matched {
            format!(
                "all {} package-relevant output(s) match the last managed build",
                observed.len()
            )
        } else {
            format!(
                "{} package-relevant output difference(s) from the last managed build",
                differences.len()
            )
        },
        differences,
        rebuild_command: Some("ost plugin build".into()),
        override_accepted: false,
    })
}

/// Compare bundle outputs with the completion published by `ost build` at the
/// enclosing project root. Root CMake builds intentionally write the same
/// discoverable bundle trees as `ost plugin build`; the member path prefix in
/// the project completion makes those producers unambiguous.
fn assess_root_bundle_build_provenance(
    bundle: &Bundle,
    target: &ost_build::Target,
    observed: &[BuildOutput],
) -> Result<Option<PluginBuildProvenance>> {
    let Some(project_root) = find_project_root(bundle.root.as_std_path())
        .and_then(|path| Utf8PathBuf::from_path_buf(path).ok())
    else {
        return Ok(None);
    };
    if canonical_root(&project_root) == canonical_root(&bundle.root) {
        return Ok(None);
    }

    let id = target.id();
    let project = load_project(&project_root)?;
    let project_version = project.effective_version(&project_root)?;
    let mut declared_intents = vec![BuildIntent::default()];
    if let Some(build) = &project.build {
        for name in build.intents.keys() {
            declared_intents.push(crate::commands::build::resolve_declared_intent(
                &project_root,
                Some(name),
            )?);
        }
    }
    let completion_candidates = declared_intents
        .into_iter()
        .filter_map(|intent| {
            let build_rel = crate::commands::build::build_dir_for_intent(&id, &intent);
            let path = project_root.join(&build_rel).join(BUILD_COMPLETION_FILE);
            path.as_std_path()
                .is_file()
                .then_some((intent, build_rel, path))
        })
        .collect::<Vec<_>>();
    // Configure publishes the root target lock before any build completes. It
    // is not producer evidence by itself and must not turn an otherwise valid
    // external/unmanaged package into a mismatch.
    if completion_candidates.is_empty() {
        return Ok(None);
    }

    let lock_path = target_state_dir(&project_root, &id).join("target.lock.json");
    let lock_exists = lock_path.as_std_path().is_file();
    let evidence_error = |detail: String,
                          completion: Option<&BuildCompletion>,
                          rebuild_command: String| PluginBuildProvenance {
        status: PluginBuildProvenanceStatus::Mismatched,
        origin: "ost-managed-root-diverged",
        build_fingerprint: completion.map(BuildCompletion::fingerprint),
        invocation: completion.and_then(|value| value.invocation.clone()),
        completed_unix: completion.map(|value| value.completed_unix),
        expected_outputs: 0,
        observed_outputs: observed.len(),
        differences: Vec::new(),
        detail,
        rebuild_command: Some(rebuild_command),
        override_accepted: false,
    };
    if !lock_exists {
        return Ok(Some(evidence_error(
            "root managed build evidence is incomplete: completion present=true, target lock present=false"
                .into(),
            None,
            "ost build".into(),
        )));
    }
    let lock: TargetLock = match read_plugin_build_json(&lock_path) {
        Ok(lock) => lock,
        Err(detail) => {
            return Ok(Some(evidence_error(detail, None, "ost build".into())));
        }
    };
    let target_identity_differs = lock.target != id
        || lock.platform != target.platform
        || lock.profile != target.profile
        || lock.variant != target.variant;
    let identity_error = if target_identity_differs {
        Some(format!(
            "last root managed build target/runtime identity does not match selected target '{id}' runtime '{}@{}'",
            target.runtime_id, target.runtime_digest
        ))
    } else if lock.runtime.id != target.runtime_id {
        Some(format!(
            "last root managed build runtime '{}' was substituted by selected runtime '{}'; rebuild the affected workspace member with `ost build`",
            lock.runtime.id, target.runtime_id
        ))
    } else if lock.runtime.digest != target.runtime_digest {
        Some(format!(
            "last root managed build runtime '{}' changed digest under the same runtime id: recorded digest '{}' != selected digest '{}'; this may be manifest-identity enrichment or replacement of the runtime payload, so rebuild the affected workspace member with `ost build`",
            target.runtime_id, lock.runtime.digest, target.runtime_digest
        ))
    } else {
        None
    };
    if let Some(detail) = identity_error {
        return Ok(Some(evidence_error(detail, None, "ost build".into())));
    }

    let Some(member) = member_relative(&canonical_root(&project_root), &bundle.root) else {
        return Ok(None);
    };
    let prefix = format!("{member}/");
    let observed_by_path = observed
        .iter()
        .map(|output| (output.path.as_str(), output))
        .collect::<BTreeMap<_, _>>();
    let mut selected = None;
    for (declared_intent, build_rel, completion_path) in completion_candidates {
        let rebuild_command = if declared_intent.name == "default" {
            "ost build".to_string()
        } else {
            format!("ost build --intent {}", declared_intent.name)
        };
        let completion: BuildCompletion = match read_plugin_build_json(&completion_path) {
            Ok(completion) => completion,
            Err(detail) => {
                select_root_build_provenance(
                    &mut selected,
                    evidence_error(detail, None, rebuild_command),
                );
                continue;
            }
        };
        let completion_error = completion
            .validate_against(&lock, &project.project.name, &project_version, &build_rel)
            .err()
            .or_else(|| {
                crate::commands::build::validate_completed_intent(
                    &completion.intent,
                    &declared_intent,
                )
                .err()
            });
        if let Some(detail) = completion_error {
            select_root_build_provenance(
                &mut selected,
                evidence_error(
                    format!("last root managed build completion is incompatible: {detail}"),
                    Some(&completion),
                    rebuild_command,
                ),
            );
            continue;
        }

        let expected = completion
            .outputs
            .iter()
            .filter_map(|output| {
                output.path.strip_prefix(&prefix).map(|relative| {
                    (
                        relative.to_string(),
                        BuildOutput {
                            path: relative.to_string(),
                            sha256: output.sha256.clone(),
                            size: output.size,
                        },
                    )
                })
            })
            .collect::<BTreeMap<_, _>>();
        if expected.is_empty() {
            continue;
        }
        let mut differences = Vec::new();
        for (path, want) in &expected {
            match observed_by_path.get(path.as_str()) {
                None => differences.push(ManagedOutputDifference {
                    path: path.clone(),
                    kind: "missing",
                    expected: Some(want.sha256.clone()),
                    observed: None,
                }),
                Some(got) if got.sha256 != want.sha256 || got.size != want.size => {
                    differences.push(ManagedOutputDifference {
                        path: path.clone(),
                        kind: "digest-mismatch",
                        expected: Some(want.sha256.clone()),
                        observed: Some(got.sha256.clone()),
                    });
                }
                Some(_) => {}
            }
        }
        for (path, got) in &observed_by_path {
            if !expected.contains_key(*path) {
                differences.push(ManagedOutputDifference {
                    path: (*path).to_string(),
                    kind: "untracked",
                    expected: None,
                    observed: Some(got.sha256.clone()),
                });
            }
        }
        let status = if differences.is_empty() {
            PluginBuildProvenanceStatus::Matched
        } else {
            PluginBuildProvenanceStatus::Mismatched
        };
        let candidate = PluginBuildProvenance {
            status,
            origin: if status == PluginBuildProvenanceStatus::Matched {
                "ost-managed-root"
            } else {
                "ost-managed-root-diverged"
            },
            build_fingerprint: Some(completion.fingerprint()),
            invocation: completion.invocation.clone(),
            completed_unix: Some(completion.completed_unix),
            expected_outputs: expected.len(),
            observed_outputs: observed.len(),
            detail: if status == PluginBuildProvenanceStatus::Matched {
                format!(
                    "all {} package-relevant output(s) match the root managed '{}' build",
                    observed.len(),
                    completion.intent.name
                )
            } else {
                format!(
                    "{} package-relevant output difference(s) from the root managed '{}' build",
                    differences.len(),
                    completion.intent.name
                )
            },
            differences,
            rebuild_command: Some(rebuild_command),
            override_accepted: false,
        };
        select_root_build_provenance(&mut selected, candidate);
    }
    Ok(selected)
}

/// Prefer byte-matching root evidence, then the newest candidate for a useful
/// mismatch diagnostic. Named intents are independent build trees, so more than
/// one valid completion may exist for the same target/runtime.
fn select_root_build_provenance(
    selected: &mut Option<PluginBuildProvenance>,
    candidate: PluginBuildProvenance,
) {
    let replace = match selected.as_ref() {
        None => true,
        Some(current) if current.status != candidate.status => {
            candidate.status == PluginBuildProvenanceStatus::Matched
        }
        Some(current) => {
            candidate.completed_unix.unwrap_or(0) >= current.completed_unix.unwrap_or(0)
        }
    };
    if replace {
        *selected = Some(candidate);
    }
}

fn read_plugin_build_json<T: serde::de::DeserializeOwned>(
    path: &Utf8Path,
) -> std::result::Result<T, String> {
    let source = std::fs::read_to_string(path.as_std_path())
        .map_err(|error| format!("cannot read managed build evidence '{path}': {error}"))?;
    serde_json::from_str(&source)
        .map_err(|error| format!("invalid managed build evidence '{path}': {error}"))
}

/// What one packaged bundle produced, so a single-bundle run and a workspace run
/// can report the same facts without packaging twice.
struct PackageOutcome {
    id: String,
    name: String,
    version: String,
    /// Target OS of the variant this member was packaged for, so an aggregate
    /// product can record one loader contract without re-resolving the target.
    os: Os,
    archive_path: Utf8PathBuf,
    packed: ost_build::PackResult,
    debug: Option<(String, ost_build::PackResult)>,
    debug_status: DebugPackageStatus,
    build_provenance: PluginBuildProvenance,
    manifest: serde_json::Value,
    stage_warnings: Vec<serde_json::Value>,
}

struct ProductOutcome {
    name: String,
    version: String,
    target: String,
    archive_path: Utf8PathBuf,
    packed: ost_build::PackResult,
    members: usize,
    stage_warnings: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DebugPackageStatus {
    /// Symbols were kept in the main archive because the caller requested it.
    Included,
    /// Recognized split-symbol files produced a sibling debug archive.
    Split,
    /// The stage contained no separate `.pdb`/`.dwo` files to split or include.
    NotProduced,
}

impl DebugPackageStatus {
    fn json(self) -> serde_json::Value {
        match self {
            Self::Included => serde_json::json!({
                "mode": "included",
                "produced": false,
                "reason": "--with-debug keeps recognized debug-symbol files in the main archive",
            }),
            Self::Split => serde_json::json!({
                "mode": "split",
                "produced": true,
                "reason": "recognized .pdb/.dwo files were split into the sibling debug archive",
            }),
            Self::NotProduced => serde_json::json!({
                "mode": "not-produced",
                "produced": false,
                "reason": "the staged build contained no separate .pdb or .dwo files; embedded ELF/Mach-O debug info is not split",
            }),
        }
    }

    fn human_reason(self) -> &'static str {
        match self {
            Self::Included => "not produced (--with-debug keeps symbols in the main archive)",
            Self::Split => "produced as a sibling symbol package",
            Self::NotProduced => {
                "not produced (the stage contained no separate .pdb/.dwo files; embedded debug info is not split)"
            }
        }
    }
}

fn package(
    bundle_path: &str,
    target: Option<String>,
    profile: Option<String>,
    clean_stage: bool,
    with_debug: bool,
    allow_unmanaged_output: bool,
    fmt: Format,
) -> Result<()> {
    let bundle = load_bundle(bundle_path)?;
    let outcome = package_bundle(
        &bundle,
        target,
        profile,
        clean_stage,
        with_debug,
        allow_unmanaged_output,
    )?;
    report_package(&outcome, fmt);
    Ok(())
}

fn package_bundle(
    bundle: &Bundle,
    target: Option<String>,
    profile: Option<String>,
    clean_stage: bool,
    with_debug: bool,
    allow_unmanaged_output: bool,
) -> Result<PackageOutcome> {
    let bundle = bundle.clone();
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
    let library_runtime = selected_library_package_runtime(&bundle, &id)?;
    for (_, relative) in &library_runtime {
        // `portable`, not `to_string`: this path is *joined* on the producing
        // host, so a Windows producer would otherwise bake `runtime/libraries\bin`
        // into a portable, digest-addressed manifest that a Linux consumer reads
        // back — and that a consumer must split on `/` to build a loader path.
        let relative = portable(relative);
        if !packaged_manifest.requires.runtime_libs.contains(&relative) {
            packaged_manifest.requires.runtime_libs.push(relative);
        }
    }
    let library_evidence = selected_workspace_library_evidence(&bundle, Some(&r))?;
    let packaged_library_dirs = library_runtime
        .iter()
        .map(|(_, relative)| portable(relative))
        .collect::<Vec<_>>();
    let packaged_library_evidence = library_evidence
        .iter()
        .map(|library| {
            serde_json::json!({
                "id": library["id"],
                "version": library["version"],
                "descriptor": ost_plugin::LIBRARY_MANIFEST,
                "cmake_package": library["cmake_package"],
                "cmake_target": library["cmake_target"],
                "prefix": serde_json::Value::Null,
                "runtime_directories": packaged_library_dirs,
                "provenance": "source-workspace",
            })
        })
        .collect::<Vec<_>>();
    // The bundle half of the closure travels with the artifact too. Without it a
    // consumer installing this package alone has no way to know a provider is
    // missing until USD fails to apply a schema it cannot find.
    let bundle_dependencies = selected_workspace_dependencies(&bundle)?;
    let packaged_bundle_evidence = bundle_dependencies
        .iter()
        .map(|dependency| {
            bundle_evidence(dependency, ost_plugin::PLUGIN_MANIFEST, "source-workspace")
        })
        .collect::<Vec<_>>();
    // …and so does the registration half those records point at. Recording a
    // resolved `bundles` closure while shipping only its libraries is what made
    // a v0.18.0 package look closed and still fail at `Usd.Stage.Open()`.
    let bundle_registration = selected_bundle_package_registration(&bundle_dependencies)?;
    let bundle_libraries = selected_bundle_package_libraries(&bundle_dependencies)?;
    for (_, relative) in &bundle_registration {
        let relative = portable(relative);
        if !packaged_manifest
            .requires
            .runtime_plugin_paths
            .contains(&relative)
        {
            packaged_manifest
                .requires
                .runtime_plugin_paths
                .push(relative);
        }
    }
    for (_, relative) in &bundle_libraries {
        let relative = portable(relative);
        if !packaged_manifest.requires.runtime_libs.contains(&relative) {
            packaged_manifest.requires.runtime_libs.push(relative);
        }
    }

    // Reruns must not fail on a stage the previous run left temporarily
    // undeletable (scanner-held handles, dogfooding report #9): stage into a
    // fresh sibling instead, and surface that as an actionable warning
    // (`--clean-stage` reclaims the stable name).
    let preferred_stage = target_state_dir(&bundle.root, &id).join("package-stage");
    let (stage, mut stage_warnings) = super::prepare_package_stage(&preferred_stage, clean_stage)?;
    stage_plugin_bundle(&bundle, &stage)?;
    for (source, relative) in &library_runtime {
        copy_tree_required(source, relative, &stage)?;
    }
    for (source, relative) in &bundle_libraries {
        copy_tree_required(source, relative, &stage)?;
    }
    for (source, relative) in &bundle_registration {
        copy_tree_required(source, relative, &stage)?;
    }
    write_packaged_manifest(&stage.join(ost_plugin::PLUGIN_MANIFEST), &packaged_manifest)?;
    write_dependency_evidence(
        &stage,
        &packaged_library_evidence,
        &packaged_bundle_evidence,
    )?;
    let packaged_bundle = Bundle {
        root: stage.clone(),
        manifest: packaged_manifest.clone(),
    };
    let staged_outputs = collect_plugin_managed_outputs(&packaged_bundle)?;
    let mut build_provenance =
        assess_plugin_build_provenance_for_outputs(&bundle, &tgt, staged_outputs)?;
    if build_provenance.status == PluginBuildProvenanceStatus::Mismatched {
        if allow_unmanaged_output {
            build_provenance.accept_unmanaged_override();
        } else {
            return Err(Error::coded(
                "PLUGIN_PACKAGE_OUTPUT_MISMATCH",
                Category::Validation,
                format!(
                    "plugin '{}' staged package output does not match its last managed build: {}",
                    bundle.manifest.name(),
                    build_provenance.mismatch_message()
                ),
            )
            .with_hint(format!(
                "rerun `{}` before packaging, or pass --allow-unmanaged-output to record an explicit external/unmanaged override",
                build_provenance.rebuild_command()
            )));
        }
    }
    if let Some(warning) = build_provenance.warning() {
        stage_warnings.push(warning);
    }
    let verification = write_verification_contract(&packaged_bundle)?;
    write_activation_files(&packaged_bundle, tgt.variant.os)?;
    let session = session_env_with(&r.env, &packaged_bundle, &[], host.os);
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
    // Pin every entry's mtime (from SOURCE_DATE_EPOCH, else epoch-0) so an
    // unchanged staged tree repacks to a byte-identical archive digest.
    let pack_opts = PackOptions {
        mtime: ost_build::source_date_epoch(),
        ..PackOptions::default()
    };

    // Ship lean by default: split debug-symbol sidecars (`.pdb`, `.dwo`) out of
    // the main archive into a sibling `*-debug` package, so the shipped artifact
    // stays small without discarding the symbols. `--with-debug` keeps them in
    // the main archive instead.
    let has_debug_symbol_files = staged.iter().any(|path| is_debug_symbol_file(path));
    let (main_files, debug_files): (Vec<_>, Vec<_>) = if with_debug {
        (staged, Vec::new())
    } else {
        staged.into_iter().partition(|p| !is_debug_symbol_file(p))
    };
    let packed = pack_dir_with(
        &stage,
        &archive_path,
        &main_files,
        pack_opts.clone(),
        &mut |_| {},
    )
    .map_err(|e| Error::io(archive_path.to_string(), e))?;

    let debug_name = plugin_debug_archive_name(name, version, &id);
    let debug_path = dist_dir.join(&debug_name);
    let debug_pack = if debug_files.is_empty() {
        // A previous lean package may have emitted this sibling. Do not leave
        // stale symbols in a dist directory whose new manifest/SHA256SUMS says
        // there is no debug archive (notably after switching to --with-debug).
        remove_stale_debug_archive(&debug_path)?;
        None
    } else {
        let dp = pack_dir_with(&stage, &debug_path, &debug_files, pack_opts, &mut |_| {})
            .map_err(|e| Error::io(debug_path.to_string(), e))?;
        Some((debug_name, dp))
    };
    let debug_status = if with_debug && has_debug_symbol_files {
        DebugPackageStatus::Included
    } else if debug_pack.is_some() {
        DebugPackageStatus::Split
    } else {
        DebugPackageStatus::NotProduced
    };

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

    // `manifest.json` is itself embedded by aggregate products. A wall-clock
    // value here made two identical `--workspace --product` invocations produce
    // different product bytes even though every member archive was stable.
    // Use the same reproducible timestamp contract as tar entries: an explicit
    // SOURCE_DATE_EPOCH, otherwise epoch 0.
    let created = ost_build::source_date_epoch();
    let files_json: Vec<_> = packed.files.iter().map(|f| f.manifest_json()).collect();
    // A sibling `*-debug` package, when symbols were split out: its own archive
    // digest/size and file list, so a consumer can pull and overlay it to restore
    // symbols in place.
    let debug_json = debug_pack.as_ref().map(|(debug_name, dp)| {
        serde_json::json!({
            "archive": debug_name,
            "archive_digest": dp.archive_digest,
            "archive_size": dp.archive_size,
            "total_size": dp.total_size,
            "files": dp.files.iter().map(|f| f.manifest_json()).collect::<Vec<_>>(),
        })
    });
    let mut manifest = serde_json::json!({
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
        // The producing tool names itself here so the registry can
        // record the artifact's origin instead of whoever imported it.
        "producer": format!("ost {}", env!("CARGO_PKG_VERSION")),
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
            "build_outputs": build_provenance.json(),
        },
        "files": files_json,
    });
    if !packaged_library_evidence.is_empty() || !packaged_bundle_evidence.is_empty() {
        manifest["dependencies"] = serde_json::json!({
            "libraries": packaged_library_evidence,
            "bundles": packaged_bundle_evidence,
        });
    }
    if let Some(debug_json) = &debug_json {
        manifest["debug"] = debug_json.clone();
    }
    manifest["activation"] = serde_json::json!({
        "schema": "openstrata.activation/v1alpha1",
        "contract": "openstrata.activation.json",
        "powershell": "activate.ps1",
        "bash": "activate.sh",
        "python": "openstrata_activate.py",
    });
    manifest["verification"] = serde_json::json!({
        "schema": PLUGIN_VERIFICATION_SCHEMA,
        "contract": PLUGIN_VERIFICATION,
        "oracle_convention": verification.oracle_convention,
        "roundtrip_oracles": verification.roundtrip.len(),
    });
    manifest["debug_package"] = debug_status.json();
    let evidence = ost_artifact::generate_evidence(&dist_dir, &mut manifest)?;
    write_text(&dist_dir.join("manifest.json"), &pretty_json(&manifest)?)?;

    // One SHA256SUMS line per shipped archive (main, then any sibling `*-debug`),
    // so a `sha256sum -c` verifies the whole dist output.
    let mut sha_lines = vec![format!(
        "{}  {archive_name}",
        bare_sha256(&packed.archive_digest)
    )];
    if let Some((debug_name, dp)) = &debug_pack {
        sha_lines.push(format!("{}  {debug_name}", bare_sha256(&dp.archive_digest)));
    }
    for layer in &evidence {
        sha_lines.push(format!("{}  {}", bare_sha256(&layer.digest), layer.path));
    }
    write_text(&dist_dir.join("SHA256SUMS"), &sha_lines.join("\n"))?;

    Ok(PackageOutcome {
        id,
        name: name.clone(),
        version: version.clone(),
        os: tgt.variant.os,
        archive_path,
        packed,
        debug: debug_pack,
        debug_status,
        build_provenance,
        manifest,
        stage_warnings,
    })
}

/// Package one workspace-built executable into a member archive.
///
/// From usd-vrm-plugins report 28 §3: a CLI tool is a user-facing deliverable
/// with no bundle that could carry it, so a release either omitted it or the
/// repository hand-rolled a second packaging path — and hand-rolled packaging is
/// what report 27 was about. This produces the *same* dist shape a bundle
/// package does (archive + `manifest.json` + `SHA256SUMS` + evidence), which is
/// what lets the aggregate product compose it without a second code path.
///
/// It stages what the build produced under the member root rather than
/// reconstructing an install view: the executables the descriptor names, plus
/// the directories it declares, so shared libraries shipped beside a tool travel
/// with it.
fn package_tool(
    tool: &ost_plugin::Tool,
    target: Option<String>,
    profile: Option<String>,
    clean_stage: bool,
    with_debug: bool,
    allow_unmanaged_output: bool,
) -> Result<PackageOutcome> {
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

    // Resolve before staging: a tool package with no tool in it is the one
    // outcome that must never be produced quietly.
    let windows = tgt.variant.os == Os::Windows;
    let executables = tool.locate_executables(&tool.root, windows)?;
    let directories = tool.built_directories();

    let preferred_stage = target_state_dir(&tool.root, &id).join("package-stage");
    let (stage, mut stage_warnings) = super::prepare_package_stage(&preferred_stage, clean_stage)?;
    for directory in &directories {
        copy_tree_required(&tool.root.join(directory), Utf8Path::new(directory), &stage)?;
    }
    // The descriptor travels with the artifact: a consumer reads the executables
    // and layout it declares without unpacking conventions from the filename.
    write_text(
        &stage.join(ost_plugin::TOOL_MANIFEST),
        &serde_yaml::to_string(&tool.manifest)
            .map_err(|error| Error::parse(ost_plugin::TOOL_MANIFEST, anyhow::Error::new(error)))?,
    )?;

    let observed = executables
        .iter()
        .map(|relative| {
            let path = stage.join(relative);
            let (digest, size) = digest_file(&path)?;
            Ok(BuildOutput {
                path: relative.clone(),
                sha256: digest,
                size,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut build_provenance = assess_tool_build_provenance(tool, &tgt, observed)?;
    if build_provenance.status == PluginBuildProvenanceStatus::Mismatched {
        if allow_unmanaged_output {
            build_provenance.accept_unmanaged_override();
        } else {
            return Err(Error::coded(
                "PLUGIN_PACKAGE_OUTPUT_MISMATCH",
                Category::Validation,
                format!(
                    "tool '{}' staged package output does not match its last managed build: {}",
                    tool.id(),
                    build_provenance.mismatch_message()
                ),
            )
            .with_hint(
                "rerun `ost build` before packaging, or pass --allow-unmanaged-output to record an explicit external/unmanaged override",
            ));
        }
    }
    if let Some(warning) = build_provenance.warning() {
        stage_warnings.push(warning);
    }

    let staged = stage_files(&stage).map_err(|e| {
        if e.kind() == std::io::ErrorKind::InvalidData {
            Error::validation(e.to_string())
        } else {
            Error::io(stage.to_string(), e)
        }
    })?;

    let name = tool.id().to_string();
    let version = tool.version().to_string();
    let archive_name = plugin_archive_name(&name, &version, &id);
    let dist_dir = tool_dist_dir(&tool.root, &name, &version, &id);
    let archive_path = dist_dir.join(&archive_name);
    let pack_opts = PackOptions {
        mtime: ost_build::source_date_epoch(),
        ..PackOptions::default()
    };

    // Same lean-by-default contract as a bundle: symbols split into a sibling
    // package unless the caller asked for them inline.
    let has_debug_symbol_files = staged.iter().any(|path| is_debug_symbol_file(path));
    let (main_files, debug_files): (Vec<_>, Vec<_>) = if with_debug {
        (staged, Vec::new())
    } else {
        staged.into_iter().partition(|p| !is_debug_symbol_file(p))
    };
    let packed = pack_dir_with(
        &stage,
        &archive_path,
        &main_files,
        pack_opts.clone(),
        &mut |_| {},
    )
    .map_err(|e| Error::io(archive_path.to_string(), e))?;
    let debug_name = plugin_debug_archive_name(&name, &version, &id);
    let debug_path = dist_dir.join(&debug_name);
    let debug_pack = if debug_files.is_empty() {
        remove_stale_debug_archive(&debug_path)?;
        None
    } else {
        let dp = pack_dir_with(&stage, &debug_path, &debug_files, pack_opts, &mut |_| {})
            .map_err(|e| Error::io(debug_path.to_string(), e))?;
        Some((debug_name, dp))
    };
    let debug_status = if with_debug && has_debug_symbol_files {
        DebugPackageStatus::Included
    } else if debug_pack.is_some() {
        DebugPackageStatus::Split
    } else {
        DebugPackageStatus::NotProduced
    };

    let mut manifest = serde_json::json!({
        "schema": 1,
        "kind": ost_artifact::TOOL_KIND,
        "tool": {
            "id": name,
            "version": version,
            "license": tool.manifest.tool.license,
            "descriptor": ost_plugin::TOOL_MANIFEST,
            "executables": executables,
            "directories": directories,
        },
        "target": id,
        "archive": archive_name,
        "archive_digest": packed.archive_digest,
        "archive_size": packed.archive_size,
        "total_size": packed.total_size,
        "created_unix": ost_build::source_date_epoch(),
        "producer": format!("ost {}", env!("CARGO_PKG_VERSION")),
        "provenance": {
            "platform": tgt.platform,
            "profile": tgt.profile,
            "variant": tgt.variant.slug(),
            "runtime": {
                "id": tgt.runtime_id,
                "digest": tgt.runtime_digest,
            },
            "build_outputs": build_provenance.json(),
        },
        "files": packed.files.iter().map(|f| f.manifest_json()).collect::<Vec<_>>(),
    });
    if !tool.manifest.requires.libraries.is_empty() {
        manifest["dependencies"] = serde_json::json!({
            "libraries": tool.manifest.requires.libraries,
        });
    }
    if let Some((debug_name, dp)) = &debug_pack {
        manifest["debug"] = serde_json::json!({
            "archive": debug_name,
            "archive_digest": dp.archive_digest,
            "archive_size": dp.archive_size,
            "total_size": dp.total_size,
            "files": dp.files.iter().map(|f| f.manifest_json()).collect::<Vec<_>>(),
        });
    }
    manifest["debug_package"] = debug_status.json();
    let evidence = ost_artifact::generate_evidence(&dist_dir, &mut manifest)?;
    write_text(&dist_dir.join("manifest.json"), &pretty_json(&manifest)?)?;

    let mut sha_lines = vec![format!(
        "{}  {archive_name}",
        bare_sha256(&packed.archive_digest)
    )];
    if let Some((debug_name, dp)) = &debug_pack {
        sha_lines.push(format!("{}  {debug_name}", bare_sha256(&dp.archive_digest)));
    }
    for layer in &evidence {
        sha_lines.push(format!("{}  {}", bare_sha256(&layer.digest), layer.path));
    }
    write_text(&dist_dir.join("SHA256SUMS"), &sha_lines.join("\n"))?;

    Ok(PackageOutcome {
        id,
        name,
        version,
        os: tgt.variant.os,
        archive_path,
        packed,
        debug: debug_pack,
        debug_status,
        build_provenance,
        manifest,
        stage_warnings,
    })
}

/// Bind a staged tool's executables to the workspace build that produced them.
///
/// A tool is built by `ost build` at the project root, not by `ost plugin build`
/// in a bundle, so its evidence is the project's build completion — which
/// records the tool executables it produced for exactly this comparison.
fn assess_tool_build_provenance(
    tool: &ost_plugin::Tool,
    target: &ost_build::Target,
    observed: Vec<BuildOutput>,
) -> Result<PluginBuildProvenance> {
    let Some(project_root) = enclosing_project_root() else {
        return Ok(untracked_build_provenance(
            observed.len(),
            "packaged outside an OpenStrata project, so no managed build evidence applies",
        ));
    };
    let id = target.id();
    let completion_path = target_build_dir(&project_root, &id).join(BUILD_COMPLETION_FILE);
    let Ok(completion) = read_plugin_build_json::<BuildCompletion>(&completion_path) else {
        return Ok(untracked_build_provenance(
            observed.len(),
            "no `ost build` completion for this target; treating the tool output as external or unmanaged",
        ));
    };
    // Outputs are recorded relative to the project root; a tool's own paths are
    // relative to its member root. Both sides are compared canonically so a case
    // difference or a symlinked temp directory does not silently detach a tool
    // from the build that produced it.
    let Some(member) = member_relative(&canonical_root(&project_root), &tool.root) else {
        return Ok(untracked_build_provenance(
            observed.len(),
            &format!(
                "the tool at {} is not inside the enclosing project {project_root}, so its \
                 outputs cannot be attributed",
                tool.root
            ),
        ));
    };
    let expected: BTreeMap<&str, &BuildOutput> = completion
        .outputs
        .iter()
        .filter_map(|output| {
            output
                .path
                .strip_prefix(&format!("{member}/"))
                .map(|relative| (relative, output))
        })
        .collect();
    if expected.is_empty() {
        return Ok(untracked_build_provenance(
            observed.len(),
            "the last managed build recorded no outputs for this tool; treating them as external or unmanaged",
        ));
    }

    let mut differences = Vec::new();
    let observed_by_path: BTreeMap<&str, &BuildOutput> = observed
        .iter()
        .map(|output| (output.path.as_str(), output))
        .collect();
    for (path, want) in &expected {
        match observed_by_path.get(path) {
            None => differences.push(ManagedOutputDifference {
                path: (*path).to_string(),
                kind: "missing",
                expected: Some(want.sha256.clone()),
                observed: None,
            }),
            Some(got) if got.sha256 != want.sha256 || got.size != want.size => {
                differences.push(ManagedOutputDifference {
                    path: (*path).to_string(),
                    kind: "digest-mismatch",
                    expected: Some(want.sha256.clone()),
                    observed: Some(got.sha256.clone()),
                })
            }
            Some(_) => {}
        }
    }
    for (path, got) in &observed_by_path {
        if !expected.contains_key(path) {
            differences.push(ManagedOutputDifference {
                path: (*path).to_string(),
                kind: "untracked",
                expected: None,
                observed: Some(got.sha256.clone()),
            });
        }
    }
    let status = if differences.is_empty() {
        PluginBuildProvenanceStatus::Matched
    } else {
        PluginBuildProvenanceStatus::Mismatched
    };
    Ok(PluginBuildProvenance {
        status,
        origin: if status == PluginBuildProvenanceStatus::Matched {
            "ost-managed"
        } else {
            "ost-managed-diverged"
        },
        build_fingerprint: Some(completion.fingerprint()),
        invocation: completion.invocation.clone(),
        completed_unix: Some(completion.completed_unix),
        expected_outputs: expected.len(),
        observed_outputs: observed.len(),
        detail: if status == PluginBuildProvenanceStatus::Matched {
            format!(
                "all {} tool executable(s) match the last managed build",
                observed.len()
            )
        } else {
            format!(
                "{} tool executable difference(s) from the last managed build",
                differences.len()
            )
        },
        differences,
        rebuild_command: Some("ost build".into()),
        override_accepted: false,
    })
}

fn untracked_build_provenance(observed: usize, detail: &str) -> PluginBuildProvenance {
    PluginBuildProvenance {
        status: PluginBuildProvenanceStatus::Untracked,
        origin: "external-or-unmanaged",
        build_fingerprint: None,
        invocation: None,
        completed_unix: None,
        expected_outputs: 0,
        observed_outputs: observed,
        differences: Vec::new(),
        detail: detail.to_string(),
        rebuild_command: None,
        override_accepted: false,
    }
}

/// The enclosing OpenStrata project root, if the caller is inside one.
fn enclosing_project_root() -> Option<Utf8PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let root = find_project_root(&cwd)?;
    Utf8PathBuf::from_path_buf(root).ok()
}

fn tool_dist_dir(root: &Utf8Path, id: &str, version: &str, target: &str) -> Utf8PathBuf {
    root.join("dist")
        .join("tools")
        .join(id)
        .join(version)
        .join(target)
}

/// `ost plugin package --workspace` — package every discovered bundle, in the
/// order the validated workspace graph puts them in.
///
/// The graph is the same one `plugin test --workspace` validates, and it is
/// validated here before anything is staged: packaging half a workspace and then
/// discovering a cycle leaves a dist directory whose contents nobody can explain.
///
/// Dependency order matters because a provider's package is what a dependent's
/// recorded closure points at. Packaging a consumer first would record a closure
/// naming an artifact that does not exist yet.
///
/// Per-bundle artifacts are always emitted. `--product` additionally wraps the
/// exact member archives and their sidecars into one aggregate artifact; it
/// never rebuilds an install view from source workspace paths.
fn package_workspace(
    target: Option<String>,
    profile: Option<String>,
    clean_stage: bool,
    with_debug: bool,
    allow_unmanaged_output: bool,
    product: bool,
    fmt: Format,
) -> Result<()> {
    let members = discover_workspace_members(Utf8Path::new("."))?;
    if members.bundles.is_empty() {
        return Err(
            Error::precondition("no plugin bundles found in the workspace member set").with_hint(
                "run from the workspace root, or pass a bundle path instead of --workspace",
            ),
        );
    }
    let bundles = members
        .bundles
        .iter()
        .map(|root| Bundle::load(root))
        .collect::<Result<Vec<_>>>()?;
    let libraries = members
        .libraries
        .iter()
        .map(|root| Library::load(root))
        .collect::<Result<Vec<_>>>()?;

    let graph = ost_plugin::validate_workspace_with_libraries(&bundles, &libraries);
    if !graph.passed {
        if fmt.is_json() {
            output::report(
                false,
                &serde_json::json!({
                    "workspace": true,
                    "graph": graph,
                    "packaged": 0,
                }),
            );
        } else {
            println!(
                "Workspace dependency graph: {} bundle(s), {} issue(s)",
                graph.nodes.len(),
                graph.issues.len()
            );
            for issue in &graph.issues {
                println!("  FAIL [{}] {}", issue.code, issue.message);
            }
        }
        std::process::exit(ost_core::Category::Validation.exit_code() as i32);
    }

    // Package providers before the bundles that depend on them.
    let order = graph.topological_order().ok_or_else(|| {
        Error::coded(
            "WORKSPACE_DEPENDENCY_GRAPH_INVALID",
            Category::Validation,
            "the validated workspace graph has no dependency order",
        )
    })?;
    let by_id: BTreeMap<String, Bundle> = bundles
        .iter()
        .cloned()
        .map(|bundle| (bundle.manifest.name().to_string(), bundle))
        .collect();

    let mut outcomes = Vec::new();
    for id in &order {
        let Some(bundle) = by_id.get(id) else {
            return Err(Error::coded(
                "WORKSPACE_DEPENDENCY_GRAPH_INVALID",
                Category::Validation,
                format!("validated workspace bundle '{id}' could not be loaded"),
            ));
        };
        let outcome = package_bundle(
            bundle,
            target.clone(),
            profile.clone(),
            clean_stage,
            with_debug,
            allow_unmanaged_output,
        )?;
        if !fmt.is_json() {
            println!(
                "== {} {} ==\n  {}",
                outcome.name, outcome.version, outcome.archive_path
            );
            if let Some((debug_name, _)) = &outcome.debug {
                println!("  debug: {debug_name} (sibling symbol package)");
            } else {
                println!("  debug: {}", outcome.debug_status.human_reason());
            }
            println!(
                "  build: {} ({})",
                outcome.build_provenance.status.as_str(),
                outcome.build_provenance.origin
            );
        }
        outcomes.push(outcome);
    }

    // Tools come last: nothing in the graph depends on an executable, and a tool
    // may load the libraries the bundles just staged.
    let tools = members
        .tools
        .iter()
        .map(|root| ost_plugin::Tool::load(root))
        .collect::<Result<Vec<_>>>()?;
    let mut tool_outcomes = Vec::new();
    for tool in &tools {
        let outcome = package_tool(
            tool,
            target.clone(),
            profile.clone(),
            clean_stage,
            with_debug,
            allow_unmanaged_output,
        )?;
        if !fmt.is_json() {
            println!(
                "== {} {} (tool) ==\n  {}",
                outcome.name, outcome.version, outcome.archive_path
            );
            println!(
                "  build: {} ({})",
                outcome.build_provenance.status.as_str(),
                outcome.build_provenance.origin
            );
        }
        tool_outcomes.push(outcome);
    }

    let members: Vec<ProductMember<'_>> = order
        .iter()
        .zip(outcomes.iter())
        .map(|(id, outcome)| ProductMember::bundle(id.clone(), outcome))
        .chain(
            tools
                .iter()
                .zip(tool_outcomes.iter())
                .map(|(tool, outcome)| ProductMember::tool(tool, outcome)),
        )
        .collect();
    let product_outcome = if product {
        Some(package_workspace_product(&members, clean_stage)?)
    } else {
        None
    };

    if fmt.is_json() {
        let package_json = |outcome: &PackageOutcome, kind: &str| {
            serde_json::json!({
                "name": outcome.name,
                "version": outcome.version,
                "member": kind,
                "target": outcome.id,
                "archive": portable(&outcome.archive_path),
                "archive_digest": outcome.packed.archive_digest,
                "archive_size": outcome.packed.archive_size,
                "debug_archive": outcome.debug.as_ref().map(|(name, _)| name.clone()),
                "debug_package": outcome.debug_status.json(),
                "build_provenance": outcome.build_provenance.json(),
            })
        };
        let packages: Vec<serde_json::Value> = outcomes
            .iter()
            .map(|outcome| package_json(outcome, "bundle"))
            .chain(
                tool_outcomes
                    .iter()
                    .map(|outcome| package_json(outcome, "tool")),
            )
            .collect();
        let warnings: Vec<serde_json::Value> = outcomes
            .iter()
            .chain(tool_outcomes.iter())
            .flat_map(|outcome| outcome.stage_warnings.clone())
            .chain(
                product_outcome
                    .iter()
                    .flat_map(|outcome| outcome.stage_warnings.clone()),
            )
            .collect();
        let product_json = product_outcome.as_ref().map(|outcome| {
            serde_json::json!({
                "name": outcome.name,
                "version": outcome.version,
                "target": outcome.target,
                "archive": portable(&outcome.archive_path),
                "archive_digest": outcome.packed.archive_digest,
                "archive_size": outcome.packed.archive_size,
                "members": outcome.members,
            })
        });
        output::report(
            true,
            &serde_json::json!({
                "workspace": true,
                "order": order,
                "tools": tool_outcomes.iter().map(|o| o.name.clone()).collect::<Vec<_>>(),
                "packages": packages,
                "product": product_json,
                "warnings": warnings,
            }),
        );
    } else {
        for warning in outcomes
            .iter()
            .chain(tool_outcomes.iter())
            .flat_map(|outcome| outcome.stage_warnings.iter())
            .chain(
                product_outcome
                    .iter()
                    .flat_map(|outcome| outcome.stage_warnings.iter()),
            )
        {
            if let Some(message) = warning["message"].as_str() {
                eprintln!("warning: {message}");
            }
        }
        println!(
            "\nWorkspace: {} package(s), in dependency order{}",
            order.len(),
            if tool_outcomes.is_empty() {
                String::new()
            } else {
                format!(", plus {} tool package(s)", tool_outcomes.len())
            }
        );
        if let Some(product) = &product_outcome {
            println!("Product:   {}", product.archive_path);
            println!("  digest:  {}", product.packed.archive_digest);
            println!("  members: {} exact package(s)", product.members);
        }
    }
    Ok(())
}

/// One member of an aggregate product: a packaged bundle, or a packaged
/// workspace-built executable.
///
/// The two differ in where they install and which producer manifest verifies
/// them, and in nothing else — which is the point. A tool reaching the product
/// through the same member archive shape is what keeps a release from needing a
/// second packaging path for its CLI deliverables (report 28 §3).
struct ProductMember<'a> {
    id: String,
    /// `bundle` or `tool`.
    kind: &'static str,
    /// Directories a tool contributes to the installed loader path, relative to
    /// its member root. Empty for a bundle, which carries its own activation
    /// contract instead.
    paths: Vec<String>,
    outcome: &'a PackageOutcome,
}

impl<'a> ProductMember<'a> {
    fn bundle(id: String, outcome: &'a PackageOutcome) -> Self {
        Self {
            id,
            kind: "bundle",
            paths: Vec::new(),
            outcome,
        }
    }

    fn tool(tool: &ost_plugin::Tool, outcome: &'a PackageOutcome) -> Self {
        Self {
            id: tool.id().to_string(),
            kind: "tool",
            paths: tool.built_directories(),
            outcome,
        }
    }

    fn is_tool(&self) -> bool {
        self.kind == "tool"
    }

    /// Where this member installs below the product prefix.
    fn destination(&self) -> String {
        let root = if self.is_tool() { "tools" } else { "bundles" };
        format!("{root}/{}", self.id)
    }

    fn license_pointer(&self) -> &'static str {
        if self.is_tool() {
            "/tool/license"
        } else {
            "/plugin/license"
        }
    }

    /// The member's `kind` field: a plugin kind for a bundle, and the artifact
    /// kind for a tool, which has no plugin kind to report.
    fn artifact_kind(&self, outcome: &PackageOutcome) -> serde_json::Value {
        if self.is_tool() {
            serde_json::Value::String("tool".into())
        } else {
            outcome
                .manifest
                .pointer("/plugin/kind")
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        }
    }
}

/// Build one aggregate artifact from the exact per-bundle package outputs.
///
/// The product deliberately contains member archives rather than recreating
/// bundle trees from the source workspace. A member's digest, producer manifest,
/// checksums, SBOM and optional provenance therefore remain independently
/// verifiable after the one product download is extracted.
fn package_workspace_product(
    members_in: &[ProductMember<'_>],
    clean_stage: bool,
) -> Result<ProductOutcome> {
    let first = members_in
        .first()
        .ok_or_else(|| {
            Error::precondition("cannot create a plugin product from an empty workspace")
        })?
        .outcome;
    if members_in
        .iter()
        .any(|member| member.outcome.id != first.id)
    {
        return Err(Error::validation(
            "all aggregate product members must target the same platform/profile variant",
        ));
    }
    let order: Vec<String> = members_in.iter().map(|member| member.id.clone()).collect();

    let cwd = std::env::current_dir().map_err(|e| Error::io("current directory", e))?;
    let project_root = find_project_root(&cwd).ok_or_else(|| {
        Error::precondition("--product requires an enclosing openstrata.toml project")
            .with_hint("run from the workspace project root")
    })?;
    let project_root = Utf8PathBuf::from_path_buf(project_root).map_err(|path| {
        Error::config(format!(
            "project root is not valid UTF-8: {}",
            path.display()
        ))
    })?;
    let project = load_project(&project_root)?;
    let name = project.project.name.clone();
    let version = project.effective_version(&project_root)?;
    let target = first.id.clone();

    let preferred_stage = project_root
        .join(STATE_DIR)
        .join("targets")
        .join(&target)
        .join("plugin-product-stage");
    let (stage, stage_warnings) = super::prepare_package_stage(&preferred_stage, clean_stage)?;

    let mut members = Vec::new();
    let mut licenses = Vec::new();
    for (position, member) in members_in.iter().enumerate() {
        let id = &member.id;
        let outcome = member.outcome;
        let member_root = Utf8Path::new("members").join(id);
        let archive_name = outcome.archive_path.file_name().ok_or_else(|| {
            Error::config(format!(
                "cannot determine package archive filename: {}",
                outcome.archive_path
            ))
        })?;
        copy_file_required(
            &outcome.archive_path,
            &member_root.join(archive_name),
            &stage,
        )?;

        let dist_dir = outcome.archive_path.parent().ok_or_else(|| {
            Error::config(format!(
                "cannot determine package dist directory: {}",
                outcome.archive_path
            ))
        })?;
        for required in ["manifest.json", "SHA256SUMS", ost_artifact::SBOM_FILE] {
            copy_file_required(
                &dist_dir.join(required),
                &member_root.join(required),
                &stage,
            )?;
        }
        let mut evidence = vec![format!(
            "{}/{}",
            portable(&member_root),
            ost_artifact::SBOM_FILE
        )];
        let provenance = dist_dir.join(ost_artifact::PROVENANCE_FILE);
        if provenance.as_std_path().is_file() {
            copy_file_required(
                &provenance,
                &member_root.join(ost_artifact::PROVENANCE_FILE),
                &stage,
            )?;
            evidence.push(format!(
                "{}/{}",
                portable(&member_root),
                ost_artifact::PROVENANCE_FILE
            ));
        }

        let debug = outcome.debug.as_ref().map(|(debug_name, packed)| {
            (
                debug_name.clone(),
                packed.archive_digest.clone(),
                packed.archive_size,
            )
        });
        if let Some((debug_name, _, _)) = &debug {
            copy_file_required(
                &dist_dir.join(debug_name),
                &member_root.join(debug_name),
                &stage,
            )?;
        }

        if let Some(license) = outcome
            .manifest
            .pointer(member.license_pointer())
            .and_then(|value| value.as_str())
        {
            if !licenses.iter().any(|existing| existing == license) {
                licenses.push(license.to_string());
            }
        }

        let debug_json = debug.map(|(archive, digest, size)| {
            serde_json::json!({
                "archive": format!("{}/{archive}", portable(&member_root)),
                "archive_digest": digest,
                "archive_size": size,
            })
        });
        members.push(serde_json::json!({
            "id": id,
            "position": position,
            // Which member shape this is, and so which tree it installs into and
            // which producer manifest verification expects. Absent in products
            // produced before v0.21.0, where every member was a bundle.
            "member": member.kind,
            "destination": member.destination(),
            "paths": member.paths,
            "name": outcome.name,
            "version": outcome.version,
            "kind": member.artifact_kind(outcome),
            "archive": format!("{}/{archive_name}", portable(&member_root)),
            "archive_digest": outcome.packed.archive_digest,
            "archive_size": outcome.packed.archive_size,
            "manifest": format!("{}/manifest.json", portable(&member_root)),
            "checksums": format!("{}/SHA256SUMS", portable(&member_root)),
            "evidence": evidence,
            "debug": debug_json,
            "dependencies": outcome.manifest.get("dependencies"),
        }));
    }

    let contract = serde_json::json!({
        "schema": "openstrata.plugin-product/v1alpha1",
        "name": name,
        "version": version,
        "target": target,
        "install": {
            "layout": "members/<member-id>/",
            // The default for a bundle member; each member carries the exact
            // destination its kind installs into (`tools/<id>/` for a tool).
            "destination": "bundles/<bundle-id>/",
            "os": first.os.as_str(),
            "order": order,
            "activation": "openstrata.activation.json",
            "contract": "run `ost plugin product verify`, then `ost plugin product install --prefix <dir>`; members are verified and installed in dependency order",
        },
        "members": members,
    });
    write_text(
        &stage.join("openstrata.product.json"),
        &pretty_json(&contract)?,
    )?;

    let staged = stage_files(&stage).map_err(|e| {
        if e.kind() == std::io::ErrorKind::InvalidData {
            Error::validation(e.to_string())
        } else {
            Error::io(stage.to_string(), e)
        }
    })?;
    let archive_name = format!("{name}-{version}-{target}-plugin-product.tar.zst");
    let dist_dir = project_root
        .join("dist")
        .join("products")
        .join(&name)
        .join(&version)
        .join(&target);
    let archive_path = dist_dir.join(&archive_name);
    let pack_opts = PackOptions {
        mtime: ost_build::source_date_epoch(),
        ..PackOptions::default()
    };
    let packed = pack_dir_with(&stage, &archive_path, &staged, pack_opts, &mut |_| {})
        .map_err(|e| Error::io(archive_path.to_string(), e))?;

    let created = ost_build::source_date_epoch();
    let mut provenance = first
        .manifest
        .get("provenance")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({ "validation": { "passed": true } }));
    provenance["validation"] = serde_json::json!({
        "passed": true,
        "product": "openstrata.product.json",
        "members": members_in.len(),
    });
    let mut manifest = serde_json::json!({
        "schema": 1,
        "kind": ost_artifact::PLUGIN_PRODUCT_KIND,
        "name": name,
        "version": version,
        "target": target,
        "licenses": licenses,
        "archive": archive_name,
        "archive_digest": packed.archive_digest,
        "archive_size": packed.archive_size,
        "total_size": packed.total_size,
        "created_unix": created,
        "producer": format!("ost {}", env!("CARGO_PKG_VERSION")),
        "provenance": provenance,
        "product": "openstrata.product.json",
        "install_order": order,
        "members": members,
        "files": packed.files.iter().map(|file| file.manifest_json()).collect::<Vec<_>>(),
    });
    let evidence = ost_artifact::generate_evidence(&dist_dir, &mut manifest)?;
    write_text(&dist_dir.join("manifest.json"), &pretty_json(&manifest)?)?;
    let mut sha_lines = vec![format!(
        "{}  {archive_name}",
        bare_sha256(&packed.archive_digest)
    )];
    for layer in &evidence {
        sha_lines.push(format!("{}  {}", bare_sha256(&layer.digest), layer.path));
    }
    write_text(&dist_dir.join("SHA256SUMS"), &sha_lines.join("\n"))?;

    Ok(ProductOutcome {
        name,
        version,
        target,
        archive_path,
        packed,
        members: members_in.len(),
        stage_warnings,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginProductContract {
    schema: String,
    name: String,
    version: String,
    target: String,
    install: PluginProductInstall,
    members: Vec<PluginProductMember>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginProductInstall {
    layout: String,
    order: Vec<String>,
    contract: String,
    /// Target OS of every member, recorded so a product whose members are all
    /// tools — none of which carries an activation contract — can still write
    /// the aggregate loader variable. Absent before v0.21.0.
    #[serde(default)]
    os: Option<String>,
    #[serde(default = "default_product_destination")]
    destination: String,
    #[serde(default = "default_product_activation")]
    activation: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginProductMember {
    id: String,
    position: usize,
    /// Which member shape this is. Products produced before v0.21.0 carried
    /// only bundles and no discriminator, so an absent field means `bundle`.
    #[serde(default)]
    member: ProductMemberKind,
    /// Where this member installs, relative to the product prefix. Absent in
    /// pre-v0.21.0 products, which installed every member under `bundles/`.
    #[serde(default)]
    destination: Option<String>,
    /// Loader directories a tool member contributes, relative to its own root.
    #[serde(default)]
    paths: Vec<String>,
    name: String,
    version: String,
    kind: String,
    archive: String,
    archive_digest: String,
    archive_size: u64,
    manifest: String,
    checksums: String,
    evidence: Vec<String>,
    debug: RequiredProductDebug,
    #[serde(rename = "dependencies")]
    dependencies: serde_json::Value,
}

/// The member shapes an aggregate product can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ProductMemberKind {
    #[default]
    Bundle,
    Tool,
}

impl PluginProductMember {
    fn is_tool(&self) -> bool {
        self.member == ProductMemberKind::Tool
    }

    /// Where this member installs below the product prefix, defaulting to the
    /// pre-v0.21.0 bundle layout when the product does not say.
    fn destination(&self) -> String {
        self.destination
            .clone()
            .unwrap_or_else(|| format!("bundles/{}", self.id))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginProductDebug {
    archive: String,
    archive_digest: String,
    archive_size: u64,
}

#[derive(Debug, Deserialize)]
#[serde(transparent)]
struct RequiredProductDebug(Option<PluginProductDebug>);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductMemberActivation {
    schema: String,
    target_os: String,
    root: String,
    environment: serde_json::Value,
    plugin_paths: Vec<String>,
    library_paths: Vec<String>,
    python_paths: Vec<String>,
    entrypoints: serde_json::Value,
    python_dll_search: serde_json::Value,
}

#[derive(Debug)]
struct ProductArchiveSource {
    archive: Utf8PathBuf,
    digest: String,
    size: u64,
}

#[derive(Debug)]
struct TemporaryProductTree {
    path: Utf8PathBuf,
    remove_on_drop: bool,
}

impl Drop for TemporaryProductTree {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = std::fs::remove_dir_all(self.path.as_std_path());
        }
    }
}

struct VerifiedPluginProduct {
    source: ProductArchiveSource,
    tree: TemporaryProductTree,
    contract: PluginProductContract,
}

fn default_product_destination() -> String {
    "bundles/<bundle-id>/".into()
}

fn default_product_activation() -> String {
    "openstrata.activation.json".into()
}

fn product(command: ProductCmd, fmt: Format) -> Result<()> {
    match command {
        ProductCmd::Verify {
            product,
            expect_digest,
        } => {
            let verified = verify_plugin_product(&product, expect_digest.as_deref())?;
            report_product_verification(&verified, fmt);
            Ok(())
        }
        ProductCmd::Install {
            product,
            prefix,
            expect_digest,
        } => install_plugin_product(&product, &prefix, expect_digest.as_deref(), fmt),
    }
}

fn verify_plugin_product(
    product: &str,
    expect_digest: Option<&str>,
) -> Result<VerifiedPluginProduct> {
    let source = resolve_product_archive(product, expect_digest)?;
    let tree = temporary_product_tree(std::env::temp_dir().as_path(), "verify")?;
    ost_artifact::extract_archive(&source.archive, &source.digest, &tree.path)?;

    let contract_path = tree.path.join("openstrata.product.json");
    let source_text = std::fs::read_to_string(contract_path.as_std_path())
        .map_err(|error| Error::io(contract_path.to_string(), error))?;
    let contract: PluginProductContract = serde_json::from_str(&source_text)
        .map_err(|error| Error::parse(contract_path.to_string(), anyhow::Error::new(error)))?;
    validate_product_contract(&contract)?;

    for member in &contract.members {
        verify_product_member(&tree.path, &contract.target, member)?;
    }

    Ok(VerifiedPluginProduct {
        source,
        tree,
        contract,
    })
}

fn resolve_product_archive(
    product: &str,
    expect_digest: Option<&str>,
) -> Result<ProductArchiveSource> {
    if let Some(digest) = expect_digest {
        validate_sha256_digest(digest, "--expect-digest")?;
    }
    let input = Utf8PathBuf::from(product);
    if !input.as_std_path().exists() {
        return Err(Error::precondition(format!(
            "plugin product path does not exist: {input}"
        )));
    }
    let explicit_archive =
        input.as_std_path().is_file() && input.file_name() != Some("manifest.json");

    let manifest_path = if input.as_std_path().is_dir() {
        Some(input.join("manifest.json"))
    } else if input.file_name() == Some("manifest.json") {
        Some(input.clone())
    } else {
        let sibling = input
            .parent()
            .unwrap_or_else(|| Utf8Path::new("."))
            .join("manifest.json");
        sibling.as_std_path().is_file().then_some(sibling)
    };

    if let Some(manifest_path) = manifest_path.filter(|path| path.as_std_path().is_file()) {
        let source = match std::fs::read_to_string(manifest_path.as_std_path()) {
            Ok(source) => source,
            Err(_) if explicit_archive => {
                return product_archive_from_file(&input, expect_digest);
            }
            Err(error) => return Err(Error::io(manifest_path.to_string(), error)),
        };
        let manifest: serde_json::Value = match serde_json::from_str(&source) {
            Ok(manifest) => manifest,
            Err(_) if explicit_archive => {
                return product_archive_from_file(&input, expect_digest);
            }
            Err(error) => {
                return Err(Error::parse(
                    manifest_path.to_string(),
                    anyhow::Error::new(error),
                ));
            }
        };
        if manifest["kind"] != ost_artifact::PLUGIN_PRODUCT_KIND {
            if explicit_archive {
                return product_archive_from_file(&input, expect_digest);
            }
            return Err(Error::validation(format!(
                "'{manifest_path}' is not an aggregate plugin product manifest"
            )));
        }
        let archive_name = match manifest["archive"].as_str() {
            Some(archive_name) => archive_name,
            None if explicit_archive => {
                return product_archive_from_file(&input, expect_digest);
            }
            None => {
                return Err(Error::validation(format!(
                    "'{manifest_path}' is missing string field 'archive'"
                )));
            }
        };
        if explicit_archive && input.file_name() != Some(archive_name) {
            // An explicitly named archive is authoritative unless the sibling
            // product manifest actually names that archive.
            return product_archive_from_file(&input, expect_digest);
        }
        let digest = manifest["archive_digest"].as_str().ok_or_else(|| {
            Error::validation(format!(
                "'{manifest_path}' is missing string field 'archive_digest'"
            ))
        })?;
        validate_sha256_digest(digest, "manifest archive_digest")?;
        if expect_digest.is_some_and(|expected| expected != digest) {
            return Err(Error::coded(
                "PLUGIN_PRODUCT_DIGEST_MISMATCH",
                Category::Validation,
                format!(
                    "product manifest pins {digest}, but --expect-digest requested {}",
                    expect_digest.unwrap_or_default()
                ),
            ));
        }
        let root = manifest_path.parent().unwrap_or_else(|| Utf8Path::new("."));
        let archive = safe_product_join(root, archive_name, "manifest archive")?;
        let (actual, size) = digest_file(&archive)?;
        if actual != digest {
            return Err(product_digest_mismatch(&archive, digest, &actual));
        }
        return Ok(ProductArchiveSource {
            archive,
            digest: digest.to_string(),
            size,
        });
    }

    if input.as_std_path().is_dir() {
        return Err(Error::precondition(format!(
            "product directory '{input}' has no manifest.json"
        )));
    }
    product_archive_from_file(&input, expect_digest)
}

fn product_archive_from_file(
    archive: &Utf8Path,
    expect_digest: Option<&str>,
) -> Result<ProductArchiveSource> {
    let (actual, size) = digest_file(archive)?;
    if let Some(expected) = expect_digest {
        if actual != expected {
            return Err(product_digest_mismatch(archive, expected, &actual));
        }
    }
    Ok(ProductArchiveSource {
        archive: archive.to_path_buf(),
        digest: actual,
        size,
    })
}

fn product_digest_mismatch(path: &Utf8Path, expected: &str, actual: &str) -> Error {
    Error::coded(
        "PLUGIN_PRODUCT_DIGEST_MISMATCH",
        Category::Validation,
        format!("product archive '{path}' hashes to {actual}, expected {expected}"),
    )
}

fn digest_file(path: &Utf8Path) -> Result<(String, u64)> {
    let mut file =
        File::open(path.as_std_path()).map_err(|error| Error::io(path.to_string(), error))?;
    ost_core::digest::sha256_hex_reader(&mut file)
        .map_err(|error| Error::io(path.to_string(), error))
}

fn validate_sha256_digest(digest: &str, field: &str) -> Result<()> {
    let Some(hex) = digest.strip_prefix("sha256:") else {
        return Err(Error::validation(format!(
            "{field} must be a full sha256:<64 hex> digest"
        )));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::validation(format!(
            "{field} must be a full sha256:<64 hex> digest"
        )));
    }
    Ok(())
}

fn validate_product_contract(contract: &PluginProductContract) -> Result<()> {
    if contract.schema != "openstrata.plugin-product/v1alpha1" {
        return Err(Error::config(format!(
            "unsupported plugin product schema '{}'",
            contract.schema
        )));
    }
    for (field, value) in [
        ("name", contract.name.as_str()),
        ("version", contract.version.as_str()),
        ("target", contract.target.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(Error::validation(format!(
                "plugin product {field} must not be empty"
            )));
        }
    }
    // `<bundle-id>` is the pre-v0.21.0 spelling of the same layout, kept
    // readable so a product produced before tool members verifies unchanged.
    if !matches!(
        contract.install.layout.as_str(),
        "members/<member-id>/" | "members/<bundle-id>/"
    ) {
        return Err(Error::validation(format!(
            "unsupported product archive layout '{}'",
            contract.install.layout
        )));
    }
    if contract.install.destination != "bundles/<bundle-id>/" {
        return Err(Error::validation(format!(
            "unsupported product installation layout '{}'",
            contract.install.destination
        )));
    }
    if contract.install.activation != "openstrata.activation.json" {
        return Err(Error::validation(format!(
            "unsupported product activation contract '{}'",
            contract.install.activation
        )));
    }
    if contract.install.contract.trim().is_empty() {
        return Err(Error::validation(
            "plugin product install.contract must not be empty",
        ));
    }
    if contract.members.is_empty() {
        return Err(Error::validation(
            "plugin product must contain at least one member",
        ));
    }

    let mut ids = BTreeSet::new();
    for (position, member) in contract.members.iter().enumerate() {
        validate_product_member_identity(member)?;
        if member.position != position {
            return Err(Error::validation(format!(
                "product member '{}' has position {}, expected {position}",
                member.id, member.position
            )));
        }
        if !ids.insert(member.id.clone()) {
            return Err(Error::validation(format!(
                "plugin product repeats member id '{}'",
                member.id
            )));
        }
    }
    let member_order = contract
        .members
        .iter()
        .map(|member| member.id.as_str())
        .collect::<Vec<_>>();
    let install_order = contract
        .install
        .order
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    if install_order != member_order {
        return Err(Error::validation(format!(
            "plugin product install order {install_order:?} does not match member order {member_order:?}"
        )));
    }
    Ok(())
}

fn validate_product_member_identity(member: &PluginProductMember) -> Result<()> {
    if member.id.is_empty()
        || member.id != member.name
        || member
            .id
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
    {
        return Err(Error::validation(format!(
            "plugin product member id/name must be one matching portable identity, got id='{}' name='{}'",
            member.id, member.name
        )));
    }
    for (field, value) in [
        ("version", member.version.as_str()),
        ("kind", member.kind.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(Error::validation(format!(
                "product member '{}' {field} must not be empty",
                member.id
            )));
        }
    }
    // The destination is product-authored and is joined onto the caller's
    // install prefix, so it is checked here — at verification, before install
    // has a chance to act on it — and must stay inside its own member root.
    let destination = member.destination();
    safe_product_join(
        Utf8Path::new("."),
        &destination,
        &format!("product member '{}' destination", member.id),
    )?;
    let expected_root = if member.is_tool() { "tools" } else { "bundles" };
    if destination != format!("{expected_root}/{}", member.id) {
        return Err(Error::validation(format!(
            "product member '{}' destination '{destination}' is not '{expected_root}/{}'",
            member.id, member.id
        )));
    }
    if !member.paths.is_empty() && !member.is_tool() {
        return Err(Error::validation(format!(
            "product member '{}' declares loader paths, which only a tool member does \
             (a bundle carries its own activation contract)",
            member.id
        )));
    }
    validate_sha256_digest(
        &member.archive_digest,
        &format!("product member '{}' archive_digest", member.id),
    )?;
    if member.evidence.is_empty() {
        return Err(Error::validation(format!(
            "product member '{}' must carry at least one evidence file",
            member.id
        )));
    }
    if !member.dependencies.is_null() && !member.dependencies.is_object() {
        return Err(Error::validation(format!(
            "product member '{}' dependencies must be an object or null",
            member.id
        )));
    }
    if let Some(debug) = &member.debug.0 {
        validate_sha256_digest(
            &debug.archive_digest,
            &format!("product member '{}' debug archive_digest", member.id),
        )?;
    }
    Ok(())
}

fn verify_product_member(
    root: &Utf8Path,
    product_target: &str,
    member: &PluginProductMember,
) -> Result<()> {
    let member_prefix = format!("members/{}/", member.id);
    for (field, relative) in [
        ("archive", member.archive.as_str()),
        ("manifest", member.manifest.as_str()),
        ("checksums", member.checksums.as_str()),
    ] {
        if !relative.starts_with(&member_prefix) {
            return Err(Error::validation(format!(
                "product member '{}' {field} must stay under '{member_prefix}'",
                member.id
            )));
        }
    }
    let archive = safe_product_join(root, &member.archive, "product member archive")?;
    let (digest, size) = digest_file(&archive)?;
    if digest != member.archive_digest || size != member.archive_size {
        return Err(Error::coded(
            "PLUGIN_PRODUCT_MEMBER_DIGEST_MISMATCH",
            Category::Validation,
            format!(
                "product member '{}' archive is {digest} ({size} bytes), expected {} ({} bytes)",
                member.id, member.archive_digest, member.archive_size
            ),
        ));
    }

    let member_manifest = safe_product_join(root, &member.manifest, "product member manifest")?;
    let manifest_source = std::fs::read_to_string(member_manifest.as_std_path())
        .map_err(|error| Error::io(member_manifest.to_string(), error))?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest_source)
        .map_err(|error| Error::parse(member_manifest.to_string(), anyhow::Error::new(error)))?;
    verify_product_member_manifest(product_target, member, &manifest)?;

    let checksums = safe_product_join(root, &member.checksums, "product member checksums")?;
    verify_member_checksums(
        checksums
            .parent()
            .ok_or_else(|| Error::validation("product member checksums have no parent"))?,
        &checksums,
        archive
            .file_name()
            .ok_or_else(|| Error::validation("product member archive has no filename"))?,
    )?;

    for evidence in &member.evidence {
        if !evidence.starts_with(&member_prefix) {
            return Err(Error::validation(format!(
                "product member '{}' evidence path must stay under '{member_prefix}'",
                member.id
            )));
        }
        let path = safe_product_join(root, evidence, "product member evidence")?;
        if !path.as_std_path().is_file() {
            return Err(Error::validation(format!(
                "product member '{}' is missing evidence '{evidence}'",
                member.id
            )));
        }
    }
    if let Some(debug) = &member.debug.0 {
        if !debug.archive.starts_with(&member_prefix) {
            return Err(Error::validation(format!(
                "product member '{}' debug archive must stay under '{member_prefix}'",
                member.id
            )));
        }
        let path = safe_product_join(root, &debug.archive, "product member debug archive")?;
        let (actual, size) = digest_file(&path)?;
        if actual != debug.archive_digest || size != debug.archive_size {
            return Err(Error::coded(
                "PLUGIN_PRODUCT_MEMBER_DIGEST_MISMATCH",
                Category::Validation,
                format!(
                    "product member '{}' debug archive is {actual} ({size} bytes), expected {} ({} bytes)",
                    member.id, debug.archive_digest, debug.archive_size
                ),
            ));
        }
    }

    let expanded = root.join("expanded").join(&member.id);
    ost_artifact::extract_archive(&archive, &member.archive_digest, &expanded)?;
    verify_member_manifest_files(&expanded, &manifest)?;
    // Each member shape is loaded by its own model, so the extracted tree is
    // checked against the contract it actually claims rather than only unpacking
    // cleanly. A tool additionally has to still contain its executables: an
    // archive that lost them verifies byte-perfectly and delivers nothing.
    if member.is_tool() {
        let tool = ost_plugin::Tool::load(&expanded).map_err(|error| {
            Error::validation(format!(
                "installed product member '{}' is not a valid workspace tool: {error}",
                member.id
            ))
        })?;
        let windows = product_target.contains("-windows-");
        tool.locate_executables(&expanded, windows)
            .map_err(|error| {
                Error::validation(format!(
                    "installed product member '{}' is missing a declared executable: {error}",
                    member.id
                ))
            })?;
    } else {
        Bundle::load(&expanded).map_err(|error| {
            Error::validation(format!(
                "installed product member '{}' is not a valid plugin bundle: {error}",
                member.id
            ))
        })?;
    }
    Ok(())
}

fn verify_product_member_manifest(
    product_target: &str,
    member: &PluginProductMember,
    manifest: &serde_json::Value,
) -> Result<()> {
    let expected_archive = Utf8Path::new(&member.archive)
        .file_name()
        .unwrap_or_default();
    // A tool member is verified against the tool producer manifest, which has a
    // `tool` identity and no plugin kind. Everything below it — archive digest,
    // checksums, evidence, file inventory — is identical for both shapes.
    let (identity, artifact_kind) = if member.is_tool() {
        ("/tool", ost_artifact::TOOL_KIND)
    } else {
        ("/plugin", ost_artifact::PLUGIN_BUNDLE_KIND)
    };
    let name_field = if member.is_tool() {
        "tool.id"
    } else {
        "plugin.name"
    };
    let name_pointer = if member.is_tool() {
        "/tool/id"
    } else {
        "/plugin/name"
    };
    let checks = [
        ("kind", manifest["kind"].as_str(), Some(artifact_kind)),
        (
            name_field,
            manifest.pointer(name_pointer).and_then(|v| v.as_str()),
            Some(member.name.as_str()),
        ),
        (
            "version",
            manifest
                .pointer(&format!("{identity}/version"))
                .and_then(|value| value.as_str()),
            Some(member.version.as_str()),
        ),
        (
            "kind detail",
            if member.is_tool() {
                Some("tool")
            } else {
                manifest
                    .pointer("/plugin/kind")
                    .and_then(|value| value.as_str())
            },
            Some(member.kind.as_str()),
        ),
        ("target", manifest["target"].as_str(), Some(product_target)),
        (
            "archive",
            manifest["archive"].as_str(),
            Some(expected_archive),
        ),
        (
            "archive_digest",
            manifest["archive_digest"].as_str(),
            Some(member.archive_digest.as_str()),
        ),
    ];
    for (field, actual, expected) in checks {
        if actual != expected {
            return Err(Error::validation(format!(
                "product member '{}' manifest {field} is {actual:?}, expected {expected:?}",
                member.id
            )));
        }
    }
    Ok(())
}

fn verify_member_checksums(
    member_root: &Utf8Path,
    checksums: &Utf8Path,
    required_archive: &str,
) -> Result<()> {
    let source = std::fs::read_to_string(checksums.as_std_path())
        .map_err(|error| Error::io(checksums.to_string(), error))?;
    let mut paths = BTreeSet::new();
    for (line_number, line) in source.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let (expected, relative) = line.split_once("  ").ok_or_else(|| {
            Error::validation(format!(
                "'{checksums}' line {} is not '<sha256>  <path>'",
                line_number + 1
            ))
        })?;
        if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error::validation(format!(
                "'{checksums}' line {} has an invalid SHA-256",
                line_number + 1
            )));
        }
        if !paths.insert(relative.to_string()) {
            return Err(Error::validation(format!(
                "'{checksums}' repeats path '{relative}'"
            )));
        }
        let path = safe_product_join(member_root, relative, "member checksum path")?;
        let (actual, _) = digest_file(&path)?;
        if bare_sha256(&actual) != expected {
            return Err(Error::coded(
                "PLUGIN_PRODUCT_MEMBER_CHECKSUM_MISMATCH",
                Category::Validation,
                format!(
                    "member file '{relative}' hashes to {}, expected sha256:{expected}",
                    bare_sha256(&actual)
                ),
            ));
        }
    }
    if !paths.contains(required_archive) {
        return Err(Error::validation(format!(
            "'{checksums}' does not cover member archive '{required_archive}'"
        )));
    }
    Ok(())
}

fn verify_member_manifest_files(root: &Utf8Path, manifest: &serde_json::Value) -> Result<()> {
    let files = manifest["files"].as_array().ok_or_else(|| {
        Error::validation("product member manifest is missing array field 'files'")
    })?;
    for entry in files {
        let relative = entry["path"].as_str().ok_or_else(|| {
            Error::validation("product member manifest file entry is missing string 'path'")
        })?;
        let expected = entry["sha256"].as_str().ok_or_else(|| {
            Error::validation(format!(
                "product member manifest entry '{relative}' is missing string 'sha256'"
            ))
        })?;
        validate_sha256_digest(expected, "product member file sha256")?;
        let expected_size = entry["size"].as_u64().ok_or_else(|| {
            Error::validation(format!(
                "product member manifest entry '{relative}' is missing integer 'size'"
            ))
        })?;
        let path = safe_product_join(root, relative, "product member file")?;
        let (actual, size) = digest_file(&path)?;
        if actual != expected || size != expected_size {
            return Err(Error::coded(
                "PLUGIN_PRODUCT_MEMBER_FILE_MISMATCH",
                Category::Validation,
                format!(
                    "installed member file '{relative}' is {actual} ({size} bytes), expected {expected} ({expected_size} bytes)"
                ),
            ));
        }
    }
    Ok(())
}

fn safe_product_join(root: &Utf8Path, relative: &str, field: &str) -> Result<Utf8PathBuf> {
    let bytes = relative.as_bytes();
    let has_drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if relative.is_empty()
        || relative.starts_with('/')
        || relative.starts_with('\\')
        || relative.contains('\\')
        || relative.contains(':')
        || has_drive
        || relative
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(Error::validation(format!(
            "{field} must be a portable path below the product root, got '{relative}'"
        )));
    }
    Ok(root.join(relative))
}

fn temporary_product_tree(parent: &std::path::Path, label: &str) -> Result<TemporaryProductTree> {
    let parent = Utf8PathBuf::from_path_buf(parent.to_path_buf()).map_err(|path| {
        Error::config(format!(
            "temporary directory is not UTF-8: {}",
            path.display()
        ))
    })?;
    std::fs::create_dir_all(parent.as_std_path())
        .map_err(|error| Error::io(parent.to_string(), error))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = parent.join(format!(
        ".ost-plugin-product-{label}-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir(&path).map_err(|error| Error::io(path.to_string(), error))?;
    Ok(TemporaryProductTree {
        path,
        remove_on_drop: true,
    })
}

fn install_plugin_product(
    product: &str,
    prefix: &str,
    expect_digest: Option<&str>,
    fmt: Format,
) -> Result<()> {
    let verified = verify_plugin_product(product, expect_digest)?;
    let prefix = Utf8PathBuf::from(prefix);
    if prefix.as_std_path().exists() {
        return Err(Error::precondition(format!(
            "product installation prefix already exists: {prefix}"
        ))
        .with_hint(
            "choose a new empty prefix; product install never overwrites an existing tree",
        ));
    }
    let parent = prefix
        .parent()
        .filter(|parent| !parent.as_str().is_empty())
        .unwrap_or_else(|| Utf8Path::new("."));
    let mut staging = temporary_product_tree(parent.as_std_path(), "install")?;

    for member in &verified.contract.members {
        let expanded = verified.tree.path.join("expanded").join(&member.id);
        let relative = member.destination();
        // The destination is product-authored data: validate it before it is
        // joined onto the caller's prefix.
        safe_product_join(Utf8Path::new("."), &relative, "product member destination")?;
        copy_tree_required(&expanded, Utf8Path::new(&relative), &staging.path)?;
    }
    write_product_activation(&staging.path, &verified.contract)?;
    let receipt = serde_json::json!({
        "schema": "openstrata.plugin-product-install/v1alpha1",
        "name": verified.contract.name,
        "version": verified.contract.version,
        "target": verified.contract.target,
        "archive_digest": verified.source.digest,
        "members": verified.contract.install.order,
        "layout": verified.contract.install.destination,
        "activation": verified.contract.install.activation,
    });
    write_text(
        &staging.path.join("openstrata.product-install.json"),
        &pretty_json(&receipt)?,
    )?;
    std::fs::rename(staging.path.as_std_path(), prefix.as_std_path())
        .map_err(|error| Error::io(format!("{} -> {prefix}", staging.path), error))?;
    // The path moved successfully; the installed tree now belongs to the user.
    staging.remove_on_drop = false;

    if fmt.is_json() {
        output::success(&serde_json::json!({
            "installed": true,
            "name": verified.contract.name,
            "version": verified.contract.version,
            "target": verified.contract.target,
            "archive_digest": verified.source.digest,
            "prefix": prefix,
            "members": verified.contract.install.order,
            "activation": prefix.join("openstrata.activation.json"),
        }));
    } else {
        println!(
            "Installed plugin product {} {} for {}",
            verified.contract.name, verified.contract.version, verified.contract.target
        );
        println!("  digest:     {}", verified.source.digest);
        println!("  prefix:     {prefix}");
        println!(
            "  members:    {} ({})",
            verified.contract.members.len(),
            verified.contract.install.order.join(", ")
        );
        println!(
            "  activation: {}",
            prefix.join("openstrata.activation.json")
        );
    }
    Ok(())
}

fn write_product_activation(root: &Utf8Path, contract: &PluginProductContract) -> Result<()> {
    let mut target_os: Option<Os> = None;
    let mut plugin_paths = Vec::new();
    let mut library_paths = Vec::new();
    let mut python_paths = Vec::new();
    for member in &contract.members {
        // A tool has no activation contract of its own — it is an executable,
        // not a plugin USD registers. Its declared directories still join the
        // aggregate loader path so the shared libraries shipped beside it
        // resolve where it was installed.
        if member.is_tool() {
            let destination = member.destination();
            for relative in &member.paths {
                safe_product_join(Utf8Path::new("."), relative, "tool loader path")?;
                let aggregate = format!("{destination}/{relative}");
                if !library_paths.contains(&aggregate) {
                    library_paths.push(aggregate);
                }
            }
            continue;
        }
        let member_root = root.join(member.destination());
        let path = member_root.join("openstrata.activation.json");
        let source = std::fs::read_to_string(path.as_std_path())
            .map_err(|error| Error::io(path.to_string(), error))?;
        let activation: ProductMemberActivation = serde_json::from_str(&source)
            .map_err(|error| Error::parse(path.to_string(), anyhow::Error::new(error)))?;
        if activation.schema != "openstrata.activation/v1alpha1" || activation.root != "." {
            return Err(Error::validation(format!(
                "product member '{}' has unsupported activation contract",
                member.id
            )));
        }
        let os = parse_product_os(&activation.target_os)?;
        if target_os.is_some_and(|selected| selected != os) {
            return Err(Error::validation(
                "plugin product members target different operating systems",
            ));
        }
        target_os = Some(os);
        // Deserializing the complete strict member contract above ensures these
        // fields were present even though the merged contract computes them.
        let _ = (
            &activation.environment,
            &activation.entrypoints,
            &activation.python_dll_search,
        );
        let destination = member.destination();
        extend_product_activation_paths(&mut plugin_paths, &destination, &activation.plugin_paths)?;
        extend_product_activation_paths(
            &mut library_paths,
            &destination,
            &activation.library_paths,
        )?;
        extend_product_activation_paths(&mut python_paths, &destination, &activation.python_paths)?;
    }
    // A product whose only members are tools has no member activation contract
    // to read the OS from, so the product records it directly. Older products
    // carry no such field and always have a bundle member that does.
    let target_os = match (target_os, contract.install.os.as_deref()) {
        (Some(os), _) => Some(os),
        (None, Some(declared)) => Some(parse_product_os(declared)?),
        (None, None) => None,
    };
    let target_os = target_os
        .ok_or_else(|| Error::validation("cannot write activation for an empty plugin product"))?;
    let loader_env = activation_loader_key(target_os);
    let activation = serde_json::json!({
        "schema": "openstrata.activation/v1alpha1",
        "target_os": target_os.as_str(),
        "root": ".",
        "environment": {
            "plugin": "PXR_PLUGINPATH_NAME",
            "loader": loader_env,
            "python": "PYTHONPATH",
        },
        "plugin_paths": plugin_paths,
        "library_paths": library_paths,
        "python_paths": python_paths,
        "entrypoints": {
            "powershell": "activate.ps1",
            "bash": "activate.sh",
            "python": "openstrata_activate.py",
        },
        "python_dll_search": {
            "windows": "import openstrata_activate before importing pxr; the module retains os.add_dll_directory handles",
        },
    });
    write_text(
        &root.join("openstrata.activation.json"),
        &pretty_json(&activation)?,
    )?;
    write_text(
        &root.join("activate.ps1"),
        &render_powershell_activation(&plugin_paths, &library_paths, &python_paths, loader_env),
    )?;
    write_text(
        &root.join("activate.sh"),
        &render_bash_activation(
            &plugin_paths,
            &library_paths,
            &python_paths,
            loader_env,
            target_os,
        ),
    )?;
    write_text(
        &root.join("openstrata_activate.py"),
        &render_python_activation(&plugin_paths, &library_paths, &python_paths),
    )
}

/// Prefix a member's own activation paths with where that member installs.
///
/// `destination` is the member's installed root (`bundles/<id>`, or
/// `tools/<id>`), already validated by the caller.
fn extend_product_activation_paths(
    output: &mut Vec<String>,
    destination: &str,
    relative_paths: &[String],
) -> Result<()> {
    for relative in relative_paths {
        // Validate the member-authored relative path independently before
        // prefixing it into the aggregate installation layout.
        safe_product_join(Utf8Path::new("."), relative, "member activation path")?;
        let aggregate = format!("{destination}/{relative}");
        if !output.contains(&aggregate) {
            output.push(aggregate);
        }
    }
    Ok(())
}

fn parse_product_os(value: &str) -> Result<Os> {
    match value {
        "linux" => Ok(Os::Linux),
        "macos" => Ok(Os::Macos),
        "windows" => Ok(Os::Windows),
        other => Err(Error::validation(format!(
            "unsupported product activation target_os '{other}'"
        ))),
    }
}

fn report_product_verification(product: &VerifiedPluginProduct, fmt: Format) {
    if fmt.is_json() {
        output::success(&serde_json::json!({
            "verified": true,
            "name": product.contract.name,
            "version": product.contract.version,
            "target": product.contract.target,
            "archive": product.source.archive,
            "archive_digest": product.source.digest,
            "archive_size": product.source.size,
            "members": product.contract.install.order,
        }));
    } else {
        println!(
            "Verified plugin product {} {} for {}",
            product.contract.name, product.contract.version, product.contract.target
        );
        println!("  digest:  {}", product.source.digest);
        println!("  archive: {}", product.source.archive);
        println!(
            "  members: {} ({})",
            product.contract.members.len(),
            product.contract.install.order.join(", ")
        );
    }
}

/// The hex digest without the `sha256:` scheme prefix (the `sha256sum -c`
/// on-disk format).
fn bare_sha256(digest: &str) -> &str {
    digest.strip_prefix("sha256:").unwrap_or(digest)
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
#[allow(clippy::too_many_arguments)]
fn run_session(
    bundle_path: &str,
    with_paths: &[String],
    plugin_paths: &[String],
    no_inject: bool,
    target: Option<String>,
    profile: Option<String>,
    command: Vec<String>,
    _fmt: Format,
) -> Result<()> {
    let bundle = load_bundle(bundle_path)?;
    let dependencies = if no_inject {
        Vec::new()
    } else {
        selected_workspace_dependencies(&bundle)?
    };
    let explicit = load_with_bundles(with_paths)?;
    let with_bundles = merge_composed_bundles(&bundle, dependencies, explicit)?;
    // An external installed/extracted tree is itself a bundle (same layout +
    // openstrata.plugin.yaml), so it composes through the same bundle_vars.
    let plugin_path_bundles = load_with_bundles(plugin_paths)?;
    if no_inject
        && !plugin_path_bundles.is_empty()
        && !plugin_path_bundles
            .iter()
            .any(|candidate| same_plugin_identity(&bundle, candidate))
    {
        eprintln!(
            "warning [PLUGIN_RUN_PLUGIN_PATH_MISMATCH]: --no-inject excludes source bundle \
             '{} {}', and none of the --plugin-path roots provides that bundle; \
             the bundle argument selects the runtime only",
            bundle.manifest.name(),
            bundle.manifest.plugin.version
        );
        eprintln!(
            "  hint: point --plugin-path at the extracted '{} {}' package, or drop --no-inject",
            bundle.manifest.name(),
            bundle.manifest.plugin.version
        );
    }
    let host = Host::detect();
    let (platform, profile) =
        selection_for_capabilities(target, profile, &bundle.manifest.requires.capabilities)?;
    let r = require_real_runtime(Some(platform.clone()), Some(profile.clone()))?;
    let library_dirs = if no_inject {
        Vec::new()
    } else {
        selected_workspace_library_runtime_dirs(&bundle, &r, true)?
    };

    // Search order (highest first): the source bundle unless --no-inject, then
    // any --plugin-path trees, then --with companions, then the runtime.
    let mut contributing: Vec<&Bundle> = Vec::new();
    if !no_inject {
        contributing.push(&bundle);
    }
    contributing.extend(plugin_path_bundles.iter());
    contributing.extend(with_bundles.iter());
    let library_dirs = library_dirs
        .iter()
        .map(Utf8PathBuf::as_path)
        .collect::<Vec<_>>();
    let session = ost_plugin::session_env_from_with_library_dirs(
        &r.env,
        &contributing,
        &library_dirs,
        host.os,
    );
    let (program, args) = prepare_session_command(&command, &r.artifact_prefix, &r.python_version)?;

    let mut cmd = Command::new(&program);
    cmd.args(&args);
    session.apply(&mut cmd); // overlay the resolved session env, no global mutation
    if let Some(toolchain) = session_toolchain_file(&bundle, &platform, &profile) {
        cmd.env("CMAKE_TOOLCHAIN_FILE", toolchain.as_str());
    }
    let status = cmd
        .status()
        .map_err(|e| Error::io(format!("run {program}"), e))?;
    // Propagate the child's exit code for CI.
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// The `toolchain.cmake` a session exports as `CMAKE_TOOLCHAIN_FILE`, so a bare
/// `cmake` inside `ost plugin run` configures the way `ost plugin build` does.
///
/// `CMAKE_PREFIX_PATH` alone is enough to *find* the runtime and not enough to
/// configure against it: `pxrConfig.cmake` does
/// `find_dependency(Python3 COMPONENTS Development Development.Module
/// Development.Embed)`, and an adopted runtime baked the interpreter paths of
/// the machine that built it — a deadsnakes 3.13 in a build container, absent on
/// every runner. `ost plugin build` resolves a host interpreter and pins all
/// three Development variables in this file; nothing exposed that, so a repo
/// writing its own workspace lane could not reproduce by hand what `ost` does
/// internally, and the lane shipped disabled (report 32 §5).
///
/// CMake ≥ 3.21 reads `CMAKE_TOOLCHAIN_FILE` from the environment, so exporting
/// it needs no change on the caller's command line. An explicit toolchain the
/// caller already set wins — this is a default, not an override. Returns `None`
/// when the target cannot be resolved or the file cannot be written; the session
/// still runs, exactly as before.
fn session_toolchain_file(bundle: &Bundle, platform: &str, profile: &str) -> Option<Utf8PathBuf> {
    if std::env::var_os("CMAKE_TOOLCHAIN_FILE").is_some() {
        return None;
    }
    let (tgt, r) = build_target(platform, profile).ok()?;
    let target_dir = target_state_dir(&bundle.root, &tgt.id());
    let toolchain = target_dir.join("toolchain.cmake");
    // A configured bundle already has one, written with its resolved compiler
    // policy and any workspace-prefix additions. Reuse it rather than rendering
    // a weaker version over the top.
    if toolchain.as_std_path().is_file() {
        return Some(toolchain);
    }
    // Not configured yet: render the same contract `plugin build` would, so the
    // first plain-CMake configure in a fresh checkout works too.
    let compiler = resolve_plugin_compiler(&bundle.root, &CompilerOpts::default()).ok()?;
    let python = ost_build::resolve_for_runtime(&r.artifact_prefix, &tgt.python_version);
    // Heal the runtime the same way `plugin build` does before rendering against
    // it. Relocation runs at build/configure time, never at `runtime pull`, so on
    // the fresh checkout this branch exists for, a just-pulled adopted runtime
    // still carries the export machine's baked paths in its own CMake files —
    // and pinning Python in the toolchain does not undo that.
    crate::commands::relocate_baked_python_if_stale(&r.artifact_prefix, python.as_ref());
    let text = ost_build::render_toolchain(&tgt, &r.artifact_prefix, &compiler, python.as_ref());
    std::fs::create_dir_all(target_dir.as_std_path()).ok()?;
    // Write through a temp file: `plugin build` serializes its writes to this
    // directory with a lease that a session deliberately does not take, so a
    // concurrent build must never observe a half-written toolchain.
    let staged = target_dir.join("toolchain.cmake.session");
    std::fs::write(staged.as_std_path(), format!("{text}\n")).ok()?;
    std::fs::rename(staged.as_std_path(), toolchain.as_std_path()).ok()?;
    Some(toolchain)
}

fn same_plugin_identity(left: &Bundle, right: &Bundle) -> bool {
    left.manifest.name() == right.manifest.name()
        && left.manifest.plugin.version == right.manifest.plugin.version
        && left.manifest.kind() == right.manifest.kind()
}

fn prepare_session_command(
    command: &[String],
    artifact_prefix: &Utf8Path,
    python_version: &str,
) -> Result<(String, Vec<String>)> {
    let (program, rest) = command
        .split_first()
        .ok_or_else(|| Error::usage("missing command"))?;
    if is_explicit_py_launcher_version_request(program, rest) {
        return Ok((program.clone(), rest.to_vec()));
    }
    if !is_runtime_python_request(program) {
        return Ok((program.clone(), rest.to_vec()));
    }
    let resolved = ost_build::resolve_run_python(artifact_prefix, python_version).ok_or_else(|| {
        let expected = ost_build::usd_python_requirement(artifact_prefix)
            .or_else(|| ost_build::python::major_minor(python_version))
            .unwrap_or_else(|| python_version.to_string());
        let searched = ost_build::run_python_search_paths(artifact_prefix, python_version);
        Error::coded(
            "RUNTIME_PYTHON_NOT_FOUND",
            Category::Precondition,
            format!(
                "no Python interpreter matching runtime ABI {expected} found for `ost plugin run -- {program}` (searched: {})",
                searched.join(", ")
            ),
        )
        .with_hint(format!(
            "install CPython {expected}, add it to PATH, or pull a runtime that bundles an interpreter under bin/"
        ))
    })?;
    merge_resolved_python_command(resolved, rest)
}

fn is_runtime_python_request(program: &str) -> bool {
    if program.contains('/') || program.contains('\\') {
        return false;
    }
    let name = program.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "python" | "python.exe" | "python3" | "python3.exe"
    ) || (cfg!(windows) && is_py_launcher_request(&name))
}

fn is_py_launcher_request(program: &str) -> bool {
    let name = program.to_ascii_lowercase();
    matches!(name.as_str(), "py" | "py.exe")
}

fn is_explicit_py_launcher_version_request(program: &str, args: &[String]) -> bool {
    is_py_launcher_request(program)
        && args
            .first()
            .is_some_and(|arg| is_py_launcher_version_selector(arg))
}

fn is_py_launcher_version_selector(arg: &str) -> bool {
    let spec = if let Some(spec) = arg.strip_prefix("-V:") {
        spec.rsplit('/').next().unwrap_or(spec)
    } else {
        let Some(spec) = arg.strip_prefix('-') else {
            return false;
        };
        if !spec.as_bytes().first().is_some_and(|b| b.is_ascii_digit()) {
            return false;
        }
        spec
    };
    let version = spec.split('-').next().unwrap_or(spec);
    let mut parts = version.split('.');
    let Some(major) = parts.next() else {
        return false;
    };
    if major.is_empty() || !major.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if let Some(minor) = parts.next() {
        if minor.is_empty() || !minor.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
    }
    parts.next().is_none()
}

fn merge_resolved_python_command(
    resolved: Vec<String>,
    rest: &[String],
) -> Result<(String, Vec<String>)> {
    let (program, leading) = resolved.split_first().ok_or_else(|| {
        Error::coded(
            "RUNTIME_PYTHON_NOT_FOUND",
            Category::Precondition,
            "runtime Python resolver returned no command",
        )
    })?;
    let mut args = leading.to_vec();
    args.extend(rest.iter().cloned());
    Ok((program.clone(), args))
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
    let dependencies = selected_workspace_dependencies(&bundle)?;
    let explicit = load_with_bundles(with_paths)?;
    let with_bundles = merge_composed_bundles(&bundle, dependencies, explicit)?;
    let host = Host::detect();
    let resolved = resolve_runtime(target, profile)?;
    let library_dirs = match resolved.as_ref() {
        Some(resolved) => selected_workspace_library_runtime_dirs(&bundle, resolved, up_to >= 2)?,
        None => Vec::new(),
    };

    let (report, report_dir) = test_bundle(
        &bundle,
        &with_bundles,
        &library_dirs,
        resolved.as_ref(),
        &host,
        up_to,
    )?;
    let libraries = selected_workspace_library_evidence(&bundle, resolved.as_ref())?;
    let dependency_bundles = selected_workspace_bundle_evidence(&bundle)?;
    write_dependency_evidence(&report_dir, &libraries, &dependency_bundles)?;

    if fmt.is_json() {
        let mut body = ost_plugin::report_json(&bundle, &report);
        if let Some(obj) = body.as_object_mut() {
            obj.insert(
                "report_dir".into(),
                serde_json::Value::String(report_dir.to_string()),
            );
            if !libraries.is_empty() {
                obj.insert(
                    "libraries".into(),
                    serde_json::Value::Array(libraries.clone()),
                );
                obj.insert(
                    "dependencies_file".into(),
                    serde_json::Value::String(report_dir.join("dependencies.json").to_string()),
                );
            }
        }
        output::report(report.passed(), &body);
    } else {
        print_report(&bundle, &report);
        println!("\nReport: {report_dir}");
    }
    finish(&report)
}

/// One bundle's package, extracted to a clean tree ready to be tested.
struct ExtractedPackage {
    bundle: Bundle,
    extract_dir: Utf8PathBuf,
    archive_path: Utf8PathBuf,
}

/// Locate a bundle's dist output for `id` and extract it to a clean directory.
///
/// The extraction is into a fresh, empty directory each run, so discovery sees
/// only the shipped layout — the whole point of testing from a package rather
/// than from the build tree, where a `plugInfo` baked to a build-only absolute
/// path still resolves.
fn extract_packaged_bundle(source: &Bundle, id: &str) -> Result<ExtractedPackage> {
    let name = &source.manifest.plugin.name;
    let version = &source.manifest.plugin.version;
    let dist_dir = plugin_dist_dir(&source.root, name, version, id);
    let manifest_path = dist_dir.join("manifest.json");
    if !manifest_path.as_std_path().is_file() {
        return Err(Error::precondition(format!(
            "no packaged artifact at {dist_dir} — run `ost plugin package` for target {id} first"
        )));
    }
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(manifest_path.as_std_path())
            .map_err(|e| Error::io(manifest_path.to_string(), e))?,
    )
    .map_err(|e| Error::parse(manifest_path.as_str(), anyhow::Error::new(e)))?;
    let archive_name = manifest["archive"].as_str().ok_or_else(|| {
        Error::validation(format!("{manifest_path} is missing an 'archive' field"))
    })?;
    let archive_digest = manifest["archive_digest"].as_str().ok_or_else(|| {
        Error::validation(format!(
            "{manifest_path} is missing an 'archive_digest' field"
        ))
    })?;
    let archive_path = dist_dir.join(archive_name);

    let extract_dir = target_state_dir(&source.root, id).join("from-package");
    if extract_dir.as_std_path().exists() {
        std::fs::remove_dir_all(extract_dir.as_std_path())
            .map_err(|e| Error::io(extract_dir.to_string(), e))?;
    }
    ost_artifact::extract_archive(&archive_path, archive_digest, &extract_dir)?;

    Ok(ExtractedPackage {
        bundle: load_bundle(extract_dir.as_str())?,
        extract_dir,
        archive_path,
    })
}

/// `ost plugin test --workspace --from-package` — verify a *packaged* workspace
/// by the same pyramid its source tree gets.
///
/// A workspace's source tree and its shipped artifacts are different things, and
/// only the second is what a consumer installs. Testing bundles from source
/// proves the sources compose; it says nothing about whether the packages do —
/// which is exactly where a `plugInfo` baked to a build-tree path, or a bundle
/// dependency that never made it into the artifact, shows up.
///
/// Every bundle is extracted first, then each is tested against the *extracted*
/// trees of its dependencies rather than their source directories. Composing
/// source bundles here would defeat the purpose: the provider on the discovery
/// path has to be the shipped one.
fn test_workspace_from_package(
    with_paths: &[String],
    target: Option<String>,
    profile: Option<String>,
    up_to: u8,
    fmt: Format,
) -> Result<()> {
    let (bundles, _libraries, graph) = load_workspace_graph()?;
    if !graph.passed {
        if fmt.is_json() {
            output::report(
                false,
                &serde_json::json!({ "workspace": true, "from_package": true, "graph": graph }),
            );
        } else {
            print_graph_summary(&graph);
            for issue in &graph.issues {
                println!("  FAIL [{}] {}", issue.code, issue.message);
            }
        }
        std::process::exit(ost_core::Category::Validation.exit_code() as i32);
    }

    let (platform, profile) = selection(target, profile).ok_or_else(|| {
        Error::usage(
            "no platform/profile: run inside an OpenStrata project or pass --target/--profile",
        )
    })?;
    let (tgt, r) = build_target(&platform, &profile)?;
    let id = tgt.id();
    let host = Host::detect();
    let explicit_bundles = load_with_bundles(with_paths)?;

    let source_by_id: BTreeMap<String, Bundle> = bundles
        .iter()
        .cloned()
        .map(|bundle| (bundle.manifest.name().to_string(), bundle))
        .collect();

    // Extract everything before testing anything: a bundle's dependencies must
    // already exist as shipped trees when its turn comes.
    let mut extracted_by_id: BTreeMap<String, ExtractedPackage> = BTreeMap::new();
    for (source_id, source) in &source_by_id {
        extracted_by_id.insert(source_id.clone(), extract_packaged_bundle(source, &id)?);
    }

    let mut results: Vec<(Bundle, DoctorReport, Utf8PathBuf, Utf8PathBuf)> = Vec::new();
    for source_id in source_by_id.keys() {
        let dependencies = graph
            .dependency_order(source_id)
            .expect("the workspace graph passed and contains every loaded bundle")
            .into_iter()
            .map(|dependency| {
                extracted_by_id
                    .get(&dependency)
                    .map(|package| package.bundle.clone())
                    .ok_or_else(|| {
                        Error::validation(format!(
                            "validated workspace provider '{dependency}' has no extracted package"
                        ))
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        let package = &extracted_by_id[source_id];
        let composed =
            merge_composed_bundles(&package.bundle, dependencies, explicit_bundles.clone())?;
        let (report, report_dir) =
            test_bundle(&package.bundle, &composed, &[], Some(&r), &host, up_to)?;
        if !fmt.is_json() {
            println!(
                "== {} (packaged: {}) ==",
                package.bundle.manifest.plugin.name, package.archive_path
            );
            print_report(&package.bundle, &report);
            println!("Report: {report_dir}\n");
        }
        results.push((
            package.bundle.clone(),
            report,
            report_dir,
            package.archive_path.clone(),
        ));
    }

    let failed = results.iter().filter(|(_, r, _, _)| !r.passed()).count();
    if fmt.is_json() {
        let items: Vec<serde_json::Value> = results
            .iter()
            .map(|(bundle, report, dir, archive)| {
                let mut body = ost_plugin::report_json(bundle, report);
                if let Some(obj) = body.as_object_mut() {
                    obj.insert(
                        "report_dir".into(),
                        serde_json::Value::String(dir.to_string()),
                    );
                    obj.insert("from_package".into(), serde_json::Value::Bool(true));
                    obj.insert(
                        "package".into(),
                        serde_json::Value::String(portable(archive)),
                    );
                }
                body
            })
            .collect();
        output::report(
            failed == 0,
            &serde_json::json!({
                "workspace": true,
                "from_package": true,
                "graph": graph,
                "bundles": items,
                "total": results.len(),
                "failed": failed,
            }),
        );
    } else {
        println!(
            "Workspace (packaged): {} bundle(s), {failed} failed",
            results.len()
        );
    }
    if failed == 0 {
        Ok(())
    } else {
        std::process::exit(ost_core::Category::Validation.exit_code() as i32);
    }
}

/// `ost plugin test --from-package` — extract the already-built package to a
/// clean directory and run the verification pyramid against the *shipped* tree.
///
/// Source-tree discovery walks the build tree, so a `plugInfo`/`LibraryPath`
/// baked to a build-only absolute path still resolves and L2 passes green — then
/// the shipped artifact fails to load on a clean host. Testing the extracted
/// package reproduces the consumer's layout and catches that before publish.
fn test_from_package(
    bundle_path: &str,
    with_paths: &[String],
    target: Option<String>,
    profile: Option<String>,
    up_to: u8,
    fmt: Format,
) -> Result<()> {
    let source = load_bundle(bundle_path)?;
    let with_bundles = load_with_bundles(with_paths)?;
    let host = Host::detect();

    let (platform, profile) =
        selection_for_capabilities(target, profile, &source.manifest.requires.capabilities)?;
    let (tgt, r) = build_target(&platform, &profile)?;
    let id = tgt.id();

    let extraction = extract_packaged_bundle(&source, &id)?;
    let (extracted, extract_dir, archive_path) = (
        extraction.bundle,
        extraction.extract_dir,
        extraction.archive_path,
    );
    let (report, report_dir) = test_bundle(&extracted, &with_bundles, &[], Some(&r), &host, up_to)?;

    if fmt.is_json() {
        let mut body = ost_plugin::report_json(&extracted, &report);
        if let Some(obj) = body.as_object_mut() {
            obj.insert(
                "report_dir".into(),
                serde_json::Value::String(report_dir.to_string()),
            );
            obj.insert("from_package".into(), serde_json::Value::Bool(true));
            obj.insert(
                "package".into(),
                serde_json::Value::String(archive_path.to_string()),
            );
        }
        output::report(report.passed(), &body);
    } else {
        println!("Testing packaged artifact: {archive_path}");
        println!("  extracted to: {extract_dir}");
        print_report(&extracted, &report);
        println!("\nReport: {report_dir}");
    }
    finish(&report)
}

/// Diagnose one bundle (L0..`up_to`) in the resolved session and write its
/// report — the shared core of `plugin test` and `plugin test --workspace`.
fn test_bundle(
    bundle: &Bundle,
    with_bundles: &[Bundle],
    library_dirs: &[Utf8PathBuf],
    resolved: Option<&crate::commands::Resolved>,
    host: &Host,
    up_to: u8,
) -> Result<(DoctorReport, Utf8PathBuf)> {
    let ctx = resolved.map(runtime_context).unwrap_or_default();
    let session = match resolved {
        Some(r) => {
            let mut contributing = Vec::with_capacity(with_bundles.len() + 1);
            contributing.push(bundle);
            contributing.extend(with_bundles.iter());
            let library_dirs = library_dirs
                .iter()
                .map(Utf8PathBuf::as_path)
                .collect::<Vec<_>>();
            let env = ost_plugin::session_env_from_with_library_dirs(
                &r.env,
                &contributing,
                &library_dirs,
                host.os,
            );
            // An adopted runtime may not bundle Python; put a matching host
            // interpreter's dir on the loader path so usdcat/usdview and the
            // pxr bindings can load pythonXY.dll and a matched `python` runs.
            crate::commands::with_host_python_on_path(
                env,
                &r.artifact_prefix,
                &r.python_version,
                host.os,
            )
        }
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
/// `ost plugin test --workspace --graph-only` — validate the dependency graph
/// and exit on that result alone.
///
/// The graph check and the per-bundle pyramid are separable: the first is
/// whole-workspace and costs milliseconds, the second is what each generated
/// bundle cell already runs. Welded together they could not be asked for
/// independently — on a fresh checkout the verb validated the graph, reported it
/// valid, then failed because nothing had been built yet, so a repo wanting the
/// graph as a cheap early PR gate had to either build everything or parse
/// `--json` (report 32 §4). This is the direct form: no build, no runtime, no
/// packaged artifact.
fn validate_workspace_graph(fmt: Format) -> Result<()> {
    let (bundles, _libraries, graph) = load_workspace_graph()?;
    if fmt.is_json() {
        output::report(
            graph.passed,
            &serde_json::json!({
                "workspace": true,
                "graph_only": true,
                "graph": graph,
                "total": bundles.len(),
            }),
        );
    } else {
        print_graph_summary(&graph);
        for issue in &graph.issues {
            println!("  FAIL [{}] {}", issue.code, issue.message);
        }
    }
    if !graph.passed {
        std::process::exit(ost_core::Category::Validation.exit_code() as i32);
    }
    Ok(())
}

/// Discover and load every workspace member, then validate the bundle/library
/// dependency graph. Shared by every `--workspace` entry point so one discovery
/// rule and one graph computation serve them all.
fn load_workspace_graph() -> Result<(Vec<Bundle>, Vec<Library>, ost_plugin::WorkspaceValidation)> {
    let members = discover_workspace_members(Utf8Path::new("."))?;
    if members.bundles.is_empty() {
        return Err(
            Error::precondition("no plugin bundles found in the workspace member set").with_hint(
                "run from the workspace root, or pass a bundle path instead of --workspace",
            ),
        );
    }
    let bundles = members
        .bundles
        .iter()
        .map(|root| Bundle::load(root))
        .collect::<Result<Vec<_>>>()?;
    let libraries = members
        .libraries
        .iter()
        .map(|root| Library::load(root))
        .collect::<Result<Vec<_>>>()?;
    // Tools have no dependency edges, but their descriptors are still declared
    // workspace members. Load them before a graph-only success can be reported
    // so an unreadable or invalid descriptor cannot disappear from the gate.
    for root in &members.tools {
        ost_plugin::Tool::load(root)?;
    }
    let graph = ost_plugin::validate_workspace_with_libraries(&bundles, &libraries);
    Ok((bundles, libraries, graph))
}

/// The one-line graph shape, in the same wording whether it passed or failed.
fn print_graph_summary(graph: &ost_plugin::WorkspaceValidation) {
    let plural = if graph.libraries.len() == 1 {
        "y"
    } else {
        "ies"
    };
    let verdict = if graph.passed {
        "valid".to_string()
    } else {
        format!("{} issue(s)", graph.issues.len())
    };
    println!(
        "Workspace dependency graph: {} bundle(s), {} bundle edge(s), {} librar{plural}, \
         {} library edge(s), {verdict}",
        graph.nodes.len(),
        graph.edges.len(),
        graph.libraries.len(),
        graph.library_edges.len()
    );
}

fn test_workspace(
    with_paths: &[String],
    target: Option<String>,
    profile: Option<String>,
    up_to: u8,
    fmt: Format,
) -> Result<()> {
    let (bundles, libraries, graph) = load_workspace_graph()?;
    if !graph.passed {
        if fmt.is_json() {
            output::report(
                false,
                &serde_json::json!({
                    "workspace": true,
                    "graph": graph,
                    "total": bundles.len(),
                    "failed": 0,
                }),
            );
        } else {
            print_graph_summary(&graph);
            for issue in &graph.issues {
                println!("  FAIL [{}] {}", issue.code, issue.message);
            }
            println!(
                "  hint: `ost plugin test --workspace --graph-only` runs this check alone, \
                 with nothing built"
            );
        }
        std::process::exit(ost_core::Category::Validation.exit_code() as i32);
    }
    if !fmt.is_json() {
        print_graph_summary(&graph);
        println!();
    }

    let explicit_bundles = load_with_bundles(with_paths)?;
    let host = Host::detect();
    // One resolution for the whole workspace: every bundle tests against the
    // same runtime session base.
    let resolved = resolve_runtime(target, profile)?;

    let by_id: BTreeMap<String, Bundle> = bundles
        .iter()
        .cloned()
        .map(|bundle| (bundle.manifest.name().to_string(), bundle))
        .collect();
    let library_by_id: BTreeMap<String, Library> = libraries
        .into_iter()
        .map(|library| (library.id().to_string(), library))
        .collect();
    let workspace = SourceWorkspace {
        root: Utf8PathBuf::from("."),
        bundles: by_id.clone(),
        libraries: library_by_id,
        graph: graph.clone(),
    };
    let mut results: Vec<(Bundle, DoctorReport, Utf8PathBuf, Vec<serde_json::Value>)> = Vec::new();
    for bundle in bundles {
        let root = bundle.root.clone();
        let dependencies = graph
            .dependency_order(bundle.manifest.name())
            .expect("the workspace graph passed and contains every loaded bundle")
            .into_iter()
            .map(|id| {
                by_id.get(&id).cloned().ok_or_else(|| {
                    Error::validation(format!(
                        "validated workspace provider '{id}' could not be loaded"
                    ))
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let composed = merge_composed_bundles(&bundle, dependencies, explicit_bundles.clone())?;
        let library_dirs = match resolved.as_ref() {
            Some(resolved) => {
                library_runtime_dirs_from_workspace(&bundle, &workspace, resolved, up_to >= 2)?
            }
            None => Vec::new(),
        };
        let (report, report_dir) = test_bundle(
            &bundle,
            &composed,
            &library_dirs,
            resolved.as_ref(),
            &host,
            up_to,
        )?;
        let library_evidence = selected_workspace_library_evidence(&bundle, resolved.as_ref())?;
        let bundle_closure = composed
            .iter()
            .filter(|dependency| dependency.manifest.name() != bundle.manifest.name())
            .map(|dependency| {
                bundle_evidence(
                    dependency,
                    &portable(&dependency.root.join(ost_plugin::PLUGIN_MANIFEST)),
                    "source-workspace",
                )
            })
            .collect::<Vec<_>>();
        write_dependency_evidence(&report_dir, &library_evidence, &bundle_closure)?;
        if !fmt.is_json() {
            println!("== {} ({root}) ==", bundle.manifest.plugin.name);
            print_report(&bundle, &report);
            println!("Report: {report_dir}\n");
        }
        results.push((bundle, report, report_dir, library_evidence));
    }

    let failed = results.iter().filter(|(_, r, _, _)| !r.passed()).count();
    let all_passed = failed == 0;
    if fmt.is_json() {
        let bundles: Vec<serde_json::Value> = results
            .iter()
            .map(|(bundle, report, dir, libraries)| {
                let mut body = ost_plugin::report_json(bundle, report);
                if let Some(obj) = body.as_object_mut() {
                    obj.insert(
                        "report_dir".into(),
                        serde_json::Value::String(dir.to_string()),
                    );
                    if !libraries.is_empty() {
                        obj.insert(
                            "libraries".into(),
                            serde_json::Value::Array(libraries.clone()),
                        );
                        obj.insert(
                            "dependencies_file".into(),
                            serde_json::Value::String(dir.join("dependencies.json").to_string()),
                        );
                    }
                }
                body
            })
            .collect();
        output::report(
            all_passed,
            &serde_json::json!({
                "workspace": true,
                "graph": graph,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceMemberKind {
    Bundle,
    Library,
    Tool,
}

#[derive(Debug, Default)]
struct WorkspaceMembers {
    bundles: Vec<Utf8PathBuf>,
    libraries: Vec<Utf8PathBuf>,
    tools: Vec<Utf8PathBuf>,
}

const WORKSPACE_SCAN_EXCLUDED: &[&str] =
    &[".git", ".strata", "target", "build", "out", "node_modules"];

/// Resolve the workspace's complete declared or discovered member set.
///
/// `[workspace].members` is authoritative when present. Without it, a bounded
/// recursive scan preserves legacy manifests while allowing nested layouts.
/// In both modes descriptor roots are compared as one set, so a descriptor can
/// never silently disappear from a green graph result.
fn discover_workspace_members(root: &Utf8Path) -> Result<WorkspaceMembers> {
    // `read_dir(".")` may return child paths without the leading `.` while
    // explicit expansion starts from `./...`. Normalize once so set comparison
    // cannot classify the same descriptor as undeclared by spelling alone.
    let root = canonical_root(root);
    let explicit = if root.join(PROJECT_MANIFEST).is_file() {
        load_project(&root)?
            .workspace
            .map(|workspace| workspace.members)
    } else {
        None
    };

    let scanned = scan_workspace_descriptors(&root)?;
    let selected = if let Some(patterns) = explicit {
        let mut selected = BTreeMap::new();
        for pattern in patterns {
            let matches = expand_workspace_member_pattern(&root, &pattern)?;
            if matches.is_empty() {
                return Err(Error::coded(
                    "WORKSPACE_MEMBER_PATTERN_EMPTY",
                    Category::Validation,
                    format!("workspace member pattern '{pattern}' matched no directories"),
                )
                .with_hint(
                    "fix or remove the pattern under [workspace].members in openstrata.toml",
                ));
            }
            for member_root in matches {
                // Literal components retain the manifest's spelling. On a
                // case-insensitive filesystem (and for Windows 8.3 aliases)
                // that can differ from the path returned by the recursive
                // read_dir scan even though both name the same directory.
                let member_root = canonical_root(&member_root);
                let kind = workspace_descriptor_kind(&member_root)?.ok_or_else(|| {
                    Error::coded(
                        "WORKSPACE_MEMBER_DESCRIPTOR_MISSING",
                        Category::Validation,
                        format!(
                            "declared workspace member '{member_root}' has no OpenStrata member descriptor"
                        ),
                    )
                    .with_hint(format!(
                        "add {}, {}, or {}, or narrow [workspace].members",
                        ost_plugin::PLUGIN_MANIFEST,
                        ost_plugin::LIBRARY_MANIFEST,
                        ost_plugin::TOOL_MANIFEST
                    ))
                })?;
                selected.insert(member_root, kind);
            }
        }

        let omitted = scanned
            .keys()
            .filter(|path| !selected.contains_key(*path))
            .map(|path| {
                let relative = portable(path.strip_prefix(&root).unwrap_or(path));
                if relative.is_empty() {
                    ".".to_string()
                } else {
                    relative
                }
            })
            .collect::<Vec<_>>();
        if !omitted.is_empty() {
            return Err(Error::coded(
                "WORKSPACE_DESCRIPTOR_NOT_DECLARED",
                Category::Validation,
                format!(
                    "workspace descriptor(s) found outside [workspace].members: {}",
                    omitted.join(", ")
                ),
            )
            .with_hint(
                "add the descriptor roots to [workspace].members or remove stale descriptors",
            ));
        }
        selected
    } else {
        // Legacy discovery historically searched below the project root. Keep
        // a root-level descriptor opt-in through the explicit `"."` member so
        // adding a plugin to a scaffolded root library does not silently turn
        // that unrelated library into source-workspace composition.
        scanned
            .into_iter()
            .filter(|(path, _)| path != &root)
            .collect()
    };

    let mut members = WorkspaceMembers::default();
    for (path, kind) in selected {
        match kind {
            WorkspaceMemberKind::Bundle => members.bundles.push(path),
            WorkspaceMemberKind::Library => members.libraries.push(path),
            WorkspaceMemberKind::Tool => members.tools.push(path),
        }
    }
    Ok(members)
}

fn workspace_descriptor_kind(root: &Utf8Path) -> Result<Option<WorkspaceMemberKind>> {
    let descriptors = [
        (ost_plugin::PLUGIN_MANIFEST, WorkspaceMemberKind::Bundle),
        (ost_plugin::LIBRARY_MANIFEST, WorkspaceMemberKind::Library),
        (ost_plugin::TOOL_MANIFEST, WorkspaceMemberKind::Tool),
    ]
    .into_iter()
    .filter(|(name, _)| root.join(name).is_file())
    .collect::<Vec<_>>();
    match descriptors.as_slice() {
        [] => Ok(None),
        [(_, kind)] => Ok(Some(*kind)),
        _ => Err(Error::coded(
            "WORKSPACE_MEMBER_DESCRIPTOR_AMBIGUOUS",
            Category::Validation,
            format!(
                "workspace member '{root}' contains multiple member descriptors: {}",
                descriptors
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
        .with_hint("keep exactly one OpenStrata member descriptor in each member directory")),
    }
}

fn scan_workspace_descriptors(
    root: &Utf8Path,
) -> Result<BTreeMap<Utf8PathBuf, WorkspaceMemberKind>> {
    let mut found = BTreeMap::new();
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    while let Some((directory, depth)) = pending.pop() {
        if let Some(kind) = workspace_descriptor_kind(&directory)? {
            found.insert(directory.clone(), kind);
        }
        if depth == ost_manifest::MAX_WORKSPACE_MEMBER_DEPTH {
            continue;
        }
        let entries = std::fs::read_dir(directory.as_std_path())
            .map_err(|error| Error::io(directory.to_string(), error))?;
        let mut children = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| Error::io(directory.to_string(), error))?;
            let file_type = entry
                .file_type()
                .map_err(|error| Error::io(entry.path().display().to_string(), error))?;
            if !file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|path| {
                Error::InvalidManifest(format!(
                    "workspace contains a non-UTF-8 directory below {}: {}",
                    directory,
                    path.display()
                ))
            })?;
            if path.file_name().is_some_and(|name| {
                WORKSPACE_SCAN_EXCLUDED.contains(&name) || name.starts_with('.')
            }) {
                continue;
            }
            children.push(path);
        }
        children.sort();
        pending.extend(children.into_iter().rev().map(|path| (path, depth + 1)));
    }
    Ok(found)
}

fn expand_workspace_member_pattern(root: &Utf8Path, pattern: &str) -> Result<Vec<Utf8PathBuf>> {
    if pattern == "." {
        return Ok(vec![root.to_path_buf()]);
    }
    let mut candidates = vec![root.to_path_buf()];
    for component in pattern.split('/') {
        let mut next = Vec::new();
        for parent in &candidates {
            if component.contains(['*', '?']) {
                let entries = std::fs::read_dir(parent.as_std_path())
                    .map_err(|error| Error::io(parent.to_string(), error))?;
                for entry in entries {
                    let entry = entry.map_err(|error| Error::io(parent.to_string(), error))?;
                    let file_type = entry
                        .file_type()
                        .map_err(|error| Error::io(entry.path().display().to_string(), error))?;
                    if !file_type.is_dir() || file_type.is_symlink() {
                        continue;
                    }
                    let name = entry.file_name();
                    let Some(name) = name.to_str() else {
                        continue;
                    };
                    if WORKSPACE_SCAN_EXCLUDED.contains(&name) || name.starts_with('.') {
                        continue;
                    }
                    if wildcard_component_matches(component, name) {
                        let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|path| {
                            Error::InvalidManifest(format!(
                                "workspace pattern '{pattern}' matched a non-UTF-8 path: {}",
                                path.display()
                            ))
                        })?;
                        next.push(path);
                    }
                }
            } else {
                let path = parent.join(component);
                let is_symlink = path
                    .symlink_metadata()
                    .map(|metadata| metadata.file_type().is_symlink())
                    .unwrap_or(false);
                if path.is_dir() && !is_symlink {
                    next.push(path);
                }
            }
        }
        next.sort();
        next.dedup();
        candidates = next;
        if candidates.is_empty() {
            break;
        }
    }
    Ok(candidates)
}

fn wildcard_component_matches(pattern: &str, name: &str) -> bool {
    let pattern = pattern.chars().collect::<Vec<_>>();
    let name = name.chars().collect::<Vec<_>>();
    let mut matched = vec![vec![false; name.len() + 1]; pattern.len() + 1];
    matched[0][0] = true;
    for index in 0..pattern.len() {
        for offset in 0..=name.len() {
            if !matched[index][offset] {
                continue;
            }
            match pattern[index] {
                '*' => {
                    matched[index + 1][offset] = true;
                    if offset < name.len() {
                        matched[index][offset + 1] = true;
                    }
                }
                '?' if offset < name.len() => matched[index + 1][offset + 1] = true,
                literal if offset < name.len() && literal == name[offset] => {
                    matched[index + 1][offset + 1] = true;
                }
                _ => {}
            }
        }
    }
    matched[pattern.len()][name.len()]
}

/// Package-relevant outputs for every workspace bundle and tool, relative to
/// the project root, for `ost build` to record in its completion.
///
/// A root CMake build commonly writes directly into each bundle's discoverable
/// registration/library/Python tree. Those bytes need the same attribution as
/// a bundle-local `ost plugin build`; otherwise the old local completion makes
/// a valid root build look like an unmanaged overwrite at package time.
pub(crate) fn workspace_managed_outputs(
    root: &Utf8Path,
    os: Os,
) -> (Vec<BuildOutput>, Vec<String>) {
    let (tool_outputs, mut warnings) = workspace_tool_outputs(root, os);
    let mut outputs = tool_outputs
        .into_iter()
        .map(|output| (output.path.clone(), output))
        .collect::<BTreeMap<_, _>>();
    let members = match discover_workspace_members(root) {
        Ok(members) => members,
        Err(error) => {
            warnings.push(format!(
                "warning: could not scan for workspace bundles under {root}: {error}"
            ));
            return (outputs.into_values().collect(), warnings);
        }
    };
    let canonical_project_root = canonical_root(root);
    for bundle_root in members.bundles {
        let bundle = match Bundle::load(&bundle_root) {
            Ok(bundle) => bundle,
            Err(error) => {
                warnings.push(format!(
                    "warning: {bundle_root}/{} could not be read, so this build records no \
                     provenance for it ({error})",
                    ost_plugin::PLUGIN_MANIFEST
                ));
                continue;
            }
        };
        let Some(member) = member_relative(&canonical_project_root, &bundle.root) else {
            warnings.push(format!(
                "warning: bundle '{}' at {} is not below the project root {root}, so its \
                 outputs cannot be attributed to this build",
                bundle.manifest.name(),
                bundle.root
            ));
            continue;
        };
        let bundle_outputs = match collect_plugin_managed_outputs(&bundle) {
            Ok(outputs) => outputs,
            Err(error) => {
                warnings.push(format!(
                    "warning: could not digest managed outputs for bundle '{}' at {} ({error})",
                    bundle.manifest.name(),
                    bundle.root
                ));
                continue;
            }
        };
        for mut output in bundle_outputs {
            output.path = format!("{member}/{}", output.path);
            if outputs.insert(output.path.clone(), output).is_some() {
                warnings.push(format!(
                    "warning: duplicate workspace managed output under member '{member}' was recorded once"
                ));
            }
        }
    }
    (outputs.into_values().collect(), warnings)
}

#[derive(Debug, Default)]
pub(crate) struct ToolBuildBaseline {
    outputs: BTreeMap<String, BuildOutput>,
}

/// Snapshot build-tree files that could satisfy a workspace tool descriptor.
///
/// The snapshot is captured before CMake runs. Staging later compares exact
/// digests against it so an output merely left behind by an older invocation
/// cannot acquire fresh managed-build provenance.
pub(crate) fn snapshot_workspace_tool_build_outputs(
    root: &Utf8Path,
    build_dir: &Utf8Path,
    os: Os,
) -> Result<ToolBuildBaseline> {
    if !build_dir.is_dir() {
        return Ok(ToolBuildBaseline::default());
    }

    let suffix = if os == Os::Windows { ".exe" } else { "" };
    let mut filenames = BTreeSet::new();
    for tool_root in discover_workspace_tools(root)? {
        let tool = ost_plugin::Tool::load(&tool_root)?;
        filenames.extend(
            tool.manifest
                .executables
                .iter()
                .map(|executable| format!("{executable}{suffix}")),
        );
    }
    if filenames.is_empty() {
        return Ok(ToolBuildBaseline::default());
    }

    let mut files = Vec::new();
    collect_build_files(build_dir, build_dir, &mut files)?;
    let mut outputs = BTreeMap::new();
    for (relative, path) in files {
        if path
            .file_name()
            .is_none_or(|filename| !filenames.contains(filename))
        {
            continue;
        }
        let (sha256, size) = digest_file(&path)?;
        outputs.insert(
            relative.clone(),
            BuildOutput {
                path: relative,
                sha256,
                size,
            },
        );
    }
    Ok(ToolBuildBaseline { outputs })
}

#[derive(Debug)]
enum ToolDestinationState {
    Missing,
    File {
        bytes: Vec<u8>,
        permissions: std::fs::Permissions,
    },
}

#[derive(Debug)]
struct ToolStagingPlan {
    source: Utf8PathBuf,
    destination: Utf8PathBuf,
    bytes: Vec<u8>,
    permissions: std::fs::Permissions,
    original: ToolDestinationState,
    note: String,
}

/// Stage executables produced by a root workspace build into each tool member.
///
/// CMake normally leaves an `add_executable` target below the root binary tree,
/// while a tool artifact is intentionally packaged from the member root named
/// by `openstrata.tool.yaml`. Bridge those two layouts after a successful
/// managed build so the bytes recorded in the root completion are the exact
/// bytes packaging will consume.
///
/// If this invocation produced no fresh candidate, an executable already
/// present in a declared member directory remains untouched. For fresh
/// build-tree output, prefer candidates below the member's relative binary-tree
/// path, then the requested configuration, and finally a globally unique
/// filename. Ambiguity is left unstaged and reported instead of guessing which
/// binary should become a release artifact.
pub(crate) fn stage_workspace_tool_executables(
    root: &Utf8Path,
    build_dir: &Utf8Path,
    os: Os,
    config: &str,
    baseline: &ToolBuildBaseline,
) -> Result<Vec<String>> {
    let tool_roots = discover_workspace_tools(root)?;
    if tool_roots.is_empty() {
        return Ok(Vec::new());
    }

    let mut build_files = Vec::new();
    collect_build_files(build_dir, build_dir, &mut build_files)?;
    let canonical_project_root = canonical_root(root);
    let mut notes = Vec::new();
    let mut plans = Vec::new();
    let mut destinations = BTreeSet::new();
    let multi_config = build_tree_is_multi_config(build_dir);
    for tool_root in tool_roots {
        let tool = ost_plugin::Tool::load(&tool_root)?;
        let member = member_relative(&canonical_project_root, &tool.root).ok_or_else(|| {
            Error::validation(format!(
                "tool '{}' at {} is outside the project root {root}",
                tool.id(),
                tool.root
            ))
        })?;
        let suffix = if os == Os::Windows { ".exe" } else { "" };
        for executable in &tool.manifest.executables {
            let filename = format!("{executable}{suffix}");
            let member_has_executable = tool
                .manifest
                .directories
                .iter()
                .any(|directory| tool.root.join(directory).join(&filename).is_file());

            let candidates = build_files
                .iter()
                .filter(|(_, path)| path.file_name() == Some(filename.as_str()))
                .collect::<Vec<_>>();
            let mut fresh_candidates = Vec::new();
            for candidate in &candidates {
                let (sha256, size) = digest_file(&candidate.1)?;
                let current = BuildOutput {
                    path: candidate.0.clone(),
                    sha256,
                    size,
                };
                if baseline.outputs.get(&candidate.0) != Some(&current) {
                    fresh_candidates.push(*candidate);
                }
            }
            let selected =
                select_tool_build_candidate(&fresh_candidates, &member, config, multi_config);
            let Some(source) = selected else {
                if fresh_candidates.is_empty() && member_has_executable {
                    continue;
                }
                let detail = match fresh_candidates.len() {
                    0 if candidates.is_empty() => format!(
                        "warning: tool '{}' declares executable '{filename}', but the successful root build produced no matching file under {build_dir}; packaging requires it below {} in the tool member",
                        tool.id(), tool.manifest.directories.join(", ")
                    ),
                    0 => format!(
                        "warning: tool '{}' executable '{filename}' only matched output left unchanged from before this build; refusing to stage stale bytes as a managed result",
                        tool.id()
                    ),
                    count => format!(
                        "warning: tool '{}' executable '{filename}' matched {count} newly produced build-tree files and could not be staged for configuration '{config}' unambiguously; set a target output directory below member '{member}'",
                        tool.id()
                    ),
                };
                notes.push(detail);
                continue;
            };

            let directory = tool
                .manifest
                .directories
                .first()
                .expect("validated tool descriptors have at least one directory");
            let destination = tool.root.join(directory).join(&filename);
            let bytes = std::fs::read(source.as_std_path())
                .map_err(|error| Error::io(source.to_string(), error))?;
            let source_permissions = std::fs::metadata(source.as_std_path())
                .map_err(|error| Error::io(source.to_string(), error))?
                .permissions();
            if !destinations.insert(destination.clone()) {
                return Err(Error::validation(format!(
                    "more than one workspace tool executable stages to {destination}"
                )));
            }
            let original = read_tool_destination_state(&destination)?;
            let note = format!(
                "staged tool '{}' executable {} -> {}",
                tool.id(),
                source,
                destination
            );
            plans.push(ToolStagingPlan {
                source: source.clone(),
                destination,
                bytes,
                permissions: source_permissions,
                original,
                note,
            });
        }
    }
    apply_tool_staging_plans(&plans)?;
    notes.extend(plans.into_iter().map(|plan| plan.note));
    Ok(notes)
}

fn read_tool_destination_state(destination: &Utf8Path) -> Result<ToolDestinationState> {
    match std::fs::symlink_metadata(destination.as_std_path()) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(Error::validation(format!(
            "refusing to stage a tool executable over symlink {destination}"
        ))),
        Ok(metadata) if metadata.is_file() => {
            let bytes = std::fs::read(destination.as_std_path())
                .map_err(|error| Error::io(destination.to_string(), error))?;
            Ok(ToolDestinationState::File {
                bytes,
                permissions: metadata.permissions(),
            })
        }
        Ok(_) => Err(Error::validation(format!(
            "tool executable destination {destination} is not a regular file"
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(ToolDestinationState::Missing)
        }
        Err(error) => Err(Error::io(destination.to_string(), error)),
    }
}

fn apply_tool_staging_plans(plans: &[ToolStagingPlan]) -> Result<()> {
    // Finish every fallible read/descriptor check before this point. Directory
    // creation can leave only empty directories; file mutations below are
    // rolled back together if any atomic write or permission update fails.
    for plan in plans {
        let parent = plan
            .destination
            .parent()
            .expect("a staged tool executable has a member directory");
        std::fs::create_dir_all(parent.as_std_path())
            .map_err(|error| Error::io(parent.to_string(), error))?;
    }

    for (index, plan) in plans.iter().enumerate() {
        let result = make_tool_destination_replaceable(&plan.destination)
            .and_then(|()| write_atomic(plan.destination.as_std_path(), &plan.bytes))
            .and_then(|()| {
                std::fs::set_permissions(plan.destination.as_std_path(), plan.permissions.clone())
                    .map_err(|error| Error::io(plan.destination.to_string(), error))
            });
        if let Err(error) = result {
            let rollback_errors = plans[..=index]
                .iter()
                .rev()
                .filter_map(|applied| restore_tool_destination(applied).err())
                .map(|rollback| rollback.to_string())
                .collect::<Vec<_>>();
            if rollback_errors.is_empty() {
                return Err(error);
            }
            return Err(Error::coded(
                "TOOL_STAGING_ROLLBACK_FAILED",
                Category::Io,
                format!(
                    "tool staging failed while copying {} -> {} ({error}); rollback also failed: {}",
                    plan.source,
                    plan.destination,
                    rollback_errors.join("; ")
                ),
            ));
        }
    }
    Ok(())
}

fn restore_tool_destination(plan: &ToolStagingPlan) -> Result<()> {
    make_tool_destination_replaceable(&plan.destination)?;
    match &plan.original {
        ToolDestinationState::Missing => match std::fs::remove_file(plan.destination.as_std_path())
        {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Error::io(plan.destination.to_string(), error)),
        },
        ToolDestinationState::File { bytes, permissions } => {
            write_atomic(plan.destination.as_std_path(), bytes)?;
            std::fs::set_permissions(plan.destination.as_std_path(), permissions.clone())
                .map_err(|error| Error::io(plan.destination.to_string(), error))
        }
    }
}

fn make_tool_destination_replaceable(destination: &Utf8Path) -> Result<()> {
    #[cfg(windows)]
    if let Ok(metadata) = std::fs::metadata(destination.as_std_path()) {
        let mut permissions = metadata.permissions();
        if permissions.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            std::fs::set_permissions(destination.as_std_path(), permissions)
                .map_err(|error| Error::io(destination.to_string(), error))?;
        }
    }
    #[cfg(not(windows))]
    let _ = destination;
    Ok(())
}

fn collect_build_files(
    root: &Utf8Path,
    directory: &Utf8Path,
    files: &mut Vec<(String, Utf8PathBuf)>,
) -> Result<()> {
    let entries = std::fs::read_dir(directory.as_std_path())
        .map_err(|error| Error::io(directory.to_string(), error))?;
    for entry in entries {
        let entry = entry.map_err(|error| Error::io(directory.to_string(), error))?;
        let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|path| {
            Error::validation(format!(
                "build output path '{}' is not valid UTF-8",
                path.display()
            ))
        })?;
        let file_type = entry
            .file_type()
            .map_err(|error| Error::io(path.to_string(), error))?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_build_files(root, &path, files)?;
        } else if file_type.is_file() {
            let relative = path.strip_prefix(root).map_err(|error| {
                Error::validation(format!(
                    "build output '{path}' escaped build tree '{root}': {error}"
                ))
            })?;
            files.push((portable(relative), path));
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(())
}

fn select_tool_build_candidate<'a>(
    candidates: &[&'a (String, Utf8PathBuf)],
    member: &str,
    config: &str,
    multi_config: bool,
) -> Option<&'a Utf8PathBuf> {
    fn below_member(relative: &str, member: &str) -> bool {
        relative == member
            || relative.starts_with(&format!("{member}/"))
            || relative.contains(&format!("/{member}/"))
    }
    fn in_config(relative: &str, config: &str) -> bool {
        relative
            .split('/')
            .any(|segment| segment.eq_ignore_ascii_case(config))
    }
    fn in_other_standard_config(relative: &str, config: &str) -> bool {
        const STANDARD_CONFIGS: &[&str] = &["Debug", "Release", "RelWithDebInfo", "MinSizeRel"];
        relative.split('/').any(|segment| {
            STANDARD_CONFIGS.iter().any(|known| {
                segment.eq_ignore_ascii_case(known) && !segment.eq_ignore_ascii_case(config)
            })
        })
    }

    let member_candidates = candidates
        .iter()
        .copied()
        .filter(|(relative, _)| below_member(relative, member))
        .collect::<Vec<_>>();
    let configured = member_candidates
        .iter()
        .copied()
        .filter(|(relative, _)| in_config(relative, config))
        .collect::<Vec<_>>();
    if configured.len() == 1 {
        return Some(&configured[0].1);
    }
    if configured.len() > 1 {
        return None;
    }
    if !multi_config {
        let configless_member = member_candidates
            .iter()
            .copied()
            .filter(|(relative, _)| !in_other_standard_config(relative, config))
            .collect::<Vec<_>>();
        if configless_member.len() == 1 {
            return Some(&configless_member[0].1);
        }
    }
    if !member_candidates.is_empty() || candidates.len() != 1 {
        return None;
    }
    let (relative, path) = candidates[0];
    (in_config(relative, config) || (!multi_config && !in_other_standard_config(relative, config)))
        .then_some(path)
}

fn build_tree_is_multi_config(build_dir: &Utf8Path) -> bool {
    std::fs::read_to_string(build_dir.join("CMakeCache.txt").as_std_path()).is_ok_and(|source| {
        source
            .lines()
            .any(|line| line.starts_with("CMAKE_CONFIGURATION_TYPES:"))
    })
}

/// The executables every workspace tool declares, as managed build outputs
/// relative to the project root, for `ost build` to record in its completion.
///
/// Returns the outputs and any warnings, and **never fails**: this runs after a
/// successful build, only to record evidence about it, so nothing here may turn
/// a built target into a failed command. A tool that has not been built yet, an
/// unreadable descriptor, or an executable that cannot be digested each drop out
/// with a warning — the packaged tool then reports `untracked` provenance, which
/// is the honest answer rather than a build the caller has to re-run.
pub(crate) fn workspace_tool_outputs(root: &Utf8Path, os: Os) -> (Vec<BuildOutput>, Vec<String>) {
    let mut outputs = Vec::new();
    let mut warnings = Vec::new();
    let tool_roots = match discover_workspace_tools(root) {
        Ok(roots) => roots,
        Err(error) => {
            warnings.push(format!(
                "warning: could not scan for workspace tools under {root}: {error}"
            ));
            return (outputs, warnings);
        }
    };
    let canonical_root = canonical_root(root);
    for tool_root in tool_roots {
        let tool = match ost_plugin::Tool::load(&tool_root) {
            Ok(tool) => tool,
            Err(error) => {
                warnings.push(format!(
                    "warning: {tool_root}/{} could not be read, so this build records no \
                     provenance for it ({error})",
                    ost_plugin::TOOL_MANIFEST
                ));
                continue;
            }
        };
        let Ok(executables) = tool.locate_executables(&tool.root, os == Os::Windows) else {
            continue;
        };
        let Some(member) = member_relative(&canonical_root, &tool.root) else {
            warnings.push(format!(
                "warning: tool '{}' at {} is not below the project root {root}, so its \
                 outputs cannot be attributed to this build",
                tool.id(),
                tool.root
            ));
            continue;
        };
        for relative in executables {
            let path = tool.root.join(&relative);
            match digest_file(&path) {
                Ok((sha256, size)) => outputs.push(BuildOutput {
                    path: format!("{member}/{relative}"),
                    sha256,
                    size,
                }),
                Err(error) => warnings.push(format!(
                    "warning: could not digest tool executable {path}, so this build \
                     records no provenance for it ({error})"
                )),
            }
        }
    }
    (outputs, warnings)
}

/// `path` canonicalized and stripped of a Windows `\\?\` verbatim prefix, so it
/// compares with the canonical member roots `ost-plugin` records. Falls back to
/// the path as given when it cannot be canonicalized.
fn canonical_root(path: &Utf8Path) -> Utf8PathBuf {
    let Ok(canon) = std::fs::canonicalize(path.as_std_path()) else {
        return path.to_path_buf();
    };
    let Ok(utf8) = Utf8PathBuf::from_path_buf(canon) else {
        return path.to_path_buf();
    };
    match utf8.as_str().strip_prefix(r"\\?\UNC\") {
        Some(rest) => Utf8PathBuf::from(format!(r"\\{rest}")),
        None => match utf8.as_str().strip_prefix(r"\\?\") {
            Some(rest) => Utf8PathBuf::from(rest),
            None => utf8,
        },
    }
}

/// A member root's portable path relative to the project root.
///
/// A member root is canonical (`Tool::load` canonicalizes it) while a project
/// root is whatever `find_project_root` walked up to, so the two are compared
/// canonically: an 8.3 name, a case difference, or a symlinked temp directory
/// must not silently detach a tool from the build that produced it.
fn member_relative(canonical_project_root: &Utf8Path, member_root: &Utf8Path) -> Option<String> {
    let relative = match member_root.strip_prefix(canonical_project_root) {
        Ok(relative) => relative.to_path_buf(),
        Err(_) => canonical_root(member_root)
            .strip_prefix(canonical_project_root)
            .ok()?
            .to_path_buf(),
    };
    Some(portable(&relative))
}

/// Workspace-built executables: immediate subdirectories and `tools/*` entries
/// holding `openstrata.tool.yaml`, in deterministic order.
///
/// Tools sit outside the dependency graph — nothing in a workspace requires an
/// executable — so they are discovered, not resolved, and always packaged after
/// the bundles whose libraries they may load.
fn discover_workspace_tools(root: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    Ok(discover_workspace_members(root)?.tools)
}

#[derive(Debug)]
struct SourceWorkspace {
    root: Utf8PathBuf,
    bundles: BTreeMap<String, Bundle>,
    libraries: BTreeMap<String, Library>,
    graph: ost_plugin::WorkspaceValidation,
}

/// Identify the conventional workspace containing `primary`. A bundle directly
/// below `<root>/` belongs to `<root>`; one below `<root>/plugins/` belongs to
/// `<root>`. Discovery runs only when the primary declares a bundle or library
/// edge, so an extracted package is not treated as a source composition by
/// directory shape alone.
///
/// A primary with no bundle or library requirements has an empty closure, so
/// discovery is skipped entirely: unrelated sibling bundles or libraries (a
/// broken manifest, a stale backup copy) cannot fail a command that needs none.
///
/// A packaged manifest keeps its declared edges, so an extracted package
/// standing alone still reaches discovery. With no sibling bundles and no
/// plain libraries there is no source composition to validate: such a tree
/// keeps behaving like a plain bundle instead of failing graph validation.
fn source_workspace_for(primary: &Bundle) -> Result<Option<SourceWorkspace>> {
    let needs_library_workspace = !primary.manifest.requires.libraries.is_empty()
        && !has_materialized_package_library_closure(primary);
    if primary.manifest.requires.bundles.is_empty() && !needs_library_workspace {
        return Ok(None);
    }
    let parent = match primary.root.parent() {
        Some(parent) => parent,
        None => return Ok(None),
    };
    let project_root = find_project_root(primary.root.as_std_path())
        .and_then(|path| Utf8PathBuf::from_path_buf(path).ok());
    let conventional_root = if parent.file_name() == Some("plugins") {
        match parent.parent() {
            Some(root) => root,
            None => return Ok(None),
        }
    } else {
        parent
    };
    let root = project_root.as_deref().unwrap_or(conventional_root);
    let members = discover_workspace_members(root)?;
    if members.bundles.len() < 2 && members.libraries.is_empty() {
        return Ok(None);
    }
    let loaded = members
        .bundles
        .iter()
        .map(|path| Bundle::load(path))
        .collect::<Result<Vec<_>>>()?;
    if !loaded.iter().any(|bundle| bundle.root == primary.root) {
        return Ok(None);
    }

    let loaded_libraries = members
        .libraries
        .iter()
        .map(|path| Library::load(path))
        .collect::<Result<Vec<_>>>()?;
    let graph = ost_plugin::validate_workspace_with_libraries(&loaded, &loaded_libraries);
    if !graph.passed {
        let details = graph
            .issues
            .iter()
            .take(8)
            .map(|issue| format!("[{}] {}", issue.code, issue.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(Error::coded(
            "WORKSPACE_DEPENDENCY_GRAPH_INVALID",
            Category::Validation,
            format!(
                "workspace dependency graph at '{root}' is invalid: {details}"
            ),
        )
        .with_hint("run `ost plugin test --workspace --up-to 1` from the workspace root for the complete graph report"));
    }
    let bundles = loaded
        .into_iter()
        .map(|bundle| (bundle.manifest.name().to_string(), bundle))
        .collect();
    let libraries = loaded_libraries
        .into_iter()
        .map(|library| (library.id().to_string(), library))
        .collect();
    Ok(Some(SourceWorkspace {
        root: root.to_owned(),
        bundles,
        libraries,
        graph,
    }))
}

fn has_materialized_package_library_closure(bundle: &Bundle) -> bool {
    let evidence = bundle.root.join("dependencies.json");
    if !evidence.as_std_path().is_file() {
        return false;
    }
    let has_recorded_libraries = std::fs::read_to_string(evidence.as_std_path())
        .ok()
        .and_then(|source| serde_json::from_str::<serde_json::Value>(&source).ok())
        .and_then(|value| value["libraries"].as_array().map(|items| !items.is_empty()))
        .unwrap_or(false);
    has_recorded_libraries
        && bundle
            .manifest
            .requires
            .runtime_libs
            .iter()
            .filter(|directory| directory.starts_with("runtime/libraries"))
            .any(|directory| bundle.path(directory).as_std_path().is_dir())
}

fn dependencies_from_workspace(
    primary: &Bundle,
    workspace: &SourceWorkspace,
) -> Result<Vec<Bundle>> {
    let order = workspace
        .graph
        .dependency_order(primary.manifest.name())
        .ok_or_else(|| {
            Error::coded(
                "WORKSPACE_DEPENDENCY_GRAPH_INVALID",
                Category::Validation,
                format!(
                    "bundle '{}' is absent from the validated workspace graph at '{}'",
                    primary.manifest.name(),
                    workspace.root
                ),
            )
        })?;
    order
        .into_iter()
        .map(|id| {
            workspace.bundles.get(&id).cloned().ok_or_else(|| {
                Error::coded(
                    "WORKSPACE_DEPENDENCY_GRAPH_INVALID",
                    Category::Validation,
                    format!("validated workspace provider '{id}' could not be loaded"),
                )
            })
        })
        .collect()
}

fn selected_workspace_dependencies(primary: &Bundle) -> Result<Vec<Bundle>> {
    match source_workspace_for(primary)? {
        Some(workspace) => dependencies_from_workspace(primary, &workspace),
        None => Ok(Vec::new()),
    }
}

fn libraries_from_workspace(primary: &Bundle, workspace: &SourceWorkspace) -> Result<Vec<Library>> {
    let order = workspace
        .graph
        .library_dependency_order(primary.manifest.name())
        .ok_or_else(|| {
            Error::coded(
                "WORKSPACE_DEPENDENCY_GRAPH_INVALID",
                Category::Validation,
                format!(
                    "library closure for bundle '{}' is absent from the validated workspace graph at '{}'",
                    primary.manifest.name(),
                    workspace.root
                ),
            )
        })?;
    order
        .into_iter()
        .map(|id| {
            workspace.libraries.get(&id).cloned().ok_or_else(|| {
                Error::coded(
                    "WORKSPACE_DEPENDENCY_GRAPH_INVALID",
                    Category::Validation,
                    format!("validated workspace library '{id}' could not be loaded"),
                )
            })
        })
        .collect()
}

fn target_id_from_resolved(resolved: &crate::commands::Resolved) -> String {
    format!(
        "{}-{}-{}",
        resolved.runtime.platform,
        resolved.runtime.variant.short_slug(),
        resolved.runtime.profile
    )
}

fn library_runtime_dirs_from_workspace(
    primary: &Bundle,
    workspace: &SourceWorkspace,
    resolved: &crate::commands::Resolved,
    require_materialized: bool,
) -> Result<Vec<Utf8PathBuf>> {
    let libraries = libraries_from_workspace(primary, workspace)?;
    let prefix = workspace
        .root
        .join(STATE_DIR)
        .join("targets")
        .join(target_id_from_resolved(resolved))
        .join("workspace-prefix");
    let mut directories = Vec::new();
    for library in libraries {
        let materialized = library.installed_runtime_dirs(&prefix);
        if require_materialized
            && !library.manifest.runtime.directories.is_empty()
            && materialized.is_empty()
        {
            return Err(Error::coded(
                "WORKSPACE_LIBRARY_RUNTIME_MISSING",
                Category::Precondition,
                format!(
                    "library '{}' {} has no installed runtime directory under '{}'",
                    library.id(),
                    library.version(),
                    prefix
                ),
            )
            .with_hint(format!(
                "run `ost plugin build {}` so the validated library closure is installed before test/run",
                primary.root
            )));
        }
        directories.extend(materialized);
    }
    directories.sort();
    directories.dedup();
    Ok(directories)
}

fn selected_workspace_library_runtime_dirs(
    primary: &Bundle,
    resolved: &crate::commands::Resolved,
    require_materialized: bool,
) -> Result<Vec<Utf8PathBuf>> {
    match source_workspace_for(primary)? {
        Some(workspace) => {
            library_runtime_dirs_from_workspace(primary, &workspace, resolved, require_materialized)
        }
        None => Ok(Vec::new()),
    }
}

fn selected_workspace_library_evidence(
    primary: &Bundle,
    resolved: Option<&crate::commands::Resolved>,
) -> Result<Vec<serde_json::Value>> {
    let Some(workspace) = source_workspace_for(primary)? else {
        let path = primary.root.join("dependencies.json");
        let Some(libraries) = std::fs::read_to_string(path.as_std_path())
            .ok()
            .and_then(|source| serde_json::from_str::<serde_json::Value>(&source).ok())
            .and_then(|value| value["libraries"].as_array().cloned())
        else {
            return Ok(Vec::new());
        };
        return Ok(libraries);
    };
    let libraries = libraries_from_workspace(primary, &workspace)?;
    let prefix = resolved.map(|resolved| {
        workspace
            .root
            .join(STATE_DIR)
            .join("targets")
            .join(target_id_from_resolved(resolved))
            .join("workspace-prefix")
    });
    Ok(libraries
        .into_iter()
        .map(|library| {
            // Every path in the record is forward-slashed: this document ships
            // inside a portable artifact and is read on hosts that never saw the
            // producer's separators.
            let runtime_directories = prefix
                .as_ref()
                .map(|prefix| {
                    library
                        .installed_runtime_dirs(prefix)
                        .into_iter()
                        .map(|directory| portable(&directory))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            serde_json::json!({
                "id": library.id(),
                "version": library.version(),
                "descriptor": portable(&library.root.join(ost_plugin::LIBRARY_MANIFEST)),
                "cmake_package": library.manifest.cmake.package,
                "cmake_target": library.manifest.cmake.target,
                "prefix": prefix.as_ref().map(|prefix| portable(prefix)),
                "runtime_directories": runtime_directories,
                "provenance": "source-workspace",
            })
        })
        .collect())
}

fn selected_library_package_runtime(
    primary: &Bundle,
    target_id: &str,
) -> Result<Vec<(Utf8PathBuf, Utf8PathBuf)>> {
    let Some(workspace) = source_workspace_for(primary)? else {
        return Ok(Vec::new());
    };
    let libraries = libraries_from_workspace(primary, &workspace)?;
    let prefix = workspace
        .root
        .join(STATE_DIR)
        .join("targets")
        .join(target_id)
        .join("workspace-prefix");
    let mut mappings = BTreeMap::<Utf8PathBuf, Utf8PathBuf>::new();
    for library in libraries {
        let mut materialized = 0usize;
        for directory in &library.manifest.runtime.directories {
            let source = prefix.join(directory);
            if source.as_std_path().is_dir() {
                let relative = Utf8Path::new("runtime/libraries").join(directory);
                mappings.entry(relative).or_insert(source);
                materialized += 1;
            }
        }
        if !library.manifest.runtime.directories.is_empty() && materialized == 0 {
            return Err(Error::coded(
                "WORKSPACE_LIBRARY_RUNTIME_MISSING",
                Category::Precondition,
                format!(
                    "library '{}' {} has no packageable runtime directory under '{}'",
                    library.id(),
                    library.version(),
                    prefix
                ),
            )
            .with_hint(format!(
                "run `ost plugin build {}` before packaging so the library closure is installed",
                primary.root
            )));
        }
    }
    Ok(mappings
        .into_iter()
        .map(|(relative, source)| (source, relative))
        .collect())
}

/// The USD *registration* half of a `requires.bundles` closure, as staged paths.
///
/// Returns `(source plugInfo root, package-relative destination)` per resolved
/// dependency bundle. `selected_library_package_runtime` above carries the link
/// half — a provider's shared libraries — and that half alone is what made
/// v0.18.0 packages assert a closure they did not have: the libraries shipped,
/// `dependencies.json` recorded the bundle as resolved, and USD still could not
/// apply the provider's schemas because its `plugInfo.json` and
/// `generatedSchema.usda` were nowhere in the artifact. A `kind: usd-schema`
/// dependency is exactly the kind whose entire value is that registration half.
///
/// A compiled provider's relative `LibraryPath` continues to resolve because
/// [`selected_bundle_package_libraries`] stages its `lib/` beside the copied
/// `plugin/` tree. That directory is also declared in `requires.runtime_libs`
/// so transitive loader dependencies can resolve it.
fn selected_bundle_package_registration(
    dependencies: &[Bundle],
) -> Result<Vec<(Utf8PathBuf, Utf8PathBuf)>> {
    let mut mappings = Vec::new();
    for dependency in dependencies {
        let source = dependency.plug_info_root();
        if !source.as_std_path().is_dir() {
            return Err(Error::coded(
                "WORKSPACE_BUNDLE_RUNTIME_MISSING",
                Category::Precondition,
                format!(
                    "bundle dependency '{}' has no packageable plugInfo root at '{}'",
                    dependency.manifest.name(),
                    source
                ),
            )
            .with_hint(format!(
                "build '{}' before packaging so its USD registration resources exist",
                dependency.root
            )));
        }
        let relative = Utf8Path::new("runtime/bundles")
            .join(dependency.manifest.name())
            .join(plug_info_root_rel(dependency));
        mappings.push((source, relative));
    }
    Ok(mappings)
}

/// The link half of each resolved plugin-bundle dependency.
///
/// Keeping the provider's `lib/` beside its copied `plugin/` tree preserves the
/// relative `../../../lib/<name>` paths generated into OpenUSD `plugInfo.json`
/// files. Codeless schemas legitimately have no library; every other bundle
/// must have built its packageable library directory before a dependent can
/// claim to carry it.
fn selected_bundle_package_libraries(
    dependencies: &[Bundle],
) -> Result<Vec<(Utf8PathBuf, Utf8PathBuf)>> {
    let mut mappings = Vec::new();
    for dependency in dependencies {
        let source = dependency.lib_dir();
        if source.as_std_path().is_dir() {
            let relative = Utf8Path::new("runtime/bundles")
                .join(dependency.manifest.name())
                .join("lib");
            mappings.push((source, relative));
        } else if !dependency.manifest.is_codeless_schema() {
            return Err(Error::coded(
                "WORKSPACE_BUNDLE_RUNTIME_MISSING",
                Category::Precondition,
                format!(
                    "bundle dependency '{}' has no packageable library directory at '{}'",
                    dependency.manifest.name(),
                    source
                ),
            )
            .with_hint(format!(
                "build '{}' before packaging so its shared library exists",
                dependency.root
            )));
        }
    }
    Ok(mappings)
}

/// Write the resolved dependency closure beside a report or into a package.
///
/// Both halves of the closure are recorded. `libraries` was always here;
/// `bundles` is what a consumer previously had no way to see. A bundle that
/// depends on another bundle's schema used to ship with nothing saying so, and
/// the omission only surfaced at runtime as a schema-application failure on the
/// consumer's machine — a missing provider is now visible in the artifact
/// itself, before anything is loaded.
fn write_dependency_evidence(
    report_dir: &Utf8Path,
    libraries: &[serde_json::Value],
    bundles: &[serde_json::Value],
) -> Result<()> {
    if libraries.is_empty() && bundles.is_empty() {
        return Ok(());
    }
    let path = report_dir.join("dependencies.json");
    let body = serde_json::to_string_pretty(&serde_json::json!({
        "schema": "openstrata.dependencies/v1alpha1",
        "libraries": libraries,
        "bundles": bundles,
    }))
    .map_err(|error| Error::parse("dependencies.json", anyhow::Error::new(error)))?;
    std::fs::write(path.as_std_path(), format!("{body}\n"))
        .map_err(|error| Error::io(path.to_string(), error))
}

/// One bundle's entry in the recorded closure.
///
/// The shape mirrors the library entries deliberately: a consumer reading
/// `dependencies.json` should not need two parsers to answer "what does this
/// artifact require, and did I get it?".
fn bundle_evidence(bundle: &Bundle, descriptor: &str, provenance: &str) -> serde_json::Value {
    serde_json::json!({
        "id": bundle.manifest.name(),
        "version": bundle.manifest.plugin.version,
        "kind": bundle.manifest.kind().as_str(),
        // The schema contract is what a dependent actually binds to, so a
        // consumer can detect a provider that moved to an incompatible one.
        "contract": bundle
            .manifest
            .schema
            .as_ref()
            .and_then(|schema| schema.contract),
        "descriptor": descriptor,
        "provenance": provenance,
    })
}

/// The bundle half of the closure, resolved from the source workspace graph.
fn selected_workspace_bundle_evidence(primary: &Bundle) -> Result<Vec<serde_json::Value>> {
    Ok(selected_workspace_dependencies(primary)?
        .iter()
        .map(|bundle| {
            bundle_evidence(
                bundle,
                &portable(&bundle.root.join(ost_plugin::PLUGIN_MANIFEST)),
                "source-workspace",
            )
        })
        .collect())
}

/// Render a path the way a portable artifact must carry it.
///
/// Staged manifests and `dependencies.json` are consumed on hosts other than the
/// one that produced them, so a Windows producer must not bake `\` separators
/// into a document a Linux consumer reads back.
fn portable(path: &Utf8Path) -> String {
    path.as_str().replace('\\', "/")
}

/// Merge graph-resolved dependencies with explicit `--with` bundles. Identity
/// is never selected by path order: an exact identity/version/kind/contract
/// duplicate is harmless and deduplicated; any disagreement is a hard error.
fn merge_composed_bundles(
    primary: &Bundle,
    resolved: Vec<Bundle>,
    explicit: Vec<Bundle>,
) -> Result<Vec<Bundle>> {
    type Signature = (String, PluginKind, Option<u64>);
    let signature = |bundle: &Bundle| -> Signature {
        (
            bundle.manifest.plugin.version.clone(),
            bundle.manifest.kind(),
            bundle
                .manifest
                .schema
                .as_ref()
                .and_then(|schema| schema.contract),
        )
    };
    let mut seen = BTreeMap::new();
    seen.insert(primary.manifest.name().to_string(), signature(primary));
    let mut merged = Vec::new();
    for bundle in resolved.into_iter().chain(explicit) {
        let id = bundle.manifest.name().to_string();
        let actual = signature(&bundle);
        if let Some(expected) = seen.get(&id) {
            if expected == &actual {
                continue;
            }
            return Err(Error::coded(
                "WORKSPACE_DUPLICATE_BUNDLE_ID",
                Category::Validation,
                format!(
                    "bundle id '{id}' resolves to conflicting identities: {:?} and {:?}",
                    expected, actual
                ),
            )
            .with_hint("remove the duplicate --with entry or make its version, kind, and schema contract agree with the workspace provider"));
        }
        seen.insert(id, actual);
        merged.push(bundle);
    }
    Ok(merged)
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
    let library_dirs = selected_workspace_library_runtime_dirs(&bundle, &r, true)?;

    let usdview = locate_runtime_tool(Some(&r), &["usdview.cmd", "usdview.exe", "usdview"])
        .ok_or_else(|| {
            Error::coded(
                "REQUIRED_TOOL_MISSING",
                ost_core::Category::Precondition,
                "usdview not found in the runtime (build/adopt one with usdview enabled)",
            )
        })?;
    let fixture_identifier = fixture_identifier(&bundle, fixture);

    let mut contributing = Vec::with_capacity(with_bundles.len() + 1);
    contributing.push(&bundle);
    contributing.extend(with_bundles.iter());
    let library_dirs = library_dirs
        .iter()
        .map(Utf8PathBuf::as_path)
        .collect::<Vec<_>>();
    let session = ost_plugin::session_env_from_with_library_dirs(
        &r.env,
        &contributing,
        &library_dirs,
        host.os,
    );
    let mut cmd = Command::new(&usdview);
    cmd.arg(&fixture_identifier);
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
    let library_dirs = selected_workspace_library_runtime_dirs(&bundle, &r, true)?;

    let mut contributing = Vec::with_capacity(with_bundles.len() + 1);
    contributing.push(&bundle);
    contributing.extend(with_bundles.iter());
    let library_dirs = library_dirs
        .iter()
        .map(Utf8PathBuf::as_path)
        .collect::<Vec<_>>();
    let session = ost_plugin::session_env_from_with_library_dirs(
        &r.env,
        &contributing,
        &library_dirs,
        host.os,
    );
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
    python_version: &str,
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
    // The usdGenSchema *script* in the runtime bin is a Python script with no
    // executable bit / `.cmd` wrapper, so it must be run *through* an interpreter.
    let gen_script = artifact_prefix.join("bin/usdGenSchema");
    if !gen_script.as_std_path().is_file() {
        clear_cohosted_schema_compile_state(compiled_dir, cmake_fragment)?;
        println!(
            "==> usdGenSchema not in the runtime; keeping the committed co-hosted schema resources"
        );
        return Ok(None);
    }

    // Resolve the interpreter from the runtime (its bundled `python3`, else a
    // version-matched host one), never a bare `python` on PATH — macOS and modern
    // Linux ship only `python3`, so a bare `python` dies with a bewildering
    // `IO_ERROR: run python: No such file or directory` mid-build.
    let interpreter =
        ost_build::resolve_run_python(artifact_prefix, python_version).ok_or_else(|| {
            let searched = ost_build::run_python_search_paths(artifact_prefix, python_version);
            Error::precondition(format!(
                "no Python interpreter found to run usdGenSchema (searched: {})",
                searched.join(", ")
            ))
            .with_hint("install python3, or pull a runtime that bundles an interpreter under bin/")
            .with_phase(PHASE_SCHEMA_GENERATE)
        })?;
    let (program, leading) = interpreter
        .split_first()
        .expect("resolve_run_python never returns an empty argv");
    let mut args: Vec<String> = leading.to_vec();
    args.extend([
        gen_script.to_string(),
        schema_src.to_string(),
        staging.to_string(),
    ]);

    reset_dir(staging)?; // also (re)creates the dir
    println!("==> regenerating co-hosted schema with usdGenSchema");
    run_step(
        PHASE_SCHEMA_GENERATE,
        std::path::Path::new(program),
        &args,
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

/// Fail early (with the doctor hint's exact fix) if the bundle's committed
/// `plugInfo.json` names a library with the wrong platform suffix for the target
/// being built — so a cross-platform-committed `.dll` on macOS surfaces here as
/// a clear precondition rather than later as USD's opaque dlopen failure. A
/// missing/unparseable/library-less plugInfo (e.g. a resource-only codeless
/// schema) is left to the doctor's structural checks; unresolved template tokens
/// mean a `plugInfo.json.in` configure step owns the concrete suffix.
fn verify_target_library_suffix(bundle: &Bundle, os: Os) -> Result<()> {
    let plug_info = bundle.plug_info();
    let Ok(src) = std::fs::read_to_string(plug_info.as_std_path()) else {
        return Ok(());
    };
    let Ok(paths) = ost_plugin::library_plugin_paths(&src) else {
        return Ok(());
    };
    let expected = ost_plugin::shared_library_suffix(os);
    for path in paths {
        if ost_plugin::contains_template_token(&path) {
            continue; // a plugInfo.json.in placeholder configure resolves per target
        }
        if !path.ends_with(expected) {
            return Err(Error::precondition(format!(
                "plugInfo.json LibraryPath '{path}' is not a {expected} library for {}",
                os.as_str()
            ))
            .with_hint(format!(
                "regenerate plugInfo.json for the {} target (ship a `plugInfo.json.in` configured with @CMAKE_SHARED_LIBRARY_SUFFIX@), so `LibraryPath` ends in {expected}",
                os.as_str()
            ))
            .with_phase(PHASE_PLUGIN_DISCOVERY));
        }
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
    // Present when the input is itself an extracted package: repackaging must not
    // silently drop a staged provider's registration half.
    for dir in &bundle.manifest.requires.runtime_plugin_paths {
        copy_tree_required(&bundle.path(dir), Utf8Path::new(dir), stage)?;
    }
    for fixture in bundle.manifest.all_fixtures() {
        copy_file_required(&bundle.path(fixture), Utf8Path::new(fixture), stage)?;
    }
    // L5's deterministic oracle convention is `<roundtrip fixture>.golden.usda`.
    // It is optional until the source bundle actually carries that file; once
    // present, copy_file_required makes it fail closed on symlinks or copy
    // errors instead of silently producing a package that drops the claim.
    for fixture in &bundle.manifest.tests.roundtrip {
        let oracle = adjacent_golden(fixture);
        let source = bundle.path(&oracle);
        match std::fs::symlink_metadata(source.as_std_path()) {
            Ok(_) => copy_file_required(&source, Utf8Path::new(&oracle), stage)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::io(source.to_string(), error)),
        }
    }
    // New scaffolds carry deterministic template/generator/input provenance.
    // Keep packaging backward-compatible with adopted/legacy bundles that do
    // not have it, but preserve it whenever present.
    let provenance = bundle.path(SCAFFOLD_PROVENANCE);
    if provenance.as_std_path().is_file() {
        copy_file_required(&provenance, Utf8Path::new(SCAFFOLD_PROVENANCE), stage)?;
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

/// Write the versioned fixture/oracle association into the archive. Both
/// content digests are repeated here even though `manifest.json files[]` hashes
/// every archived file: this document preserves which golden verifies which
/// fixture after extraction, while the producer manifest points consumers to
/// the contract itself.
fn write_verification_contract(bundle: &Bundle) -> Result<PluginVerification> {
    let verification = PluginVerification::from_bundle(bundle)?;
    let value = serde_json::to_value(&verification)
        .map_err(|error| Error::parse(PLUGIN_VERIFICATION, anyhow::Error::new(error)))?;
    write_text(
        &bundle.root.join(PLUGIN_VERIFICATION),
        &pretty_json(&value)?,
    )?;
    Ok(verification)
}

/// Emit the consumer-facing activation contract carried by every packaged
/// bundle. The JSON file is the stable, portable description; the PowerShell
/// and Bash snippets make it directly usable without parsing the plugin YAML,
/// and the Python bootstrap covers Python 3.8+'s Windows DLL-search behavior by
/// retaining `os.add_dll_directory()` handles for the life of the process.
fn write_activation_files(bundle: &Bundle, target_os: Os) -> Result<()> {
    let plugin_paths = unique_portable_paths(
        std::iter::once(plug_info_root_rel(bundle))
            .chain(
                bundle
                    .manifest
                    .requires
                    .runtime_plugin_paths
                    .iter()
                    .map(Utf8PathBuf::from),
            )
            .collect(),
    );
    let library_paths = unique_portable_paths(
        std::iter::once(Utf8PathBuf::from("lib"))
            .chain(
                bundle
                    .manifest
                    .requires
                    .runtime_libs
                    .iter()
                    .map(Utf8PathBuf::from),
            )
            .collect(),
    );
    let python_paths = vec!["python".to_string()];
    let loader_env = activation_loader_key(target_os);

    let contract = serde_json::json!({
        "schema": "openstrata.activation/v1alpha1",
        "target_os": target_os.as_str(),
        "root": ".",
        "environment": {
            "plugin": "PXR_PLUGINPATH_NAME",
            "loader": loader_env,
            "python": "PYTHONPATH",
        },
        "plugin_paths": plugin_paths,
        "library_paths": library_paths,
        "python_paths": python_paths,
        "entrypoints": {
            "powershell": "activate.ps1",
            "bash": "activate.sh",
            "python": "openstrata_activate.py",
        },
        "python_dll_search": {
            "windows": "import openstrata_activate before importing pxr; the module retains os.add_dll_directory handles",
        },
    });
    write_text(
        &bundle.root.join("openstrata.activation.json"),
        &pretty_json(&contract)?,
    )?;

    write_text(
        &bundle.root.join("activate.ps1"),
        &render_powershell_activation(&plugin_paths, &library_paths, &python_paths, loader_env),
    )?;
    write_text(
        &bundle.root.join("activate.sh"),
        &render_bash_activation(
            &plugin_paths,
            &library_paths,
            &python_paths,
            loader_env,
            target_os,
        ),
    )?;
    write_text(
        &bundle.root.join("openstrata_activate.py"),
        &render_python_activation(&plugin_paths, &library_paths, &python_paths),
    )
}

fn activation_loader_key(os: Os) -> &'static str {
    match os {
        Os::Linux => "LD_LIBRARY_PATH",
        Os::Macos => "DYLD_LIBRARY_PATH",
        Os::Windows => "PATH",
    }
}

fn unique_portable_paths(paths: Vec<Utf8PathBuf>) -> Vec<String> {
    let mut result = Vec::new();
    for path in paths {
        let path = portable(&path);
        if !result.contains(&path) {
            result.push(path);
        }
    }
    result
}

fn render_powershell_activation(
    plugin_paths: &[String],
    library_paths: &[String],
    python_paths: &[String],
    loader_env: &str,
) -> String {
    let mut script = String::from(
        "# Generated by `ost plugin package`; dot-source this file.\n\
$openStrataRoot = $PSScriptRoot\n\
function Add-OpenStrataPath([string]$Name, [string]$Relative) {\n\
    $full = [IO.Path]::GetFullPath((Join-Path $openStrataRoot $Relative))\n\
    if (-not (Test-Path -LiteralPath $full -PathType Container)) { return }\n\
    $current = [Environment]::GetEnvironmentVariable($Name, 'Process')\n\
    $value = if ([string]::IsNullOrEmpty($current)) { $full } else { $full + [IO.Path]::PathSeparator + $current }\n\
    [Environment]::SetEnvironmentVariable($Name, $value, 'Process')\n\
}\n",
    );
    for path in plugin_paths.iter().rev() {
        script.push_str(&format!(
            "Add-OpenStrataPath 'PXR_PLUGINPATH_NAME' '{}'\n",
            powershell_single_quote(path)
        ));
    }
    for path in library_paths.iter().rev() {
        script.push_str(&format!(
            "Add-OpenStrataPath '{}' '{}'\n",
            powershell_single_quote(loader_env),
            powershell_single_quote(path)
        ));
    }
    for path in python_paths.iter().rev() {
        script.push_str(&format!(
            "Add-OpenStrataPath 'PYTHONPATH' '{}'\n",
            powershell_single_quote(path)
        ));
    }
    script.push_str(
        "Remove-Item Function:\\Add-OpenStrataPath\n\
Remove-Variable openStrataRoot\n\
# On Windows, Python 3.8+ consumers must also `import openstrata_activate` before `pxr`.\n",
    );
    script
}

fn powershell_single_quote(value: &str) -> String {
    value.replace('\'', "''")
}

fn render_bash_activation(
    plugin_paths: &[String],
    library_paths: &[String],
    python_paths: &[String],
    loader_env: &str,
    target_os: Os,
) -> String {
    let separator = if target_os == Os::Windows { ";" } else { ":" };
    let mut script = format!(
        "# Generated by `ost plugin package`; source this file from Bash.\n\
_ost_root=\"$(cd -- \"$(dirname -- \"${{BASH_SOURCE[0]}}\")\" && pwd -P)\"\n\
_ost_prepend() {{\n\
    local _key=\"$1\" _relative=\"$2\" _full _current\n\
    _full=\"${{_ost_root}}/${{_relative}}\"\n\
    [[ -d \"$_full\" ]] || return 0\n\
    if declare -p \"$_key\" >/dev/null 2>&1; then _current=\"${{!_key}}\"; else _current=\"\"; fi\n\
    if [[ -n \"$_current\" ]]; then printf -v \"$_key\" '%s{separator}%s' \"$_full\" \"$_current\"; else printf -v \"$_key\" '%s' \"$_full\"; fi\n\
    export \"$_key\"\n\
}}\n"
    );
    for path in plugin_paths.iter().rev() {
        script.push_str(&format!(
            "_ost_prepend PXR_PLUGINPATH_NAME {}\n",
            bash_single_quote(path)
        ));
    }
    for path in library_paths.iter().rev() {
        script.push_str(&format!(
            "_ost_prepend {} {}\n",
            bash_single_quote(loader_env),
            bash_single_quote(path)
        ));
    }
    for path in python_paths.iter().rev() {
        script.push_str(&format!(
            "_ost_prepend PYTHONPATH {}\n",
            bash_single_quote(path)
        ));
    }
    script.push_str("unset -f _ost_prepend\nunset _ost_root\n");
    script
}

fn bash_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn render_python_activation(
    plugin_paths: &[String],
    library_paths: &[String],
    python_paths: &[String],
) -> String {
    let plugin_paths = serde_json::to_string(plugin_paths).expect("string arrays serialize");
    let library_paths = serde_json::to_string(library_paths).expect("string arrays serialize");
    let python_paths = serde_json::to_string(python_paths).expect("string arrays serialize");
    format!(
        r#""""Activate this extracted plugin package for the current Python process.

Import this module before importing ``pxr``. On Windows/Python 3.8+ the retained
``os.add_dll_directory`` handles make transitive packaged DLLs discoverable.
"""
from __future__ import annotations

import os
import sys
from pathlib import Path

_ROOT = Path(__file__).resolve().parent
_PLUGIN_PATHS = {plugin_paths}
_LIBRARY_PATHS = {library_paths}
_PYTHON_PATHS = {python_paths}
_DLL_DIRECTORY_HANDLES = []
_DLL_DIRECTORY_PATHS = set()


def _existing(relative_paths):
    return [str((_ROOT / relative).resolve()) for relative in relative_paths if (_ROOT / relative).is_dir()]


def _prepend(name, paths):
    current = os.environ.get(name)
    values = list(paths)
    if current:
        values.append(current)
    if values:
        os.environ[name] = os.pathsep.join(values)


def activate():
    plugin_paths = _existing(_PLUGIN_PATHS)
    library_paths = _existing(_LIBRARY_PATHS)
    python_paths = _existing(_PYTHON_PATHS)
    _prepend("PXR_PLUGINPATH_NAME", plugin_paths)
    _prepend("PATH" if os.name == "nt" else ("DYLD_LIBRARY_PATH" if sys.platform == "darwin" else "LD_LIBRARY_PATH"), library_paths)
    _prepend("PYTHONPATH", python_paths)
    for path in reversed(python_paths):
        if path not in sys.path:
            sys.path.insert(0, path)
    if os.name == "nt" and hasattr(os, "add_dll_directory"):
        for path in library_paths:
            if path not in _DLL_DIRECTORY_PATHS:
                _DLL_DIRECTORY_HANDLES.append(os.add_dll_directory(path))
                _DLL_DIRECTORY_PATHS.add(path)
    return {{
        "plugin_paths": plugin_paths,
        "library_paths": library_paths,
        "python_paths": python_paths,
    }}


ACTIVATION = activate()
"#
    )
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

fn plugin_debug_archive_name(name: &str, version: &str, id: &str) -> String {
    format!("{name}-{version}-{id}-debug.tar.zst")
}

fn remove_stale_debug_archive(path: &Utf8Path) -> Result<()> {
    match std::fs::remove_file(path.as_std_path()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(path.to_string(), e)),
    }
}

/// Debug-symbol sidecar files split out of a lean package: the MSVC program
/// database (`.pdb`) and split-DWARF objects (`.dwo`). Debug info embedded in an
/// ELF/Mach-O binary is not a separate file and is left in place — stripping it
/// needs the toolchain (`strip`/`objcopy`), not a file move.
fn is_debug_symbol_file(path: &Utf8Path) -> bool {
    matches!(
        path.extension().map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("pdb") | Some("dwo")
    )
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

fn report_package(outcome: &PackageOutcome, fmt: Format) {
    let PackageOutcome {
        id,
        archive_path,
        packed,
        debug,
        debug_status,
        build_provenance,
        stage_warnings,
        ..
    } = outcome;
    if fmt.is_json() {
        let mut obj = serde_json::json!({
            "packaged": true,
            "target": id,
            "archive": archive_path.to_string(),
            "archive_digest": packed.archive_digest,
            "archive_size": packed.archive_size,
            "files": packed.files.len(),
            "debug_package": (*debug_status).json(),
            "build_provenance": build_provenance.json(),
        });
        if let Some((debug_name, dp)) = debug {
            obj["debug"] = serde_json::json!({
                "archive": debug_name,
                "archive_digest": dp.archive_digest,
                "archive_size": dp.archive_size,
                "files": dp.files.len(),
            });
        }
        output::report_with_warnings(true, &obj, stage_warnings);
        return;
    }
    for w in stage_warnings {
        if let Some(msg) = w["message"].as_str() {
            eprintln!("warning: {msg}");
        }
    }
    println!("Packaged plugin target {id}");
    println!("  archive:  {archive_path}");
    println!("  digest:   {}", packed.archive_digest);
    println!(
        "  size:     {} bytes ({} file(s), {} uncompressed)",
        packed.archive_size,
        packed.files.len(),
        packed.total_size
    );
    if let Some((debug_name, dp)) = debug {
        println!(
            "  debug:    {debug_name} ({} bytes, {} file(s)) — sibling symbol package",
            dp.archive_size,
            dp.files.len()
        );
    } else {
        println!("  debug:    {}", (*debug_status).human_reason());
    }
    println!(
        "  build:    {} ({}) — {}",
        build_provenance.status.as_str(),
        build_provenance.origin,
        build_provenance.detail
    );
    println!("  manifest.json + SHA256SUMS written alongside the archive");
}

/// Determine platform+profile from explicit flags or the enclosing project.
/// Returns `None` when neither is available.
pub(crate) fn selection(
    target: Option<String>,
    profile: Option<String>,
) -> Option<(String, String)> {
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

/// Select the narrowest profile that satisfies a packaged component when the
/// caller names a target but no project/profile. This keeps the historical
/// explicit/project selection rules, while avoiding a misleading default to
/// `core` for a package that declares `usd-stage-read`.
fn selection_for_capabilities(
    target: Option<String>,
    profile: Option<String>,
    required: &[String],
) -> Result<(String, String)> {
    if profile.is_some() || target.is_none() {
        return selection(target, profile).ok_or_else(|| {
            Error::usage(
                "no platform/profile: run inside an OpenStrata project or pass --target/--profile",
            )
        });
    }

    let platform = target.expect("checked above");
    if required.is_empty() {
        return Ok((platform, "core".into()));
    }
    let catalog = ProfileCatalog::load()?;
    let mut candidates = catalog
        .iter()
        .filter(|candidate| {
            required
                .iter()
                .all(|capability| candidate.capabilities().contains(capability))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|candidate| (candidate.capabilities().len(), candidate.id.as_str()));
    let Some(first) = candidates.first() else {
        return Err(Error::config(format!(
            "default profile 'core' cannot satisfy required capabilities [{}] for target {platform}",
            required.join(", ")
        ))
        .with_hint(
            "declare a profile providing those capabilities, then pass `--profile <name>`",
        ));
    };
    let minimum = first.capabilities().len();
    let minimal = candidates
        .iter()
        .take_while(|candidate| candidate.capabilities().len() == minimum)
        .map(|candidate| candidate.id.clone())
        .collect::<Vec<_>>();
    if minimal.len() != 1 {
        return Err(Error::config(format!(
            "default profile 'core' cannot satisfy required capabilities [{}], and multiple minimal profiles qualify: {}",
            required.join(", "),
            minimal.join(", ")
        ))
        .with_hint(format!("pass `--profile {}`", minimal.join("` or `--profile "))));
    }
    Ok((platform, minimal[0].clone()))
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

/// The named phases of a plugin build, attributed onto any failure so `--json`
/// and human output name *which* step failed (design §14.4) instead of leaving
/// triage to bisect a mix of Python-exec, USD-loader and staging errors.
const PHASE_SCHEMA_GENERATE: &str = "schema-generate";
const PHASE_CONFIGURE: &str = "configure";
const PHASE_COMPILE_LINK: &str = "compile-link";
const PHASE_SCHEMA_MERGE: &str = "schema-merge";
const PHASE_PLUGIN_DISCOVERY: &str = "plugin-discovery";

/// Re-tag an error with the build phase it occurred in. Coded errors gain the
/// phase slot directly; other variants are re-wrapped so the phase still
/// surfaces while their stable code/category are preserved.
fn in_phase(phase: &'static str, e: Error) -> Error {
    match e {
        Error::Coded { .. } => e.with_phase(phase),
        other => {
            let (code, category, message) = (other.code(), other.category(), other.to_string());
            Error::coded(code, category, message).with_phase(phase)
        }
    }
}

fn run_step(
    phase: &'static str,
    program: &std::path::Path,
    args: &[String],
    env: &[(String, String)],
) -> Result<()> {
    println!("==> [{phase}] {} {}", program.display(), args.join(" "));
    let mut cmd = Command::new(program);
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v); // overlay the MSVC delta, no global mutation
    }
    let status = cmd.status().map_err(|e| {
        Error::external_tool(format!(
            "failed to launch {} for the {phase} phase: {e}",
            program.display()
        ))
        .with_phase(phase)
    })?;
    if !status.success() {
        let code = status
            .code()
            .map(|c| format!(" (exit {c})"))
            .unwrap_or_default();
        return Err(Error::external_tool(format!(
            "the {phase} phase failed{code}: {}",
            program.display()
        ))
        .with_phase(phase));
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
pub(crate) fn target_state_dir(root: &Utf8Path, id: &str) -> Utf8PathBuf {
    root.join(STATE_DIR).join("targets").join(id)
}

/// Per-target CMake build tree inside a bundle: `build/<id>`. Keeping the build
/// tree under the target id prevents one target reusing another's CMake cache.
pub(crate) fn target_build_dir(root: &Utf8Path, id: &str) -> Utf8PathBuf {
    root.join("build").join(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ost_core::host::Arch;

    #[test]
    fn root_build_stages_declared_tool_executables_into_the_member() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = Utf8PathBuf::from_path_buf(
            std::env::temp_dir().join(format!("ost-tool-stage-{}-{nonce}", std::process::id())),
        )
        .unwrap();
        let tool_root = root.join("tools/motion_retarget");
        let build = root.join("build/target");
        let os = if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        };
        let filename = if cfg!(windows) {
            "motion_retarget.exe"
        } else {
            "motion_retarget"
        };
        std::fs::create_dir_all(tool_root.as_std_path()).unwrap();
        std::fs::create_dir_all(build.join("tools/motion_retarget/Release").as_std_path()).unwrap();
        std::fs::write(
            tool_root.join(ost_plugin::TOOL_MANIFEST).as_std_path(),
            "schema: openstrata.tool/v1alpha1\n\
             tool: { id: motion_retarget, version: 0.4.0 }\n\
             executables: [motion_retarget]\n\
             directories: [bin]\n",
        )
        .unwrap();
        let produced = build.join("tools/motion_retarget/Release").join(filename);
        std::fs::write(produced.as_std_path(), b"managed tool bytes").unwrap();
        std::fs::create_dir_all(tool_root.join("bin").as_std_path()).unwrap();
        std::fs::write(tool_root.join("bin").join(filename), b"stale member bytes").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                produced.as_std_path(),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }

        let notes = stage_workspace_tool_executables(
            &root,
            &build,
            os,
            "Release",
            &ToolBuildBaseline::default(),
        )
        .unwrap();

        let staged = tool_root.join("bin").join(filename);
        assert_eq!(
            std::fs::read(staged.as_std_path()).unwrap(),
            b"managed tool bytes"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                std::fs::metadata(staged.as_std_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o111,
                0
            );
        }
        assert_eq!(notes.len(), 1);
        let (outputs, warnings) = workspace_tool_outputs(&root, os);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(outputs.len(), 1);
        assert_eq!(
            outputs[0].path,
            format!("tools/motion_retarget/bin/{filename}")
        );
        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn tool_staging_never_guesses_between_ambiguous_build_outputs() {
        let candidates = [
            (
                "other/Debug/tool".to_string(),
                Utf8PathBuf::from("build/other/Debug/tool"),
            ),
            (
                "another/Release/tool".to_string(),
                Utf8PathBuf::from("build/another/Release/tool"),
            ),
        ];
        let references = candidates.iter().collect::<Vec<_>>();
        assert!(select_tool_build_candidate(&references, "tools/tool", "Release", false).is_none());
    }

    #[test]
    fn tool_staging_rejects_a_candidate_from_another_configuration() {
        let candidates = [(
            "tools/tool/Debug/tool".to_string(),
            Utf8PathBuf::from("build/tools/tool/Debug/tool"),
        )];
        let references = candidates.iter().collect::<Vec<_>>();

        assert!(select_tool_build_candidate(&references, "tools/tool", "Release", false).is_none());
        assert!(select_tool_build_candidate(&references, "tools/tool", "Debug", false).is_some());
    }

    #[test]
    fn tool_staging_does_not_fall_back_around_a_member_configuration_mismatch() {
        let candidates = [
            (
                "tools/tool/Debug/tool".to_string(),
                Utf8PathBuf::from("build/tools/tool/Debug/tool"),
            ),
            (
                "Release/tool".to_string(),
                Utf8PathBuf::from("build/Release/tool"),
            ),
        ];
        let references = candidates.iter().collect::<Vec<_>>();

        assert!(select_tool_build_candidate(&references, "tools/tool", "Release", true).is_none());
    }

    #[test]
    fn tool_staging_does_not_promote_an_unchanged_build_tree_file() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = Utf8PathBuf::from_path_buf(
            std::env::temp_dir().join(format!("ost-tool-stale-{}-{nonce}", std::process::id())),
        )
        .unwrap();
        let tool_root = root.join("tools/motion_retarget");
        let build = root.join("build/target");
        let os = if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        };
        let filename = if cfg!(windows) {
            "motion_retarget.exe"
        } else {
            "motion_retarget"
        };
        std::fs::create_dir_all(tool_root.as_std_path()).unwrap();
        std::fs::create_dir_all(build.join("tools/motion_retarget/Release").as_std_path()).unwrap();
        std::fs::write(
            tool_root.join(ost_plugin::TOOL_MANIFEST).as_std_path(),
            "schema: openstrata.tool/v1alpha1\n\
             tool: { id: motion_retarget, version: 0.4.0 }\n\
             executables: [motion_retarget]\n\
             directories: [bin]\n",
        )
        .unwrap();
        let produced = build.join("tools/motion_retarget/Release").join(filename);
        std::fs::write(produced.as_std_path(), b"old build bytes").unwrap();
        let baseline = snapshot_workspace_tool_build_outputs(&root, &build, os).unwrap();

        let notes =
            stage_workspace_tool_executables(&root, &build, os, "Release", &baseline).unwrap();

        assert!(!tool_root.join("bin").join(filename).exists());
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("left unchanged from before this build"));
        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn tool_staging_preflights_every_descriptor_before_writing() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = Utf8PathBuf::from_path_buf(
            std::env::temp_dir().join(format!("ost-tool-preflight-{}-{nonce}", std::process::id())),
        )
        .unwrap();
        let good = root.join("tools/a-good");
        let invalid = root.join("tools/z-invalid");
        let build = root.join("build/target");
        let os = if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        };
        let filename = if cfg!(windows) { "good.exe" } else { "good" };
        std::fs::create_dir_all(good.join("bin").as_std_path()).unwrap();
        std::fs::create_dir_all(invalid.as_std_path()).unwrap();
        std::fs::create_dir_all(build.join("tools/a-good/Release").as_std_path()).unwrap();
        std::fs::write(
            good.join(ost_plugin::TOOL_MANIFEST).as_std_path(),
            "schema: openstrata.tool/v1alpha1\n\
             tool: { id: good, version: 0.4.0 }\n\
             executables: [good]\n\
             directories: [bin]\n",
        )
        .unwrap();
        std::fs::write(
            invalid.join(ost_plugin::TOOL_MANIFEST).as_std_path(),
            "schema: openstrata.tool/v1alpha1\n\
             tool: { id: invalid, version: 0.4.0 }\n\
             executables: [invalid]\n\
             directories: []\n",
        )
        .unwrap();
        let destination = good.join("bin").join(filename);
        std::fs::write(destination.as_std_path(), b"original member bytes").unwrap();
        std::fs::write(
            build.join("tools/a-good/Release").join(filename),
            b"new build bytes",
        )
        .unwrap();

        assert!(stage_workspace_tool_executables(
            &root,
            &build,
            os,
            "Release",
            &ToolBuildBaseline::default(),
        )
        .is_err());
        assert_eq!(
            std::fs::read(destination.as_std_path()).unwrap(),
            b"original member bytes"
        );
        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

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

    fn product_member() -> PluginProductMember {
        PluginProductMember {
            id: "toy".into(),
            position: 0,
            member: ProductMemberKind::Bundle,
            destination: None,
            paths: Vec::new(),
            name: "toy".into(),
            version: "0.1.0".into(),
            kind: "usd-fileformat".into(),
            archive: "members/toy/toy-0.1.0.tar.zst".into(),
            archive_digest: format!("sha256:{}", "ab".repeat(32)),
            archive_size: 1,
            manifest: "members/toy/manifest.json".into(),
            checksums: "members/toy/SHA256SUMS".into(),
            evidence: vec!["members/toy/sbom.spdx.json".into()],
            debug: RequiredProductDebug(None),
            dependencies: serde_json::Value::Null,
        }
    }

    #[test]
    fn workspace_member_component_globs_are_segment_local() {
        for (pattern, name) in [
            ("*", "hydra2"),
            ("hydra*", "hydra2"),
            ("h?dra2", "hydra2"),
            ("アダプタ*", "アダプタ2"),
        ] {
            assert!(
                wildcard_component_matches(pattern, name),
                "{pattern} / {name}"
            );
        }
        for (pattern, name) in [("hydra?", "hydra20"), ("usd*", "hydra2"), ("?", "ab")] {
            assert!(
                !wildcard_component_matches(pattern, name),
                "{pattern} / {name}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn explicit_member_identity_uses_canonical_windows_casing() {
        let root = unique_tmp("workspace-member-casing");
        let member = root.join("plugins").join("alpha");
        std::fs::create_dir_all(member.as_std_path()).unwrap();
        write_test_file(
            &root.join(PROJECT_MANIFEST),
            "[project]\nname = 'case-test'\n\
             [requires]\nplatform = 'cy2026'\n\
             [workspace]\nmembers = ['PLUGINS/*']\n",
        );
        write_test_file(&member.join(ost_plugin::PLUGIN_MANIFEST), "placeholder\n");

        let discovered = discover_workspace_members(&root).unwrap();
        assert_eq!(discovered.bundles, vec![canonical_root(&member)]);
        let _ = std::fs::remove_dir_all(root.as_std_path());
    }

    /// `ost build` calls this *after* the build succeeded, only to record
    /// evidence about it. An unreadable tool descriptor must therefore not turn
    /// a built target into a failed command — which would also strand the target
    /// lease as takeover evidence for a build that actually finished.
    #[test]
    fn an_unreadable_tool_descriptor_warns_instead_of_failing_the_build() {
        let root = unique_tmp("tool-descriptor-warning");
        let bad = root.join("tools").join("broken");
        std::fs::create_dir_all(bad.as_std_path()).unwrap();
        write_test_file(
            &bad.join(ost_plugin::TOOL_MANIFEST),
            "schema: openstrata.tool/v0-not-a-schema\ntool: { id: broken }\n",
        );

        let (outputs, warnings) = workspace_tool_outputs(&root, Os::Linux);

        assert!(outputs.is_empty(), "a tool that cannot be read has none");
        assert_eq!(warnings.len(), 1, "and is reported once: {warnings:?}");
        assert!(
            warnings[0].contains(ost_plugin::TOOL_MANIFEST) && warnings[0].contains("broken"),
            "the warning names the descriptor: {}",
            warnings[0]
        );
        let _ = std::fs::remove_dir_all(root.as_std_path());
    }

    /// A tool whose descriptor is fine but which has not been built yet is not
    /// even a warning: `ost build` is what produces it, and a workspace may add
    /// the descriptor before its CMake target lands.
    #[test]
    fn a_tool_that_is_not_built_yet_is_silent() {
        let root = unique_tmp("tool-not-built");
        let tool = root.join("tools").join("motion_retarget");
        std::fs::create_dir_all(tool.as_std_path()).unwrap();
        write_test_file(
            &tool.join(ost_plugin::TOOL_MANIFEST),
            "schema: openstrata.tool/v1alpha1\n\
             tool: { id: motion_retarget, version: 0.4.0 }\n\
             executables: [motion_retarget]\n",
        );

        let (outputs, warnings) = workspace_tool_outputs(&root, Os::Linux);

        assert!(outputs.is_empty());
        assert!(
            warnings.is_empty(),
            "not built is not a warning: {warnings:?}"
        );
        let _ = std::fs::remove_dir_all(root.as_std_path());
    }

    #[test]
    fn root_build_evidence_includes_workspace_bundle_outputs() {
        let root = unique_tmp("workspace-bundle-outputs");
        let bundle = root.join("plugins").join("toy");
        std::fs::create_dir_all(bundle.as_std_path()).unwrap();
        write_test_file(
            &root.join(PROJECT_MANIFEST),
            "[project]\nname = 'workspace'\nversion = '0.1.0'\n\
             [requires]\nplatform = 'cy2026'\nprofile = 'usd'\n\
             [workspace]\nmembers = ['plugins/*']\n",
        );
        write_test_file(
            &bundle.join(ost_plugin::PLUGIN_MANIFEST),
            "plugin: { name: toy, version: 0.1.0, kind: usd-fileformat }\n\
             runtime: { openusd: '>=25.05,<26.0' }\n\
             provides: [usd-fileformat:toy]\n\
             usd: { plug_info: plugin/resources/toy/plugInfo.json }\n",
        );
        write_test_file(
            &bundle.join("plugin/resources/toy/plugInfo.json"),
            r#"{ "Plugins": [{ "Type": "library", "Name": "toy" }] }"#,
        );
        write_test_file(&bundle.join("lib/libToy.so"), "managed bytes");

        let (outputs, warnings) = workspace_managed_outputs(&root, Os::Linux);

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let paths = outputs
            .iter()
            .map(|output| output.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                "plugins/toy/lib/libToy.so",
                "plugins/toy/plugin/resources/toy/plugInfo.json"
            ]
        );
        assert!(outputs
            .iter()
            .all(|output| output.sha256.starts_with("sha256:") && output.size > 0));
        let _ = std::fs::remove_dir_all(root.as_std_path());
    }

    #[test]
    fn explicit_product_archive_ignores_an_unrelated_sibling_manifest() {
        let root = unique_tmp("explicit-product-archive");
        std::fs::create_dir_all(root.as_std_path()).unwrap();
        let archive = root.join("downloaded-product.tar.zst");
        std::fs::write(archive.as_std_path(), b"standalone product bytes").unwrap();
        write_test_file(
            &root.join("manifest.json"),
            &pretty_json(&publishable_manifest()).unwrap(),
        );

        let source = resolve_product_archive(archive.as_str(), None)
            .expect("an explicit archive must not be shadowed by an unrelated manifest");
        let (digest, size) = digest_file(&archive).unwrap();
        assert_eq!(source.archive, archive);
        assert_eq!(source.digest, digest);
        assert_eq!(source.size, size);

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn product_member_target_must_match_the_product_target() {
        let mut manifest = publishable_manifest();
        manifest["archive"] = "toy-0.1.0.tar.zst".into();
        manifest["target"] = "cy2026-linux-x86_64-glibc228-py313-usd".into();

        let error = verify_product_member_manifest(
            "cy2026-windows-x86_64-msvc143-py313-usd",
            &product_member(),
            &manifest,
        )
        .unwrap_err();

        assert!(error.to_string().contains("manifest target"), "{error}");
        assert!(
            error
                .to_string()
                .contains("cy2026-windows-x86_64-msvc143-py313-usd"),
            "{error}"
        );
    }

    #[test]
    fn legacy_product_install_fields_remain_optional_in_v1alpha1() {
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../schemas/plugin-product.schema.json"
        ))
        .unwrap();
        let required = schema["properties"]["install"]["required"]
            .as_array()
            .unwrap();

        assert!(required.iter().any(|value| value == "layout"));
        assert!(required.iter().any(|value| value == "order"));
        assert!(required.iter().any(|value| value == "contract"));
        assert!(!required.iter().any(|value| value == "destination"));
        assert!(!required.iter().any(|value| value == "activation"));
    }

    #[test]
    fn debug_symbol_files_are_classified_for_the_lean_split() {
        // Split out of the lean main package into the sibling `*-debug` archive.
        for debug in ["lib/usdToy.pdb", "lib/foo.PDB", "lib/bar.dwo"] {
            assert!(
                is_debug_symbol_file(Utf8Path::new(debug)),
                "{debug} should be split out as a debug symbol"
            );
        }
        // Kept in the lean main package: the loadable binary, bindings, plugInfo,
        // notices — none are debug sidecars. Embedded ELF debug info rides along
        // inside the `.so`.
        for keep in [
            "lib/usdToy.dll",
            "lib/usdToy.so",
            "lib/usdToy.dylib",
            "plugin/usd/plugInfo.json",
            "python/pxr/Toy/__init__.py",
            "NOTICE.md",
        ] {
            assert!(
                !is_debug_symbol_file(Utf8Path::new(keep)),
                "{keep} must stay in the main package"
            );
        }
    }

    #[test]
    fn plugin_run_resolves_only_bare_python_requests() {
        for program in ["python", "python3", "python.exe", "python3.exe"] {
            assert!(
                is_runtime_python_request(program),
                "{program} should request runtime-aware Python resolution"
            );
        }
        if cfg!(windows) {
            assert!(is_runtime_python_request("py"));
            assert!(is_runtime_python_request("py.exe"));
        } else {
            assert!(!is_runtime_python_request("py"));
        }
        for explicit in ["./python", "/usr/bin/python", "C:\\Python313\\python.exe"] {
            assert!(
                !is_runtime_python_request(explicit),
                "{explicit} is explicit and must not be replaced"
            );
        }
    }

    #[test]
    fn plugin_run_leaves_py_launcher_version_selectors_explicit() {
        for selector in ["-3", "-3.11", "-3.11-64", "-V:PythonCore/3.11"] {
            let rest = vec![
                selector.to_string(),
                "-c".to_string(),
                "print(1)".to_string(),
            ];
            let mut command = vec!["py".to_string()];
            command.extend(rest.clone());
            let (program, args) = prepare_session_command(&command, Utf8Path::new("."), "3.11")
                .expect("explicit py launcher selector should not need runtime resolution");
            assert_eq!(program, "py");
            assert_eq!(args, rest);
        }

        assert!(!is_explicit_py_launcher_version_request(
            "py",
            &["-c".to_string(), "print(1)".to_string()]
        ));
    }

    #[test]
    fn plugin_run_preserves_args_after_resolved_python() {
        let rest = vec!["-c".to_string(), "print(1)".to_string()];
        let (program, args) =
            merge_resolved_python_command(vec!["C:/Python313/python.exe".into()], &rest).unwrap();
        assert_eq!(program, "C:/Python313/python.exe");
        assert_eq!(args, rest);

        let (program, args) = merge_resolved_python_command(
            vec!["C:/Python313/python.exe".into(), "-I".into()],
            &rest,
        )
        .unwrap();
        assert_eq!(program, "C:/Python313/python.exe");
        assert_eq!(args, ["-I", "-c", "print(1)"]);
    }

    #[test]
    fn repack_without_a_debug_sidecar_removes_the_previous_one() {
        let root = std::env::temp_dir().join(format!("ost-stale-debug-{}", std::process::id()));
        let root = Utf8PathBuf::from_path_buf(root).unwrap();
        std::fs::create_dir_all(root.as_std_path()).unwrap();
        let sidecar = root.join("toy-debug.tar.zst");
        std::fs::write(sidecar.as_std_path(), b"old symbols").unwrap();

        remove_stale_debug_archive(&sidecar).unwrap();
        assert!(!sidecar.as_std_path().exists());
        remove_stale_debug_archive(&sidecar).unwrap();

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn packaged_python_activation_covers_windows_dll_search() {
        let script = render_python_activation(
            &["plugin/resources/toy".into()],
            &["lib".into(), "runtime/libraries/bin".into()],
            &["python".into()],
        );
        assert!(script.starts_with("\"\"\"Activate"), "{script}");
        assert!(script.contains("os.add_dll_directory(path)"));
        assert!(script.contains("_DLL_DIRECTORY_HANDLES.append"));
        assert!(script.contains("runtime/libraries/bin"));
        assert!(!script.contains("\\\"\\\"\\\""));
    }

    #[test]
    fn activation_scripts_preserve_declared_priority() {
        let plugins = vec![
            "plugin/resources/toy".into(),
            "runtime/bundles/schema".into(),
        ];
        let libraries = vec!["lib".into(), "runtime/libraries/bin".into()];
        let python = vec!["python".into()];
        let ps = render_powershell_activation(&plugins, &libraries, &python, "PATH");
        assert!(
            ps.find("runtime/bundles/schema").unwrap() < ps.find("plugin/resources/toy").unwrap(),
            "prepend calls run in reverse so the primary ends first: {ps}"
        );
        let bash =
            render_bash_activation(&plugins, &libraries, &python, "LD_LIBRARY_PATH", Os::Linux);
        assert!(bash.contains("printf -v \"$_key\" '%s:%s'"));
        assert!(bash.contains("_ost_prepend 'LD_LIBRARY_PATH' 'runtime/libraries/bin'"));
        assert!(
            !bash.contains("${!_key-}"),
            "invalid indirect expansion: {bash}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn bash_activation_sources_with_unset_environment_variables() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
            "ost-bash-activation-{}-{nonce}",
            std::process::id()
        )))
        .unwrap();
        for relative in ["plugin/resources/toy", "lib", "python"] {
            std::fs::create_dir_all(root.join(relative).as_std_path()).unwrap();
        }
        let script_path = root.join("activate.sh");
        let script = render_bash_activation(
            &["plugin/resources/toy".into()],
            &["lib".into()],
            &["python".into()],
            "LD_LIBRARY_PATH",
            Os::Linux,
        );
        std::fs::write(script_path.as_std_path(), script).unwrap();

        let output = Command::new("bash")
            .args([
                "-uc",
                "source \"$1\"; printf '%s\\n' \"$PXR_PLUGINPATH_NAME\" \"$LD_LIBRARY_PATH\" \"$PYTHONPATH\"",
                "bash",
            ])
            .arg(script_path.as_std_path())
            .env_remove("PXR_PLUGINPATH_NAME")
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("PYTHONPATH")
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "activation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("plugin/resources/toy"), "{stdout}");
        assert!(stdout.contains("/lib"), "{stdout}");
        assert!(stdout.contains("/python"), "{stdout}");

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn bundle_registration_refuses_a_missing_provider_root() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
            "ost-missing-provider-{}-{nonce}",
            std::process::id()
        )))
        .unwrap();
        std::fs::create_dir_all(root.as_std_path()).unwrap();
        let manifest = ost_plugin::PluginManifest::parse(
            r#"
plugin: { name: schema, version: 0.1.0, kind: usd-schema }
runtime: { openusd: ">=24.11,<25.0" }
usd: { plug_info: plugin/resources/schema/plugInfo.json }
schema: { codeless: true, contract: 1 }
"#,
        )
        .unwrap();
        let dependency = Bundle {
            root: root.clone(),
            manifest,
        };

        let error = selected_bundle_package_registration(&[dependency]).unwrap_err();

        assert!(
            error.to_string().contains("no packageable plugInfo root"),
            "{error}"
        );
        std::fs::remove_dir_all(root.as_std_path()).ok();
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

    fn managed_output_test_bundle(tag: &str) -> (Utf8PathBuf, Bundle, ost_build::Target) {
        let root = unique_tmp(tag);
        write_test_file(
            &root.join("openstrata.plugin.yaml"),
            "plugin: { name: toy, version: 0.1.0, kind: usd-fileformat }\n\
             runtime: { openusd: '>=25.05,<26.0' }\n\
             provides: [usd-fileformat:toy]\n\
             usd: { plug_info: plugin/resources/toy/plugInfo.json }\n",
        );
        write_test_file(
            &root.join("plugin/resources/toy/plugInfo.json"),
            r#"{ "Plugins": [{ "Type": "library", "Name": "toy" }] }"#,
        );
        write_test_file(&root.join("lib/libToy.so"), "managed bytes");
        let bundle = Bundle::load(&root).unwrap();
        let target = ost_build::Target {
            platform: "cy2026".into(),
            profile: "usd".into(),
            variant: variant(
                Os::Linux,
                Abi::Glibc {
                    version: "2.28".into(),
                },
            ),
            runtime_id: "openstrata-cy2026-usd".into(),
            runtime_digest: format!("sha256:{}", "ab".repeat(32)),
            python_version: "3.13.x".into(),
            cxx_standard: "20".into(),
            capabilities: vec!["usd-stage-read".into()],
            generator: "Ninja".into(),
        };
        (root, bundle, target)
    }

    #[test]
    fn managed_plugin_outputs_match_then_detect_an_overwrite() {
        let (root, bundle, target) = managed_output_test_bundle("managed-output-match");
        let id = target.id();
        let target_dir = target_state_dir(&bundle.root, &id);
        let build_dir = target_build_dir(&bundle.root, &id);
        std::fs::create_dir_all(target_dir.as_std_path()).unwrap();
        std::fs::create_dir_all(build_dir.as_std_path()).unwrap();
        let toolchain = target_dir.join("toolchain.cmake");
        write_test_file(&toolchain, "# managed");
        let completion = write_plugin_build_completion(
            &bundle,
            &target,
            &ost_build::LockCompiler::default(),
            &toolchain,
            &build_dir,
            Some("test-plugin-build"),
        )
        .unwrap();
        assert_eq!(completion.invocation.as_deref(), Some("test-plugin-build"));

        let matched = assess_plugin_build_provenance(&bundle, &target).unwrap();
        assert_eq!(matched.status, PluginBuildProvenanceStatus::Matched);
        assert_eq!(matched.expected_outputs, 2);
        assert!(matched.differences.is_empty());

        write_test_file(&bundle.path("lib/libToy.so"), "plain CMake replacement");
        let mismatched = assess_plugin_build_provenance(&bundle, &target).unwrap();
        assert_eq!(mismatched.status, PluginBuildProvenanceStatus::Mismatched);
        let changed = mismatched
            .differences
            .iter()
            .find(|difference| difference.path == "lib/libToy.so")
            .unwrap();
        assert_eq!(changed.kind, "digest-mismatch");
        assert!(changed.expected.as_deref().unwrap().starts_with("sha256:"));
        assert!(changed.observed.as_deref().unwrap().starts_with("sha256:"));
        assert!(mismatched.mismatch_message().contains("last managed build"));

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn staged_plugin_outputs_are_the_package_provenance_snapshot() {
        let (root, bundle, target) = managed_output_test_bundle("managed-output-stage");
        let id = target.id();
        let target_dir = target_state_dir(&bundle.root, &id);
        let build_dir = target_build_dir(&bundle.root, &id);
        std::fs::create_dir_all(target_dir.as_std_path()).unwrap();
        std::fs::create_dir_all(build_dir.as_std_path()).unwrap();
        let toolchain = target_dir.join("toolchain.cmake");
        write_test_file(&toolchain, "# managed");
        write_plugin_build_completion(
            &bundle,
            &target,
            &ost_build::LockCompiler::default(),
            &toolchain,
            &build_dir,
            Some("test-plugin-build"),
        )
        .unwrap();

        let stage_root = unique_tmp("managed-output-stage-copy");
        write_test_file(
            &stage_root.join("openstrata.plugin.yaml"),
            "plugin: { name: toy, version: 0.1.0, kind: usd-fileformat }\n\
             runtime: { openusd: '>=25.05,<26.0' }\n\
             provides: [usd-fileformat:toy]\n\
             usd: { plug_info: plugin/resources/toy/plugInfo.json }\n",
        );
        write_test_file(
            &stage_root.join("plugin/resources/toy/plugInfo.json"),
            r#"{ "Plugins": [{ "Type": "library", "Name": "toy" }] }"#,
        );
        write_test_file(&stage_root.join("lib/libToy.so"), "managed bytes");
        let staged_bundle = Bundle::load(&stage_root).unwrap();
        let staged_outputs = collect_plugin_managed_outputs(&staged_bundle).unwrap();

        // A writer changing the source after staging must not change what the
        // package reports about the already-copied archive inputs.
        write_test_file(&bundle.path("lib/libToy.so"), "changed after staging");
        assert_eq!(
            assess_plugin_build_provenance(&bundle, &target)
                .unwrap()
                .status,
            PluginBuildProvenanceStatus::Mismatched
        );
        assert_eq!(
            assess_plugin_build_provenance_for_outputs(&bundle, &target, staged_outputs)
                .unwrap()
                .status,
            PluginBuildProvenanceStatus::Matched
        );

        std::fs::remove_dir_all(root.as_std_path()).ok();
        std::fs::remove_dir_all(stage_root.as_std_path()).ok();
    }

    #[test]
    fn outputs_without_a_managed_completion_are_reported_untracked() {
        let (root, bundle, target) = managed_output_test_bundle("managed-output-untracked");

        let provenance = assess_plugin_build_provenance(&bundle, &target).unwrap();

        assert_eq!(provenance.status, PluginBuildProvenanceStatus::Untracked);
        assert_eq!(provenance.origin, "external-or-unmanaged");
        assert_eq!(provenance.observed_outputs, 2);
        assert_eq!(
            provenance.warning().unwrap()["code"],
            "PLUGIN_PACKAGE_OUTPUT_UNTRACKED"
        );
        std::fs::remove_dir_all(root.as_std_path()).ok();
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
    fn cohosted_schema_errors_are_phase_attributed() {
        let root = unique_tmp("schema-phase");
        write_test_file(
            &root.join("openstrata.plugin.yaml"),
            "plugin:\n  name: toy\n  version: 0.1.0\n  kind: usd-fileformat\n\
             runtime:\n  openusd: \">=25.05,<27.0\"\n\
             provides:\n  - usd-fileformat:toy\n  - usd-schema:ToyAPI\n\
             usd:\n  plug_info: plugin/resources/toy/plugInfo.json\n\
             schema:\n  source: schema/missing.usda\n",
        );
        write_test_file(
            &root.join("plugin/resources/toy/plugInfo.json"),
            r#"{ "Plugins": [{ "Type": "library", "Name": "toy" }] }"#,
        );
        let bundle = Bundle::load(&root).expect("bundle loads");

        let err = prepare_cohosted_schema(
            &bundle,
            Utf8Path::new("/missing/runtime"),
            "3.11",
            &root.join("schema-gen"),
            &root.join("compiled"),
            &root.join("schema-sources.cmake"),
            &[],
        )
        .map_err(|e| in_phase(PHASE_SCHEMA_GENERATE, e))
        .unwrap_err();

        assert_eq!(err.code(), "INVALID_CONFIG");
        assert_eq!(err.phase(), Some(PHASE_SCHEMA_GENERATE));

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
