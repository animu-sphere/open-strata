// SPDX-License-Identifier: Apache-2.0
//! `ost build` — configure and build a target with CMake + Ninja (§8.2).
//!
//! `ost build` regenerates the target's CMake files (same as `ost configure`),
//! then drives CMake: `cmake --preset <id>` to configure, `cmake --build` to
//! compile. Ninja is located on PATH, via `OST_NINJA`, or `--ninja <path>`, and
//! passed to CMake as `CMAKE_MAKE_PROGRAM` so it works even off PATH.
//!
//! OpenStrata decides *what* to build; CMake/Ninja remain the build truth.
//!
//! Order of operations (P0 "no side effects before checks"):
//!   1. resolve project root + manifest, then platform/profile/runtime
//!   2. preflight: CMakeLists.txt, runtime, CMake, Ninja, compiler
//!   3. `--check`  → report checks and stop (no writes)
//!   4. `--dry-run`→ show planned commands + files and stop (no writes)
//!   5. generate the target's `.strata/` files
//!   6. CMake configure, then CMake build

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};
use clap::Args;

use ost_build::{
    BuildCompletion, BuildIntent, BuildProjectIdentity, Target, TargetLock, BUILD_COMPLETION_FILE,
};
use ost_core::fs::write_atomic;
use ost_core::host::Os;
use ost_core::paths::STATE_DIR;
use ost_core::{tools, Error, Result};
use ost_runtime::EnvSet;

use crate::commands::compiler::CompilerOpts;
use crate::commands::configure::{
    build_target_with_generator, generate_with_generator, load_project, resolve_compiler,
    resolve_selection, target_output_paths,
};
use crate::output::{self, Format};
use crate::progress::{ProgressMode, Reporter};

#[derive(Debug, Args)]
pub struct BuildArgs {
    /// Platform target, e.g. `cy2026`. Defaults to the project's platform.
    #[arg(long)]
    target: Option<String>,

    /// Profile to build. Defaults to the project's profile.
    #[arg(long)]
    profile: Option<String>,

    /// CMake generator. Ninja remains the default.
    #[arg(long, default_value = "Ninja")]
    generator: String,

    /// Build configuration (CMAKE_BUILD_TYPE and multi-config --config).
    #[arg(long, default_value = "Release")]
    config: String,

    /// Run preflight checks only, without generating files or building.
    #[arg(long)]
    check: bool,

    /// Print the commands and files that would be produced, without executing
    /// or writing anything.
    #[arg(long)]
    dry_run: bool,

    /// Parallel jobs: a number, or `auto` to let Ninja decide.
    #[arg(long)]
    jobs: Option<String>,

    /// Path to the ninja executable if it is not on PATH.
    #[arg(long)]
    ninja: Option<String>,

    /// Do not auto-load the MSVC developer environment (Windows).
    #[arg(long)]
    no_vcvars: bool,

    /// Progress rendering: `auto` (human on a TTY, plain otherwise), `plain`, or
    /// `json` (one JSON event per line).
    #[arg(long, value_enum, default_value_t = ProgressMode::Auto)]
    progress: ProgressMode,

    /// Suppress progress output; child output goes to the log. Failures, the
    /// exit code and the log path are still reported.
    #[arg(long)]
    quiet: bool,

    /// Fire a desktop notification when the build finishes (no-op over SSH/CI).
    #[arg(long)]
    notify: bool,

    /// Configure timeout in seconds; 0 disables it.
    #[arg(long, default_value_t = 600)]
    configure_timeout: u64,

    /// Build timeout in seconds; 0 disables it.
    #[arg(long, default_value_t = 7200)]
    build_timeout: u64,

    #[command(flatten)]
    compiler: CompilerOpts,
}

