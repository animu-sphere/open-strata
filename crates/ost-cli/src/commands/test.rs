// SPDX-License-Identifier: Apache-2.0
//! `ost test` — run a managed target's tests under the runtime that built it.
//!
//! This is a deliberate command rather than a mode of `ost build`: plain build
//! semantics do not change under anyone's feet, and a caller that wants tests
//! has to say so.
//!
//! ## What it is for
//!
//! A bare `ctest --test-dir build/<id>` — the shape v0.17.0's generated CI
//! emitted, and the shape the hdMerlin dogfooding pass ran — inherits none of
//! what the build was configured with. It sees the host PATH rather than the
//! runtime's, no MSVC environment, and no idea which configuration was built.
//! On Windows it then reports PASS from a test executable that never loaded its
//! USD DLLs, or hangs past the job timeout with its children orphaned.
//!
//! `ost test` propagates the build's own truth instead of re-deriving it:
//!
//! * the **runtime** and its environment, layered exactly as `ost build` layers
//!   it over the MSVC developer environment;
//! * the **configuration** and **generator**, read from the build completion
//!   record rather than accepted again from the caller — testing `Debug`
//!   binaries against a `Release` build is precisely the mismatch that record
//!   exists to prevent;
//! * the **build fingerprint**, so the resulting `tested` evidence names the
//!   build it exercised and stops being true when that build is replaced.
//!
//! Timeouts are enforced at both scopes — per test (CTest's own `--timeout`) and
//! over the whole run — and the overall one tears down the process *tree*, so a
//! hung test's children cannot outlive it and hold the job open.
//!
//! ## What it records
//!
//! A [`TestCompletion`] under the build directory, written whether the tests
//! passed or failed: that the run reached a conclusion is a different fact from
//! every test passing, and `ost validate` reports them differently. `tested` is
//! its own claim — not implied by `built`, not implied by `packaged`, and not
//! the same as a host-side plugin or renderer check.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};
use clap::Args;

use ost_build::{
    BuildCompletion, LeaseMode, TargetLease, TargetLock, TestCompletion, TestTotals,
    BUILD_COMPLETION_FILE, TARGET_LEASE_FILE, TEST_COMPLETION_FILE,
};
use ost_core::fs::write_atomic;
use ost_core::host::Os;
use ost_core::paths::STATE_DIR;
use ost_core::{tools, Error, Result};

use crate::commands::configure::{build_target, load_project, resolve_selection};
use crate::output::{self, Format};
use crate::progress::{ProgressMode, Reporter};

/// Where CTest writes the JUnit report we count results from.
///
/// Counting from a machine-readable report rather than scraping CTest's console
/// output means a test whose own stdout happens to look like a CTest summary
/// cannot forge the totals.
const JUNIT_FILE: &str = ".ost-test-results.xml";

#[derive(Debug, Args)]
pub struct TestArgs {
    /// Platform target, e.g. `cy2026`. Defaults to the project's platform.
    #[arg(long)]
    target: Option<String>,

    /// Profile to test. Defaults to the project's profile.
    #[arg(long)]
    profile: Option<String>,

    /// Only run tests whose name matches this regular expression (CTest `-R`).
    #[arg(long)]
    filter: Option<String>,

    /// Per-test timeout in seconds; 0 disables it.
    #[arg(long, default_value_t = 300)]
    test_timeout: u64,

    /// Timeout for the whole run in seconds; 0 disables it. On expiry the test
    /// process tree is terminated, not just the CTest process.
    #[arg(long, default_value_t = 3600)]
    timeout: u64,

    /// Parallel test jobs.
    #[arg(long)]
    jobs: Option<u32>,

    /// Path to the ctest executable if it is not on PATH.
    #[arg(long)]
    ctest: Option<String>,

    /// Do not auto-load the MSVC developer environment (Windows).
    #[arg(long)]
    no_vcvars: bool,

    /// Print the command that would run, without executing or writing anything.
    #[arg(long)]
    dry_run: bool,

