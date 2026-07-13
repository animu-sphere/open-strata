// SPDX-License-Identifier: Apache-2.0
//! `ost renderer` — renderer-project developer workflows.
//!
//! The renderer remains one ordinary CMake project. This command does not add
//! another build/package lifecycle; it bridges an already-built optional Hydra
//! adapter into the matching OpenUSD runtime session for interactive usdview.

use std::process::Command;

use camino::{Utf8Path, Utf8PathBuf};
use clap::Subcommand;
use serde_json::Value;

use ost_core::host::Os;
use ost_core::paths::STATE_DIR;
use ost_core::{tools, Category, Error, Host, Result};
use ost_manifest::RendererManifest;
use ost_runtime::{EnvOp, EnvVar, RuntimeManifest, MANIFEST_FILE};

use crate::commands::configure::resolve_selection;
use crate::commands::{resolve, with_host_python_on_path, Resolved};

#[derive(Debug, Subcommand)]
pub enum RendererCmd {
    /// Open a scene in usdview with the built Hydra renderer selected.
    View {
        /// USD scene to open. Defaults to the installed usdview smoke scene.
        scene: Option<Utf8PathBuf>,

        /// Hydra-enabled CMake build tree, relative to the project root.
        #[arg(long, default_value = "out-hydra")]
        build_dir: Utf8PathBuf,

        /// CMake configuration to install and inspect.
        #[arg(long, default_value = "Release")]
        config: String,

        /// Platform target, e.g. `cy2026`. Defaults to the project's platform.
        #[arg(long)]
        target: Option<String>,

        /// Runtime profile. Defaults to `lookdev` for Hydra/usdview capability.
        #[arg(long)]
        profile: Option<String>,

        /// Camera prim passed to usdview.
        #[arg(long, default_value = "/Camera")]
        camera: String,

        /// Override the renderer display name read from installed plugInfo.json.
        #[arg(long)]
        renderer: Option<String>,
    },
}

pub fn run(cmd: RendererCmd) -> Result<()> {
    match cmd {
        RendererCmd::View {
            scene,
            build_dir,
            config,
            target,
            profile,
            camera,
            renderer,
        } => view(ViewArgs {
            scene,
            build_dir,
            config,
            target,
            profile,
            camera,
            renderer,
        }),
    }
}

struct ViewArgs {
    scene: Option<Utf8PathBuf>,
    build_dir: Utf8PathBuf,
    config: String,
    target: Option<String>,
    profile: Option<String>,
    camera: String,
    renderer: Option<String>,
}

fn view(args: ViewArgs) -> Result<()> {
    // Renderer projects intentionally default to the host-neutral `core`
    // profile. Interactive Hydra inspection is the exception: it requires an
    // imaging/usdview runtime, so use the existing `lookdev` capability profile
    // unless explicitly replaced (an adopted full SDK may also use `usd`).
    let (root, platform, profile) = resolve_selection(
        args.target,
        Some(args.profile.unwrap_or_else(|| "lookdev".into())),
    )?;
    let manifest = RendererManifest::load(&root)?;
    let adapter = manifest.composition.adapters.get("hydra2").ok_or_else(|| {
        Error::config("renderer composition has no `hydra2` adapter in openstrata.renderer.yaml")
    })?;
    let runtime = require_real_runtime(&platform, &profile)?;

    let build_dir = rooted(&root, &args.build_dir);
    validate_hydra_build(&build_dir, &runtime.artifact_prefix)?;
    let explicit_scene = args.scene.map(|scene| rooted(&root, &scene));
    if let Some(scene) = &explicit_scene {
        if !scene.as_std_path().is_file() {
            return Err(Error::precondition(format!(
                "USD scene does not exist: {scene}"
            )));
        }
    }

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
        .args(["--renderer", &renderer, "--camera", &args.camera]);
    session.apply(&mut command);

    println!("==> usdview: renderer={renderer} scene={scene}");
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

fn validate_hydra_build(build_dir: &Utf8Path, runtime_root: &Utf8Path) -> Result<()> {
    let cache = build_dir.join("CMakeCache.txt");
    let source = std::fs::read_to_string(cache.as_std_path()).map_err(|_| {
        Error::precondition(format!("Hydra build tree not configured at '{build_dir}'"))
            .with_hint(
                "configure and build the optional adapter first (see the generated README), \
             or pass its tree with `--build-dir`",
            )
            .with_phase("renderer-view-preflight")
    })?;
    let enabled = source.lines().any(|line| {
        let upper = line.to_ascii_uppercase();
        upper.contains("_ENABLE_HYDRA2:BOOL=")
            && (upper.ends_with("=ON") || upper.ends_with("=TRUE") || upper.ends_with("=1"))
    });
    if !enabled {
        return Err(Error::precondition(format!(
            "CMake build tree '{build_dir}' does not enable the Hydra 2 adapter"
        ))
        .with_hint("reconfigure it with `-D<RENDERER>_ENABLE_HYDRA2=ON`")
        .with_phase("renderer-view-preflight"));
    }
    if let Some(pxr_dir) = cache_path(&source, "pxr_DIR") {
        let pxr_dir = Utf8PathBuf::from(pxr_dir);
        if pxr_dir.as_str() != "pxr_DIR-NOTFOUND" && !path_is_within(&pxr_dir, runtime_root) {
            return Err(Error::coded(
                "RUNTIME_BUILD_MISMATCH",
                Category::Precondition,
                format!(
                    "Hydra build uses OpenUSD at '{pxr_dir}', but profile runtime root is \
                     '{runtime_root}'"
                ),
            )
            .with_hint(
                "select the runtime used for this build with `--target/--profile`, or \
                 reconfigure the Hydra build against that runtime",
            )
            .with_phase("renderer-view-preflight"));
        }
    }
    Ok(())
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

fn require_real_runtime(platform: &str, profile: &str) -> Result<Resolved> {
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
    let manifest = std::fs::read_to_string(resolved.prefix.join(MANIFEST_FILE).as_std_path())
        .ok()
        .and_then(|source| RuntimeManifest::from_json(&source).ok());
    if !manifest.is_some_and(|manifest| manifest.source.is_real()) {
        return Err(Error::coded(
            "REAL_RUNTIME_REQUIRED",
            Category::Precondition,
            "runtime is mock; usdview needs a real OpenUSD runtime",
        )
        .with_hint(format!(
            "adopt one with `ost runtime pull {platform} --profile {profile} --from-usd <path>`"
        )));
    }
    Ok(resolved)
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
        assert!(validate_hydra_build(&build, &runtime).is_ok());

        std::fs::write(
            build.join("CMakeCache.txt").as_std_path(),
            "SAMPLE_RENDERER_ENABLE_HYDRA2:BOOL=OFF\n",
        )
        .unwrap();
        assert!(validate_hydra_build(&build, &runtime).is_err());
        std::fs::remove_dir_all(build.as_std_path()).unwrap();
        std::fs::remove_dir_all(runtime.as_std_path()).unwrap();
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

        let error = validate_hydra_build(&build, &runtime).unwrap_err();
        assert_eq!(error.code(), "RUNTIME_BUILD_MISMATCH");
        std::fs::remove_dir_all(build.as_std_path()).unwrap();
        std::fs::remove_dir_all(runtime.as_std_path()).unwrap();
        std::fs::remove_dir_all(other.as_std_path()).unwrap();
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
