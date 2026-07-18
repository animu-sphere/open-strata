// SPDX-License-Identifier: Apache-2.0
//! `ost renderer` — renderer-project developer workflows.
//!
//! The renderer remains one ordinary CMake project. This command does not add
//! another build/package lifecycle; it requests an optional Hydra adapter from
//! the common managed build service, then bridges its installed product into
//! the matching OpenUSD runtime session for interactive usdview.

use std::collections::BTreeMap;
use std::process::Command;

use camino::{Utf8Path, Utf8PathBuf};
use clap::{Args, Subcommand};
use serde_json::Value;

use ost_core::fs::write_atomic;
use ost_core::host::Os;
use ost_core::paths::{find_project_root, PROJECT_MANIFEST, STATE_DIR};
use ost_core::{tools, Category, Error, Host, Result};
use ost_manifest::{
    set_version_file, FrameContract, Project, ProjectMeta, RenderProducts, RendererComposition,
    RendererIdentity, RendererManifest, RendererReport, RendererValidation, Requires,
    RENDERER_MANIFEST, RENDERER_SCHEMA,
};
use ost_runtime::{EnvOp, EnvVar, RuntimeManifest, MANIFEST_FILE};

use crate::commands::build::{self, BuildArgs};
use crate::commands::configure::{build_target, resolve_selection};
use crate::commands::{resolve, with_host_python_on_path, Resolved};
use crate::output::{self, Format};

const RENDERER_ADOPTION_FILE: &str = "openstrata.renderer-adoption.json";

#[derive(Debug, Subcommand)]
pub enum RendererCmd {
    /// Safely adopt an existing CMake renderer without overwriting source.
    Adopt(RendererAdoptArgs),

    /// Merge independently produced renderer reports with conflict checks.
    Merge(RendererMergeArgs),

    /// Open a scene in usdview with the built Hydra renderer selected.
    View {
        /// USD scene to open. Defaults to the installed usdview smoke scene.
        scene: Option<Utf8PathBuf>,

        /// External/prebuilt Hydra CMake tree. Omit for an OST-managed build.
        #[arg(long)]
        build_dir: Option<Utf8PathBuf>,

        /// CMake configuration to install and inspect.
        #[arg(long, default_value = "Release")]
        config: String,

        /// CMake generator for the managed build. Ninja remains the default.
        #[arg(long)]
        generator: Option<String>,

        /// Platform target, e.g. `cy2026`. Defaults to the project's platform.
        #[arg(long)]
        target: Option<String>,

        /// Runtime profile. Auto-selects a unique pulled usdview runtime.
        #[arg(long)]
        profile: Option<String>,

        /// Camera prim to view through. Omitted by default: the scene is
        /// inspected and a camera is used only if one is actually there,
        /// otherwise usdview opens on its free camera.
        #[arg(long)]
        camera: Option<String>,

        /// Override the renderer display name read from installed plugInfo.json.
        #[arg(long)]
        renderer: Option<String>,
    },

    /// Build and launch the standalone native viewport adapter.
    Viewport {
        /// CMake configuration to build.
        #[arg(long, default_value = "Release")]
        config: String,

        /// CMake generator for the managed build. Ninja remains the default.
        #[arg(long)]
        generator: Option<String>,

        /// Platform target, e.g. `cy2026`. Defaults to the project's platform.
        #[arg(long)]
        target: Option<String>,

        /// Profile for the managed build. Defaults to the project's profile;
        /// the standalone viewport needs no OpenUSD runtime.
        #[arg(long)]
        profile: Option<String>,

        /// Arguments passed to the viewport executable after `--`, e.g.
        /// `ost renderer viewport -- --frames 8 --hidden`.
        #[arg(last = true)]
        args: Vec<String>,
    },
}

#[derive(Debug, Args)]
pub struct RendererAdoptArgs {
    /// Renderer/project identity.
    #[arg(long)]
    name: String,

    /// Existing CMake target for the host-neutral core.
    #[arg(long)]
    core: String,

    /// Existing CMake target for renderer extraction.
    #[arg(long)]
    extraction: String,

    /// Backend mapping in KIND=TARGET form, e.g. vulkan=merlin-vulkan.
    #[arg(long)]
    backend: String,

    /// Existing headless adapter target.
    #[arg(long)]
    headless: String,

    /// Existing optional Hydra 2 adapter target.
    #[arg(long)]
    hydra2: Option<String>,

    /// Existing optional standalone native viewport target.
    #[arg(long)]
    viewport: Option<String>,

    /// Platform for a missing openstrata.toml.
    #[arg(long)]
    platform: Option<String>,

    /// Host-neutral project profile for a missing openstrata.toml.
    #[arg(long, default_value = "core")]
    profile: String,

    /// Inline project version for a missing openstrata.toml (default 0.1.0).
    #[arg(long, conflicts_with = "version_file")]
    version: Option<String>,

    /// Existing repo-relative authoritative version file to adopt.
    #[arg(long, conflicts_with = "version")]
    version_file: Option<String>,

    /// Apply the plan. Without this flag the command is a read-only dry run.
    #[arg(long)]
    write: bool,

    /// Replace an existing, different renderer manifest (never source/CMake).
    #[arg(long)]
    replace_manifest: bool,

    /// Record unresolved target labels instead of refusing the write.
    #[arg(long)]
    allow_unresolved: bool,
}

#[derive(Debug, Args)]
pub struct RendererMergeArgs {
    /// Base renderer report.
    #[arg(long)]
    base: Utf8PathBuf,

    /// Overlay renderer report.
    #[arg(long)]
    overlay: Utf8PathBuf,

    /// Output renderer report.
    #[arg(long)]
    out: Utf8PathBuf,

    /// Explicitly replace duplicate assertion ids.
    #[arg(long)]
    replace: bool,
}

pub fn run(cmd: RendererCmd, fmt: Format) -> Result<()> {
    match cmd {
        RendererCmd::Adopt(args) => adopt(args, fmt),
        RendererCmd::Merge(args) => merge_reports(args, fmt),
        RendererCmd::View {
            scene,
            build_dir,
            config,
            generator,
            target,
            profile,
            camera,
            renderer,
        } => view(ViewArgs {
            scene,
            build_dir,
            config,
            generator,
            target,
            profile,
            camera,
            renderer,
        }),
        RendererCmd::Viewport {
            config,
            generator,
            target,
            profile,
            args,
        } => viewport(ViewportArgs {
            config,
            generator,
            target,
            profile,
            args,
        }),
    }
}

struct ViewportArgs {
    config: String,
    generator: Option<String>,
    target: Option<String>,
    profile: Option<String>,
    args: Vec<String>,
}

struct ViewArgs {
    scene: Option<Utf8PathBuf>,
    build_dir: Option<Utf8PathBuf>,
    config: String,
    generator: Option<String>,
    target: Option<String>,
    profile: Option<String>,
    camera: Option<String>,
    renderer: Option<String>,
}