    /// Progress rendering: `auto` (human on a TTY, plain otherwise), `plain`, or
    /// `json` (one JSON event per line).
    #[arg(long, value_enum, default_value_t = ProgressMode::Auto)]
    progress: ProgressMode,

    /// Suppress progress output; child output goes to the log.
    #[arg(long)]
    quiet: bool,

    /// Fire a desktop notification when the run finishes (no-op over SSH/CI).
    #[arg(long)]
    notify: bool,

    /// What to do when another invocation is already writing this target:
    /// `fail` immediately, `wait` for it (see --busy-timeout), or `read-only`
    /// to proceed without taking the target lease.
    #[arg(long, default_value = "fail", value_parser = ["fail", "wait", "read-only"])]
    on_busy: String,

    /// How long `--on-busy wait` waits, in seconds; 0 waits indefinitely.
    #[arg(long, default_value_t = 600)]
    busy_timeout: u64,
}

pub fn run(args: TestArgs, fmt: Format) -> Result<()> {
    let (root, platform, profile) = resolve_selection(args.target.clone(), args.profile.clone())?;
    let (target, resolved) = build_target(&platform, &profile)?;
    let id = target.id();

    // 1. A test run is only meaningful against a completed build, and the build
    //    record is also where the configuration and generator come from. Read it
    //    before anything else so an untested-because-unbuilt target says so.
    let relative_build_dir = Utf8PathBuf::from(format!("build/{id}"));
    let build_dir = root.join(&relative_build_dir);
    let project = load_project(&root)?;
    let project_version = project.effective_version(&root)?;
    let lock = read_lock(&root, &id)?;
    let build = read_build_completion(&build_dir)?;
    build
        .validate_against(
            &lock,
            &project.project.name,
            &project_version,
            &relative_build_dir,
        )
        .map_err(|detail| {
            Error::precondition(format!("target '{id}' is not built: {detail}"))
                .with_hint("run `ost build` before `ost test`")
        })?;

    // The configuration is propagated, not re-chosen: CMAKE_BUILD_TYPE as the
    // build actually used it. A multi-config generator needs it at test time
    // too, which is the case a re-specified default silently gets wrong.
    let configuration = build
        .intent
        .cache
        .get("CMAKE_BUILD_TYPE")
        .cloned()
        .unwrap_or_else(|| "Release".to_string());

    if !resolved.pulled {
        return Err(
            Error::precondition(format!("runtime '{}' not pulled", target.runtime_id)).with_hint(
                format!(
                    "run `ost runtime pull {} --profile {}` first",
                    target.platform, target.profile
                ),
            ),
        );
    }

    let ctest_prog = locate_ctest(args.ctest.as_deref())?;
    let junit_path = build_dir.join(JUNIT_FILE);
    let ctest_args = ctest_args(&relative_build_dir, &configuration, &junit_path, &args);

    // 2. `--dry-run`: show the planned command and stop without writing.
    if args.dry_run {
        let command = render_cmd(&ctest_prog, &ctest_args);
        if fmt.is_json() {
            output::success(&serde_json::json!({
                "dry_run": true,
                "target": id,
                "configuration": configuration,
                "generator": build.generator,
                "build_fingerprint": build.fingerprint(),
                "command": command,
            }));
        } else {
            println!("# dry run — would execute in {root}:");
            println!("{command}");
            println!("# configuration: {configuration} (from the build record)");
            println!("# generator:     {}", build.generator);
        }
        return Ok(());
    }

    // 3. Testing writes into the build tree (CTest's own state, the JUnit report
    //    and the completion record), so it is a writer of the target and takes
    //    the same lease configure and build take.
    let mode = LeaseMode::parse(&args.on_busy, args.busy_timeout)?;
    let lease_path = root
        .join(STATE_DIR)
        .join("targets")
        .join(&id)
        .join(TARGET_LEASE_FILE);
    let lease = TargetLease::acquire(&lease_path, &id, "ost test", mode)?;

    let mut rep = Reporter::new(args.progress, 1, args.quiet).with_notify(args.notify, "ost test");
    if let Some(takeover) = lease.takeover() {
        rep.note(&takeover.describe());
    }
    let log = root
        .join(STATE_DIR)
        .join("targets")
        .join(&id)
        .join("test.log");
    rep.set_log(&log);

    // 4. The same environment layering `ost build` uses: the MSVC delta first,
    //    then the runtime resolved over it so USD's bin/lib prepend ahead of the
    //    compiler's PATH. Without this a test binary loads the host's libraries,
    //    or none at all.
    let mut msvc_env: Vec<(String, String)> = Vec::new();
    if target.os() == Os::Windows && !args.no_vcvars && tools::which("cl").is_none() {
        match ost_build::msvc::bootstrap() {
            Ok(Some(env)) => msvc_env = env.vars,
            Ok(None) => {}
            Err(error) => eprintln!("warning: failed to load the MSVC environment: {error}"),
        }
    }
    let extra_env = crate::commands::build::layer_runtime_env(&resolved.env, &msvc_env);
    rep.note(&format!(
        "runtime env {} ({} vars)",
        target.runtime_id,
        resolved.env.vars.len()
    ));
    if let Some(invocation) = lease.invocation() {
        rep.note(&format!("target lease {invocation}"));
    }

    // A stale report from a previous run must not be counted as this one's.
    let _ = std::fs::remove_file(junit_path.as_std_path());

    // 5. Run CTest. Its exit status comes back rather than ending the process,
    //    because a failing run still has evidence to publish.
    rep.phase("Running tests");
    let status = rep.run_status(
        &ctest_prog,
        &ctest_args,
        &root,
        &extra_env,
        (args.timeout > 0).then(|| Duration::from_secs(args.timeout)),
    )?;

    // 6. Record what happened — pass or fail — then report it.
    //
    //    Evidence requires that tests actually ran. A CTest that failed before
    //    executing anything (an empty suite, a bad filter, a usage error) leaves
    //    nothing true to say about the target, so no record is published: an
    //    absent `tested` claim is the honest outcome, not a zeroed one.
    let Some(totals) = read_totals(&junit_path).filter(|totals| totals.total > 0) else {
        lease.release();
        rep.done();
        return Err(
            Error::external_tool(format!("no tests ran for target '{id}'")).with_hint(format!(
                "register tests with add_test() in CMake, or relax --filter; see {log}"
            )),
        );
    };
    let completed_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut completion = TestCompletion::new(&build, &configuration, totals, completed_unix);
    if let Some(invocation) = lease.invocation() {
        completion = completion.with_invocation(invocation);
    }
    let body = completion
        .to_json()
        .map_err(|error| Error::parse(TEST_COMPLETION_FILE, anyhow::Error::new(error)))?;
    write_atomic(
        build_dir.join(TEST_COMPLETION_FILE).as_std_path(),
        format!("{body}\n").as_bytes(),
    )?;
    lease.release();

    if !status.success() {
        rep.done();
        // The record is already on disk, so the failure can be described from
        // it rather than from an exit code alone.
        //
        // CTest can also exit non-zero with no failure of its own recorded — a
        // run cut short by the overall timeout, or killed. Rounding that up to
        // "1 test failed" would invent a count no report backs, which is the
        // kind of claim this release exists to stop making; say what is known
        // instead.
        let detail = if totals.failed > 0 {
            format!(
                "{} of {} tests failed in target '{id}'",
                totals.failed, totals.total
            )
        } else {
            format!(
                "the test run for target '{id}' did not complete successfully \
                 (ctest exited {}), though none of its {} tests reported a failure",
                status
                    .code()
                    .map_or_else(|| "on a signal".to_string(), |code| code.to_string()),
                totals.total
            )
        };
        return Err(
            Error::external_tool(detail).with_hint(format!("see {log} for the failing output"))
        );
    }

    rep.done();
    if fmt.is_json() {
        output::success(&serde_json::json!({
            "target": id,
            "configuration": configuration,
            "build_fingerprint": completion.build_fingerprint,
            "invocation": completion.invocation,
            "totals": completion.totals,
        }));
    } else {
        rep.note(&format!(
            "Tested target {id}: {} of {} passed ({configuration})",
            totals.passed, totals.total
        ));
    }
    Ok(())
}