impl BuildArgs {
    /// Internal managed-build request used by domain workflows such as
    /// `renderer view`. It deliberately shares the ordinary build lifecycle
    /// instead of spawning another `ost` process or inventing a second builder.
    pub(crate) fn managed(
        target: String,
        profile: String,
        generator: Option<String>,
        config: String,
    ) -> Self {
        Self {
            target: Some(target),
            profile: Some(profile),
            generator: generator.unwrap_or_else(|| "Ninja".into()),
            config,
            check: false,
            dry_run: false,
            jobs: None,
            ninja: None,
            no_vcvars: false,
            progress: ProgressMode::Auto,
            quiet: false,
            notify: false,
            configure_timeout: 600,
            build_timeout: 7200,
            compiler: CompilerOpts::default(),
        }
    }
}

pub fn run(args: BuildArgs, fmt: Format) -> Result<()> {
    run_with_intent(args, fmt, BuildIntent::default())
}

pub(crate) fn run_with_intent(args: BuildArgs, fmt: Format, intent: BuildIntent) -> Result<()> {
    // 1. Resolve the project and the effective target. `build_target` resolves
    //    the runtime without writing anything, so checks and dry-run stay free
    //    of side effects.
    let (root, platform, profile) = resolve_selection(args.target.clone(), args.profile.clone())?;
    if args.generator.trim().is_empty() {
        return Err(Error::usage("--generator must not be empty"));
    }
    let (target, resolved) = build_target_with_generator(&platform, &profile, &args.generator)?;
    let id = target.id();

    // Resolve the compiler policy early so an invalid one fails before any work.
    let compiler = resolve_compiler(&root, &args.compiler)?;

    // 2. Preflight: gather every check without touching the work tree.
    let pre = preflight(&root, &target, resolved.pulled, &args);

    // 3. `--check`: report and stop. Non-zero exit if any required check failed.
    if args.check {
        report_checks(&id, &pre, fmt);
        // report_checks emitted this command's report (its `ok` carries the
        // outcome); a failing required check is a missing precondition, so exit
        // with that category code directly rather than returning an Err that
        // would render a second document on stdout (§14.3/§14.4).
        if pre.first_failure().is_some() {
            std::process::exit(ost_core::Category::Precondition.exit_code() as i32);
        }
        return Ok(());
    }

    // For a real or planned build, a failing required check is fatal — and we
    // surface it before any file is written.
    if let Some(failure) = pre.first_failure() {
        return Err(failure.to_error());
    }

    let cmake_prog = pre.cmake.clone().unwrap_or_else(|| PathBuf::from("cmake"));
    // CMake wants forward slashes even on Windows.
    let ninja_arg = args
        .generator
        .eq_ignore_ascii_case("Ninja")
        .then(|| {
            pre.ninja
                .as_ref()
                .map(|p| p.display().to_string().replace('\\', "/"))
        })
        .flatten();

    let mut configure_args = vec!["--preset".to_string(), id.clone()];
    if let Some(np) = &ninja_arg {
        configure_args.push(format!("-DCMAKE_MAKE_PROGRAM={np}"));
    }
    let mut intent = intent;
    intent
        .cache
        .entry("CMAKE_BUILD_TYPE".into())
        .or_insert_with(|| args.config.clone());
    for (key, value) in &intent.cache {
        configure_args.push(format!("-D{key}={value}"));
    }

    let mut build_args = vec![
        "--build".to_string(),
        format!("build/{id}"),
        "--config".to_string(),
        args.config.clone(),
    ];
    if let Some(jobs) = &args.jobs {
        if let Ok(n) = jobs.parse::<u32>() {
            build_args.push("-j".to_string());
            build_args.push(n.to_string());
        }
    }

    // 4. `--dry-run`: show the planned commands and the files that would be
    //    generated, then stop without writing anything.
    if args.dry_run {
        let configure_cmd = render_cmd(&cmake_prog, &configure_args);
        let build_cmd = render_cmd(&cmake_prog, &build_args);
        let mut files = target_output_paths(&id);
        files.push(format!("build/{id}/{BUILD_COMPLETION_FILE}"));

        // Surface the runtime env additions (the OpenStrata-managed prepends,
        // not the inherited environment) so they can be inspected without a run.
        let runtime_env = resolved.env.pairs();

        if fmt.is_json() {
            // Emit ordered [key, value] pairs, not an object: a single `EnvSet`
            // can carry the same key more than once (on Windows both `bin` and
            // `lib` prepend `PATH`), and an object would collapse those to the
            // last entry, silently dropping prepends.
            let env_pairs: Vec<serde_json::Value> = runtime_env
                .iter()
                .map(|(k, v)| serde_json::json!([k, v]))
                .collect();
            output::success(&serde_json::json!({
                "dry_run": true,
                "target": id,
                "root": root.to_string(),
                "bootstrap_msvc": pre.will_bootstrap_msvc,
                "build_intent": intent,
                "commands": [configure_cmd, build_cmd],
                "would_generate": files,
                "runtime_env": env_pairs,
            }));
            return Ok(());
        }

        println!("# dry run — would execute in {root}:");
        if pre.will_bootstrap_msvc {
            println!("# (would auto-load the MSVC environment via vcvars64.bat)");
        }
        println!("{configure_cmd}");
        println!("{build_cmd}");
        println!("# would apply runtime env (prepended):");
        for (k, v) in &runtime_env {
            println!("#   {k}={v}");
        }
        println!("# would generate:");
        for f in &files {
            println!("#   {f}");
        }
        return Ok(());
    }

    // The real build runs as timed phases through the shared reporter: generate,
    // configure, build, verify. The reporter renders for a TTY, CI or as a JSON
    // event stream, tees child output to the per-target log, names the failing
    // phase + exit code, and (with --notify) fires a desktop toast at the end.
    let mut rep = Reporter::new(args.progress, 4, args.quiet).with_notify(args.notify, "ost build");

    // 5. Generate the target's `.strata/` files now that checks have passed.
    rep.phase("Generating toolchain and presets");
    let build_dir = root.join("build").join(&id);
    invalidate_completion(&build_dir)?;
    let g = generate_with_generator(&root, &platform, &profile, &compiler, &args.generator)?;
    debug_assert_eq!(g.id, id);
    // Subprocess output from here on is teed to a per-target build log.
    let log = root
        .join(".strata")
        .join("targets")
        .join(&id)
        .join("build.log");
    rep.set_log(&log);

    // 6. Inject the MSVC developer environment (cl.exe, Windows SDK) if needed.
    let mut msvc_env: Vec<(String, String)> = Vec::new();
    if pre.will_bootstrap_msvc {
        match ost_build::msvc::bootstrap() {
            Ok(Some(env)) => {
                rep.note(&format!(
                    "msvc env   {} ({} vars)",
                    env.vcvars.display(),
                    env.vars.len()
                ));
                msvc_env = env.vars;
            }
            Ok(None) => eprintln!(
                "warning: MSVC not found; relying on the current environment (cl must be on PATH)"
            ),
            Err(e) => eprintln!("warning: failed to load the MSVC environment: {e}"),
        }
    }

    // 7. Apply the *runtime* environment to CMake and Ninja too, not just to
    //    `ost run`/`test`. Without it configure/build see a different PATH,
    //    PYTHONPATH, loader path and CMAKE_PREFIX_PATH than execution does.
    //    Layer it over the MSVC delta so USD's bin/lib prepend in front of the
    //    compiler's PATH rather than clobbering it.
    let extra_env = layer_runtime_env(&resolved.env, &msvc_env);
    rep.note(&format!(
        "runtime env {} ({} vars)",
        target.runtime_id,
        resolved.env.vars.len()
    ));

    // 8. Configure, then build — each a phase whose subprocess streams through
    //    the reporter (heartbeat while quiet, log capture, failure reporting).
    rep.phase("Configuring CMake");
    rep.run(
        &cmake_prog,
        &configure_args,
        &root,
        &extra_env,
        timeout(args.configure_timeout),
    )?;
    rep.phase("Building targets");
    rep.run(
        &cmake_prog,
        &build_args,
        &root,
        &extra_env,
        timeout(args.build_timeout),
    )?;

    // 9. Verify the build produced outputs — a successful build with an empty
    //    tree means the preset built nothing useful.
    rep.phase("Verifying outputs");
    verify_build(&root, &id)?;
    write_completion(&root, &id, &intent)?;

    rep.done();
    rep.note(&format!("Built target {id}"));
    Ok(())
}

