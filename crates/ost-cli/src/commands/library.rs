// SPDX-License-Identifier: Apache-2.0
//! `ost library` — build, test, package, and verify one descriptor-owned CMake library.
//!
//! Plain libraries participate in plugin workspace composition, but an optional
//! adapter also needs to remain independently shippable. These commands reuse
//! the workspace library builder while giving one descriptor its own install
//! prefix and digest-bound completion record.

use std::fs::File;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};
use clap::Subcommand;
use serde::{Deserialize, Serialize};

use ost_build::{pack_dir, stage_files};
use ost_core::digest;
use ost_core::fs::write_atomic;
use ost_core::{tools, Category, Error, Result};
use ost_plugin::{Library, LIBRARY_MANIFEST};
use ost_runtime::{RuntimeManifest, MANIFEST_FILE};

use crate::commands::compiler::CompilerOpts;
use crate::commands::configure::build_target;
use crate::commands::plugin;
use crate::output::{self, Format};

const LIBRARY_BUILD_SCHEMA: &str = "openstrata.library-build/v1";
const LIBRARY_BUILD_FILE: &str = "library-build.json";
const LIBRARY_TEST_FILE: &str = "library-test.json";
const LIBRARY_CONSUMER_FILE: &str = "library-consumer.json";
const LIBRARY_JUNIT_FILE: &str = ".ost-library-test-results.xml";

#[derive(Debug, Subcommand)]
pub enum LibraryCmd {
    /// Configure, build, and install one plain CMake library.
    Build {
        /// Directory containing openstrata.library.yaml.
        #[arg(default_value = ".")]
        library: Utf8PathBuf,

        /// Platform target, e.g. `cy2026`. Defaults to the enclosing project's.
        #[arg(long)]
        target: Option<String>,

        /// Runtime profile. Defaults to the enclosing project's profile.
        #[arg(long)]
        profile: Option<String>,

        /// Print the CMake plan without executing it.
        #[arg(long)]
        dry_run: bool,

        /// Path to the Ninja executable if it is not on PATH.
        #[arg(long)]
        ninja: Option<String>,

        #[command(flatten)]
        compiler: CompilerOpts,
    },

    /// Run CTest for one completed library build.
    Test {
        /// Directory containing openstrata.library.yaml.
        #[arg(default_value = ".")]
        library: Utf8PathBuf,

        /// Platform target, e.g. `cy2026`. Defaults to the enclosing project's.
        #[arg(long)]
        target: Option<String>,

        /// Runtime profile. Defaults to the enclosing project's profile.
        #[arg(long)]
        profile: Option<String>,

        /// Only run tests whose name matches this CTest regular expression.
        #[arg(long)]
        filter: Option<String>,

        /// Per-test timeout in seconds; 0 disables it.
        #[arg(long, default_value_t = 300)]
        timeout: u64,

        /// Path to ctest if it is not on PATH.
        #[arg(long)]
        ctest: Option<String>,

        /// Print the command without executing or writing evidence.
        #[arg(long)]
        dry_run: bool,
    },

    /// Package one library's isolated install tree as a tar.zst artifact.
    Package {
        /// Directory containing openstrata.library.yaml.
        #[arg(default_value = ".")]
        library: Utf8PathBuf,

        /// Platform target, e.g. `cy2026`. Defaults to the enclosing project's.
        #[arg(long)]
        target: Option<String>,

        /// Runtime profile. Defaults to the enclosing project's profile.
        #[arg(long)]
        profile: Option<String>,
    },