fn read_lock(root: &Utf8Path, id: &str) -> Result<TargetLock> {
    let path = root
        .join(STATE_DIR)
        .join("targets")
        .join(id)
        .join("target.lock.json");
    let source = std::fs::read_to_string(path.as_std_path()).map_err(|_| {
        Error::precondition(format!("target '{id}' is not configured"))
            .with_hint("run `ost configure` (or `ost build`) first")
    })?;
    serde_json::from_str(&source)
        .map_err(|error| Error::parse(path.to_string(), anyhow::Error::new(error)))
}

fn read_build_completion(build_dir: &Utf8Path) -> Result<BuildCompletion> {
    let path = build_dir.join(BUILD_COMPLETION_FILE);
    let source = std::fs::read_to_string(path.as_std_path()).map_err(|_| {
        Error::precondition(format!("no completed build at {build_dir}"))
            .with_hint("run `ost build` before `ost test`")
    })?;
    serde_json::from_str(&source)
        .map_err(|error| Error::parse(path.to_string(), anyhow::Error::new(error)))
}

fn locate_ctest(override_path: Option<&str>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return Ok(candidate);
        }
        return Err(Error::precondition(format!("ctest not found at '{path}'")));
    }
    tools::which("ctest").ok_or_else(|| {
        Error::precondition("`ctest` not found on PATH")
            .with_hint("install CMake 3.23 or later and add it to PATH, or pass --ctest <path>")
    })
}