fn adopt(args: RendererAdoptArgs, fmt: Format) -> Result<()> {
    let root = renderer_command_root()?;
    let cmake_root = root.join("CMakeLists.txt");
    if !cmake_root.as_std_path().is_file() {
        return Err(Error::precondition(format!(
            "existing renderer has no root CMakeLists.txt at {cmake_root}"
        )));
    }

    let (backend_kind, backend_target) = args.backend.split_once('=').ok_or_else(|| {
        Error::usage("--backend must use KIND=TARGET form, e.g. vulkan=merlin-vulkan")
    })?;
    if backend_kind.trim().is_empty() || backend_target.trim().is_empty() {
        return Err(Error::usage("--backend KIND and TARGET must not be empty"));
    }

    let project_path = root.join(PROJECT_MANIFEST);
    let (project, project_action, project_body) = if project_path.as_std_path().is_file() {
        let source = std::fs::read_to_string(project_path.as_std_path())
            .map_err(|error| Error::io(project_path.to_string(), error))?;
        if args.version.is_some() {
            return Err(Error::usage(
                "--version only applies when openstrata.toml is missing; use the existing manifest or --version-file",
            ));
        }
        let updated = args
            .version_file
            .as_deref()
            .map(|path| set_version_file(&source, path))
            .transpose()?
            .flatten();
        let project = Project::from_toml(updated.as_deref().unwrap_or(&source))?;
        if project.project.name != args.name {
            return Err(Error::config(format!(
                "existing project name '{}' does not match adopted renderer '{}'",
                project.project.name, args.name
            )));
        }
        let action = if updated.is_some() {
            "update-version-source"
        } else {
            "keep"
        };
        (project, action, updated)
    } else {
        let platform = args.platform.clone().ok_or_else(|| {
            Error::usage("--platform is required when openstrata.toml is missing")
        })?;
        let project = Project {
            project: ProjectMeta {
                name: args.name.clone(),
                version: if args.version_file.is_some() {
                    None
                } else {
                    Some(args.version.clone().unwrap_or_else(|| "0.1.0".into()))
                },
                version_file: args.version_file.clone(),
                description: Some("Adopted OpenStrata renderer project".into()),
            },
            requires: Requires {
                platform,
                profile: args.profile.clone(),
                capabilities: Vec::new(),
                extensions: Vec::new(),
            },
            build: None,
        };
        let body = project.to_toml()?;
        (project, "create", Some(body))
    };
    let project_version = project.effective_version(&root)?;

    let mut units = BTreeMap::new();
    units.insert("backend".into(), backend_target.to_string());
    units.insert("core".into(), args.core.clone());
    units.insert("extraction".into(), args.extraction.clone());
    let mut adapters = BTreeMap::new();
    adapters.insert("headless".into(), args.headless.clone());
    if let Some(hydra2) = &args.hydra2 {
        adapters.insert("hydra2".into(), hydra2.clone());
    }
    // The viewport hosts the project's own bootstrap scene, so it joins the
    // adapter map without becoming a scene input.
    if let Some(viewport) = &args.viewport {
        adapters.insert("viewport".into(), viewport.clone());
    }
    let mut scene_inputs = vec!["headless".into()];
    if args.hydra2.is_some() {
        scene_inputs.push("hydra2".into());
    }
    let manifest = RendererManifest {
        schema: RENDERER_SCHEMA.into(),
        renderer: RendererIdentity {
            name: args.name.clone(),
        },
        composition: RendererComposition {
            backend: backend_kind.to_string(),
            scene_inputs,
            units,
            adapters,
        },
        render_products: RenderProducts {
            required: vec!["color".into(), "depth".into()],
        },
        frame: FrameContract {
            contexts: 3,
            completion: "explicit".into(),
        },
        validation: RendererValidation {
            gpu_smoke: true,
            validation_messages_are_errors: true,
            assertions: renderer_assertions(),
        },
    };
    manifest.validate()?;
    let manifest_body = serde_yaml::to_string(&manifest)
        .map_err(|error| Error::parse(RENDERER_MANIFEST, anyhow::Error::new(error)))?;
    let manifest_path = root.join(RENDERER_MANIFEST);
    let renderer_action = if manifest_path.as_std_path().is_file() {
        let current = RendererManifest::load(&root)?;
        if current == manifest {
            "keep"
        } else {
            "replace"
        }
    } else {
        "create"
    };

    let labels: Vec<(String, String)> = manifest
        .composition
        .units
        .iter()
        .chain(manifest.composition.adapters.iter())
        .map(|(role, target)| (role.clone(), target.clone()))
        .collect();
    let resolution: Vec<serde_json::Value> = labels
        .iter()
        .map(|(role, target)| {
            serde_json::json!({
                "role": role,
                "target": target,
                "resolved": cmake_sources_contain(&root, target),
            })
        })
        .collect();
    let unresolved: Vec<String> = resolution
        .iter()
        .filter(|item| item["resolved"] == false)
        .filter_map(|item| item["target"].as_str().map(str::to_string))
        .collect();

    if args.write && !unresolved.is_empty() && !args.allow_unresolved {
        return Err(Error::precondition(format!(
            "adoption target labels were not found in CMake sources: {}",
            unresolved.join(", ")
        ))
        .with_hint("correct the mappings or pass --allow-unresolved to record them explicitly"));
    }
    if args.write && renderer_action == "replace" && !args.replace_manifest {
        return Err(Error::precondition(format!(
            "{RENDERER_MANIFEST} already differs from the adoption plan"
        ))
        .with_hint("review the dry run, then pass --replace-manifest --write"));
    }

    let adoption = serde_json::json!({
        "schema": "openstrata.renderer-adoption/v1",
        "mode": "adopted",
        "renderer": args.name,
        "project": {
            "name": project.project.name,
            "version": project_version,
            "version_source": project.project.version_file.as_deref().unwrap_or("openstrata.toml"),
        },
        "mapping": {
            "backend": backend_kind,
            "targets": resolution,
        },
        "unresolved": unresolved,
    });
    let adoption_body = serde_json::to_string_pretty(&adoption)
        .map_err(|error| Error::parse(RENDERER_ADOPTION_FILE, anyhow::Error::new(error)))?;

    if args.write {
        if let Some(body) = project_body {
            write_atomic(project_path.as_std_path(), format!("{body}\n").as_bytes())?;
        }
        if renderer_action != "keep" {
            write_atomic(manifest_path.as_std_path(), manifest_body.as_bytes())?;
        }
        let adoption_path = root.join(RENDERER_ADOPTION_FILE);
        write_atomic(
            adoption_path.as_std_path(),
            format!("{adoption_body}\n").as_bytes(),
        )?;
    }

    let data = serde_json::json!({
        "dry_run": !args.write,
        "root": root,
        "actions": {
            "openstrata.toml": project_action,
            "openstrata.renderer.yaml": renderer_action,
            "openstrata.renderer-adoption.json": if args.write { "write" } else { "would-write" },
        },
        "mapping": resolution,
        "unresolved": adoption["unresolved"],
    });
    if fmt.is_json() {
        output::success(&data);
    } else {
        println!(
            "Renderer adoption {} for {}",
            if args.write { "applied" } else { "dry run" },
            root
        );
        println!("  {PROJECT_MANIFEST}: {project_action}");
        println!("  {RENDERER_MANIFEST}: {renderer_action}");
        for item in data["mapping"].as_array().into_iter().flatten() {
            println!(
                "  {:<12} {:<32} {}",
                item["role"].as_str().unwrap_or_default(),
                item["target"].as_str().unwrap_or_default(),
                if item["resolved"] == true {
                    "resolved"
                } else {
                    "UNRESOLVED"
                }
            );
        }
        if !args.write {
            println!("\nReview the plan, then rerun with --write.");
        }
    }
    Ok(())
}