fn timeout(seconds: u64) -> Option<Duration> {
    (seconds > 0).then(|| Duration::from_secs(seconds))
}

fn invalidate_completion(build_dir: &Utf8Path) -> Result<()> {
    let path = build_dir.join(BUILD_COMPLETION_FILE);
    match std::fs::remove_file(path.as_std_path()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io(path.to_string(), error)),
    }
}

fn write_completion(root: &Utf8Path, id: &str, intent: &BuildIntent) -> Result<()> {
    let lock_path = root
        .join(STATE_DIR)
        .join("targets")
        .join(id)
        .join("target.lock.json");
    let source = std::fs::read_to_string(lock_path.as_std_path())
        .map_err(|error| Error::io(lock_path.to_string(), error))?;
    let lock: TargetLock = serde_json::from_str(&source)
        .map_err(|error| Error::parse(lock_path.to_string(), anyhow::Error::new(error)))?;
    let project = load_project(root)?;
    let project_version = project.effective_version(root)?;
    let relative_build = Utf8PathBuf::from(format!("build/{id}"));
    let completed_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let completion = BuildCompletion::from_lock(
        &lock,
        BuildProjectIdentity {
            name: project.project.name,
            version: project_version,
        },
        relative_build.as_str(),
        intent.clone(),
        completed_unix,
    );
    let body = completion
        .to_json()
        .map_err(|error| Error::parse(BUILD_COMPLETION_FILE, anyhow::Error::new(error)))?;
    let path = root.join(&relative_build).join(BUILD_COMPLETION_FILE);
    write_atomic(path.as_std_path(), format!("{body}\n").as_bytes())
}

