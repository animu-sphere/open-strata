// SPDX-License-Identifier: Apache-2.0
//! SDK activation and opt-in consumer probes. Structural validation never
//! silently executes component binaries or CMake package code.

use super::*;
use ost_core::{host::Os, Host};
use ost_runtime::EnvSet;
use std::process::Command;

fn absolute(root: &Utf8Path) -> Result<Utf8PathBuf> {
    let path = std::path::absolute(root).map_err(|e| Error::io(root.to_string(), e))?;
    Utf8PathBuf::from_path_buf(path).map_err(|_| Error::usage("SDK prefix must be UTF-8"))
}

fn activation(root: &Utf8Path, lock: &RuntimeCompositionLock) -> Result<EnvSet> {
    lock.sdk
        .as_ref()
        .ok_or_else(|| {
            composition_error(
                "COMPOSITION_SDK_REQUIRED",
                "legacy lock has no SDK; compose again to create an SDK lock",
            )
        })?
        .activate(&absolute(root)?, Os::current())
}

fn require_host(lock: &RuntimeCompositionLock) -> Result<()> {
    let host = Host::detect().slug();
    if lock.resolved.target != host && !lock.resolved.target.starts_with(&format!("{host}-")) {
        return Err(composition_error(
            "COMPOSITION_HOST_MISMATCH",
            format!(
                "runtime target '{}' cannot execute on '{host}'",
                lock.resolved.target
            ),
        ));
    }
    Ok(())
}

pub fn environment(root: &Utf8Path, shell: Option<&str>, fmt: Format) -> Result<()> {
    let lock = verify_prefix(root)?;
    let env = activation(root, &lock)?;
    let shell = crate::commands::devshell::pick_shell(shell, Os::current())?;
    if fmt.is_json() {
        output::success(
            &json!({"schema": "openstrata.runtime-environment/v1alpha1", "runtime_digest": lock.runtime_digest,
            "target": lock.resolved.target, "os": Os::current(), "env": env.pairs(), "isolated": true}),
        );
    } else {
        print!("{}", env.render(shell));
    }
    Ok(())
}

fn isolated(command: &mut Command, env: &EnvSet) {
    for key in [
        "PATH",
        "LD_LIBRARY_PATH",
        "DYLD_LIBRARY_PATH",
        "DYLD_FALLBACK_LIBRARY_PATH",
        "PYTHONPATH",
        "PYTHONHOME",
        "NODE_PATH",
        "PXR_PLUGINPATH_NAME",
        "CMAKE_PREFIX_PATH",
        "CMAKE_MODULE_PATH",
        "CMAKE_LIBRARY_PATH",
        "CMAKE_INCLUDE_PATH",
        "CMAKE_FRAMEWORK_PATH",
        "CMAKE_APPBUNDLE_PATH",
    ] {
        command.env_remove(key);
    }
    command.envs(env.pairs());
}

fn program_path(program: &str, env: &EnvSet) -> Result<Utf8PathBuf> {
    let path = Utf8Path::new(program);
    if path.is_absolute() {
        return Ok(path.into());
    }
    if path.components().count() == 1 {
        for (_, value) in env.pairs().iter().filter(|(key, _)| key == "PATH") {
            for directory in value.split(env.sep).filter(|p| !p.is_empty()) {
                let candidate = Utf8Path::new(directory).join(program);
                if candidate.is_file() {
                    return Ok(candidate);
                }
                #[cfg(windows)]
                if candidate.extension().is_none() && candidate.with_extension("exe").is_file() {
                    return Ok(candidate.with_extension("exe"));
                }
            }
        }
    }
    Err(composition_error(
        "COMPOSITION_EXECUTABLE_UNREACHABLE",
        format!("'{program}' is not on the SDK PATH; use an absolute path for an external tool"),
    ))
}

