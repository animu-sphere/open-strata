// SPDX-License-Identifier: Apache-2.0
//! `ost library` — build, test, and package one descriptor-owned CMake library.
//!
//! Plain libraries participate in plugin workspace composition, but an optional
//! adapter also needs to remain independently shippable. These commands reuse
//! the workspace library builder while giving one descriptor its own install
//! prefix and digest-bound completion record.

use std::fs::File;
use std::process::Command;
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
    files: Vec<LibraryFile>,
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
    let library = Library::load(root)?;
    let (platform, profile) = selection(target, profile)?;
    let (target, resolved) = build_target(&platform, &profile)?;
    let id = target.id();
    let prefix = isolated_prefix(&library, &id);

    if !dry_run {
        remove_existing_prefix(&prefix)?;
    }
    plugin::build_library_one(
        &library,
        Some(platform),
        Some(profile),
        dry_run,
        ninja,
        compiler,
        &prefix,
    )?;
    if dry_run {
        return Ok(());
    }

    let files = snapshot_files(&prefix)?;
    if files.is_empty() {
        return Err(Error::validation(format!(
            "library '{}' installed no files into '{prefix}'",
            library.id()
        ))
        .with_hint("add CMake install rules for the library target, headers, and config package"));
    }
    let runtime = runtime_identity(&resolved.prefix, &target.runtime_id)?;
    let record = LibraryBuildRecord {
        schema: LIBRARY_BUILD_SCHEMA.into(),
        library: LibraryIdentity {
            id: library.id().into(),
            version: library.version().into(),
        },
        target: id.clone(),
        runtime,
        descriptor_sha256: descriptor_digest(&library)?,
        build_dir: portable(&plugin::target_build_dir(&library.root, &id)),
        install_prefix: portable(&prefix),
        files,
    };
    write_json(&build_record_path(&library, &id), &record)?;

    if fmt.is_json() {
        output::success(&serde_json::json!({
            "built": true,
            "library": record.library,
            "target": record.target,
            "runtime": record.runtime,
            "build_dir": record.build_dir,
            "install_prefix": record.install_prefix,
            "files": record.files.len(),
            "record": build_record_path(&library, &id),
        }));
    } else {
        println!("Built library {} {}", library.id(), library.version());
        println!("  target:  {id}");
        println!("  install: {prefix}");
        println!("  files:   {}", record.files.len());
        println!("  record:  {}", build_record_path(&library, &id));
    }
    Ok(())
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
    resolved.env.apply(&mut command);
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
            "kind": "library",
            "descriptor": LIBRARY_MANIFEST,
            "descriptor_sha256": record.descriptor_sha256,
            "cmake": {
                "package": library.manifest.cmake.package,
                "target": library.manifest.cmake.target,
            },
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
            "files": packed.files.len(),
        }));
    } else {
        println!("Packaged library {} {}", library.id(), library.version());
        println!("  target:  {}", target.id());
        println!("  archive: {archive}");
        println!("  digest:  {}", packed.archive_digest);
        println!("  files:   {}", packed.files.len());
    }
    Ok(())
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
}