/// Confirm `build/<id>` exists and is non-empty after a successful build, so a
/// no-op preset does not pass silently.
fn verify_build(root: &Utf8Path, id: &str) -> Result<()> {
    let build_dir = root.join("build").join(id);
    let non_empty = std::fs::read_dir(build_dir.as_std_path())
        .map(|mut d| d.next().is_some())
        .unwrap_or(false);
    if !non_empty {
        return Err(Error::validation(format!(
            "build completed but produced no outputs under build/{id}"
        )));
    }
    Ok(())
}

/// A single preflight check and its outcome.
struct Check {
    name: &'static str,
    status: Status,
}

enum Status {
    /// Required check passed; carries a short human detail.
    Ok(String),
    /// Required check failed; carries the reason and an actionable hint.
    Failed { detail: String, hint: String },
    /// Non-gating information (e.g. detected compiler).
    Info(String),
}

impl Check {
    fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Check {
            name,
            status: Status::Ok(detail.into()),
        }
    }
    fn failed(name: &'static str, detail: impl Into<String>, hint: impl Into<String>) -> Self {
        Check {
            name,
            status: Status::Failed {
                detail: detail.into(),
                hint: hint.into(),
            },
        }
    }
    fn info(name: &'static str, detail: impl Into<String>) -> Self {
        Check {
            name,
            status: Status::Info(detail.into()),
        }
    }
    /// Render a failed check as an actionable error (used on the build path).
    /// A failed required check is a missing prerequisite (runtime, tool); carry
    /// the hint in the structured slot so `--json` surfaces it as `error.hint`
    /// rather than inlining it into the message (§14.3/§14.4).
    fn to_error(&self) -> Error {
        match &self.status {
            Status::Failed { detail, hint } => {
                Error::precondition(detail.clone()).with_hint(hint.clone())
            }
            _ => Error::precondition(format!("check '{}' failed", self.name)),
        }
    }
}

/// The outcome of preflight: the per-item checks plus the located tools the
/// build path will reuse.
struct Preflight {
    checks: Vec<Check>,
    cmake: Option<PathBuf>,
    ninja: Option<PathBuf>,
    will_bootstrap_msvc: bool,
}

impl Preflight {
    /// First failed required check, if any.
    fn first_failure(&self) -> Option<&Check> {
        self.checks
            .iter()
            .find(|c| matches!(c.status, Status::Failed { .. }))
    }
}