pub fn execute(root: &Utf8Path, args: &[String], fmt: Format) -> Result<()> {
    let lock = verify_prefix(root)?;
    require_host(&lock)?;
    let env = activation(root, &lock)?;
    let program = args
        .first()
        .ok_or_else(|| Error::usage("runtime exec requires a command after --"))?;
    let mut command = Command::new(program_path(program, &env)?);
    command.args(&args[1..]);
    isolated(&mut command, &env);
    let status = if fmt.is_json() {
        let result = command.output().map_err(|e| {
            composition_error(
                "COMPOSITION_LAUNCH_FAILED",
                format!("cannot launch '{program}': {e}"),
            )
        })?;
        output::report(
            result.status.success(),
            &json!({"schema": "openstrata.runtime-execution/v1alpha1", "runtime_digest": lock.runtime_digest,
            "program": program, "args": &args[1..], "exit_code": result.status.code(), "success": result.status.success(),
            "stdout": String::from_utf8_lossy(&result.stdout), "stderr": String::from_utf8_lossy(&result.stderr)}),
        );
        result.status
    } else {
        command.status().map_err(|e| {
            composition_error(
                "COMPOSITION_LAUNCH_FAILED",
                format!("cannot launch '{program}': {e}"),
            )
        })?
    };
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

fn reachable(root: &Utf8Path, path: &Utf8Path) -> bool {
    let Ok(root) = fs::canonicalize(root) else {
        return false;
    };
    fs::canonicalize(path).is_ok_and(|path| path.starts_with(root))
}

pub(super) fn validate(
    root: &Utf8Path,
    lock: &RuntimeCompositionLock,
    packages: &[String],
    fmt: Format,
) -> Result<()> {
    let root = absolute(root)?;
    let env = activation(&root, lock)?;
    let sdk = lock.sdk.as_ref().expect("activation checked SDK");
    let mut checks = Vec::new();
    for (key, value) in env.pairs() {
        if sdk.settings.contains_key(&key) {
            continue;
        }
        for path in value.split(env.sep) {
            let passed = reachable(&root, Utf8Path::new(path));
            checks.push(json!({"name": "activation-path", "variable": key, "path": path, "status": if passed { "passed" } else { "failed" }}));
        }
    }
    // Inspect both preserved and projected plugInfo documents; projection can
    // break a relative LibraryPath even when the original remains valid.
    let files = lock
        .inventory
        .iter()
        .map(|e| (&e.file, &e.component, &e.file.path))
        .chain(sdk.files.iter().map(|e| (&e.file, &e.component, &e.source)));
    for (file, owner, source_path) in
        files.filter(|(f, _, _)| Utf8Path::new(&f.path).file_name() == Some("plugInfo.json"))
    {
        // OpenUSD ships usdGenSchema's input under this exact resource path.
        // Its placeholders are not loadable plugin metadata. Keep the template
        // in the SDK and its integrity inventory, but do not resolve its paths.
        // Do not exempt similarly named files from arbitrary plugin components.
        let source_prefix = format!("components/{owner}/");
        if source_path.strip_prefix(&source_prefix)
            == Some("lib/usd/usd/resources/codegenTemplates/plugInfo.json")
            && lock
                .resolved
                .components
                .iter()
                .any(|component| component.id == *owner && component.kind == "runtime")
        {
            checks.push(json!({"name": "plugin-template", "metadata": file.path,
                "status": "skipped", "detail": "usdGenSchema input template, not plugin registration metadata"}));
            continue;
        }
        let path = root.join(&file.path);
        let source = fs::read_to_string(&path).map_err(|e| Error::io(path.to_string(), e))?;
        let info = ost_plugin::parse_plug_info(&source).map_err(|e| {
            composition_error(
                "COMPOSITION_PLUGIN_METADATA_INVALID",
                format!("{}: {e}", file.path),
            )
        })?;
        for plugin in info["Plugins"].as_array().into_iter().flatten() {
            let base = path
                .parent()
                .expect("plugin parent")
                .join(plugin["Root"].as_str().unwrap_or("."));
            for field in ["LibraryPath", "ResourcePath"] {
                if let Some(relative) = plugin[field].as_str().filter(|p| !p.is_empty()) {
                    let target = base.join(relative);
                    let expected_kind = if field == "LibraryPath" {
                        target.is_file()
                    } else {
                        target.is_dir()
                    };
                    checks.push(json!({"name": "plugin-path", "plugin": plugin["Name"], "metadata": file.path,
                        "field": field, "path": target, "status": if expected_kind && reachable(&root, &target) { "passed" } else { "failed" }}));
                }
            }
            let schema = plugin
                .pointer("/Info/Types")
                .and_then(Value::as_object)
                .is_some_and(|types| types.values().any(|t| t.get("schemaKind").is_some()));
            if schema {
                let schema = base
                    .join(plugin["ResourcePath"].as_str().unwrap_or("."))
                    .join("generatedSchema.usda");
                checks.push(json!({"name": "schema-resource", "metadata": file.path, "path": schema,
                    "status": if schema.is_file() && reachable(&root, &schema) { "passed" } else { "failed" }}));
            }
        }
    }
    for package in packages {
        require_host(lock)?;
        checks.push(cmake_probe(&root, package, &env)?);
    }
    checks.push(json!({"name": "native-loader-plugin-resolver-execution", "status": "not-run",
        "detail": "Path checks do not load libraries or discover USD types. Use runtime exec with a component-owned probe."}));
    let passed = checks.iter().all(|c| c["status"] != "failed");
    if fmt.is_json() {
        output::report(
            passed,
            &json!({"schema": "openstrata.runtime-sdk-validation/v1alpha1", "runtime_digest": lock.runtime_digest, "checks": checks}),
        );
    } else {
        for check in &checks {
            println!(
                "[{}] {} {}",
                check["status"].as_str().unwrap_or_default(),
                check["name"].as_str().unwrap_or_default(),
                check
            );
        }
    }
    if !passed {
        std::process::exit(ost_core::Category::Validation.exit_code() as i32);
    }
    Ok(())
}

fn cmake_probe(root: &Utf8Path, package: &str, env: &EnvSet) -> Result<Value> {
    if package.is_empty()
        || !package
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_+.-".contains(&b))
    {
        return Err(Error::usage(
            "CMake package name must contain only letters, digits, '_', '+', '.', '-'",
        ));
    }
    let cmake = ost_core::tools::which("cmake").ok_or_else(|| {
        composition_error(
            "COMPOSITION_CMAKE_UNAVAILABLE",
            "install CMake to run the requested package probe",
        )
    })?;
    let temporary = Utf8PathBuf::from_path_buf(std::env::temp_dir())
        .map_err(|_| Error::usage("temporary directory must be UTF-8"))?;
    let staging =
        Staging::for_destination(&temporary.join(format!("ost-sdk-probe-{}", std::process::id())))?;
    // Pass prefix as a -D argument, never interpolate an untrusted filesystem
    // path into CMake source. NO_DEFAULT_PATH excludes user/system registries.
    fs::write(staging.0.join("CMakeLists.txt"), format!("cmake_minimum_required(VERSION 3.20)\nproject(OpenStrataSdkProbe NONE)\nfind_package({package} CONFIG REQUIRED PATHS \"${{OST_SDK_PREFIX}}\" NO_DEFAULT_PATH)\n"))
        .map_err(|e| Error::io(staging.0.to_string(), e))?;
    let mut command = Command::new(cmake);
    command.args([
        "-S",
        staging.0.as_str(),
        "-B",
        staging.0.join("build").as_str(),
    ]);
    command.arg(format!("-DOST_SDK_PREFIX={root}"));
    command.arg(format!("-DCMAKE_PREFIX_PATH={root}"));
    command.args([
        "-DCMAKE_FIND_USE_PACKAGE_REGISTRY=FALSE",
        "-DCMAKE_FIND_USE_SYSTEM_PACKAGE_REGISTRY=FALSE",
        "-DCMAKE_FIND_USE_SYSTEM_ENVIRONMENT_PATH=FALSE",
        "-DCMAKE_FIND_USE_CMAKE_SYSTEM_PATH=FALSE",
    ]);
    if let Some(ninja) = ost_core::tools::which("ninja") {
        command
            .args(["-G", "Ninja"])
            .arg(format!("-DCMAKE_MAKE_PROGRAM={}", ninja.display()));
    }
    isolated(&mut command, env);
    for (key, _) in std::env::vars().filter(|(key, _)| {
        key.starts_with("CMAKE_") || key.ends_with("_ROOT") || key.ends_with("_DIR")
    }) {
        command.env_remove(key);
    }
    command.current_dir(&staging.0);
    let output = command.output().map_err(|e| {
        composition_error("COMPOSITION_CMAKE_FAILED", format!("cannot run CMake: {e}"))
    })?;
    Ok(
        json!({"name": "cmake-package", "package": package, "status": if output.status.success() { "passed" } else { "failed" },
        "exit_code": output.status.code(), "stdout": String::from_utf8_lossy(&output.stdout), "stderr": String::from_utf8_lossy(&output.stderr)}),
    )
}