/// Build CTest's argument list.
fn ctest_args(
    build_dir: &Utf8Path,
    configuration: &str,
    junit_path: &Utf8Path,
    args: &TestArgs,
) -> Vec<String> {
    let mut out = vec![
        "--test-dir".to_string(),
        build_dir.as_str().replace('\\', "/"),
        // Multi-config generators need this to find the right binaries; it is
        // harmless for single-config ones.
        "--build-config".to_string(),
        configuration.to_string(),
        "--output-on-failure".to_string(),
        "--output-junit".to_string(),
        junit_path.as_str().replace('\\', "/"),
        // A suite with nothing in it is CTest's quiet success by default, which
        // would let `ost test` publish a `tested` record asserting "0 of 0
        // passed" — a PASS bound to no work at all, which is the exact defect
        // class this release exists to remove.
        "--no-tests=error".to_string(),
    ];
    if args.test_timeout > 0 {
        out.push("--timeout".to_string());
        out.push(args.test_timeout.to_string());
    }
    if let Some(jobs) = args.jobs {
        out.push("-j".to_string());
        out.push(jobs.to_string());
    }
    if let Some(filter) = &args.filter {
        out.push("-R".to_string());
        out.push(filter.clone());
    }
    out
}

/// Count results from CTest's JUnit report.
///
/// Only the `<testsuite>` element's own attributes are read; that is where CTest
/// puts the run totals, and reading them beats re-deriving counts from the
/// individual cases.
fn read_totals(junit_path: &Utf8Path) -> Option<TestTotals> {
    let xml = std::fs::read_to_string(junit_path.as_std_path()).ok()?;
    let open = xml.find("<testsuite")?;
    let end = xml[open..].find('>')? + open;
    let tag = &xml[open..end];

    let total = attribute(tag, "tests").unwrap_or(0);
    let failures = attribute(tag, "failures").unwrap_or(0);
    let errors = attribute(tag, "errors").unwrap_or(0);
    let skipped = attribute(tag, "skipped").unwrap_or(0);
    // An errored test did not pass, so it counts against the run just as a
    // failing assertion does.
    let failed = failures + errors;
    Some(TestTotals {
        total,
        passed: total.saturating_sub(failed + skipped),
        failed,
    })
}

/// Read one integer XML attribute out of an element's opening tag.
fn attribute(tag: &str, name: &str) -> Option<u32> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    rest[..end].parse().ok()
}

fn render_cmd(program: &std::path::Path, args: &[String]) -> String {
    let mut rendered = quote(&program.display().to_string());
    for arg in args {
        rendered.push(' ');
        rendered.push_str(&quote(arg));
    }
    rendered
}