/// Run every preflight check without writing to the work tree.
fn preflight(root: &Utf8Path, target: &Target, pulled: bool, args: &BuildArgs) -> Preflight {
    let mut checks = Vec::new();

    // CMakeLists.txt in the project root — without it CMake fails with a raw,
    // confusing error, so we diagnose it ourselves (P0 first-build path).
    let cml = root.join("CMakeLists.txt");
    if cml.as_std_path().is_file() {
        checks.push(Check::ok("CMakeLists.txt", cml.to_string()));
    } else {
        checks.push(Check::failed(
            "CMakeLists.txt",
            "no CMakeLists.txt found in project root",
            "run `ost init --template cpp-library`, or use `ost init --bare` for an existing CMake project",
        ));
    }

    // The runtime must be pulled before a real build.
    if pulled {
        checks.push(Check::ok("runtime", target.runtime_id.clone()));
    } else {
        checks.push(Check::failed(
            "runtime",
            format!("runtime '{}' not pulled", target.runtime_id),
            format!(
                "run `ost runtime pull {} --profile {}` first",
                target.platform, target.profile
            ),
        ));
    }

    // CMake itself.
    let cmake = locate("cmake", None);
    match &cmake {
        Some(p) => checks.push(Check::ok("cmake", p.display().to_string())),
        None => checks.push(Check::failed(
            "cmake",
            "`cmake` not found on PATH",
            "install CMake 3.23 or later and add it to PATH",
        )),
    }

    // Ninja. On Windows the MSVC developer environment we auto-load also puts a
    // Ninja on PATH, so an explicit one is not strictly required there.
    let wants_ninja = args.generator.eq_ignore_ascii_case("Ninja");
    let ninja = wants_ninja
        .then(|| {
            locate(
                "ninja",
                args.ninja
                    .clone()
                    .or_else(|| std::env::var("OST_NINJA").ok()),
            )
        })
        .flatten();
    let will_bootstrap_msvc =
        target.os() == Os::Windows && !args.no_vcvars && tools::which("cl").is_none();
    match (&ninja, wants_ninja) {
        (Some(p), _) => checks.push(Check::ok("ninja", p.display().to_string())),
        (None, true) if will_bootstrap_msvc => checks.push(Check::ok(
            "ninja",
            "not on PATH; expected from the MSVC developer environment (vcvars64.bat)",
        )),
        (None, true) => checks.push(Check::failed(
            "ninja",
            "`ninja` not found",
            "add it to PATH, set OST_NINJA, or pass --ninja <path>",
        )),
        (None, false) => checks.push(Check::info(
            "generator",
            format!("{} (selected explicitly)", args.generator),
        )),
    }

    // Compiler detection is informational here; compiler *policy* (host vs
    // runtime vs explicit) is selected at configure time.
    checks.push(Check::info(
        "compiler",
        detect_compiler(target.os(), will_bootstrap_msvc),
    ));

    Preflight {
        checks,
        cmake,
        ninja,
        will_bootstrap_msvc,
    }
}

/// Best-effort host compiler discovery for the preflight report.
fn detect_compiler(os: Os, will_bootstrap_msvc: bool) -> String {
    if os == Os::Windows {
        if will_bootstrap_msvc {
            return "MSVC (loaded via vcvars64.bat)".to_string();
        }
        if let Some(p) = tools::which("cl") {
            return format!("MSVC: {}", p.display());
        }
    }
    for c in ["cc", "clang", "gcc"] {
        if let Some(p) = tools::which(c) {
            return format!("{c}: {}", p.display());
        }
    }
    "not detected (CMake will search at configure time)".to_string()
}