fn merge_reports(args: RendererMergeArgs, fmt: Format) -> Result<()> {
    let root = renderer_command_root()?;
    let base_path = rooted(&root, &args.base);
    let overlay_path = rooted(&root, &args.overlay);
    let out_path = rooted(&root, &args.out);
    let manifest = RendererManifest::load(&root)?;
    let base = RendererReport::load(&base_path)?;
    let overlay = RendererReport::load(&overlay_path)?;
    let merged = base.merge(&overlay, args.replace)?;
    merged.validate_against(&manifest)?;
    let body = serde_json::to_string_pretty(&merged)
        .map_err(|error| Error::parse(out_path.to_string(), anyhow::Error::new(error)))?;
    write_atomic(out_path.as_std_path(), format!("{body}\n").as_bytes())?;
    let producer = merged.producer.as_ref().map(|session| session.id.clone());
    if fmt.is_json() {
        output::success(&serde_json::json!({
            "merged": true,
            "base": base_path,
            "overlay": overlay_path,
            "out": out_path,
            "checks": merged.checks.len(),
            "producer": producer,
        }));
    } else {
        println!(
            "Merged {} + {} -> {} ({} checks)",
            base_path,
            overlay_path,
            out_path,
            merged.checks.len()
        );
        if let Some(producer) = &producer {
            println!("  owning producer session: {producer}");
        }
    }
    Ok(())
}

fn renderer_command_root() -> Result<Utf8PathBuf> {
    let cwd = std::env::current_dir().map_err(|error| Error::io(".", error))?;
    let root = find_project_root(&cwd).unwrap_or(cwd);
    Utf8PathBuf::from_path_buf(root)
        .map_err(|path| Error::config(format!("non-UTF-8 project path: {}", path.display())))
}

fn renderer_assertions() -> Vec<String> {
    [
        "renderer.core.boundary",
        "renderer.backend.capability",
        "renderer.gpu.frame",
        "renderer.validation.messages",
        "renderer.render_product.color",
        "renderer.render_product.depth",
        "renderer.frame.persistence",
        "renderer.install_tree",
        "renderer.plugin.discovery",
        "renderer.delegate.creation",
        "renderer.render_buffer.cpu",
        "renderer.host.first_frame",
        "renderer.host.stable_update",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn cmake_sources_contain(root: &Utf8Path, target: &str) -> bool {
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(dir.as_std_path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let Ok(path) = Utf8PathBuf::from_path_buf(entry.path()) else {
                continue;
            };
            if file_type.is_dir() {
                if !matches!(
                    path.file_name(),
                    Some(".git" | ".strata" | "build" | "target" | "dist")
                ) {
                    pending.push(path);
                }
                continue;
            }
            let is_cmake = path.file_name() == Some("CMakeLists.txt")
                || path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("cmake"));
            if is_cmake
                && std::fs::read_to_string(path.as_std_path())
                    .is_ok_and(|source| source.contains(target))
            {
                return true;
            }
        }
    }
    false
}

fn view(args: ViewArgs) -> Result<()> {
    // Renderer projects intentionally default to host-neutral `core`. For a
    // view, prefer the single already-pulled real Hydra-capable profile; this
    // makes an adopted `usd` runtime work without repeating `--profile` while
    // refusing to guess when several distinct choices exist.
    let (root, platform, project_profile) = resolve_selection(args.target.clone(), None)?;
    let profile = select_view_profile(&platform, args.profile, &project_profile)?;
    let manifest = RendererManifest::load(&root)?;
    let adapter = manifest.composition.adapters.get("hydra2").ok_or_else(|| {
        Error::config("renderer composition has no `hydra2` adapter in openstrata.renderer.yaml")
    })?;
    let (runtime, runtime_manifest) = require_real_runtime(&platform, &profile)?;

    let explicit_scene = args.scene.map(|scene| rooted(&root, &scene));
    if let Some(scene) = &explicit_scene {
        if !scene.as_std_path().is_file() {
            return Err(Error::precondition(format!(
                "USD scene does not exist: {scene}"
            )));
        }
    }

    let build_dir = match args.build_dir {
        Some(build_dir) => {
            let build_dir = rooted(&root, &build_dir);
            println!("==> using external Hydra build tree: {build_dir}");
            validate_hydra_build(
                &build_dir,
                &runtime.artifact_prefix,
                &runtime.runtime.id(),
                &runtime_manifest.digest,
            )?;
            build_dir
        }
        None => managed_hydra_build(
            &root,
            &platform,
            &profile,
            &args.config,
            args.generator,
            &runtime,
            &runtime_manifest.digest,
        )?,
    };

    let cmake = tools::which("cmake").ok_or_else(|| {
        Error::coded(
            "REQUIRED_TOOL_MISSING",
            Category::Precondition,
            "`cmake` not found on PATH",
        )
    })?;
    let preferred_stage = root
        .join(STATE_DIR)
        .join("renderer-view")
        .join(&manifest.renderer.name)
        .join(config_dir_name(&args.config));
    let staging = ost_core::fs::prepare_staging_dir(preferred_stage.as_std_path(), false)?;
    let fell_back = staging.fell_back(preferred_stage.as_std_path());
    let stage = Utf8PathBuf::from_path_buf(staging.path).map_err(|path| {
        Error::config(format!("non-UTF-8 renderer view stage: {}", path.display()))
    })?;
    if fell_back {
        eprintln!("warning: previous renderer view tree is still open; staging into '{stage}'");
    }

    println!(
        "==> installing Hydra view tree: {} ({})",
        build_dir, args.config
    );
    let mut install = Command::new(&cmake);
    install
        .arg("--install")
        .arg(build_dir.as_std_path())
        .args(["--config", &args.config, "--prefix"])
        .arg(stage.as_std_path());
    runtime.env.apply(&mut install);
    let status = install
        .status()
        .map_err(|error| Error::io(format!("run {}", cmake.display()), error))?;
    if !status.success() {
        return Err(Error::external_tool(format!(
            "CMake install for renderer view failed{}",
            exit_detail(&status)
        ))
        .with_phase("renderer-view-install"));
    }

    let plugin = find_renderer_plugin(&stage, adapter)?;
    let scene = match explicit_scene {
        Some(scene) => scene,
        None => find_named_file(&stage, "usdview-smoke.usda")?.ok_or_else(|| {
            Error::precondition(format!(
                "the installed renderer tree at '{stage}' has no usdview-smoke.usda"
            ))
            .with_hint("pass a scene explicitly: `ost renderer view path/to/scene.usda`")
        })?,
    };
    let usdview = locate_runtime_tool(&runtime, &["usdview.cmd", "usdview.exe", "usdview"])
        .ok_or_else(|| {
            Error::coded(
                "REQUIRED_TOOL_MISSING",
                Category::Precondition,
                "usdview not found in the selected real runtime",
            )
            .with_hint(format!(
                "adopt or build a `{profile}` runtime with imaging/usdview enabled"
            ))
        })?;

    let mut session = with_host_python_on_path(
        runtime.env.clone(),
        &runtime.artifact_prefix,
        &runtime.python_version,
        Host::detect().os,
    );
    // Last prepend wins priority in EnvSet, so the project renderer is selected
    // ahead of any same-named plugin already present in the base runtime.
    session.vars.push(EnvVar {
        key: "PXR_PLUGINPATH_NAME".into(),
        op: EnvOp::Prepend(portable_path(&plugin.resource_dir)),
    });

    let renderer = args.renderer.unwrap_or(plugin.display_name);
    let mut command = usdview_command(&runtime, &usdview)?;
    command
        .arg(scene.as_std_path())
        .args(["--renderer", &renderer]);

    // Camera selection is automatic. `--camera /Camera` used to be passed
    // unconditionally, so any scene without a prim at that exact path — most
    // scenes — opened on an error about a camera the author never claimed to
    // have. A camera is now named only when the scene actually contains one.
    let selection = select_camera(&scene, args.camera.as_deref());
    if let Some(camera) = &selection.camera {
        command.args(["--camera", camera]);
    }
    session.apply(&mut command);

    println!("==> usdview: renderer={renderer} scene={scene}");
    println!("==> camera: {}", selection.describe());
    let status = command
        .status()
        .map_err(|error| Error::io(format!("run {usdview}"), error))?;
    if !status.success() {
        return Err(Error::external_tool(format!(
            "usdview exited unsuccessfully{}",
            exit_detail(&status)
        ))
        .with_phase("renderer-view-host"));
    }
    Ok(())
}