    /// Build, install, and link a generated consumer against the declared package closure.
    VerifyConsumer {
        /// Directory containing openstrata.library.yaml.
        #[arg(default_value = ".")]
        library: Utf8PathBuf,

        /// Platform target, e.g. `cy2026`. Defaults to the enclosing project's.
        #[arg(long)]
        target: Option<String>,

        /// Runtime profile. Defaults to the enclosing project's profile.
        #[arg(long)]
        profile: Option<String>,

        /// Path to the Ninja executable if it is not on PATH.
        #[arg(long)]
        ninja: Option<String>,

        #[command(flatten)]
        compiler: CompilerOpts,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LibraryFile {
    path: String,
    sha256: String,
    size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LibraryBuildRecord {
    schema: String,
    library: LibraryIdentity,
    target: String,
    runtime: RuntimeIdentity,
    descriptor_sha256: String,
    build_dir: String,
    install_prefix: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    dependencies: Vec<LibraryDependencyRecord>,
    files: Vec<LibraryFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LibraryDependencyRecord {
    id: String,
    version: String,
    descriptor_sha256: String,
    install_prefix: String,
    build_record_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LibraryIdentity {
    id: String,
    version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RuntimeIdentity {
    id: String,
    digest: String,
}

pub fn run(cmd: LibraryCmd, fmt: Format) -> Result<()> {
    match cmd {
        LibraryCmd::Build {
            library,
            target,
            profile,
            dry_run,
            ninja,
            compiler,
        } => build(&library, target, profile, dry_run, ninja, compiler, fmt),
        LibraryCmd::Test {
            library,
            target,
            profile,
            filter,
            timeout,
            ctest,
            dry_run,
        } => test(
            &library, target, profile, filter, timeout, ctest, dry_run, fmt,
        ),
        LibraryCmd::Package {
            library,
            target,
            profile,
        } => package(&library, target, profile, fmt),
        LibraryCmd::VerifyConsumer {
            library,
            target,
            profile,
            ninja,
            compiler,
        } => verify_consumer(&library, target, profile, ninja, compiler, fmt),
    }
}

#[allow(clippy::too_many_arguments)]
fn build(
    root: &Utf8Path,
    target: Option<String>,
    profile: Option<String>,
    dry_run: bool,
    ninja: Option<String>,
    compiler: CompilerOpts,
    fmt: Format,
) -> Result<()> {
    build_inner(root, target, profile, dry_run, ninja, compiler, fmt, true)
}

#[allow(clippy::too_many_arguments)]
fn build_inner(
    root: &Utf8Path,
    target: Option<String>,
    profile: Option<String>,
    dry_run: bool,
    ninja: Option<String>,
    compiler: CompilerOpts,
    fmt: Format,
    emit_output: bool,
) -> Result<()> {
    let library = Library::load(root)?;
    let dependencies = plugin::selected_workspace_libraries_for_library(&library)?;
    let (platform, profile) = selection(target, profile)?;
    let (target, resolved) = build_target(&platform, &profile)?;
    let id = target.id();
    let runtime = if dry_run {
        None
    } else {
        Some(runtime_identity(&resolved.prefix, &target.runtime_id)?)
    };
    let mut record = None;
    for member in dependencies.iter().chain(std::iter::once(&library)) {
        let prerequisites = plugin::selected_workspace_libraries_for_library(member)?;
        let prerequisite_prefixes = prerequisites
            .iter()
            .map(|dependency| isolated_prefix(dependency, &id))
            .collect::<Vec<_>>();
        if emit_output && !fmt.is_json() && member.id() != library.id() {
            println!("== build prerequisite {} ==", member.id());
        }
        let built = build_library_member(
            member,
            &platform,
            &profile,
            &id,
            runtime.as_ref(),
            &prerequisites,
            &prerequisite_prefixes,
            dry_run,
            ninja.clone(),
            compiler.clone(),
            !emit_output,
        )?;
        if member.id() == library.id() {
            record = built;
        }
    }
    if dry_run {
        return Ok(());
    }
    let record = record.expect("the primary library is always built last");
    let prefix = isolated_prefix(&library, &id);

    if !emit_output {
        return Ok(());
    }
    if fmt.is_json() {
        output::success(&serde_json::json!({
            "built": true,
            "library": record.library,
            "target": record.target,
            "runtime": record.runtime,
            "build_dir": record.build_dir,
            "install_prefix": record.install_prefix,
            "dependencies": record.dependencies,
            "files": record.files.len(),
            "record": build_record_path(&library, &id),
        }));
    } else {
        println!("Built library {} {}", library.id(), library.version());
        println!("  target:  {id}");
        println!("  install: {prefix}");
        println!("  dependencies: {}", record.dependencies.len());
        println!("  files:   {}", record.files.len());
        println!("  record:  {}", build_record_path(&library, &id));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_library_member(
    library: &Library,
    platform: &str,
    profile: &str,
    target_id: &str,
    runtime: Option<&RuntimeIdentity>,
    prerequisites: &[Library],
    prerequisite_prefixes: &[Utf8PathBuf],
    dry_run: bool,
    ninja: Option<String>,
    compiler: CompilerOpts,
    quiet: bool,
) -> Result<Option<LibraryBuildRecord>> {
    let prefix = isolated_prefix(library, target_id);
    if !dry_run {
        remove_existing_prefix(&prefix)?;
    }
    plugin::build_library_one(
        library,
        Some(platform.to_string()),
        Some(profile.to_string()),
        dry_run,
        ninja,
        compiler,
        &prefix,
        prerequisite_prefixes,
        quiet,
    )?;
    if dry_run {
        return Ok(None);
    }
    let runtime = runtime.expect("a non-dry build resolves runtime identity");

    let files = snapshot_files(&prefix)?;
    if files.is_empty() {
        return Err(Error::validation(format!(
            "library '{}' installed no files into '{prefix}'",
            library.id()
        ))
        .with_hint("add CMake install rules for the library target, headers, and config package"));
    }
    let record = LibraryBuildRecord {
        schema: LIBRARY_BUILD_SCHEMA.into(),
        library: LibraryIdentity {
            id: library.id().into(),
            version: library.version().into(),
        },
        target: target_id.into(),
        runtime: runtime.clone(),
        descriptor_sha256: descriptor_digest(library)?,
        build_dir: portable(&plugin::target_build_dir(&library.root, target_id)),
        install_prefix: portable(&prefix),
        dependencies: dependency_records(prerequisites, target_id)?,
        files,
    };
    write_json(&build_record_path(library, target_id), &record)?;
    Ok(Some(record))
}

#[allow(clippy::too_many_arguments)]
fn test(
    root: &Utf8Path,
    target: Option<String>,
    profile: Option<String>,
    filter: Option<String>,
    timeout: u64,
    ctest: Option<String>,
    dry_run: bool,
    fmt: Format,
) -> Result<()> {
    let library = Library::load(root)?;
    let (platform, profile) = selection(target, profile)?;
    let (target, resolved) = build_target(&platform, &profile)?;
    let id = target.id();
    validated_build_record(&library, &id, &target.runtime_id, &resolved.prefix)?;
    let prerequisites = plugin::selected_workspace_libraries_for_library(&library)?;
    let mut runtime_directories = library.installed_runtime_dirs(&isolated_prefix(&library, &id));
    runtime_directories.extend(prerequisites.iter().flat_map(|prerequisite| {
        prerequisite.installed_runtime_dirs(&isolated_prefix(prerequisite, &id))
    }));
    let runtime_directory_refs = runtime_directories
        .iter()
        .map(Utf8PathBuf::as_path)
        .collect::<Vec<_>>();
    let test_env = ost_plugin::session_env_from_with_library_dirs(
        &resolved.env,
        &[],
        &runtime_directory_refs,
        target.os(),
    );
    let ctest = ctest
        .map(Utf8PathBuf::from)
        .or_else(|| tools::which("ctest").and_then(|path| Utf8PathBuf::from_path_buf(path).ok()))
        .ok_or_else(|| {
            Error::coded(
                "REQUIRED_TOOL_MISSING",
                Category::Precondition,
                "`ctest` not found on PATH",
            )
        })?;
    let build_dir = plugin::target_build_dir(&library.root, &id);
    let junit_path = build_dir.join(LIBRARY_JUNIT_FILE);
    let mut args = vec![
        "--test-dir".to_string(),
        build_dir.to_string(),
        "--output-on-failure".to_string(),
        "--output-junit".to_string(),
        junit_path.to_string(),
    ];
    if timeout > 0 {
        args.extend(["--timeout".into(), timeout.to_string()]);
    }
    if let Some(filter) = filter {
        args.extend(["-R".into(), filter]);
    }
    if dry_run {
        if fmt.is_json() {
            output::success(&serde_json::json!({
                "dry_run": true,
                "library": library.id(),
                "target": id,
                "command": render_command(&ctest, &args),
            }));
        } else {
            println!("# dry run — would execute:");
            println!("{}", render_command(&ctest, &args));
        }
        return Ok(());
    }

    for stale in [&junit_path, &test_record_path(&library, &id)] {
        match std::fs::remove_file(stale.as_std_path()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::io(stale.to_string(), error)),
        }
    }
    let started_unix = unix_now();
    let mut command = Command::new(ctest.as_std_path());
    command.args(&args).current_dir(library.root.as_std_path());
    test_env.apply(&mut command);
    let status = command
        .status()
        .map_err(|error| Error::io(format!("run {ctest}"), error))?;
    let Some(totals) =
        crate::commands::test::read_totals(&junit_path).filter(|totals| totals.total > 0)
    else {
        return Err(
            Error::external_tool(format!("no tests ran for library '{}'", library.id()))
                .with_hint("register tests with add_test() in CMake, or relax --filter"),
        );
    };
    let evidence = serde_json::json!({
        "schema": "openstrata.library-test/v1",
        "library": {"id": library.id(), "version": library.version()},
        "target": id,
        "build_record_sha256": file_digest(&build_record_path(&library, &target.id()))?.0,
        "started_unix": started_unix,
        "completed_unix": unix_now(),
        "exit_code": status.code(),
        "outcome": if status.success() { "success" } else { "failure" },
        "tests": totals,
    });
    write_value(&test_record_path(&library, &target.id()), &evidence)?;
    if !status.success() {
        return Err(Error::external_tool(format!(
            "CTest failed for library '{}'{}",
            library.id(),
            exit_detail(status.code())
        ))
        .with_phase("library-test")
        .with_data(evidence));
    }
    if fmt.is_json() {
        output::success(&serde_json::json!({
            "tested": true,
            "library": library.id(),
            "target": target.id(),
            "record": test_record_path(&library, &target.id()),
        }));
    } else {
        println!("Tested library {} ({})", library.id(), target.id());
        println!("  record: {}", test_record_path(&library, &target.id()));
    }
    Ok(())
}

fn package(
    root: &Utf8Path,
    target: Option<String>,
    profile: Option<String>,
    fmt: Format,
) -> Result<()> {
    let library = Library::load(root)?;
    let (platform, profile) = selection(target, profile)?;
    let (target, resolved) = build_target(&platform, &profile)?;
    let id = target.id();
    let record = validated_build_record(&library, &id, &target.runtime_id, &resolved.prefix)?;
    let prefix = isolated_prefix(&library, &id);
    let staged = stage_files(&prefix).map_err(stage_error(&prefix))?;
    let dist = library
        .root
        .join("dist")
        .join(library.id())
        .join(library.version())
        .join(&id);
    let archive_name = format!("{}-{}-{id}.tar.zst", library.id(), library.version());
    let archive = dist.join(&archive_name);
    let packed = pack_dir(&prefix, &archive, &staged)
        .map_err(|error| Error::io(archive.to_string(), error))?;
    let created_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let component_abi = match target.variant.os {
        ost_core::host::Os::Linux => "libstdcxx".to_string(),
        ost_core::host::Os::Macos => "libcxx".to_string(),
        ost_core::host::Os::Windows => match &target.variant.abi {
            ost_core::variant::Abi::Msvc { toolset } => format!("msvc{toolset}"),
            other => other.describe(),
        },
    };
    let mut manifest = serde_json::json!({
        "schema": 1,
        "name": library.id(),
        "version": library.version(),
        "target": id,
        "archive": archive_name,
        "archive_digest": packed.archive_digest,
        "archive_size": packed.archive_size,
        "total_size": packed.total_size,
        "created_unix": created_unix,
        "producer": format!("ost {}", env!("CARGO_PKG_VERSION")),
        "component": {
            "schema": ost_artifact::COMPONENT_SCHEMA,
            "id": library.id(),
            "kind": "library",
            "version": library.version(),
            "provides": [
                {"capability": format!("library:{}", library.id()), "version": library.version(), "singleton": true},
                {"capability": format!("cmake:{}", library.manifest.cmake.package), "version": library.version(), "singleton": true}
            ],
            "requires": record.dependencies.iter().map(|dependency| serde_json::json!({
                "capability": format!("library:{}", dependency.id),
                "version": dependency.version,
            })).collect::<Vec<_>>(),
            "environment": [
                {"variable": if target.variant.os == ost_core::host::Os::Windows { "PATH" } else if target.variant.os == ost_core::host::Os::Macos { "DYLD_LIBRARY_PATH" } else { "LD_LIBRARY_PATH" }, "operation": "prepend", "values": ["lib"]},
                {"variable": "CMAKE_PREFIX_PATH", "operation": "prepend", "values": ["."]}
            ],
            "install": packed.files.iter().map(|file| serde_json::json!({
                "source": file.path,
                "destination": file.path,
            })).collect::<Vec<_>>(),
            "compatibility": {
                "targets": [target.variant.slug()],
                "abi": component_abi,
            },
            "descriptor": LIBRARY_MANIFEST,
            "descriptor_sha256": record.descriptor_sha256,
            "cmake": {
                "package": library.manifest.cmake.package,
                "target": library.manifest.cmake.target,
            },
            "dependencies": {
                "libraries": record.dependencies.iter().map(|dependency| serde_json::json!({
                    "id": dependency.id,
                    "version": dependency.version,
                    "descriptor_sha256": dependency.descriptor_sha256,
                    "build_record_sha256": dependency.build_record_sha256,
                })).collect::<Vec<_>>(),
            }
        },
        "provenance": {
            "runtime": record.runtime,
            "build_record_sha256": file_digest(&build_record_path(&library, &target.id()))?.0,
        },
        "files": packed.files.iter().map(|file| file.manifest_json()).collect::<Vec<_>>(),
    });
    let evidence = ost_artifact::generate_evidence(&dist, &mut manifest)?;
    write_value(&dist.join("manifest.json"), &manifest)?;
    let archive_digest = packed
        .archive_digest
        .strip_prefix("sha256:")
        .unwrap_or(&packed.archive_digest);
    let mut sums = vec![format!("{archive_digest}  {archive_name}")];
    sums.extend(evidence.iter().map(|item| {
        format!(
            "{}  {}",
            item.digest.strip_prefix("sha256:").unwrap_or(&item.digest),
            item.path
        )
    }));
    write_atomic(
        dist.join("SHA256SUMS").as_std_path(),
        format!("{}\n", sums.join("\n")).as_bytes(),
    )?;

    if fmt.is_json() {
        output::success(&serde_json::json!({
            "packaged": true,
            "library": {"id": library.id(), "version": library.version()},
            "target": target.id(),
            "archive": archive,
            "archive_digest": packed.archive_digest,
            "dependencies": record.dependencies,
            "files": packed.files.len(),
        }));
    } else {
        println!("Packaged library {} {}", library.id(), library.version());
        println!("  target:  {}", target.id());
        println!("  archive: {archive}");
        println!("  digest:  {}", packed.archive_digest);
        println!("  dependencies: {}", record.dependencies.len());
        println!("  files:   {}", packed.files.len());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn verify_consumer(
    root: &Utf8Path,
    target: Option<String>,
    profile: Option<String>,
    ninja: Option<String>,
    compiler: CompilerOpts,
    fmt: Format,
) -> Result<()> {
    let library = Library::load(root)?;
    let package = library.manifest.package.as_ref().ok_or_else(|| {
        Error::config(format!("library '{}' has no package mode", library.id())).with_hint(
            "declare package.standalone and package.aggregate_member in openstrata.library.yaml",
        )
    })?;
    if !package.standalone {
        return Err(Error::coded(
            "LIBRARY_NOT_STANDALONE",
            Category::Precondition,
            format!(
                "library '{}' is aggregate-only and cannot be verified as a standalone consumer package",
                library.id()
            ),
        ));
    }
    let contract = library.manifest.package_contract.as_ref().ok_or_else(|| {
        Error::config(format!(
            "library '{}' has no package_contract",
            library.id()
        ))
        .with_hint(
            "declare the installed package name, exported targets, and optional consumer probe",
        )
    })?;

    // `verify-consumer` owns the complete lifecycle: refresh the selected
    // library and its declared closure before creating an unrelated consumer
    // build tree. Suppress the nested build renderer so JSON remains one value.
    build_inner(
        root,
        target.clone(),
        profile.clone(),
        false,
        ninja.clone(),
        compiler,
        fmt,
        false,
    )?;

    let (platform, profile) = selection(target, profile)?;
    let (target, resolved) = build_target(&platform, &profile)?;
    let target_id = target.id();
    validated_build_record(&library, &target_id, &target.runtime_id, &resolved.prefix)?;
    let compiler_record = library_compiler_record(&library, &target_id)?;
    let prerequisites = plugin::selected_workspace_libraries_for_library(&library)?;
    let mut closure = prerequisites
        .iter()
        .map(|dependency| {
            (
                dependency.id().to_string(),
                isolated_prefix(dependency, &target_id),
            )
        })
        .collect::<Vec<_>>();
    closure.push((
        library.id().to_string(),
        isolated_prefix(&library, &target_id),
    ));
    if closure
        .iter()
        .any(|(_, prefix)| prefix.as_str().contains(';'))
    {
        return Err(Error::precondition(
            "library consumer prefixes containing ';' are not supported by CMake list arguments",
        ));
    }

    let consumer_root = plugin::target_state_dir(&library.root, &target_id).join("consumer");
    if consumer_root.as_std_path().exists() {
        std::fs::remove_dir_all(consumer_root.as_std_path())
            .map_err(|error| Error::io(consumer_root.to_string(), error))?;
    }
    let source_dir = consumer_root.join("src");
    let consumer_build_dir = consumer_root.join("build");
    std::fs::create_dir_all(source_dir.as_std_path())
        .map_err(|error| Error::io(source_dir.to_string(), error))?;
    write_atomic(
        source_dir.join("CMakeLists.txt").as_std_path(),
        consumer_cmake(contract, closure.len()).as_bytes(),
    )?;
    write_atomic(
        source_dir.join("main.cpp").as_std_path(),
        consumer_source(contract).as_bytes(),
    )?;

    let cmake = tools::which("cmake").ok_or_else(|| {
        Error::coded(
            "REQUIRED_TOOL_MISSING",
            Category::Precondition,
            "`cmake` not found on PATH",
        )
    })?;
    let ninja = ninja
        .map(std::path::PathBuf::from)
        .or_else(|| tools::which("ninja"))
        .ok_or_else(|| {
            Error::coded(
                "REQUIRED_TOOL_MISSING",
                Category::Precondition,
                "`ninja` not found on PATH",
            )
        })?;
    let mut configure_args = vec![
        "-S".to_string(),
        source_dir.to_string(),
        "-B".to_string(),
        consumer_build_dir.to_string(),
        "-G".to_string(),
        "Ninja".to_string(),
        "-DCMAKE_BUILD_TYPE=Release".to_string(),
        format!(
            "-DCMAKE_MAKE_PROGRAM={}",
            ninja.display().to_string().replace('\\', "/")
        ),
        "-DCMAKE_FIND_USE_PACKAGE_REGISTRY=FALSE".to_string(),
        "-DCMAKE_FIND_USE_SYSTEM_PACKAGE_REGISTRY=FALSE".to_string(),
    ];
    configure_args.extend(
        closure.iter().enumerate().map(|(index, (_, prefix))| {
            format!("-DOST_CONSUMER_PREFIX_{index}={}", portable(prefix))
        }),
    );
    if let Some(cxx) = &compiler_record.cxx {
        configure_args.push(format!("-DCMAKE_CXX_COMPILER={cxx}"));
    }

    let build_args = vec![
        "--build".to_string(),
        consumer_build_dir.to_string(),
        "--config".to_string(),
        "Release".to_string(),
    ];
    let msvc_env = consumer_msvc_env(target.os());
    let configure = run_consumer_step(&cmake, &configure_args, &source_dir, &msvc_env)?;
    let mut report = consumer_report(
        &library,
        &target_id,
        contract,
        &closure,
        &compiler_record,
        &source_dir,
        &consumer_build_dir,
        &configure,
        None,
    );
    let record_path = consumer_root.join(LIBRARY_CONSUMER_FILE);
    if !configure.status.success() {
        write_value(&record_path, &report)?;
        return Err(Error::coded(
            "LIBRARY_CONSUMER_CONFIGURE_FAILED",
            Category::Validation,
            format!(
                "installed package '{}' did not configure from its declared closure{}",
                contract.package_name,
                exit_detail(configure.status.code())
            ),
        )
        .with_hint(format!("inspect {record_path}"))
        .with_phase("library-consumer-configure")
        .with_data(report));
    }

    let linked = run_consumer_step(&cmake, &build_args, &source_dir, &msvc_env)?;
    report = consumer_report(
        &library,
        &target_id,
        contract,
        &closure,
        &compiler_record,
        &source_dir,
        &consumer_build_dir,
        &configure,
        Some(&linked),
    );
    write_value(&record_path, &report)?;
    if !linked.status.success() {
        return Err(Error::coded(
            "LIBRARY_CONSUMER_LINK_FAILED",
            Category::Validation,
            format!(
                "installed targets for package '{}' did not compile and link{}",
                contract.package_name,
                exit_detail(linked.status.code())
            ),
        )
        .with_hint(format!("inspect {record_path}"))
        .with_phase("library-consumer-link")
        .with_data(report));
    }

    if fmt.is_json() {
        output::success(&report);
    } else {
        println!(
            "Verified installed consumer for {} {}",
            library.id(),
            library.version()
        );
        println!("  package: {}", contract.package_name);
        println!("  targets: {}", contract.exported_targets.join(", "));
        println!("  closure: {} package(s)", closure.len());
        println!("  record:  {record_path}");
    }
    Ok(())
}

fn consumer_cmake(contract: &ost_plugin::LibraryPackageContract, closure_len: usize) -> String {
    let prefix_variables = (0..closure_len)
        .map(|index| {
            format!(
                "if(DEFINED OST_CONSUMER_PREFIX_{index})\n  list(APPEND CMAKE_PREFIX_PATH \"${{OST_CONSUMER_PREFIX_{index}}}\")\nendif()"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "cmake_minimum_required(VERSION 3.23)\nproject(OpenStrataLibraryConsumer LANGUAGES CXX)\nset(CMAKE_FIND_USE_PACKAGE_REGISTRY FALSE)\nset(CMAKE_FIND_USE_SYSTEM_PACKAGE_REGISTRY FALSE)\n{prefix_variables}\nfind_package({} CONFIG REQUIRED PATHS ${{CMAKE_PREFIX_PATH}} NO_DEFAULT_PATH)\nadd_executable(openstrata-consumer main.cpp)\ntarget_compile_features(openstrata-consumer PRIVATE cxx_std_17)\ntarget_link_libraries(openstrata-consumer PRIVATE {})\n",
        contract.package_name,
        contract.exported_targets.join(" ")
    )
}

fn consumer_source(contract: &ost_plugin::LibraryPackageContract) -> String {
    let Some(consumer) = &contract.consumer else {
        return "int main() { return 0; }\n".into();
    };
    let include = consumer
        .include
        .as_ref()
        .map(|include| format!("#include <{include}>\n"))
        .unwrap_or_default();
    let body = consumer.symbol.as_ref().map_or_else(
        || "return 0;".to_string(),
        |symbol| {
            format!(
                "auto openstrata_consumer_symbol = &{symbol};\n  return openstrata_consumer_symbol == nullptr;"
            )
        },
    );
    format!("{include}int main() {{\n  {body}\n}}\n")
}

fn run_consumer_step(
    program: &std::path::Path,
    args: &[String],
    current_dir: &Utf8Path,
    environment: &[(String, String)],
) -> Result<Output> {
    let mut command = Command::new(program);
    command.args(args).current_dir(current_dir.as_std_path());
    for (key, _) in std::env::vars().filter(|(key, _)| {
        key.starts_with("CMAKE_") || key.ends_with("_ROOT") || key.ends_with("_DIR")
    }) {
        command.env_remove(key);
    }
    command.env_remove("CMAKE_PREFIX_PATH");
    for (key, value) in environment {
        command.env(key, value);
    }
    command
        .output()
        .map_err(|error| Error::io(format!("run {}", program.display()), error))
}

fn consumer_msvc_env(os: ost_core::host::Os) -> Vec<(String, String)> {
    if os != ost_core::host::Os::Windows || tools::which("cl").is_some() {
        return Vec::new();
    }
    ost_build::msvc::bootstrap()
        .ok()
        .flatten()
        .map(|environment| environment.vars)
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn consumer_report(
    library: &Library,
    target_id: &str,
    contract: &ost_plugin::LibraryPackageContract,
    closure: &[(String, Utf8PathBuf)],
    compiler: &ost_build::LockCompiler,
    source_dir: &Utf8Path,
    build_dir: &Utf8Path,
    configure: &Output,
    linked: Option<&Output>,
) -> serde_json::Value {
    let step = |output: &Output| {
        serde_json::json!({
            "status": if output.status.success() { "pass" } else { "fail" },
            "exit_code": output.status.code(),
            "stdout": bounded_output(&output.stdout),
            "stderr": bounded_output(&output.stderr),
        })
    };
    serde_json::json!({
        "schema": "openstrata.library-consumer-verification/v1alpha1",
        "library": {"id": library.id(), "version": library.version()},
        "target": target_id,
        "package": contract.package_name,
        "exported_targets": contract.exported_targets,
        "public_headers": contract.public_headers,
        "compiler": compiler,
        "closure": closure.iter().map(|(id, prefix)| serde_json::json!({"id": id, "prefix": prefix})).collect::<Vec<_>>(),
        "build_record_sha256": file_digest(&build_record_path(library, target_id)).ok().map(|value| value.0),
        "source_dir": source_dir,
        "build_dir": build_dir,
        "checks": {
            "configure": step(configure),
            "link": linked.map(step).unwrap_or_else(|| serde_json::json!({"status": "not-run", "reason": "configure failed"})),
        },
        "outcome": if configure.status.success() && linked.is_some_and(|output| output.status.success()) { "success" } else { "failure" },
    })
}

fn library_compiler_record(library: &Library, target_id: &str) -> Result<ost_build::LockCompiler> {
    let path = plugin::target_state_dir(&library.root, target_id).join("compiler.lock.json");
    let source = std::fs::read_to_string(path.as_std_path()).map_err(|error| {
        Error::precondition(format!(
            "library '{}' compiler evidence is unavailable: {error}",
            library.id()
        ))
        .with_hint(format!("rerun `ost library build {}`", library.root))
    })?;
    serde_json::from_str(&source).map_err(|error| {
        Error::precondition(format!(
            "library compiler evidence '{path}' is invalid: {error}"
        ))
        .with_hint(format!("rerun `ost library build {}`", library.root))
    })
}

fn bounded_output(bytes: &[u8]) -> String {
    const LIMIT: usize = 32 * 1024;
    let start = bytes.len().saturating_sub(LIMIT);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

fn selection(target: Option<String>, profile: Option<String>) -> Result<(String, String)> {
    plugin::selection(target, profile).ok_or_else(|| {
        Error::usage(
            "no platform/profile: run inside an OpenStrata project or pass --target/--profile",
        )
    })
}

fn isolated_prefix(library: &Library, target_id: &str) -> Utf8PathBuf {
    plugin::target_state_dir(&library.root, target_id).join("library-prefix")
}

fn build_record_path(library: &Library, target_id: &str) -> Utf8PathBuf {
    plugin::target_state_dir(&library.root, target_id).join(LIBRARY_BUILD_FILE)
}

fn test_record_path(library: &Library, target_id: &str) -> Utf8PathBuf {
    plugin::target_state_dir(&library.root, target_id).join(LIBRARY_TEST_FILE)
}

fn remove_existing_prefix(prefix: &Utf8Path) -> Result<()> {
    if prefix.as_std_path().exists() {
        std::fs::remove_dir_all(prefix.as_std_path())
            .map_err(|error| Error::io(prefix.to_string(), error))?;
    }
    std::fs::create_dir_all(prefix.as_std_path())
        .map_err(|error| Error::io(prefix.to_string(), error))
}

fn validated_build_record(
    library: &Library,
    target_id: &str,
    runtime_id: &str,
    runtime_prefix: &Utf8Path,
) -> Result<LibraryBuildRecord> {
    let path = build_record_path(library, target_id);
    let source = std::fs::read_to_string(path.as_std_path()).map_err(|error| {
        Error::precondition(format!("library '{}' is not built: {error}", library.id()))
            .with_hint(format!("run `ost library build {}` first", library.root))
    })?;
    let record: LibraryBuildRecord = serde_json::from_str(&source).map_err(|error| {
        Error::precondition(format!("library build record '{path}' is invalid: {error}"))
            .with_hint(format!("rerun `ost library build {}`", library.root))
    })?;
    let runtime = runtime_identity(runtime_prefix, runtime_id)?;
    let prerequisites = plugin::selected_workspace_libraries_for_library(library)?;
    for prerequisite in &prerequisites {
        validated_build_record(prerequisite, target_id, runtime_id, runtime_prefix)?;
    }
    let expected_dependencies = dependency_records(&prerequisites, target_id)?;
    let expected_descriptor = descriptor_digest(library)?;
    let expected_build_dir = plugin::target_build_dir(&library.root, target_id);
    let expected_prefix = isolated_prefix(library, target_id);
    let observed_files = snapshot_files(&expected_prefix)?;
    if record.schema != LIBRARY_BUILD_SCHEMA
        || record.library.id != library.id()
        || record.library.version != library.version()
        || record.target != target_id
        || record.runtime != runtime
        || record.descriptor_sha256 != expected_descriptor
        || !recorded_path_matches(&record.build_dir, &expected_build_dir)
        || !recorded_path_matches(&record.install_prefix, &expected_prefix)
        || record.dependencies != expected_dependencies
        || record.files != observed_files
    {
        return Err(Error::precondition(format!(
            "library '{}' build evidence no longer matches its descriptor, runtime, or install tree",
            library.id()
        ))
        .with_hint(format!("rerun `ost library build {}`", library.root)));
    }
    Ok(record)
}

fn dependency_records(
    prerequisites: &[Library],
    target_id: &str,
) -> Result<Vec<LibraryDependencyRecord>> {
    prerequisites
        .iter()
        .map(|dependency| {
            let record_path = build_record_path(dependency, target_id);
            Ok(LibraryDependencyRecord {
                id: dependency.id().into(),
                version: dependency.version().into(),
                descriptor_sha256: descriptor_digest(dependency)?,
                install_prefix: portable(&isolated_prefix(dependency, target_id)),
                build_record_sha256: file_digest(&record_path)
                    .map_err(|error| {
                        Error::precondition(format!(
                            "library '{}' prerequisite build evidence is unavailable: {error}",
                            dependency.id()
                        ))
                        .with_hint(format!(
                            "rerun `ost library build {}` so its declared closure is rebuilt",
                            dependency.root
                        ))
                    })?
                    .0,
            })
        })
        .collect()
}

fn runtime_identity(prefix: &Utf8Path, expected_id: &str) -> Result<RuntimeIdentity> {
    let path = prefix.join(MANIFEST_FILE);
    let source = std::fs::read_to_string(path.as_std_path()).map_err(|error| {
        Error::precondition(format!("runtime manifest '{path}' is unavailable: {error}"))
    })?;
    let manifest = RuntimeManifest::from_json(&source)
        .map_err(|error| Error::parse(path.to_string(), anyhow::Error::new(error)))?;
    if manifest.id != expected_id {
        return Err(Error::precondition(format!(
            "runtime manifest records '{}' but target requires '{expected_id}'",
            manifest.id
        )));
    }
    Ok(RuntimeIdentity {
        id: expected_id.into(),
        digest: manifest.digest,
    })
}

fn descriptor_digest(library: &Library) -> Result<String> {
    file_digest(&library.root.join(LIBRARY_MANIFEST)).map(|value| value.0)
}

fn snapshot_files(prefix: &Utf8Path) -> Result<Vec<LibraryFile>> {
    let paths = stage_files(prefix).map_err(stage_error(prefix))?;
    paths
        .into_iter()
        .map(|path| {
            let relative = path.strip_prefix(prefix).map_err(|error| {
                Error::validation(format!(
                    "installed path '{path}' escaped '{prefix}': {error}"
                ))
            })?;
            let (sha256, size) = file_digest(&path)?;
            Ok(LibraryFile {
                path: portable(relative),
                sha256,
                size,
            })
        })
        .collect()
}

fn file_digest(path: &Utf8Path) -> Result<(String, u64)> {
    let metadata = std::fs::symlink_metadata(path.as_std_path())
        .map_err(|error| Error::io(path.to_string(), error))?;
    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(path.as_std_path())
            .map_err(|error| Error::io(path.to_string(), error))?;
        let target = target.to_string_lossy();
        return Ok((digest::sha256_hex(target.as_bytes()), target.len() as u64));
    }
    let mut file =
        File::open(path.as_std_path()).map_err(|error| Error::io(path.to_string(), error))?;
    digest::sha256_hex_reader(&mut file).map_err(|error| Error::io(path.to_string(), error))
}

fn stage_error(root: &Utf8Path) -> impl FnOnce(std::io::Error) -> Error + use<'_> {
    move |error| {
        if error.kind() == std::io::ErrorKind::InvalidData {
            Error::validation(error.to_string())
        } else {
            Error::io(root.to_string(), error)
        }
    }
}

fn write_json(path: &Utf8Path, value: &impl Serialize) -> Result<()> {
    let body = serde_json::to_vec_pretty(value)
        .map_err(|error| Error::parse(path.to_string(), anyhow::Error::new(error)))?;
    write_atomic(path.as_std_path(), &body)
}

fn write_value(path: &Utf8Path, value: &serde_json::Value) -> Result<()> {
    write_json(path, value)
}

fn portable(path: &Utf8Path) -> String {
    path.as_str().replace('\\', "/")
}

fn recorded_path_matches(recorded: &str, expected: &Utf8Path) -> bool {
    let recorded = recorded.replace('\\', "/");
    let recorded = recorded.trim_end_matches('/');
    let expected = portable(expected);
    let expected = expected.trim_end_matches('/');
    if cfg!(windows) {
        recorded.eq_ignore_ascii_case(expected)
    } else {
        recorded == expected
    }
}

fn render_command(program: &Utf8Path, args: &[String]) -> String {
    std::iter::once(program.as_str())
        .chain(args.iter().map(String::as_str))
        .map(|value| {
            if value.contains(' ') {
                format!("\"{value}\"")
            } else {
                value.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn exit_detail(code: Option<i32>) -> String {
    code.map_or_else(
        || " (terminated by signal)".into(),
        |code| format!(" (exit {code})"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_named_archive_is_stable() {
        let id = "optionalAdapter";
        let version = "1.2.3";
        let target = "cy2026-linux-x86_64-py313-core";
        assert_eq!(
            format!("{id}-{version}-{target}.tar.zst"),
            "optionalAdapter-1.2.3-cy2026-linux-x86_64-py313-core.tar.zst"
        );
    }

    #[test]
    fn file_snapshot_detects_installed_byte_drift() {
        let root = std::env::temp_dir().join(format!(
            "ost-library-snapshot-{}-{}",
            std::process::id(),
            unix_now()
        ));
        let root = Utf8PathBuf::from_path_buf(root).unwrap();
        std::fs::create_dir_all(root.join("lib").as_std_path()).unwrap();
        let file = root.join("lib/adapter.bin");
        std::fs::write(file.as_std_path(), b"first").unwrap();
        let first = snapshot_files(&root).unwrap();
        std::fs::write(file.as_std_path(), b"second").unwrap();
        let second = snapshot_files(&root).unwrap();
        assert_ne!(first, second);
        assert_eq!(first[0].path, "lib/adapter.bin");
        std::fs::remove_dir_all(root.as_std_path()).unwrap();
    }

    #[test]
    fn recorded_paths_must_match_the_current_managed_tree() {
        let expected = Utf8Path::new("C:/checkout/library/.strata/targets/example/library-prefix");
        assert!(recorded_path_matches(
            "C:\\checkout\\library\\.strata\\targets\\example\\library-prefix/",
            expected
        ));
        assert!(!recorded_path_matches(
            "C:/other/library/.strata/targets/example/library-prefix",
            expected
        ));
    }

    #[test]
    fn generated_consumer_uses_only_declared_prefix_variables() {
        let manifest = ost_plugin::LibraryManifest::parse(
            "schema: openstrata.library/v1alpha1\nlibrary: { id: sample, version: 1.0.0 }\ncmake: { package: Sample, target: 'Sample::sample' }\npackage: { standalone: true, aggregate_member: true }\npackage_contract:\n  package_name: Sample\n  exported_targets: ['Sample::sample']\n  consumer: { include: sample/sample.hpp, symbol: 'Sample::version' }\n",
        )
        .unwrap();
        let contract = manifest.package_contract.as_ref().unwrap();
        let cmake = consumer_cmake(contract, 2);
        assert!(cmake.contains("OST_CONSUMER_PREFIX_0"));
        assert!(cmake.contains("OST_CONSUMER_PREFIX_1"));
        assert!(!cmake.contains("OST_CONSUMER_PREFIX_2"));
        assert!(cmake.contains("NO_DEFAULT_PATH"));
        let source = consumer_source(contract);
        assert!(source.contains("#include <sample/sample.hpp>"));
        assert!(source.contains("&Sample::version"));
    }
}