fn quote(value: &str) -> String {
    if value.contains(' ') {
        format!("\"{value}\"")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> TestArgs {
        TestArgs {
            target: None,
            profile: None,
            filter: None,
            test_timeout: 300,
            timeout: 3600,
            jobs: None,
            ctest: None,
            no_vcvars: false,
            dry_run: false,
            progress: ProgressMode::Auto,
            quiet: false,
            notify: false,
            on_busy: "fail".into(),
            busy_timeout: 600,
        }
    }

    /// The configuration and a per-test timeout must both reach CTest: the first
    /// is how a multi-config build finds its binaries, the second is what stops
    /// one hung test from consuming the whole run's budget.
    #[test]
    fn ctest_args_carry_configuration_and_per_test_timeout() {
        let built = ctest_args(
            Utf8Path::new("build/cy2026-linux-x86_64-py313-usd"),
            "RelWithDebInfo",
            Utf8Path::new("build/x/.ost-test-results.xml"),
            &args(),
        );
        let joined = built.join(" ");
        assert!(joined.contains("--test-dir build/cy2026-linux-x86_64-py313-usd"));
        assert!(joined.contains("--build-config RelWithDebInfo"));
        assert!(joined.contains("--timeout 300"));
        assert!(joined.contains("--output-junit"));
    }

    #[test]
    fn per_test_timeout_can_be_disabled_and_filters_pass_through() {
        let mut a = args();
        a.test_timeout = 0;
        a.filter = Some("^renderer".into());
        a.jobs = Some(4);
        let built = ctest_args(
            Utf8Path::new("build/x"),
            "Release",
            Utf8Path::new("build/x/r.xml"),
            &a,
        );
        let joined = built.join(" ");
        assert!(!joined.contains("--timeout"), "0 disables it: {joined}");
        assert!(joined.contains("-R ^renderer"));
        assert!(joined.contains("-j 4"));
    }

    /// Paths handed to CTest are forward-slashed, so a Windows build directory
    /// does not reach it with separators it treats as escapes.
    #[test]
    fn ctest_paths_are_forward_slashed() {
        let built = ctest_args(
            Utf8Path::new("build\\cy2026-windows"),
            "Release",
            Utf8Path::new("build\\cy2026-windows\\r.xml"),
            &args(),
        );
        assert!(built.iter().all(|arg| !arg.contains('\\')), "{built:?}");
    }

    fn junit(attrs: &str) -> Utf8PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ost-junit-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = Utf8PathBuf::from_path_buf(dir.join("r.xml")).unwrap();
        std::fs::write(
            path.as_std_path(),
            format!("<?xml version=\"1.0\"?>\n<testsuite {attrs}>\n</testsuite>\n"),
        )
        .unwrap();
        path
    }

    #[test]
    fn totals_come_from_the_junit_report() {
        let path = junit("name=\"x\" tests=\"10\" failures=\"2\" errors=\"0\" skipped=\"1\"");
        let totals = read_totals(&path).expect("totals parse");
        assert_eq!(totals.total, 10);
        assert_eq!(totals.failed, 2);
        // Skipped tests did not pass either.
        assert_eq!(totals.passed, 7);
        std::fs::remove_dir_all(path.parent().unwrap().as_std_path()).ok();
    }

    /// A test that errored out never passed, so it must count as a failure
    /// rather than quietly inflating the passed column.
    #[test]
    fn errored_tests_count_as_failures() {
        let path = junit("tests=\"4\" failures=\"1\" errors=\"1\" skipped=\"0\"");
        let totals = read_totals(&path).expect("totals parse");
        assert_eq!(totals.failed, 2);
        assert_eq!(totals.passed, 2);
        std::fs::remove_dir_all(path.parent().unwrap().as_std_path()).ok();
    }

    /// No report means no counts to invent — the run is recorded with zeroes
    /// rather than an assumed pass.
    #[test]
    fn a_missing_report_yields_no_totals() {
        assert!(read_totals(Utf8Path::new("does/not/exist.xml")).is_none());
        assert_eq!(TestTotals::default().total, 0);
    }
}