fn viewport(args: ViewportArgs) -> Result<()> {
    let (root, platform, profile) = resolve_selection(args.target, args.profile)?;
    let manifest = RendererManifest::load(&root)?;
    let adapter = manifest
        .composition
        .adapters
        .get("viewport")
        .ok_or_else(|| {
            Error::config(
                "renderer composition has no `viewport` adapter in openstrata.renderer.yaml",
            )
            .with_hint(
                "declare `composition.adapters.viewport: <cmake-target>`, or adopt the \
                 existing target with `ost renderer adopt ... --viewport <target>`",
            )
        })?
        .clone();

    // One ordinary managed build with the viewport intent. The standalone
    // viewport renders the project bootstrap scene, so the project's
    // host-neutral profile suffices and no runtime fingerprint is recorded.
    let (target, _) = build_target(&platform, &profile)?;
    let build_dir = root.join("build").join(target.id());
    let mut cache = BTreeMap::new();
    cache.insert("OST_RENDERER_ADAPTERS".into(), "viewport".into());
    cache.insert("CMAKE_BUILD_TYPE".into(), args.config.clone());
    let mut intent = ost_build::BuildIntent {
        name: "renderer-viewport".into(),
        cache,
    };
    println!("==> preparing managed viewport build: {build_dir}");
    build::run_with_intent(
        BuildArgs::managed(
            platform.clone(),
            profile.clone(),
            args.generator.clone(),
            args.config.clone(),
        ),
        Format::Human,
        intent.clone(),
    )?;

    // Adopted projects may expose a project-specific viewport option without
    // consuming OST_RENDERER_ADAPTERS. Discover the one exact option from
    // CMake's cache and repeat through the same managed build service.
    let source = read_cmake_cache_for(
        &build_dir,
        "viewport",
        "renderer-viewport-preflight",
        "rerun `ost renderer viewport` to configure the managed viewport build",
    )?;
    let options = viewport_option_entries(&source);
    if !options.iter().any(|(_, enabled)| *enabled) {
        if let [(option, false)] = options.as_slice() {
            println!("==> adopted renderer mapping: enabling {option}");
            intent.cache.insert(option.clone(), "ON".into());
            build::run_with_intent(
                BuildArgs::managed(platform, profile, args.generator, args.config.clone()),
                Format::Human,
                intent,
            )?;
        }
    }

    let exe_name = if Host::detect().os == Os::Windows {
        format!("{adapter}.exe")
    } else {
        adapter.clone()
    };
    let executable = pick_built_executable(find_all_named_files(&build_dir, &exe_name)?, &args.config)
        .ok_or_else(|| {
            Error::precondition(format!(
                "the managed build did not produce viewport executable '{exe_name}' under '{build_dir}'"
            ))
            .with_hint(
                "ensure the project builds its viewport target when configured with \
                 OST_RENDERER_ADAPTERS=viewport",
            )
            .with_phase("renderer-viewport-discovery")
        })?;

    println!("==> viewport: {executable}");
    let status = Command::new(executable.as_std_path())
        .args(&args.args)
        .status()
        .map_err(|error| Error::io(format!("run {executable}"), error))?;
    match status.code() {
        Some(0) => Ok(()),
        // The viewport smoke contract: 77 means this environment cannot
        // present (no display, no Vulkan-capable device), not a failure.
        Some(77) => Err(Error::coded(
            "PRESENTATION_UNAVAILABLE",
            Category::Precondition,
            "the viewport reported that this environment cannot present",
        )
        .with_hint("run on a host with a display and a Vulkan 1.3 capable device")
        .with_phase("renderer-viewport-host")),
        _ => Err(Error::external_tool(format!(
            "the viewport exited unsuccessfully{}",
            exit_detail(&status)
        ))
        .with_phase("renderer-viewport-host")),
    }
}

// A multi-config generator nests executables under a per-config directory;
// prefer the requested configuration when several builds exist.
fn pick_built_executable(candidates: Vec<Utf8PathBuf>, config: &str) -> Option<Utf8PathBuf> {
    if candidates.len() > 1 {
        if let Some(matching) = candidates.iter().find(|path| {
            path.components()
                .any(|component| component.as_str().eq_ignore_ascii_case(config))
        }) {
            return Some(matching.clone());
        }
    }
    candidates.into_iter().next()
}