/// Render the preflight report for humans or as JSON.
fn report_checks(id: &str, pre: &Preflight, fmt: Format) {
    if fmt.is_json() {
        let checks: Vec<_> = pre
            .checks
            .iter()
            .map(|c| match &c.status {
                Status::Ok(detail) => {
                    serde_json::json!({ "name": c.name, "status": "ok", "detail": detail })
                }
                Status::Failed { detail, hint } => serde_json::json!({
                    "name": c.name, "status": "failed", "detail": detail, "hint": hint,
                }),
                Status::Info(detail) => {
                    serde_json::json!({ "name": c.name, "status": "info", "detail": detail })
                }
            })
            .collect();
        output::report(
            pre.first_failure().is_none(),
            &serde_json::json!({
                "target": id,
                "checks": checks,
            }),
        );
        return;
    }

    println!("Preflight checks for target {id}:");
    let mut failed = 0u32;
    for c in &pre.checks {
        match &c.status {
            Status::Ok(detail) => println!("  [ok]   {:<14} {detail}", c.name),
            Status::Info(detail) => println!("  [info] {:<14} {detail}", c.name),
            Status::Failed { detail, hint } => {
                failed += 1;
                println!("  [fail] {:<14} {detail}", c.name);
                println!("         hint: {hint}");
            }
        }
    }
    if failed == 0 {
        println!("\nall checks passed");
    } else {
        println!("\n{failed} check(s) failed");
    }
}

/// Find an executable: an explicit override path first, then PATH.
fn locate(program: &str, override_path: Option<String>) -> Option<PathBuf> {
    if let Some(p) = override_path {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    tools::which(program)
}

fn render_cmd(program: &Path, args: &[String]) -> String {
    let mut s = quote(&program.display().to_string());
    for a in args {
        s.push(' ');
        s.push_str(&quote(a));
    }
    s
}

fn quote(s: &str) -> String {
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s.to_string()
    }
}

/// Compose the environment for CMake/Ninja: the MSVC delta (its `INCLUDE`/`LIB`
/// and a `PATH` that already folds in the original `PATH`) followed by the
/// runtime env resolved *over* that delta, so the runtime's PATH/loader prepends
/// sit in front of the compiler's. Later entries win in `Command::envs`, so the
/// runtime values override the shared keys while MSVC-only keys survive.
fn layer_runtime_env(runtime: &EnvSet, msvc: &[(String, String)]) -> Vec<(String, String)> {
    let mut base: std::collections::HashMap<String, String> = std::env::vars().collect();
    for (k, v) in msvc {
        base.insert(k.clone(), v.clone());
    }
    let mut env = msvc.to_vec();
    env.extend(runtime.resolve_over(&base));
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pre(checks: Vec<Check>) -> Preflight {
        Preflight {
            checks,
            cmake: None,
            ninja: None,
            will_bootstrap_msvc: false,
        }
    }

    #[test]
    fn first_failure_skips_ok_and_info() {
        let p = pre(vec![
            Check::ok("cmake", "/usr/bin/cmake"),
            Check::info("compiler", "cc"),
            Check::failed("runtime", "not pulled", "run `ost runtime pull`"),
        ]);
        let f = p.first_failure().expect("a failure");
        assert_eq!(f.name, "runtime");
    }

    #[test]
    fn no_failure_when_all_ok() {
        let p = pre(vec![Check::ok("cmake", "x"), Check::info("compiler", "cc")]);
        assert!(p.first_failure().is_none());
    }

    #[test]
    fn failed_check_renders_actionable_error() {
        let c = Check::failed(
            "CMakeLists.txt",
            "no CMakeLists.txt found in project root",
            "run `ost init --template cpp-library`",
        );
        let err = c.to_error();
        // The detail is the message; the hint rides the structured slot so
        // `--json` surfaces it as `error.hint` (§14.3/§14.4).
        assert!(err.to_string().contains("no CMakeLists.txt found"));
        assert_eq!(err.category(), ost_core::Category::Precondition);
        assert_eq!(err.hint(), Some("run `ost init --template cpp-library`"));
    }

    #[test]
    fn planned_files_cover_target_and_root_outputs() {
        let files = target_output_paths("cy2026-linux-x86_64-py313-usd");
        assert!(files
            .iter()
            .any(|f| f == ".strata/targets/cy2026-linux-x86_64-py313-usd/toolchain.cmake"));
        // The root-level outputs a build would touch must be listed too.
        assert!(files.iter().any(|f| f == "CMakeUserPresets.json"));
        assert!(!files.iter().any(|f| f == "CMakePresets.json"));
        assert!(files.iter().any(|f| f == "strata.lock"));
    }
}