fn select_view_profile(
    platform: &str,
    explicit: Option<String>,
    project_profile: &str,
) -> Result<String> {
    if let Some(profile) = explicit {
        return Ok(profile);
    }

    let mut names = Vec::new();
    for name in [project_profile, "lookdev", "usd"] {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let mut available = Vec::new();
    for name in names {
        let Ok(candidate) = resolve(platform, name) else {
            continue;
        };
        if !candidate.pulled {
            continue;
        }
        let manifest = std::fs::read_to_string(candidate.prefix.join(MANIFEST_FILE).as_std_path())
            .ok()
            .and_then(|source| RuntimeManifest::from_json(&source).ok());
        if manifest.is_some_and(|manifest| manifest.source.is_real())
            && locate_runtime_tool(&candidate, &["usdview.cmd", "usdview.exe", "usdview"]).is_some()
        {
            available.push(name.to_string());
        }
    }
    match available.as_slice() {
        [profile] => Ok(profile.clone()),
        [] => Ok("lookdev".into()),
        profiles => Err(Error::precondition(format!(
            "multiple real usdview runtimes are available for {platform}: {}",
            profiles.join(", ")
        ))
        .with_hint("select the build/runtime identity explicitly with `--profile <profile>`")),
    }
}

fn managed_hydra_build(
    root: &Utf8Path,
    platform: &str,
    profile: &str,
    config: &str,
    generator: Option<String>,
    runtime: &Resolved,
    runtime_digest: &str,
) -> Result<Utf8PathBuf> {
    let (target, _) = build_target(platform, profile)?;
    let build_dir = root.join("build").join(target.id());
    let mut cache = BTreeMap::new();
    cache.insert("OST_RENDERER_ADAPTERS".into(), "hydra2".into());
    cache.insert(
        "OST_RUNTIME_ROOT".into(),
        portable_path(&runtime.artifact_prefix),
    );
    cache.insert("OST_RUNTIME_ID".into(), runtime.runtime.id());
    cache.insert("OST_RUNTIME_DIGEST".into(), runtime_digest.into());
    cache.insert("CMAKE_BUILD_TYPE".into(), config.into());
    let mut intent = ost_build::BuildIntent {
        name: "renderer-hydra2".into(),
        cache,
    };

    println!("==> preparing managed Hydra build: {build_dir}");
    build::run_with_intent(
        BuildArgs::managed(
            platform.to_string(),
            profile.to_string(),
            generator.clone(),
            config.to_string(),
        ),
        Format::Human,
        intent.clone(),
    )?;

    // Adopted v0.16 projects may expose the established *_ENABLE_HYDRA2 cache
    // option but not yet consume OST_RENDERER_ADAPTERS. Discover the one exact
    // option from CMake's own cache and repeat through the same build service;
    // new/generated projects take the one-pass standard-intent path above.
    let source = read_cmake_cache(&build_dir)?;
    let options = hydra_option_entries(&source);
    if !options.iter().any(|(_, enabled)| *enabled) {
        if let [(option, false)] = options.as_slice() {
            println!("==> adopted renderer mapping: enabling {option}");
            intent.cache.insert(option.clone(), "ON".into());
            build::run_with_intent(
                BuildArgs::managed(
                    platform.to_string(),
                    profile.to_string(),
                    generator,
                    config.to_string(),
                ),
                Format::Human,
                intent,
            )?;
        }
    }

    validate_hydra_build(
        &build_dir,
        &runtime.artifact_prefix,
        &runtime.runtime.id(),
        runtime_digest,
    )?;
    Ok(build_dir)
}

fn rooted(root: &Utf8Path, path: &Utf8Path) -> Utf8PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn config_dir_name(config: &str) -> String {
    let normalized = config
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    if normalized.is_empty() {
        "default".into()
    } else {
        normalized
    }
}

fn read_cmake_cache(build_dir: &Utf8Path) -> Result<String> {
    read_cmake_cache_for(
        build_dir,
        "Hydra",
        "renderer-view-preflight",
        "omit `--build-dir` for an OST-managed build, or configure and build the \
         external optional adapter first",
    )
}

fn read_cmake_cache_for(
    build_dir: &Utf8Path,
    adapter: &str,
    phase: &str,
    hint: &str,
) -> Result<String> {
    let cache = build_dir.join("CMakeCache.txt");
    std::fs::read_to_string(cache.as_std_path()).map_err(|_| {
        Error::precondition(format!(
            "{adapter} build tree not configured at '{build_dir}'"
        ))
        .with_hint(hint)
        .with_phase(phase)
    })
}

fn hydra_option_entries(source: &str) -> Vec<(String, bool)> {
    adapter_option_entries(source, "_ENABLE_HYDRA2")
}

fn viewport_option_entries(source: &str) -> Vec<(String, bool)> {
    adapter_option_entries(source, "_ENABLE_VIEWPORT")
}

fn adapter_option_entries(source: &str, suffix: &str) -> Vec<(String, bool)> {
    source
        .lines()
        .filter_map(|line| {
            let (entry, value) = line.split_once('=')?;
            let name = entry.split_once(':').map_or(entry, |(name, _)| name);
            name.to_ascii_uppercase()
                .ends_with(suffix)
                .then(|| (name.to_string(), cmake_cache_truthy(value)))
        })
        .collect()
}

fn validate_hydra_build(
    build_dir: &Utf8Path,
    runtime_root: &Utf8Path,
    runtime_id: &str,
    runtime_digest: &str,
) -> Result<()> {
    let source = read_cmake_cache(build_dir)?;
    // A `-D<RENDERER>_ENABLE_HYDRA2=YES` configure stores an UNINITIALIZED
    // cache entry, so accept any entry type and the CMake truthy value set.
    let enabled = hydra_option_entries(&source)
        .iter()
        .any(|(_, enabled)| *enabled);
    if !enabled {
        return Err(Error::precondition(format!(
            "CMake build tree '{build_dir}' does not enable the Hydra 2 adapter"
        ))
        .with_hint("reconfigure it with `-D<RENDERER>_ENABLE_HYDRA2=ON`")
        .with_phase("renderer-view-preflight"));
    }
    if let Some(recorded_id) = cache_path(&source, "OST_RUNTIME_ID") {
        if recorded_id != runtime_id {
            return Err(Error::coded(
                "RUNTIME_BUILD_MISMATCH",
                Category::Precondition,
                format!(
                    "Hydra build records runtime '{recorded_id}', but the selected runtime is '{}'",
                    runtime_id
                ),
            )
            .with_hint(
                "select the runtime used for this build with `--target/--profile`, or \
                 reconfigure the Hydra build against that runtime",
            )
            .with_phase("renderer-view-preflight"));
        }
    }
    if let Some(recorded_digest) = cache_path(&source, "OST_RUNTIME_DIGEST") {
        if recorded_digest != runtime_digest {
            return Err(Error::coded(
                "RUNTIME_BUILD_MISMATCH",
                Category::Precondition,
                format!(
                    "Hydra build records runtime digest '{recorded_digest}', but the selected runtime digest is '{runtime_digest}'"
                ),
            )
            .with_hint(
                "select the exact runtime used for this build, or reconfigure the Hydra build against the selected runtime",
            )
            .with_phase("renderer-view-preflight"));
        }
    }

    let mut roots = Vec::new();
    for key in ["OST_RUNTIME_ROOT", "pxr_DIR", "PXR_DIR", "OpenUSD_DIR"] {
        if let Some(value) = cache_path(&source, key) {
            if !value.ends_with("-NOTFOUND") && !value.trim().is_empty() {
                roots.push(Utf8PathBuf::from(value));
            }
        }
    }
    if let Some(prefixes) = cache_path(&source, "CMAKE_PREFIX_PATH") {
        roots.extend(
            prefixes
                .split(';')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(Utf8PathBuf::from),
        );
    }
    if roots.is_empty() {
        return Err(Error::coded(
            "RUNTIME_BUILD_FINGERPRINT_MISSING",
            Category::Precondition,
            format!("Hydra build tree '{build_dir}' does not record how OpenUSD was discovered"),
        )
        .with_hint(
            "omit `--build-dir` for a fingerprinted managed build, or reconfigure the \
             external tree with CMAKE_PREFIX_PATH/pxr_DIR pointing at the selected runtime",
        )
        .with_phase("renderer-view-preflight"));
    }
    if !roots
        .iter()
        .any(|candidate| path_is_within(candidate, runtime_root))
    {
        return Err(Error::coded(
            "RUNTIME_BUILD_MISMATCH",
            Category::Precondition,
            format!(
                "Hydra build OpenUSD roots ({}) do not match profile runtime root '{runtime_root}'",
                roots
                    .iter()
                    .map(|path| path.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
        .with_hint(
            "select the runtime used for this build with `--target/--profile`, or \
             reconfigure the Hydra build against that runtime",
        )
        .with_phase("renderer-view-preflight"));
    }
    Ok(())
}

fn cmake_cache_truthy(value: &str) -> bool {
    let value = value.trim();
    value.eq_ignore_ascii_case("on")
        || value.eq_ignore_ascii_case("true")
        || value.eq_ignore_ascii_case("yes")
        || value.eq_ignore_ascii_case("y")
        || value.parse::<i64>().is_ok_and(|number| number != 0)
}

fn cache_path<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    source.lines().find_map(|line| {
        let (field, value) = line.split_once('=')?;
        let name = field.split_once(':').map_or(field, |(name, _)| name);
        name.eq_ignore_ascii_case(key).then_some(value.trim())
    })
}

fn path_is_within(candidate: &Utf8Path, root: &Utf8Path) -> bool {
    let canonical = |path: &Utf8Path| {
        std::fs::canonicalize(path.as_std_path())
            .ok()
            .and_then(|path| Utf8PathBuf::from_path_buf(path).ok())
    };
    let candidate = canonical(candidate).unwrap_or_else(|| candidate.to_path_buf());
    let root = canonical(root).unwrap_or_else(|| root.to_path_buf());
    let candidate = portable_path(&candidate)
        .trim_end_matches('/')
        .to_ascii_lowercase();
    let root = portable_path(&root)
        .trim_end_matches('/')
        .to_ascii_lowercase();
    candidate == root || candidate.starts_with(&format!("{root}/"))
}

struct RendererPlugin {
    resource_dir: Utf8PathBuf,
    display_name: String,
}

fn find_renderer_plugin(stage: &Utf8Path, adapter: &str) -> Result<RendererPlugin> {
    let manifests = find_all_named_files(stage, "plugInfo.json")?;
    for path in manifests {
        let source = std::fs::read_to_string(path.as_std_path())
            .map_err(|error| Error::io(path.to_string(), error))?;
        let value: Value = serde_json::from_str(&source)
            .map_err(|error| Error::parse(path.to_string(), anyhow::Error::new(error)))?;
        let Some(plugins) = value.get("Plugins").and_then(Value::as_array) else {
            continue;
        };
        for plugin in plugins {
            if plugin.get("Name").and_then(Value::as_str) != Some(adapter) {
                continue;
            }
            let Some(types) = plugin.pointer("/Info/Types").and_then(Value::as_object) else {
                continue;
            };
            for type_info in types.values() {
                let is_renderer = type_info
                    .get("bases")
                    .and_then(Value::as_array)
                    .is_some_and(|bases| {
                        bases
                            .iter()
                            .any(|base| base.as_str() == Some("HdRendererPlugin"))
                    });
                if !is_renderer {
                    continue;
                }
                let display_name = type_info
                    .get("displayName")
                    .and_then(Value::as_str)
                    .filter(|name| !name.trim().is_empty())
                    .ok_or_else(|| {
                        Error::config(format!(
                            "renderer plugin '{adapter}' has no displayName in {path}"
                        ))
                    })?;
                let resource_dir = path.parent().ok_or_else(|| {
                    Error::config(format!("plugin manifest has no parent directory: {path}"))
                })?;
                return Ok(RendererPlugin {
                    resource_dir: resource_dir.to_path_buf(),
                    display_name: display_name.to_string(),
                });
            }
        }
    }
    Err(Error::precondition(format!(
        "installed tree '{stage}' does not contain Hydra renderer plugin '{adapter}'"
    ))
    .with_hint("build the adapter, then rerun `ost renderer view`")
    .with_phase("renderer-view-discovery"))
}

fn find_named_file(root: &Utf8Path, name: &str) -> Result<Option<Utf8PathBuf>> {
    Ok(find_all_named_files(root, name)?.into_iter().next())
}

fn find_all_named_files(root: &Utf8Path, name: &str) -> Result<Vec<Utf8PathBuf>> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = std::fs::read_dir(dir.as_std_path())
            .map_err(|error| Error::io(dir.to_string(), error))?;
        for entry in entries {
            let entry = entry.map_err(|error| Error::io(dir.to_string(), error))?;
            let ty = entry
                .file_type()
                .map_err(|error| Error::io(entry.path().display().to_string(), error))?;
            let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|path| {
                Error::config(format!(
                    "non-UTF-8 path under renderer stage: {}",
                    path.display()
                ))
            })?;
            if ty.is_dir() {
                pending.push(path);
            } else if ty.is_file() && path.file_name() == Some(name) {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

fn require_real_runtime(platform: &str, profile: &str) -> Result<(Resolved, RuntimeManifest)> {
    let resolved = resolve(platform, profile)?;
    if !resolved.pulled {
        return Err(Error::coded(
            "RUNTIME_NOT_FOUND",
            Category::Precondition,
            format!("runtime '{}' not pulled", resolved.runtime.id()),
        )
        .with_hint(format!(
            "adopt one with `ost runtime pull {platform} --profile {profile} --from-usd <path>`"
        )));
    }
    let manifest_path = resolved.prefix.join(MANIFEST_FILE);
    let source = std::fs::read_to_string(manifest_path.as_std_path())
        .map_err(|error| Error::io(manifest_path.to_string(), error))?;
    let manifest = RuntimeManifest::from_json(&source)
        .map_err(|error| Error::parse(manifest_path.to_string(), anyhow::Error::new(error)))?;
    if !manifest.source.is_real() {
        return Err(Error::coded(
            "REAL_RUNTIME_REQUIRED",
            Category::Precondition,
            "runtime is mock; usdview needs a real OpenUSD runtime",
        )
        .with_hint(format!(
            "adopt one with `ost runtime pull {platform} --profile {profile} --from-usd <path>`"
        )));
    }
    Ok((resolved, manifest))
}

fn locate_runtime_tool(runtime: &Resolved, names: &[&str]) -> Option<Utf8PathBuf> {
    let bin = runtime.artifact_prefix.join("bin");
    names.iter().find_map(|name| {
        let path = bin.join(name);
        path.as_std_path().is_file().then_some(path)
    })
}

fn usdview_command(runtime: &Resolved, usdview: &Utf8Path) -> Result<Command> {
    let extension = usdview.extension().unwrap_or_default().to_ascii_lowercase();
    if Host::detect().os != Os::Windows || matches!(extension.as_str(), "exe" | "cmd" | "bat") {
        return Ok(Command::new(usdview.as_std_path()));
    }

    // Some Windows OpenUSD installs ship usdview as an extensionless Python
    // script rather than a .cmd wrapper. Launch that through the interpreter
    // matching the adopted runtime instead of relying on file associations.
    let python = ost_build::resolve_for_runtime(&runtime.artifact_prefix, &runtime.python_version)
        .ok_or_else(|| {
            Error::coded(
                "REQUIRED_TOOL_MISSING",
                Category::Precondition,
                "a Python interpreter matching the OpenUSD runtime was not found",
            )
        })?;
    let mut command = Command::new(&python.executable);
    command.arg(usdview.as_std_path());
    Ok(command)
}

/// Which camera the view will use, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CameraSelection {
    /// `None` selects usdview's free camera.
    camera: Option<String>,
    reason: CameraReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CameraReason {
    /// The caller named a camera and the scene has it.
    Requested,
    /// The caller named one the scene does not define.
    RequestedMissing(String),
    /// Automatically selected the scene's only camera.
    Discovered,
    /// The scene defines no camera.
    NoneInScene,
    /// The scene could not be inspected (binary/compressed layer, unreadable).
    Unknown,
}

impl CameraSelection {
    fn describe(&self) -> String {
        match (&self.camera, &self.reason) {
            (Some(camera), CameraReason::Requested) => format!("{camera} (requested)"),
            (Some(camera), CameraReason::Discovered) => format!("{camera} (found in scene)"),
            (None, CameraReason::RequestedMissing(requested)) => format!(
                "free camera — the scene defines no '{requested}', so the request was not honored"
            ),
            (None, CameraReason::NoneInScene) => "free camera — the scene defines none".into(),
            (None, CameraReason::Unknown) => {
                "free camera — the scene could not be inspected for cameras".into()
            }
            // A camera with no supporting reason would be exactly the silent
            // guess this function exists to remove.
            (camera, reason) => format!("{camera:?} ({reason:?})"),
        }
    }
}

/// Choose the camera to view through, honoring a request only when the scene
/// backs it up.
///
/// Inspection is textual and deliberately conservative. A crosswalk through the
/// runtime's Python would be authoritative but would also make opening a scene
/// depend on a working `pxr` import, and a camera *hint* must never be the thing
/// that stops a view from opening. So: a definite answer where the layer is
/// readable text, and the free camera — reported as such — everywhere else.
fn select_camera(scene: &Utf8Path, requested: Option<&str>) -> CameraSelection {
    let cameras = scene_cameras(scene);

    match (requested, cameras) {
        // Nothing readable to check against: honor the request rather than
        // second-guessing a caller who knows the scene better than we do.
        (Some(requested), None) => CameraSelection {
            camera: Some(requested.to_string()),
            reason: CameraReason::Requested,
        },
        (Some(requested), Some(cameras)) => {
            let found = cameras.iter().any(|camera| {
                camera == requested || camera.rsplit('/').next() == requested.rsplit('/').next()
            });
            if found {
                CameraSelection {
                    camera: Some(requested.to_string()),
                    reason: CameraReason::Requested,
                }
            } else {
                CameraSelection {
                    camera: None,
                    reason: CameraReason::RequestedMissing(requested.to_string()),
                }
            }
        }
        (None, Some(cameras)) => match cameras.first() {
            Some(camera) => CameraSelection {
                camera: Some(camera.clone()),
                reason: CameraReason::Discovered,
            },
            None => CameraSelection {
                camera: None,
                reason: CameraReason::NoneInScene,
            },
        },
        (None, None) => CameraSelection {
            camera: None,
            reason: CameraReason::Unknown,
        },
    }
}

/// Camera prim names declared in a readable USD text layer.
///
/// `None` means "could not tell" — a binary `.usdc`, a `.usdz` package, or an
/// unreadable file — which is a different answer from `Some(vec![])`, "there are
/// definitely none". The two lead to different reported reasons.
fn scene_cameras(scene: &Utf8Path) -> Option<Vec<String>> {
    let extension = scene.extension().unwrap_or_default().to_ascii_lowercase();
    if !matches!(extension.as_str(), "usda" | "usd") {
        return None;
    }
    let text = std::fs::read_to_string(scene.as_std_path()).ok()?;
    // A `.usd` layer may be binary despite the extension; crossing into it would
    // produce nonsense matches, so treat it as un-inspectable.
    if text.contains('\0') {
        return None;
    }

    let mut cameras = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("def Camera ") else {
            continue;
        };
        // `def Camera "main" ( ... ) {`
        let Some(open) = rest.find('"') else { continue };
        let Some(close) = rest[open + 1..].find('"') else {
            continue;
        };
        let name = &rest[open + 1..open + 1 + close];
        if !name.is_empty() {
            cameras.push(format!("/{name}"));
        }
    }
    Some(cameras)
}

fn portable_path(path: &Utf8Path) -> String {
    path.to_string().replace('\\', "/")
}

fn exit_detail(status: &std::process::ExitStatus) -> String {
    status
        .code()
        .map(|code| format!(" (exit {code})"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> Utf8PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("ost-renderer-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Utf8PathBuf::from_path_buf(path).unwrap()
    }

    #[test]
    fn locates_matching_installed_hydra_renderer_metadata() {
        let stage = temp_dir("plugin");
        let resources = stage.join("lib/usd/hdSampleRenderer/resources");
        std::fs::create_dir_all(resources.as_std_path()).unwrap();
        std::fs::write(
            resources.join("plugInfo.json").as_std_path(),
            r#"{
              "Plugins": [{
                "Name": "hdSampleRenderer",
                "Info": {"Types": {
                  "HdSampleRendererPlugin": {
                    "bases": ["HdRendererPlugin"],
                    "displayName": "SampleRenderer"
                  }
                }}
              }]
            }"#,
        )
        .unwrap();

        let plugin = find_renderer_plugin(&stage, "hdSampleRenderer").unwrap();
        assert_eq!(plugin.resource_dir, resources);
        assert_eq!(plugin.display_name, "SampleRenderer");
        std::fs::remove_dir_all(stage.as_std_path()).unwrap();
    }

    #[test]
    fn hydra_build_preflight_requires_enabled_cache_entry() {
        let build = temp_dir("cache");
        let runtime = temp_dir("runtime");
        let pxr_dir = runtime.join("lib/cmake/pxr");
        std::fs::create_dir_all(pxr_dir.as_std_path()).unwrap();
        std::fs::write(
            build.join("CMakeCache.txt").as_std_path(),
            format!(
                "SAMPLE_RENDERER_ENABLE_HYDRA2:BOOL=ON\npxr_DIR:PATH={}\n",
                portable_path(&pxr_dir)
            ),
        )
        .unwrap();
        assert!(validate_hydra_build(&build, &runtime, "runtime", "sha256:runtime").is_ok());

        // A plain `-D` configure stores UNINITIALIZED entries with any CMake
        // truthy spelling; the advanced marker must never count as enabled.
        std::fs::write(
            build.join("CMakeCache.txt").as_std_path(),
            format!(
                "SAMPLE_RENDERER_ENABLE_HYDRA2:UNINITIALIZED=YES\nOST_RUNTIME_ROOT:UNINITIALIZED={}\n",
                portable_path(&runtime)
            ),
        )
        .unwrap();
        assert!(validate_hydra_build(&build, &runtime, "runtime", "sha256:runtime").is_ok());

        std::fs::write(
            build.join("CMakeCache.txt").as_std_path(),
            "SAMPLE_RENDERER_ENABLE_HYDRA2:BOOL=OFF\nSAMPLE_RENDERER_ENABLE_HYDRA2-ADVANCED:INTERNAL=1\n",
        )
        .unwrap();
        assert!(validate_hydra_build(&build, &runtime, "runtime", "sha256:runtime").is_err());
        std::fs::remove_dir_all(build.as_std_path()).unwrap();
        std::fs::remove_dir_all(runtime.as_std_path()).unwrap();
    }

    #[test]
    fn viewport_option_entries_accept_cmake_cache_truthy_values() {
        let source = concat!(
            "SAMPLE_RENDERER_ENABLE_VIEWPORT:BOOL=ON\n",
            "SAMPLE_RENDERER_ENABLE_VIEWPORT-ADVANCED:INTERNAL=1\n",
            "OTHER_ENABLE_HYDRA2:BOOL=ON\n",
        );
        assert_eq!(
            viewport_option_entries(source),
            vec![("SAMPLE_RENDERER_ENABLE_VIEWPORT".into(), true)]
        );
    }

    #[test]
    fn hydra_build_preflight_rejects_another_openusd_runtime() {
        let build = temp_dir("mismatch-build");
        let runtime = temp_dir("mismatch-runtime");
        let other = temp_dir("mismatch-other");
        std::fs::write(
            build.join("CMakeCache.txt").as_std_path(),
            format!(
                "SAMPLE_RENDERER_ENABLE_HYDRA2:BOOL=ON\npxr_DIR:PATH={}\n",
                portable_path(&other.join("lib/cmake/pxr"))
            ),
        )
        .unwrap();

        let error =
            validate_hydra_build(&build, &runtime, "runtime", "sha256:runtime").unwrap_err();
        assert_eq!(error.code(), "RUNTIME_BUILD_MISMATCH");
        std::fs::remove_dir_all(build.as_std_path()).unwrap();
        std::fs::remove_dir_all(runtime.as_std_path()).unwrap();
        std::fs::remove_dir_all(other.as_std_path()).unwrap();
    }

    #[test]
    fn hydra_build_preflight_rejects_a_stale_runtime_digest() {
        let build = temp_dir("digest-build");
        let runtime = temp_dir("digest-runtime");
        std::fs::write(
            build.join("CMakeCache.txt").as_std_path(),
            format!(
                "SAMPLE_RENDERER_ENABLE_HYDRA2:BOOL=ON\nOST_RUNTIME_ROOT:PATH={}\nOST_RUNTIME_ID:STRING=runtime\nOST_RUNTIME_DIGEST:STRING=sha256:old\n",
                portable_path(&runtime)
            ),
        )
        .unwrap();

        let error = validate_hydra_build(&build, &runtime, "runtime", "sha256:new").unwrap_err();
        assert_eq!(error.code(), "RUNTIME_BUILD_MISMATCH");
        assert!(error.to_string().contains("sha256:old"));
        assert!(error.to_string().contains("sha256:new"));
        std::fs::remove_dir_all(build.as_std_path()).unwrap();
        std::fs::remove_dir_all(runtime.as_std_path()).unwrap();
    }

    #[test]
    fn relative_view_paths_are_project_relative() {
        let root = Utf8Path::new("/project");
        assert_eq!(
            rooted(root, Utf8Path::new("out-hydra")),
            root.join("out-hydra")
        );
        assert_eq!(config_dir_name("Rel With Deb Info"), "rel-with-deb-info");
    }
}

#[cfg(test)]
mod camera_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scene(tag: &str, name: &str, body: &str) -> Utf8PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ost-camera-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = Utf8PathBuf::from_path_buf(dir.join(name)).unwrap();
        std::fs::write(path.as_std_path(), body).unwrap();
        path
    }

    const WITH_CAMERA: &str = r#"#usda 1.0
def Xform "root" {
    def Camera "shotCam" {
        float focalLength = 35
    }
}
"#;

    const WITHOUT_CAMERA: &str = r#"#usda 1.0
def Xform "root" {
    def Mesh "body" {}
}
"#;

    /// The regression: `--camera /Camera` was passed unconditionally, so a scene
    /// that never declared that prim opened on an error instead of a view.
    #[test]
    fn a_scene_without_cameras_selects_the_free_camera() {
        let path = scene("none", "scene.usda", WITHOUT_CAMERA);
        let selection = select_camera(&path, None);
        assert_eq!(selection.camera, None);
        assert_eq!(selection.reason, CameraReason::NoneInScene);
        assert!(selection.describe().contains("free camera"));
        std::fs::remove_dir_all(path.parent().unwrap().as_std_path()).ok();
    }

    #[test]
    fn a_scenes_only_camera_is_selected_automatically() {
        let path = scene("one", "scene.usda", WITH_CAMERA);
        let selection = select_camera(&path, None);
        assert_eq!(selection.camera.as_deref(), Some("/shotCam"));
        assert_eq!(selection.reason, CameraReason::Discovered);
        std::fs::remove_dir_all(path.parent().unwrap().as_std_path()).ok();
    }

    #[test]
    fn a_requested_camera_present_in_the_scene_is_honored() {
        let path = scene("req", "scene.usda", WITH_CAMERA);
        let selection = select_camera(&path, Some("/shotCam"));
        assert_eq!(selection.camera.as_deref(), Some("/shotCam"));
        assert_eq!(selection.reason, CameraReason::Requested);
        std::fs::remove_dir_all(path.parent().unwrap().as_std_path()).ok();
    }

    /// A request the scene cannot satisfy falls back to the free camera and says
    /// so, rather than handing usdview a prim path that does not resolve.
    #[test]
    fn a_requested_camera_missing_from_the_scene_falls_back_and_reports_why() {
        let path = scene("missing", "scene.usda", WITH_CAMERA);
        let selection = select_camera(&path, Some("/Camera"));
        assert_eq!(selection.camera, None);
        assert_eq!(
            selection.reason,
            CameraReason::RequestedMissing("/Camera".into())
        );
        let described = selection.describe();
        assert!(
            described.contains("free camera") && described.contains("/Camera"),
            "{described}"
        );
        std::fs::remove_dir_all(path.parent().unwrap().as_std_path()).ok();
    }

    /// "Cannot tell" is not "there are none": an un-inspectable layer must not
    /// silently discard an explicit request the caller knows to be right.
    #[test]
    fn an_uninspectable_scene_honors_an_explicit_request() {
        let path = scene("binary", "scene.usdc", "PXR-USDC\0binary");
        assert_eq!(scene_cameras(&path), None);

        let requested = select_camera(&path, Some("/shotCam"));
        assert_eq!(requested.camera.as_deref(), Some("/shotCam"));

        // With no request there is nothing to go on, and that is reported
        // distinctly from a scene known to have no cameras.
        let automatic = select_camera(&path, None);
        assert_eq!(automatic.camera, None);
        assert_eq!(automatic.reason, CameraReason::Unknown);
        assert!(automatic.describe().contains("could not be inspected"));
        std::fs::remove_dir_all(path.parent().unwrap().as_std_path()).ok();
    }

    /// A `.usd` layer is text or binary depending on how it was written.
    #[test]
    fn a_binary_usd_layer_is_not_scanned_as_text() {
        let path = scene("binusd", "scene.usd", "PXR-USDC\0def Camera \"ghost\"");
        assert_eq!(scene_cameras(&path), None);
        std::fs::remove_dir_all(path.parent().unwrap().as_std_path()).ok();
    }
}
