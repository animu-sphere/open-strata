// SPDX-License-Identifier: Apache-2.0
//! End-to-end tests for the build lifecycle (§ test 方針).
//!
//! These drive the real `ost` binary against a throwaway store (`OST_HOME`) and a
//! throwaway project, so nothing touches the developer's machine. Most assert the
//! *file-level* guarantees that hold without a C++ toolchain:
//!
//! - an existing `CMakePresets.json` survives `ost configure` byte-for-byte, and a
//!   malformed one is never clobbered (the user's hand-written file is sacred);
//! - `ost build --check` / `--dry-run` write nothing;
//! - different platform/profile targets get their own `.strata/targets/<id>` so a
//!   second target can never reuse the first's CMake cache.
//!
//! The full `init → pull → build → package` round-trip needs cmake + ninja + a
//! compiler, so it is gated on their availability and skips cleanly otherwise.
//! On Windows it is additionally opt-in: local shells and coding agents often
//! have the toolchain installed, but the native build can be slow and a stopped
//! test run can leave the just-built `ost.exe` locked.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

use camino::Utf8PathBuf;

use ost_build::{LeaseMode, TargetLease, TARGET_LEASE_FILE};

/// The `ost` binary built for this test run.
fn ost_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ost")
}

/// A throwaway store + project pair, cleaned up on drop.
struct Sandbox {
    home: PathBuf,
    work: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let base =
            std::env::temp_dir().join(format!("ost-it-{tag}-{}-{n}-{nanos}", std::process::id()));
        let home = base.join("home");
        let work = base.join("work");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        Sandbox { home, work }
    }

    /// Run `ost <args>` in the project dir against the sandbox store.
    fn ost(&self, args: &[&str]) -> Output {
        self.ost_env(args, &[])
    }

    /// Like [`ost`], with extra environment variables on the spawned process.
    fn ost_env(&self, args: &[&str], envs: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(ost_bin());
        cmd.args(args)
            .current_dir(&self.work)
            .env("OST_HOME", &self.home)
            // Don't let a developer's adopt/build env leak into the mock pull.
            .env_remove("OST_USD_ROOT")
            .env_remove("OST_USD_SRC")
            .env_remove("OST_USD_DEPS")
            // CI evidence is opt-in per test invocation.
            .env_remove("OST_CI_CELL")
            .env_remove("OST_CI_LANE")
            .env_remove("OST_CI_RUNNER_PROFILE")
            .env_remove("OST_CI_RUNS_ON")
            .env_remove("OST_CI_RUNTIME_ARTIFACT")
            .env_remove("OST_CI_PLUGIN_ARTIFACT")
            .env_remove("OST_CI_MINIMUM_TRUST");
        for (key, value) in envs {
            cmd.env(key, value);
        }
        cmd.output().expect("spawn ost")
    }

    fn work_file(&self, rel: &str) -> PathBuf {
        self.work.join(rel)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // Best-effort: the parent of home/work is the unique base dir.
        if let Some(base) = self.home.parent() {
            let _ = std::fs::remove_dir_all(base);
        }
    }
}

/// Combined stdout+stderr as a String, for assertions and failure messages.
fn out_text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// Snapshot every file under `dir` as `relpath -> bytes`, for no-side-effect
/// assertions.
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut map = BTreeMap::new();
    walk(dir, dir, &mut map);
    map
}

fn walk(root: &Path, dir: &Path, map: &mut BTreeMap<String, Vec<u8>>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, map);
        } else if let Ok(bytes) = std::fs::read(&path) {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            map.insert(rel, bytes);
        }
    }
}

fn managed_build_output(root: &Path, relative: &str) -> ost_build::BuildOutput {
    let bytes = std::fs::read(root.join(relative)).unwrap();
    ost_build::BuildOutput {
        path: relative.replace('\\', "/"),
        sha256: ost_core::digest::sha256_hex(&bytes),
        size: bytes.len() as u64,
    }
}

/// The single generated target directory under `.strata/targets/`.
fn single_target_dir(work: &Path) -> PathBuf {
    let targets = work.join(".strata").join("targets");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&targets)
        .unwrap_or_else(|e| panic!("read {}: {e}", targets.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    assert_eq!(dirs.len(), 1, "expected one target dir, found {dirs:?}");
    dirs.pop().unwrap()
}

/// Scaffold a project and pull its (mock) runtime. Panics on any step failing so
/// a setup break is obvious. `ost init` scaffolds a `usd`-profile project on
/// `cy2026`, so pull that to match what configure/build resolve.
fn init_and_pull(sb: &Sandbox) {
    let init = sb.ost(&["init", "--platform", "cy2026"]);
    assert!(init.status.success(), "init failed:\n{}", out_text(&init));
    let pull = sb.ost(&["runtime", "pull", "cy2026", "--profile", "usd"]);
    assert!(pull.status.success(), "pull failed:\n{}", out_text(&pull));
}

/// A representative hand-authored CMakePresets.json: every section plus a vendor
/// block and an unknown top-level field that must survive untouched.
const RICH_PRESETS: &str = r#"{
  "version": 6,
  "configurePresets": [
    { "name": "user-debug", "binaryDir": "${sourceDir}/out", "cacheVariables": { "CMAKE_BUILD_TYPE": "Debug" } }
  ],
  "buildPresets": [ { "name": "user-debug", "configurePreset": "user-debug" } ],
  "testPresets": [ { "name": "user-debug", "configurePreset": "user-debug" } ],
  "workflowPresets": [ { "name": "ci", "steps": [ { "type": "configure", "name": "user-debug" } ] } ],
  "vendor": { "acme.com/ide": { "favourite": true } },
  "x-unknown-extension": { "keep": "me" }
}
"#;

#[test]
fn configure_preserves_a_hand_authored_cmakepresets() {
    let sb = Sandbox::new("preserve-presets");
    init_and_pull(&sb);

    let presets = sb.work_file("CMakePresets.json");
    std::fs::write(&presets, RICH_PRESETS).unwrap();
    let before = std::fs::read(&presets).unwrap();

    let cfg = sb.ost(&["configure"]);
    assert!(
        cfg.status.success(),
        "configure failed:\n{}",
        out_text(&cfg)
    );

    // The user's committed presets file is byte-identical.
    assert_eq!(
        std::fs::read(&presets).unwrap(),
        before,
        "ost configure must not touch the user's CMakePresets.json"
    );

    // The target's generated files all landed under .strata/targets/<id>/.
    let target = single_target_dir(&sb.work);
    for f in [
        "toolchain.cmake",
        "env.json",
        "target.lock.json",
        "CMakePresets.json",
    ] {
        assert!(
            target.join(f).is_file(),
            "missing generated {f} in {target:?}"
        );
    }

    // The tool-owned user-presets file carries the per-target include.
    let user = sb.work_file("CMakeUserPresets.json");
    assert!(user.is_file(), "CMakeUserPresets.json should be generated");
    let body = std::fs::read_to_string(&user).unwrap();
    assert!(
        body.contains("include"),
        "user presets should include the target presets:\n{body}"
    );
}

#[test]
fn configure_rejects_malformed_cmakepresets_without_clobbering() {
    let sb = Sandbox::new("malformed-presets");
    init_and_pull(&sb);

    let presets = sb.work_file("CMakePresets.json");
    let garbage = "{ this is : not json ]";
    std::fs::write(&presets, garbage).unwrap();

    let cfg = sb.ost(&["configure"]);
    assert!(
        !cfg.status.success(),
        "configure should fail on a malformed CMakePresets.json, not treat it as empty"
    );
    // The malformed file is left exactly as the user wrote it — never overwritten.
    assert_eq!(std::fs::read_to_string(&presets).unwrap(), garbage);
}

#[test]
fn build_check_writes_nothing() {
    let sb = Sandbox::new("check-no-writes");
    init_and_pull(&sb);

    let before = snapshot(&sb.work);
    // --check only reports; its exit status depends on tool availability, but the
    // work tree must be untouched either way.
    let _ = sb.ost(&["build", "--check"]);
    let after = snapshot(&sb.work);

    assert_eq!(before, after, "ost build --check must have no side effects");
}

#[test]
fn build_dry_run_writes_nothing_and_plans_commands() {
    let sb = Sandbox::new("dryrun-no-writes");
    init_and_pull(&sb);

    let before = snapshot(&sb.work);
    let out = sb.ost(&["build", "--dry-run"]);
    let after = snapshot(&sb.work);

    assert_eq!(
        before, after,
        "ost build --dry-run must have no side effects"
    );
    assert!(
        out.status.success(),
        "dry-run should succeed:\n{}",
        out_text(&out)
    );
    let text = out_text(&out);
    assert!(
        text.contains("--preset"),
        "should plan the configure command:\n{text}"
    );
    assert!(
        text.contains("--build"),
        "should plan the build command:\n{text}"
    );
    assert!(
        text.contains("would generate"),
        "should list planned files:\n{text}"
    );
    assert!(
        text.contains("CMakeUserPresets.json"),
        "should name the tool-owned root presets file:\n{text}"
    );
    assert!(
        !text
            .lines()
            .any(|line| line.trim_end() == "#   CMakePresets.json"),
        "must not claim the user's root CMakePresets.json is generated:\n{text}"
    );
}

/// The target lease is cross-process exclusion, so the assertion that matters is
/// made against a *real* second process: this test holds the lease and the `ost`
/// child must be refused. Same-process locking would prove much less.
#[test]
fn a_second_writer_is_refused_while_the_target_is_leased() {
    let sb = Sandbox::new("lease-busy");
    init_and_pull(&sb);

    // Configure once so the target directory (and its id) exist.
    let first = sb.ost(&["configure"]);
    assert!(
        first.status.success(),
        "configure failed:\n{}",
        out_text(&first)
    );
    let target_dir = single_target_dir(&sb.work);
    let id = target_dir
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    // A clean run leaves no owner behind. The file itself persists by design —
    // the lock lives on the inode, so unlinking it would let a later writer
    // acquire a fresh file at the same path and exclude nobody.
    let lease_path = Utf8PathBuf::from_path_buf(target_dir.join(TARGET_LEASE_FILE)).unwrap();
    assert!(
        lease_is_unowned(lease_path.as_std_path()),
        "a completed configure must release its lease"
    );

    let held = TargetLease::acquire(&lease_path, &id, "ost build", LeaseMode::Fail)
        .expect("the freed lease can be taken");

    // `fail` is the default: a second writer stops rather than interleaving.
    let busy = sb.ost(&["configure", "--json"]);
    assert!(
        !busy.status.success(),
        "a leased target must refuse writers"
    );
    let text = out_text(&busy);
    assert!(
        text.contains("TARGET_BUSY"),
        "expected TARGET_BUSY:\n{text}"
    );
    // The refusal has to name the holder, or the user cannot act on it.
    let invocation = held.invocation().expect("an invocation");
    assert!(
        text.contains(invocation),
        "busy error must name the holding invocation:\n{text}"
    );

    // `read-only` is the documented way through: it never contends.
    let attached = sb.ost(&["configure", "--on-busy", "read-only"]);
    assert!(
        attached.status.success(),
        "read-only must proceed while the target is leased:\n{}",
        out_text(&attached)
    );

    // Releasing frees the target for the next writer.
    held.release();
    let after = sb.ost(&["configure"]);
    assert!(
        after.status.success(),
        "a released lease must free the target:\n{}",
        out_text(&after)
    );
}

/// A writer killed mid-build leaves its record behind. The next invocation must
/// inherit the target and say whose run it inherited — not wedge on a lock no
/// live process holds.
#[test]
fn a_stale_lease_record_is_taken_over_and_reported() {
    let sb = Sandbox::new("lease-stale");
    init_and_pull(&sb);
    assert!(sb.ost(&["configure"]).status.success());

    let target_dir = single_target_dir(&sb.work);
    let id = target_dir
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let lease_path = target_dir.join(TARGET_LEASE_FILE);

    // A record whose pid cannot be live: positive as a c_int (a negative value
    // would address a process group), far above any platform's pid_max, and not
    // a multiple of 4 as Windows pids always are.
    let stale = serde_json::json!({
        "schema": "openstrata.target-lease/v1",
        "invocation": "deadbeefdeadbeef",
        "command": "ost build",
        "target": id,
        "pid": 0x7FFF_FFFEu32,
        "host": hostname_of_this_process(),
        "acquired_unix": 1,
    });
    std::fs::write(&lease_path, serde_json::to_vec_pretty(&stale).unwrap()).unwrap();

    let out = sb.ost(&["configure"]);
    assert!(
        out.status.success(),
        "a dead owner must not wedge the target:\n{}",
        out_text(&out)
    );
    let text = out_text(&out);
    assert!(
        text.contains("took over the target lease") && text.contains("deadbeefdeadbeef"),
        "the takeover must name the run it inherited:\n{text}"
    );
    assert!(
        lease_is_unowned(&lease_path),
        "the completed run must release the lease it took over"
    );
}

/// Whether a lease file names no owner — either absent, or present and cleared.
///
/// A release clears the record in place rather than unlinking the file, so
/// "nobody holds this target" is an empty record, not a missing path.
fn lease_is_unowned(path: &Path) -> bool {
    match std::fs::read_to_string(path) {
        Ok(body) => body.trim().is_empty(),
        Err(_) => true,
    }
}

/// An external tree gets `runtime-compatible` only after an explicit import, and
/// only while it still matches. `configured` and `built` stay skipped either
/// way — those claim OpenStrata did the work.
#[test]
fn external_provenance_upgrades_only_runtime_compatibility() {
    let sb = Sandbox::new("external-import");
    init_and_pull(&sb);
    assert!(sb.ost(&["configure"]).status.success());

    let id = single_target_dir(&sb.work)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    // The mock runtime's prefix, which a real external tree would have resolved
    // `pxr` from.
    let runtime_root = sb
        .home
        .join("runtimes")
        .join(format!("openstrata-{id}"))
        .to_string_lossy()
        .replace('\\', "/");

    // Stand in for a tree configured outside OpenStrata: only its CMake cache
    // exists, which is exactly what the import is required to work from.
    let external = sb.work.join("external-build");
    std::fs::create_dir_all(&external).unwrap();
    let cache = format!(
        "CMAKE_HOME_DIRECTORY:INTERNAL={source}\n\
         CMAKE_CACHEFILE_DIR:INTERNAL={build}\n\
         CMAKE_GENERATOR:INTERNAL=Ninja\n\
         CMAKE_BUILD_TYPE:STRING=Release\n\
         CMAKE_CXX_COMPILER:FILEPATH=/usr/bin/c++\n\
         pxr_DIR:PATH={runtime_root}\n",
        source = sb.work.to_string_lossy().replace('\\', "/"),
        build = external.to_string_lossy().replace('\\', "/"),
    );
    let cache_path = external.join("CMakeCache.txt");
    std::fs::write(&cache_path, &cache).unwrap();

    // Before any import, the tree makes no claim about any runtime.
    let before = sb.ost(&["validate", "--build-dir", "external-build"]);
    let text = out_text(&before);
    assert!(
        text.contains("no imported provenance"),
        "an un-imported tree must not claim compatibility:\n{text}"
    );

    let import = sb.ost(&["external", "import", "--build-dir", "external-build"]);
    assert!(
        import.status.success(),
        "import failed:\n{}",
        out_text(&import)
    );

    let after = sb.ost(&["validate", "--build-dir", "external-build"]);
    let text = out_text(&after);
    assert!(
        text.contains("[ok  ] runtime-compatible"),
        "a full identity match upgrades runtime compatibility:\n{text}"
    );
    // …but never these two, whatever the import said.
    assert!(
        text.contains("[skip] configured") && text.contains("[skip] built"),
        "an import must not claim `ost build` configured or built the tree:\n{text}"
    );

    // Reconfiguring the tree invalidates the record rather than silently
    // continuing to vouch for it.
    std::fs::write(&cache_path, cache.replace("Release", "Debug")).unwrap();
    let stale = sb.ost(&["validate", "--build-dir", "external-build"]);
    let text = out_text(&stale);
    assert!(
        text.contains("[FAIL] runtime-compatible") && text.contains("reconfigured"),
        "a reconfigured tree must stop verifying:\n{text}"
    );
}

/// Visual Studio records compiler identity below CMakeFiles instead of in the
/// top-level cache. A core-only import must consume that real generator shape
/// without inventing an OpenUSD requirement; an explicit USD capability turns
/// that requirement back on.
#[test]
fn external_import_is_generator_and_capability_aware() {
    let sb = Sandbox::new("external-vs-core");
    init_and_pull(&sb);
    let pull = sb.ost(&["runtime", "pull", "cy2026", "--profile", "core"]);
    assert!(
        pull.status.success(),
        "core pull failed:\n{}",
        out_text(&pull)
    );

    let external = sb.work.join("vs-core-build");
    let compiler_dir = external.join("CMakeFiles").join("3.31.0");
    std::fs::create_dir_all(&compiler_dir).unwrap();
    let portable_build = external.to_string_lossy().replace('\\', "/");
    std::fs::write(
        external.join("CMakeCache.txt"),
        format!(
            "CMAKE_HOME_DIRECTORY:INTERNAL={source}\n\
             CMAKE_CACHEFILE_DIR:INTERNAL={portable_build}\n\
             CMAKE_GENERATOR:INTERNAL=Visual Studio 17 2022\n\
             CMAKE_GENERATOR_PLATFORM:INTERNAL=x64\n\
             CMAKE_GENERATOR_TOOLSET:INTERNAL=v143\n\
             CMAKE_CONFIGURATION_TYPES:STRING=Debug;Release\n",
            source = sb.work.to_string_lossy().replace('\\', "/"),
        ),
    )
    .unwrap();
    std::fs::write(
        compiler_dir.join("CMakeCXXCompiler.cmake"),
        "set(CMAKE_CXX_COMPILER \"C:/MSVC/bin/cl.exe\")\n\
         set(CMAKE_CXX_COMPILER_ID \"MSVC\")\n\
         set(CMAKE_CXX_COMPILER_VERSION \"19.43\")\n",
    )
    .unwrap();

    let imported = sb.ost(&[
        "--json",
        "external",
        "import",
        "--build-dir",
        "vs-core-build",
        "--profile",
        "core",
        "--capability",
        "build-cxx",
    ]);
    assert!(imported.status.success(), "{}", out_text(&imported));
    let imported: serde_json::Value = serde_json::from_slice(&imported.stdout).unwrap();
    let provenance = &imported["data"]["provenance"];
    assert_eq!(provenance["schema"], "openstrata.external-build/v2");
    assert_eq!(provenance["toolchain"]["generator_flavor"], "visual-studio");
    assert_eq!(provenance["toolchain"]["multi_config"], true);
    assert_eq!(
        provenance["toolchain"]["cxx_compiler_source"],
        "CMakeFiles/3.31.0/CMakeCXXCompiler.cmake:CMAKE_CXX_COMPILER"
    );
    let requirements = provenance["requirements"].as_array().unwrap();
    assert!(requirements.iter().any(|requirement| {
        requirement["name"] == "openusd.runtime" && requirement["status"] == "not-applicable"
    }));

    let validated = sb.ost(&[
        "validate",
        "--build-dir",
        "vs-core-build",
        "--profile",
        "core",
    ]);
    let text = out_text(&validated);
    assert!(validated.status.success(), "{text}");
    assert!(text.contains("OpenUSD binding not applicable"), "{text}");

    let usd_requested = sb.ost(&[
        "external",
        "import",
        "--build-dir",
        "vs-core-build",
        "--profile",
        "core",
        "--capability",
        "usd-stage-read",
    ]);
    let text = out_text(&usd_requested);
    assert!(!usd_requested.status.success(), "{text}");
    assert!(text.contains("OpenUSD runtime binding"), "{text}");
    assert!(text.contains("pxr_ROOT"), "{text}");
}

#[test]
fn external_import_compiler_failure_has_applicable_remediation() {
    let sb = Sandbox::new("external-compiler-remediation");
    init_and_pull(&sb);
    let pull = sb.ost(&["runtime", "pull", "cy2026", "--profile", "core"]);
    assert!(pull.status.success(), "{}", out_text(&pull));
    let external = sb.work.join("incomplete-vs-build");
    std::fs::create_dir_all(&external).unwrap();
    std::fs::write(
        external.join("CMakeCache.txt"),
        format!(
            "CMAKE_HOME_DIRECTORY:INTERNAL={source}\n\
             CMAKE_CACHEFILE_DIR:INTERNAL={build}\n\
             CMAKE_GENERATOR:INTERNAL=Visual Studio 17 2022\n",
            source = sb.work.to_string_lossy().replace('\\', "/"),
            build = external.to_string_lossy().replace('\\', "/"),
        ),
    )
    .unwrap();

    let imported = sb.ost(&[
        "external",
        "import",
        "--build-dir",
        "incomplete-vs-build",
        "--profile",
        "core",
    ]);
    let text = out_text(&imported);
    assert!(!imported.status.success(), "{text}");
    assert!(text.contains("visual-studio"), "{text}");
    assert!(
        text.contains("CMakeFiles/<version>/CMakeCXXCompiler.cmake"),
        "{text}"
    );
    assert!(text.contains("finish configuring the tree"), "{text}");
    assert!(!text.contains("pxr_ROOT"), "{text}");
}

#[test]
fn validate_only_recommends_external_import_for_a_cmake_tree() {
    let sb = Sandbox::new("external-validate-remediation");
    init_and_pull(&sb);
    std::fs::create_dir_all(sb.work.join("not-configured")).unwrap();

    let validated = sb.ost(&["validate", "--build-dir", "not-configured"]);
    let text = out_text(&validated);
    assert!(validated.status.success(), "{text}");
    assert!(text.contains("[skip] runtime-compatible"), "{text}");
    assert!(text.contains("CMakeCache.txt not found"), "{text}");
    assert!(
        text.contains("point --build-dir at a configured CMake build tree"),
        "{text}"
    );
    assert!(!text.contains("ost external import"), "{text}");
}

/// A tree that resolved OpenUSD from somewhere else is not evidence about this
/// runtime, and must be refused at import rather than recorded and trusted.
#[test]
fn importing_a_tree_built_against_another_runtime_is_refused() {
    let sb = Sandbox::new("external-foreign");
    init_and_pull(&sb);
    assert!(sb.ost(&["configure"]).status.success());

    let external = sb.work.join("foreign-build");
    std::fs::create_dir_all(&external).unwrap();
    std::fs::write(
        external.join("CMakeCache.txt"),
        format!(
            "CMAKE_HOME_DIRECTORY:INTERNAL={source}\n\
             CMAKE_CACHEFILE_DIR:INTERNAL={build}\n\
             CMAKE_GENERATOR:INTERNAL=Ninja\n\
             CMAKE_CXX_COMPILER:FILEPATH=/usr/bin/c++\n\
             pxr_DIR:PATH=/somewhere/else/usd\n",
            source = sb.work.to_string_lossy().replace('\\', "/"),
            build = external.to_string_lossy().replace('\\', "/"),
        ),
    )
    .unwrap();

    let import = sb.ost(&["external", "import", "--build-dir", "foreign-build"]);
    assert!(
        !import.status.success(),
        "a foreign pxr root must be refused"
    );
    let text = out_text(&import);
    assert!(
        text.contains("not from the selected runtime"),
        "the refusal must name the mismatch:\n{text}"
    );
}

/// The lease record's `host` is compared against this machine's name, so the
/// test has to produce it the same way the implementation does.
fn hostname_of_this_process() -> String {
    // Round-tripping through an acquired lease avoids duplicating the platform
    // specific lookup, and asserts the two agree by construction.
    let dir = std::env::temp_dir().join(format!("ost-host-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = Utf8PathBuf::from_path_buf(dir.join("probe.json")).unwrap();
    let lease = TargetLease::acquire(&path, "probe", "probe", LeaseMode::Fail).unwrap();
    let host = lease.owner().unwrap().host.clone();
    lease.release();
    std::fs::remove_dir_all(&dir).ok();
    host
}

#[test]
fn distinct_profiles_get_separate_target_trees() {
    let sb = Sandbox::new("separate-targets");
    init_and_pull(&sb); // usd

    // Configure the project's default (usd) target.
    let c1 = sb.ost(&["configure", "--profile", "usd"]);
    assert!(
        c1.status.success(),
        "configure usd failed:\n{}",
        out_text(&c1)
    );

    // Pull and configure a second, different profile (core) on the same project.
    let p2 = sb.ost(&["runtime", "pull", "cy2026", "--profile", "core"]);
    assert!(p2.status.success(), "pull core failed:\n{}", out_text(&p2));
    let c2 = sb.ost(&["configure", "--profile", "core"]);
    assert!(
        c2.status.success(),
        "configure core failed:\n{}",
        out_text(&c2)
    );

    // Two distinct target dirs exist — neither can reuse the other's build tree.
    let targets = sb.work.join(".strata").join("targets");
    let ids: Vec<String> = std::fs::read_dir(&targets)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(ids.len(), 2, "expected two target dirs, got {ids:?}");
    assert_ne!(
        ids[0], ids[1],
        "core and usd targets must have different ids"
    );
    // The build trees CMake would use are keyed by the same id (build/<id>).
    for id in &ids {
        assert!(
            id.contains("cy2026"),
            "target id should carry the platform: {id}"
        );
    }
}

const NATIVE_LIFECYCLE_ENV: &str = "OST_RUN_NATIVE_LIFECYCLE";

/// Whether a full build can run here: cmake + ninja, and a usable compiler. On
/// Windows the compiler is MSVC, which `ost build` bootstraps via vcvars — so
/// defer to the *same* detection (`ost_build::msvc::bootstrap`) instead of
/// assuming Visual Studio is installed. A missing toolchain skips, never fails.
fn native_lifecycle_ready() -> Result<(), String> {
    if cfg!(windows) && std::env::var_os(NATIVE_LIFECYCLE_ENV).is_none() {
        return Err(format!(
            "native build lifecycle tests are opt-in on Windows; set {NATIVE_LIFECYCLE_ENV}=1"
        ));
    }

    fn on_path(exe: &str) -> bool {
        let probe = if cfg!(windows) { "where" } else { "which" };
        Command::new(probe)
            .arg(exe)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    if !on_path("cmake") {
        return Err("cmake not found on PATH".to_string());
    }
    if !on_path("ninja") {
        return Err("ninja not found on PATH".to_string());
    }
    if cfg!(windows) {
        // Same MSVC bootstrap the build uses; `Ok(None)` means no VS → skip.
        return match ost_build::msvc::bootstrap() {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err("MSVC developer environment not found".to_string()),
            Err(e) => Err(format!("MSVC bootstrap failed: {e}")),
        };
    }
    if ["cc", "clang", "gcc"].iter().any(|c| on_path(c)) {
        Ok(())
    } else {
        Err("no C/C++ compiler found on PATH".to_string())
    }
}

#[test]
fn full_lifecycle_init_build_package() {
    if let Err(reason) = native_lifecycle_ready() {
        eprintln!("skipping full_lifecycle: {reason}");
        return;
    }
    let sb = Sandbox::new("full-lifecycle");
    init_and_pull(&sb);

    // Build, in plain mode so the assertions don't depend on a TTY.
    let build = sb.ost(&["build", "--progress", "plain"]);
    assert!(
        build.status.success(),
        "build failed:\n{}",
        out_text(&build)
    );

    let target = single_target_dir(&sb.work);
    assert!(
        target.join("build.log").is_file(),
        "build should write a build.log"
    );
    let build_dir_id = target.file_name().unwrap().to_string_lossy().into_owned();
    let build_dir = sb.work.join("build").join(&build_dir_id);
    assert!(
        std::fs::read_dir(&build_dir)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false),
        "build/{build_dir_id} should be non-empty"
    );

    // Package: the cpp-library template carries install() rules, so the tree is
    // non-empty and packaging succeeds with a real artifact + manifest.
    let pkg = sb.ost(&["package"]);
    assert!(pkg.status.success(), "package failed:\n{}", out_text(&pkg));
    let dist = sb.work.join("dist");
    let archive = find_first(&dist, "tar.zst");
    assert!(
        archive.is_some(),
        "an archive should be produced under {dist:?}"
    );
    let manifest = find_first(&dist, "manifest.json");
    assert!(
        manifest.is_some(),
        "a manifest.json should accompany the archive"
    );
}

#[test]
fn multi_config_package_uses_the_completed_configuration_and_generator() {
    if let Err(reason) = native_lifecycle_ready() {
        eprintln!("skipping multi_config_package: {reason}");
        return;
    }
    let sb = Sandbox::new("multi-config-package");
    init_and_pull(&sb);

    let build = sb.ost(&[
        "build",
        "--generator",
        "Ninja Multi-Config",
        "--config",
        "Debug",
        "--progress",
        "plain",
    ]);
    assert!(
        build.status.success(),
        "multi-config build failed:\n{}",
        out_text(&build)
    );

    let package = sb.ost(&["package"]);
    assert!(
        package.status.success(),
        "multi-config package failed:\n{}",
        out_text(&package)
    );
    let manifest_path =
        find_first(&sb.work.join("dist"), "manifest.json").expect("package manifest");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["provenance"]["generator"], "Ninja Multi-Config");
}

#[test]
fn renderer_template_scaffolds_one_project_and_a_strict_manifest() {
    let sb = Sandbox::new("renderer-template");
    let init = sb.ost(&[
        "--json",
        "init",
        "--platform",
        "cy2026",
        "--template",
        "renderer",
        "--name",
        "sample-renderer",
    ]);
    assert!(init.status.success(), "init failed:\n{}", out_text(&init));
    let output: serde_json::Value = serde_json::from_slice(&init.stdout).unwrap();
    assert_eq!(output["data"]["template"], "renderer");

    for path in [
        "openstrata.renderer.yaml",
        "core/render-world/CMakeLists.txt",
        "core/render-extraction/CMakeLists.txt",
        "backend/vulkan/CMakeLists.txt",
        "adapters/headless/CMakeLists.txt",
        "validation/CMakeLists.txt",
    ] {
        assert!(
            sb.work_file(path).is_file(),
            "missing renderer output {path}"
        );
    }

    let source = std::fs::read_to_string(sb.work_file("openstrata.renderer.yaml")).unwrap();
    let manifest = ost_manifest::RendererManifest::parse(&source).unwrap();
    assert_eq!(manifest.renderer.name, "sample-renderer");
    assert_eq!(
        manifest.composition.units["core"],
        "sample-renderer-render-core"
    );
    assert!(manifest
        .validation
        .assertions
        .iter()
        .any(|id| id == "renderer.render_product.color"));

    // Internal packs are target boundaries in one project, not generated
    // workspace package descriptors.
    assert!(!sb.work_file("openstrata.library.yaml").exists());
    assert!(!sb.work_file("openstrata.plugin.yaml").exists());
}

#[test]
fn renderer_attach_session_records_external_unverified_origin() {
    let sb = Sandbox::new("renderer-attach-session");
    let init = sb.ost(&[
        "init",
        "--platform",
        "cy2026",
        "--template",
        "renderer",
        "--name",
        "sample-renderer",
    ]);
    assert!(init.status.success(), "init failed:\n{}", out_text(&init));

    let root = camino::Utf8Path::from_path(sb.work.as_path()).unwrap();
    let manifest = ost_manifest::RendererManifest::load(root).unwrap();
    let report_path = sb.work_file("build/external/renderer-report.json");
    std::fs::create_dir_all(report_path.parent().unwrap()).unwrap();
    let checks = manifest
        .validation
        .assertions
        .iter()
        .take(1)
        .map(|id| serde_json::json!({ "id": id, "status": "pass" }))
        .collect::<Vec<_>>();
    std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "openstrata.renderer-report/v1alpha1",
            "renderer": { "name": "sample-renderer" },
            "checks": checks,
        }))
        .unwrap(),
    )
    .unwrap();

    let attach = sb.ost(&[
        "--json",
        "renderer",
        "attach-session",
        "build/external/renderer-report.json",
        "--target",
        "external-release",
        "--started-unix",
        "100",
        "--completed-unix",
        "120",
        "--outcome",
        "success",
        "--session-id",
        "external-run-1",
    ]);
    assert!(
        attach.status.success(),
        "attach failed:\n{}",
        out_text(&attach)
    );
    let output: serde_json::Value = serde_json::from_slice(&attach.stdout).unwrap();
    assert_eq!(output["data"]["verification"], "external-unverified");

    let report = ost_manifest::RendererReport::load(
        camino::Utf8Path::from_path(report_path.as_path()).unwrap(),
    )
    .unwrap();
    report.validate_overlay_against(&manifest).unwrap();
    let producer = report.producer.as_ref().unwrap();
    assert_eq!(producer.id, "external-run-1");
    assert_eq!(producer.kind, "external-unverified");
    assert!(report
        .checks
        .iter()
        .all(|check| check.producer.as_deref() == Some("external-run-1")));

    let reattach = sb.ost(&[
        "renderer",
        "attach-session",
        "build/external/renderer-report.json",
        "--target",
        "external-release",
        "--started-unix",
        "200",
        "--completed-unix",
        "220",
        "--outcome",
        "success",
    ]);
    assert!(!reattach.status.success());
    assert!(out_text(&reattach).contains("already carries producer provenance"));

    // A failed/incomplete producer may have stranded a PASS before it died.
    // Attaching that outcome must persist the truth, then normal evidence
    // validation refuses the PASS because its owner never completed.
    let incomplete_path = sb.work_file("build/external/renderer-incomplete-report.json");
    std::fs::write(
        &incomplete_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "openstrata.renderer-report/v1alpha1",
            "renderer": { "name": "sample-renderer" },
            "checks": [{
                "id": manifest.validation.assertions[0].clone(),
                "status": "pass",
            }],
        }))
        .unwrap(),
    )
    .unwrap();
    let incomplete = sb.ost(&[
        "renderer",
        "attach-session",
        "build/external/renderer-incomplete-report.json",
        "--target",
        "external-release",
        "--started-unix",
        "300",
        "--outcome",
        "incomplete",
    ]);
    assert!(
        incomplete.status.success(),
        "incomplete attach failed:\n{}",
        out_text(&incomplete)
    );
    let report = ost_manifest::RendererReport::load(
        camino::Utf8Path::from_path(incomplete_path.as_path()).unwrap(),
    )
    .unwrap();
    report
        .validate_overlay_structure_against(&manifest)
        .unwrap();
    assert!(report
        .validate_overlay_against(&manifest)
        .unwrap_err()
        .to_string()
        .contains("never completed"));
}

#[test]
fn renderer_managed_build_and_test_stamp_the_owning_sessions() {
    if let Err(reason) = native_lifecycle_ready() {
        eprintln!("skipping renderer managed session lifecycle: {reason}");
        return;
    }
    let sb = Sandbox::new("renderer-managed-session");
    let init = sb.ost(&[
        "init",
        "--platform",
        "cy2026",
        "--template",
        "renderer",
        "--name",
        "sample-renderer",
    ]);
    assert!(init.status.success(), "init failed:\n{}", out_text(&init));
    let pull = sb.ost(&["runtime", "pull", "cy2026", "--profile", "core"]);
    assert!(pull.status.success(), "pull failed:\n{}", out_text(&pull));

    let build = sb.ost(&["build", "--progress", "plain"]);
    assert!(
        build.status.success(),
        "renderer build failed:\n{}",
        out_text(&build)
    );
    let target = single_target_dir(&sb.work);
    let id = target.file_name().unwrap().to_string_lossy().into_owned();
    let report_path = sb.work.join("build").join(&id).join("renderer-report.json");
    let report = ost_manifest::RendererReport::load(
        camino::Utf8Path::from_path(report_path.as_path()).unwrap(),
    )
    .unwrap();
    let build_producer = report.producer.as_ref().expect("managed build producer");
    assert_eq!(build_producer.kind, "ost-build");
    assert_eq!(build_producer.target, id);
    assert!(build_producer.can_assert_pass());

    let test = sb.ost(&["test", "--progress", "plain"]);
    assert!(
        test.status.success(),
        "renderer test failed:\n{}",
        out_text(&test)
    );
    let test_report_path = sb
        .work
        .join("build")
        .join(&id)
        .join("renderer-ctest-report.json");
    let report = ost_manifest::RendererReport::load(
        camino::Utf8Path::from_path(test_report_path.as_path()).unwrap(),
    )
    .unwrap();
    let test_producer = report.producer.as_ref().expect("managed test producer");
    assert_eq!(test_producer.kind, "ost-test");
    assert_eq!(test_producer.target, id);
    assert!(test_producer.can_assert_pass());
    assert_ne!(build_producer.id, test_producer.id);
}

#[test]
fn renderer_adopt_is_dry_run_first_and_idempotent() {
    let sb = Sandbox::new("renderer-adopt");
    std::fs::write(
        sb.work_file("CMakeLists.txt"),
        r#"cmake_minimum_required(VERSION 3.24)
project(hdExisting)
add_library(existing-core INTERFACE)
add_library(existing-extraction INTERFACE)
add_library(existing-vulkan INTERFACE)
add_executable(existing-headless main.cpp)
add_library(hdExisting MODULE adapter.cpp)
"#,
    )
    .unwrap();

    let args = [
        "renderer",
        "adopt",
        "--name",
        "hdExisting",
        "--core",
        "existing-core",
        "--extraction",
        "existing-extraction",
        "--backend",
        "vulkan=existing-vulkan",
        "--headless",
        "existing-headless",
        "--hydra2",
        "hdExisting",
        "--platform",
        "cy2026",
    ];
    let dry = sb.ost(&args);
    assert!(dry.status.success(), "dry run failed:\n{}", out_text(&dry));
    assert!(!sb.work_file("openstrata.toml").exists());
    assert!(!sb.work_file("openstrata.renderer.yaml").exists());

    let mut write_args = args.to_vec();
    write_args.push("--write");
    let write = sb.ost(&write_args);
    assert!(
        write.status.success(),
        "adoption write failed:\n{}",
        out_text(&write)
    );
    let manifest = ost_manifest::RendererManifest::load(
        camino::Utf8Path::from_path(sb.work.as_path()).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest.composition.units["core"], "existing-core");
    assert_eq!(manifest.composition.adapters["hydra2"], "hdExisting");
    assert!(sb.work_file("openstrata.renderer-adoption.json").is_file());
    assert!(!sb.work_file("openstrata.scaffold.yaml").exists());

    let rerun = sb.ost(&write_args);
    assert!(
        rerun.status.success(),
        "idempotent rerun failed:\n{}",
        out_text(&rerun)
    );
}

#[test]
fn validate_surfaces_renderer_pass_fail_skip_evidence() {
    let sb = Sandbox::new("renderer-validate");
    let init = sb.ost(&[
        "init",
        "--platform",
        "cy2026",
        "--template",
        "renderer",
        "--name",
        "sample-renderer",
    ]);
    assert!(init.status.success(), "init failed:\n{}", out_text(&init));
    let project = std::fs::read_to_string(sb.work_file("openstrata.toml")).unwrap();
    assert!(project.contains("profile = \"core\""));
    let pull = sb.ost(&["runtime", "pull", "cy2026", "--profile", "core"]);
    assert!(pull.status.success(), "pull failed:\n{}", out_text(&pull));
    let configure = sb.ost(&["configure"]);
    assert!(
        configure.status.success(),
        "configure failed:\n{}",
        out_text(&configure)
    );

    let target = single_target_dir(&sb.work);
    let id = target.file_name().unwrap().to_string_lossy().into_owned();
    let build = sb.work.join("build").join(&id);
    std::fs::create_dir_all(&build).unwrap();
    let lock: ost_build::TargetLock =
        serde_json::from_str(&std::fs::read_to_string(target.join("target.lock.json")).unwrap())
            .unwrap();
    let completion = ost_build::BuildCompletion::from_lock(
        &lock,
        ost_build::BuildProjectIdentity {
            name: "sample-renderer".into(),
            version: "0.1.0".into(),
        },
        format!("build/{}", lock.target),
        ost_build::BuildIntent::default(),
        1,
    );
    std::fs::write(
        build.join(ost_build::BUILD_COMPLETION_FILE),
        completion.to_json().unwrap(),
    )
    .unwrap();
    std::fs::write(
        build.join("renderer-report.json"),
        r#"{
          "schema":"openstrata.renderer-report/v1alpha1",
          "renderer":{"name":"sample-renderer"},
          "producer":{"id":"headless-1","kind":"renderer-harness",
                      "target":"sample-renderer-headless",
                      "started_unix":1750000000,"completed_unix":1750000030,
                      "outcome":"success"},
          "checks":[
            {"id":"renderer.core.boundary","status":"pass"},
            {"id":"renderer.backend.capability","status":"skip","detail":"GPU unavailable"},
            {"id":"renderer.gpu.frame","status":"skip","detail":"GPU unavailable"},
            {"id":"renderer.validation.messages","status":"skip","detail":"validation layer unavailable"},
            {"id":"renderer.render_product.color","status":"skip","detail":"GPU frame skipped"},
            {"id":"renderer.render_product.depth","status":"skip","detail":"GPU frame skipped"},
            {"id":"renderer.frame.persistence","status":"skip","detail":"GPU frame skipped"},
            {"id":"renderer.install_tree","status":"pass"},
            {"id":"renderer.plugin.discovery","status":"skip","detail":"Hydra adapter disabled"},
            {"id":"renderer.delegate.creation","status":"skip","detail":"Hydra adapter disabled"},
            {"id":"renderer.render_buffer.cpu","status":"skip","detail":"Hydra adapter disabled"},
            {"id":"renderer.host.first_frame","status":"skip","detail":"usdview unavailable"},
            {"id":"renderer.host.stable_update","status":"skip","detail":"usdview unavailable"}
          ]
        }"#,
    )
    .unwrap();

    let validate = sb.ost(&["--json", "validate"]);
    assert!(
        validate.status.success(),
        "validate failed:\n{}",
        out_text(&validate)
    );
    let output: serde_json::Value = serde_json::from_slice(&validate.stdout).unwrap();
    let checks = output["data"]["checks"].as_array().unwrap();
    assert!(checks
        .iter()
        .any(|check| { check["name"] == "renderer-manifest" && check["status"] == "pass" }));
    assert!(checks.iter().any(|check| {
        check["name"] == "renderer.backend.capability" && check["status"] == "skip"
    }));
    assert!(checks
        .iter()
        .any(|check| { check["name"] == "renderer.install_tree" && check["status"] == "pass" }));
    // Every assertion names the producer session behind it, rather than
    // presenting the report as one anonymous verdict.
    assert!(
        checks.iter().any(|check| {
            check["name"] == "renderer.install_tree"
                && check["detail"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("producer headless-1")
        }),
        "a surfaced check must name its producer: {checks:#?}"
    );

    let external_dir = format!("build/{id}");
    let external = sb.ost(&["--json", "validate", "--build-dir", &external_dir]);
    assert!(
        external.status.success(),
        "external evidence validation failed:\n{}",
        out_text(&external)
    );
    let output: serde_json::Value = serde_json::from_slice(&external.stdout).unwrap();
    let checks = output["data"]["checks"].as_array().unwrap();
    assert!(checks
        .iter()
        .any(|check| check["name"] == "built" && check["status"] == "skip"));
    assert!(checks
        .iter()
        .any(|check| check["name"] == "external-build" && check["status"] == "pass"));

    // The same report with its producer session stripped is what a pre-v0.18.0
    // harness writes, and the shape the hdMerlin defect took: PASSes nothing
    // stands behind. `ost validate` must refuse it rather than surface them.
    let report_path = build.join("renderer-report.json");
    let report = std::fs::read_to_string(&report_path).unwrap();
    let mut stripped: serde_json::Value = serde_json::from_str(&report).unwrap();
    stripped.as_object_mut().unwrap().remove("producer");
    std::fs::write(&report_path, serde_json::to_string(&stripped).unwrap()).unwrap();

    let unowned = sb.ost(&["--json", "validate"]);
    assert!(
        !unowned.status.success(),
        "an unowned PASS must not validate:\n{}",
        out_text(&unowned)
    );
    let output: serde_json::Value = serde_json::from_slice(&unowned.stdout).unwrap();
    let checks = output["data"]["checks"].as_array().unwrap();
    let evidence = checks
        .iter()
        .find(|check| check["name"] == "renderer-evidence")
        .expect("renderer-evidence check");
    assert_eq!(evidence["status"], "fail");
    assert!(
        evidence["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("records no producer session"),
        "the refusal must name the missing producer: {evidence:#?}"
    );
}

#[test]
fn validate_rejects_a_partial_build_directory() {
    let sb = Sandbox::new("partial-build");
    init_and_pull(&sb);
    let configure = sb.ost(&["configure"]);
    assert!(
        configure.status.success(),
        "configure failed:\n{}",
        out_text(&configure)
    );

    let target = single_target_dir(&sb.work);
    let id = target.file_name().unwrap().to_string_lossy().into_owned();
    let build = sb.work.join("build").join(id);
    std::fs::create_dir_all(&build).unwrap();
    std::fs::write(build.join("CMakeCache.txt"), "partial configure\n").unwrap();

    let validate = sb.ost(&["--json", "validate"]);
    assert!(
        !validate.status.success(),
        "a partial directory must not pass validation"
    );
    let output: serde_json::Value = serde_json::from_slice(&validate.stdout).unwrap();
    let checks = output["data"]["checks"].as_array().unwrap();
    assert!(checks
        .iter()
        .any(|check| check["name"] == "built" && check["status"] == "fail"));
}

#[test]
fn build_failure_names_the_phase_and_log() {
    if let Err(reason) = native_lifecycle_ready() {
        eprintln!("skipping build_failure: {reason}");
        return;
    }
    let sb = Sandbox::new("build-failure");
    init_and_pull(&sb);

    // Break the scaffolded source so the build (not configure) phase fails.
    let src = find_first(&sb.work.join("src"), ".cpp").expect("a template .cpp source");
    std::fs::write(&src, "this is not valid c++ @@@\n").unwrap();

    let out = sb.ost(&["build", "--progress", "plain"]);
    assert!(!out.status.success(), "a broken source must fail the build");
    let text = out_text(&out);
    assert!(
        text.contains("status=failed"),
        "should report a failed phase:\n{text}"
    );
    assert!(
        text.contains("build.log"),
        "should point at the build log:\n{text}"
    );
}

#[test]
fn plugin_build_failure_carries_the_phase_in_json() {
    if let Err(reason) = native_lifecycle_ready() {
        eprintln!("skipping plugin_build_failure phase test: {reason}");
        return;
    }
    let sb = Sandbox::new("plugin-build-phase");
    init_and_pull(&sb);
    let new = sb.ost(&[
        "plugin",
        "new",
        "usd-fileformat",
        "toy",
        "--extension",
        "toy",
    ]);
    assert!(
        new.status.success(),
        "plugin new failed:\n{}",
        out_text(&new)
    );

    // A plugin build against the mock runtime deterministically fails at the
    // configure phase: the mock provides no real pxrConfig for `find_package(pxr)`.
    // That is enough to prove the failure is phase-attributed in the JSON envelope.
    let out = sb.ost(&[
        "--json",
        "plugin",
        "build",
        "toy",
        "--target",
        "cy2026",
        "--profile",
        "usd",
    ]);
    assert!(
        !out.status.success(),
        "a plugin build on a mock runtime must fail"
    );
    // The child's build output shares stdout with the envelope, so scan for the
    // phase-attributed error fields rather than parsing one JSON document.
    let text = out_text(&out);
    assert!(
        text.contains("\"phase\": \"configure\""),
        "the build failure should attribute the configure phase:\n{text}"
    );
    assert!(
        text.contains("\"code\": \"EXTERNAL_TOOL_FAILED\""),
        "a failed build tool is an external-tool failure:\n{text}"
    );
}

#[test]
fn generated_plugin_scaffolds_and_inspects() {
    let sb = Sandbox::new("plugin-scaffold");
    init_and_pull(&sb);

    // Scaffold a file-format plugin bundle inside the project.
    let new = sb.ost(&[
        "plugin",
        "new",
        "usd-fileformat",
        "toy",
        "--extension",
        "toy",
    ]);
    assert!(
        new.status.success(),
        "plugin new failed:\n{}",
        out_text(&new)
    );
    let bundle = sb.work.join("toy");
    assert!(
        bundle.join("openstrata.plugin.yaml").is_file(),
        "the bundle should carry its plugin manifest"
    );
    assert!(
        bundle.join("cmake/OpenStrataPlugin.cmake").is_file(),
        "the bundle should carry its self-contained CMake helper"
    );

    // Inspect is a static Level 0 report. A freshly scaffolded bundle legitimately
    // fails the *one* check that needs a built `.so` — so assert the structural
    // checks pass rather than the overall exit code (no compiler in CI).
    let inspect = sb.ost(&["plugin", "inspect", "toy"]);
    let text = out_text(&inspect);
    assert!(
        text.contains("[PASS] L0 bundle.manifest"),
        "manifest should validate:\n{text}"
    );
    assert!(
        text.contains("[PASS] L0 bundle.plug_info"),
        "plugInfo.json should validate:\n{text}"
    );
    assert!(
        text.contains("plugin.shared_library"),
        "inspect should report the (not-yet-built) shared library:\n{text}"
    );
    assert!(
        text.contains("license: Apache-2.0"),
        "inspect should surface the scaffolded plugin's license:\n{text}"
    );

    let package = sb.ost(&["plugin", "package", "toy"]);
    assert_eq!(
        package.status.code(),
        Some(5),
        "unbuilt plugin package should fail validation:\n{}",
        out_text(&package)
    );
    assert!(
        out_text(&package).contains("did not pass static packaging validation"),
        "package failure should be actionable:\n{}",
        out_text(&package)
    );

    let lib = bundle.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(
        lib.join(format!("libToyFileFormat{}", std::env::consts::DLL_SUFFIX)),
        b"fake shared library for static package validation",
    )
    .unwrap();
    // A source L5 oracle is verification content, not merely a neighboring
    // developer file: packaging must preserve and bind it to the fixture.
    std::fs::write(
        bundle.join("tests/fixtures/basic.toy.golden.usda"),
        b"#usda 1.0\n",
    )
    .unwrap();
    let package = sb.ost(&["plugin", "package", "toy"]);
    assert!(
        package.status.success(),
        "plugin package failed:\n{}",
        out_text(&package)
    );
    let dist = bundle.join("dist");
    assert!(
        find_first(&dist, "tar.zst").is_some(),
        "plugin package should write an archive under dist/"
    );
    let manifest = find_first(&dist, "manifest.json").expect("plugin manifest exists");
    let manifest_text = std::fs::read_to_string(manifest).unwrap();
    let manifest_value: serde_json::Value = serde_json::from_str(&manifest_text).unwrap();
    assert!(manifest_text.contains("\"kind\": \"openstrata.plugin-bundle\""));
    assert!(manifest_text.contains("\"cxx_abi\""));
    assert!(
        manifest_text.contains("openstrata.scaffold.yaml"),
        "package should preserve deterministic scaffold provenance:\n{manifest_text}"
    );
    assert!(
        manifest_text.contains("\"license\": \"Apache-2.0\""),
        "package manifest should record the plugin license:\n{manifest_text}"
    );
    assert_eq!(
        manifest_value["verification"]["schema"],
        "openstrata.plugin-verification/v1alpha1"
    );
    assert_eq!(
        manifest_value["verification"]["contract"],
        "openstrata.verification.json"
    );
    assert_eq!(manifest_value["verification"]["roundtrip_oracles"], 1);
    assert_eq!(
        manifest_value["provenance"]["build_outputs"]["status"],
        "untracked"
    );
    assert_eq!(
        manifest_value["provenance"]["build_outputs"]["origin"],
        "external-or-unmanaged"
    );
    let golden_file = manifest_value["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["path"] == "tests/fixtures/basic.toy.golden.usda")
        .expect("the adjacent golden must be archived and hashed");
    let contract_file = manifest_value["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["path"] == "openstrata.verification.json")
        .expect("the versioned verification contract must be archived");
    assert!(contract_file["sha256"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    let contract_path = find_first(&bundle.join(".strata"), "openstrata.verification.json")
        .expect("package stage carries the verification contract");
    let contract: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(contract_path).unwrap()).unwrap();
    assert_eq!(
        contract["roundtrip"][0]["fixture"],
        "tests/fixtures/basic.toy"
    );
    assert_eq!(
        contract["roundtrip"][0]["oracle"],
        "tests/fixtures/basic.toy.golden.usda"
    );
    assert_eq!(
        contract["roundtrip"][0]["oracle_sha256"],
        golden_file["sha256"]
    );
    assert!(find_first(&dist, "SHA256SUMS").is_some());
}

#[test]
fn plugin_package_refuses_overwritten_managed_outputs_without_an_explicit_override() {
    let sb = Sandbox::new("plugin-output-provenance");
    init_and_pull(&sb);
    let scaffold = sb.ost(&[
        "plugin",
        "new",
        "usd-fileformat",
        "toy",
        "--extension",
        "toy",
    ]);
    assert!(scaffold.status.success(), "{}", out_text(&scaffold));
    let configured = sb.ost(&["configure"]);
    assert!(configured.status.success(), "{}", out_text(&configured));

    let bundle = sb.work_file("toy");
    let library_relative = format!("lib/libToyFileFormat{}", std::env::consts::DLL_SUFFIX);
    let library = bundle.join(&library_relative);
    std::fs::create_dir_all(library.parent().unwrap()).unwrap();
    std::fs::write(&library, b"managed build bytes").unwrap();

    // The project target lock carries the same selected target/runtime identity
    // a successful `ost plugin build` writes into the bundle target state.
    let project_target = single_target_dir(&sb.work);
    let target_id = project_target.file_name().unwrap().to_string_lossy();
    let lock: ost_build::TargetLock = serde_json::from_str(
        &std::fs::read_to_string(project_target.join("target.lock.json")).unwrap(),
    )
    .unwrap();
    let bundle_target = bundle.join(".strata/targets").join(target_id.as_ref());
    let bundle_build = bundle.join("build").join(target_id.as_ref());
    std::fs::create_dir_all(&bundle_target).unwrap();
    std::fs::create_dir_all(&bundle_build).unwrap();
    std::fs::write(
        bundle_target.join("target.lock.json"),
        lock.to_json().unwrap(),
    )
    .unwrap();
    let mut intent = ost_build::BuildIntent::default();
    intent
        .cache
        .insert("CMAKE_BUILD_TYPE".into(), "Release".into());
    let completion = ost_build::BuildCompletion::from_lock(
        &lock,
        ost_build::BuildProjectIdentity {
            name: "toy".into(),
            version: "0.1.0".into(),
        },
        format!("build/{target_id}"),
        intent,
        1,
    )
    .with_outputs(vec![
        managed_build_output(&bundle, &library_relative),
        managed_build_output(&bundle, "plugin/resources/toy/plugInfo.json"),
        managed_build_output(&bundle, "plugin/resources/toy/plugInfo.json.in"),
    ]);
    std::fs::write(
        bundle_build.join(ost_build::BUILD_COMPLETION_FILE),
        completion.to_json().unwrap(),
    )
    .unwrap();

    let matched = sb.ost(&["--json", "plugin", "package", "toy"]);
    assert!(matched.status.success(), "{}", out_text(&matched));
    let matched: serde_json::Value = serde_json::from_slice(&matched.stdout).unwrap();
    assert_eq!(matched["data"]["build_provenance"]["status"], "matched");
    assert_eq!(matched["data"]["build_provenance"]["origin"], "ost-managed");

    std::fs::write(&library, b"plain CMake replacement").unwrap();
    let refused = sb.ost(&["--json", "plugin", "package", "toy"]);
    assert_eq!(refused.status.code(), Some(5), "{}", out_text(&refused));
    let refusal = out_text(&refused);
    assert!(
        refusal.contains("PLUGIN_PACKAGE_OUTPUT_MISMATCH"),
        "{refusal}"
    );
    assert!(
        refusal.contains(&library_relative.replace('\\', "/")),
        "{refusal}"
    );
    assert!(refusal.contains("expected sha256:"), "{refusal}");
    assert!(refusal.contains("observed sha256:"), "{refusal}");
    assert!(refusal.contains("last managed build sha256:"), "{refusal}");

    let overridden = sb.ost(&[
        "--json",
        "plugin",
        "package",
        "toy",
        "--allow-unmanaged-output",
    ]);
    assert!(overridden.status.success(), "{}", out_text(&overridden));
    let overridden: serde_json::Value = serde_json::from_slice(&overridden.stdout).unwrap();
    let provenance = &overridden["data"]["build_provenance"];
    assert_eq!(provenance["status"], "mismatched");
    assert_eq!(provenance["origin"], "external-or-unmanaged-override");
    assert_eq!(provenance["override_accepted"], true);
    assert_eq!(
        overridden["warnings"][0]["code"],
        "PLUGIN_PACKAGE_OUTPUT_MISMATCH_OVERRIDDEN"
    );

    let manifest_path =
        find_first(&bundle.join("dist"), "manifest.json").expect("package manifest");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path).unwrap()).unwrap();
    assert_eq!(
        manifest["provenance"]["build_outputs"]["status"],
        "mismatched"
    );
    assert_eq!(
        manifest["provenance"]["build_outputs"]["origin"],
        "external-or-unmanaged-override"
    );
}

#[test]
fn generated_asset_resolver_requires_scheme_and_records_provenance() {
    let sb = Sandbox::new("resolver-scaffold");
    init_and_pull(&sb);

    let missing = sb.ost(&["plugin", "new", "usd-asset-resolver", "studio-assets"]);
    assert_eq!(missing.status.code(), Some(4), "{}", out_text(&missing));
    assert!(out_text(&missing).contains("needs --scheme"));

    let out = sb.ost(&[
        "--json",
        "plugin",
        "new",
        "usd-asset-resolver",
        "studio-assets",
        "--scheme",
        "studio",
    ]);
    assert!(
        out.status.success(),
        "resolver scaffold failed:\n{}",
        out_text(&out)
    );
    let body: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["kind"], "usd-asset-resolver");

    let root = sb.work.join("studio-assets");
    let provenance: serde_yaml::Value = serde_yaml::from_str(
        &std::fs::read_to_string(root.join("openstrata.scaffold.yaml")).unwrap(),
    )
    .unwrap();
    assert_eq!(provenance["template"]["id"], "usd-asset-resolver-cpp");
    assert_eq!(provenance["inputs"]["scheme"], "studio");
    assert!(root
        .join("plugin/resources/studio-assets/plugInfo.json")
        .is_file());
    assert!(root.join("cmake/OpenStrataPlugin.cmake").is_file());
}

#[test]
fn generated_package_resolver_requires_extension_and_records_provenance() {
    let sb = Sandbox::new("pkg-resolver-scaffold");
    init_and_pull(&sb);

    let missing = sb.ost(&["plugin", "new", "usd-package-resolver", "shot-pack"]);
    assert_eq!(missing.status.code(), Some(4), "{}", out_text(&missing));
    assert!(out_text(&missing).contains("needs --extension"));

    let out = sb.ost(&[
        "--json",
        "plugin",
        "new",
        "usd-package-resolver",
        "shot-pack",
        "--extension",
        "pack",
    ]);
    assert!(
        out.status.success(),
        "package resolver scaffold failed:\n{}",
        out_text(&out)
    );
    let body: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["kind"], "usd-package-resolver");
    assert_eq!(body["data"]["template"], "usd-package-resolver-cpp");

    let root = sb.work.join("shot-pack");
    let provenance: serde_yaml::Value = serde_yaml::from_str(
        &std::fs::read_to_string(root.join("openstrata.scaffold.yaml")).unwrap(),
    )
    .unwrap();
    assert_eq!(provenance["template"]["id"], "usd-package-resolver-cpp");
    assert_eq!(provenance["inputs"]["extension"], "pack");
    assert!(root
        .join("plugin/resources/shot-pack/plugInfo.json")
        .is_file());
    assert!(root.join("cmake/OpenStrataPlugin.cmake").is_file());
    // The sidecar package entry lands so the smoke fixture's packaged
    // sublayer path resolves once the plugin is built.
    assert!(root
        .join("tests/fixtures/basic.pack.contents/content/inner.usda")
        .is_file());
}

#[test]
fn generated_openexec_plugin_requires_schema_contract_inputs() {
    let sb = Sandbox::new("exec-scaffold");

    let missing = sb.ost(&["plugin", "new", "usd-exec", "pose-eval"]);
    assert_eq!(missing.status.code(), Some(4), "{}", out_text(&missing));
    assert!(out_text(&missing).contains("needs --schema-bundle"));

    let partial = sb.ost(&[
        "plugin",
        "new",
        "usd-exec",
        "pose-eval",
        "--schema-bundle",
        "rig-schema",
    ]);
    assert_eq!(partial.status.code(), Some(2), "{}", out_text(&partial));
    assert!(out_text(&partial).contains("must be provided together"));

    let out = sb.ost(&[
        "--json",
        "plugin",
        "new",
        "usd-exec",
        "pose-eval",
        "--schema-bundle",
        "rig-schema",
        "--schema-type",
        "RigContractAPI",
    ]);
    assert!(
        out.status.success(),
        "OpenExec scaffold failed:\n{}",
        out_text(&out)
    );
    let body: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(body["data"]["kind"], "usd-exec");
    assert_eq!(body["data"]["template"], "usd-exec-cpp");

    let root = sb.work.join("pose-eval");
    let provenance: serde_yaml::Value = serde_yaml::from_str(
        &std::fs::read_to_string(root.join("openstrata.scaffold.yaml")).unwrap(),
    )
    .unwrap();
    assert_eq!(provenance["template"]["id"], "usd-exec-cpp");
    assert_eq!(provenance["inputs"]["schema_bundle"], "rig-schema");
    assert_eq!(provenance["inputs"]["schema_type"], "RigContractAPI");
    assert!(root.join("src/PoseEvalPlugin.cpp").is_file());
}

#[test]
fn compiled_schema_template_is_selected_explicitly_and_reported() {
    let sb = Sandbox::new("schema-cpp-scaffold");
    let out = sb.ost(&[
        "--json",
        "plugin",
        "new",
        "usd-schema",
        "vrm-schema",
        "--template",
        "usd-schema-cpp",
    ]);
    assert!(out.status.success(), "{}", out_text(&out));

    let body: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(body["data"]["template"], "usd-schema-cpp");
    let root = sb.work.join("vrm-schema");
    let provenance: serde_yaml::Value = serde_yaml::from_str(
        &std::fs::read_to_string(root.join("openstrata.scaffold.yaml")).unwrap(),
    )
    .unwrap();
    assert_eq!(provenance["template"]["id"], "usd-schema-cpp");
    assert!(root.join("generated/contractAPI.cpp").is_file());
    assert!(root.join("tests/consumer/CMakeLists.txt").is_file());

    let wrong = sb.ost(&[
        "plugin",
        "new",
        "usd-fileformat",
        "wrong",
        "--extension",
        "wrong",
        "--template",
        "usd-schema-cpp",
    ]);
    assert_eq!(wrong.status.code(), Some(4), "{}", out_text(&wrong));
    assert!(out_text(&wrong).contains("not available for plugin kind 'usd-fileformat'"));
}

/// `ost plugin test --workspace` discovers bundles at the root and under
/// plugins/*, tests each, and aggregates. Codeless schema scaffolds pass
/// L0/L1 without a build, so this runs against the mock runtime.
#[test]
fn workspace_test_discovers_and_tests_every_bundle() {
    let sb = Sandbox::new("wstest");
    init_and_pull(&sb);

    for args in [
        vec!["plugin", "new", "usd-schema", "alpha"],
        vec![
            "plugin",
            "new",
            "usd-schema",
            "beta",
            "--dir",
            "plugins/beta",
        ],
    ] {
        let out = sb.ost(&args);
        assert!(out.status.success(), "scaffold failed:\n{}", out_text(&out));
    }
    let v: serde_json::Value = serde_json::from_slice(
        &sb.ost(&["--json", "plugin", "test", "--workspace", "--up-to", "1"])
            .stdout,
    )
    .unwrap();
    assert_eq!(v["ok"], true, "workspace test passes: {v}");
    assert_eq!(v["data"]["total"], 2);
    assert_eq!(v["data"]["failed"], 0);
    assert_eq!(v["data"]["graph"]["passed"], true);
    assert_eq!(v["data"]["graph"]["nodes"].as_array().unwrap().len(), 2);
    let bundles = v["data"]["bundles"].as_array().unwrap();
    assert_eq!(bundles.len(), 2);
    for bundle in bundles {
        let dir = bundle["report_dir"].as_str().unwrap();
        assert!(Path::new(dir).is_dir(), "report dir exists: {dir}");
    }

    // Usage errors: a bundle path with --workspace, or neither.
    assert_eq!(
        sb.ost(&["--json", "plugin", "test", "alpha", "--workspace"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(sb.ost(&["--json", "plugin", "test"]).status.code(), Some(2));
}

#[test]
fn workspace_test_rejects_an_invalid_dependency_graph_before_bundle_tests() {
    let sb = Sandbox::new("wsgraph");
    init_and_pull(&sb);
    for name in ["alpha", "beta"] {
        let out = sb.ost(&["plugin", "new", "usd-schema", name]);
        assert!(out.status.success(), "scaffold failed:\n{}", out_text(&out));
    }
    for (name, dependency) in [("alpha", "beta"), ("beta", "alpha")] {
        let path = sb.work_file(&format!("{name}/openstrata.plugin.yaml"));
        let source = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            path,
            format!(
                "{source}requires:\n  bundles:\n    - {{ id: {dependency}, version: '>=0.1,<0.2', contract: 1 }}\n"
            ),
        )
        .unwrap();
    }

    let out = sb.ost(&["--json", "plugin", "test", "--workspace", "--up-to", "0"]);
    assert_eq!(out.status.code(), Some(5), "{}", out_text(&out));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["data"]["graph"]["passed"], false);
    let codes: Vec<_> = value["data"]["graph"]["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|issue| issue["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"WORKSPACE_DEPENDENCY_DIRECTION_FORBIDDEN"));
    assert!(codes.contains(&"WORKSPACE_DEPENDENCY_CYCLE"));
    assert!(
        !sb.work_file("alpha/.strata/reports").exists(),
        "graph validation must run before bundle tests"
    );
}

#[test]
fn selected_and_workspace_tests_compose_manifest_dependencies_without_with() {
    let sb = Sandbox::new("wsclosure");
    init_and_pull(&sb);
    for args in [
        vec!["plugin", "new", "usd-schema", "schema"],
        vec![
            "plugin",
            "new",
            "usd-fileformat",
            "consumer",
            "--extension",
            "toy",
        ],
    ] {
        let out = sb.ost(&args);
        assert!(out.status.success(), "scaffold failed:\n{}", out_text(&out));
    }
    let lib = sb.work_file(&format!(
        "consumer/lib/{}ConsumerFileFormat{}",
        "lib",
        std::env::consts::DLL_SUFFIX
    ));
    std::fs::create_dir_all(lib.parent().unwrap()).unwrap();
    std::fs::write(lib, b"test library marker").unwrap();

    let path = sb.work_file("consumer/openstrata.plugin.yaml");
    let source = std::fs::read_to_string(&path).unwrap();
    let source = source.replace(
        "requires:\n  capabilities: [usd-stage-read]\n",
        "requires:\n  capabilities: [usd-stage-read]\n  bundles:\n    - { id: schema, version: '>=0.1,<0.2', contract: 1 }\n",
    );
    std::fs::write(
        &path,
        format!("manifest:\n  schema: openstrata.plugin/v1alpha1\n{source}"),
    )
    .unwrap();

    let build = sb.ost(&["plugin", "build", "consumer", "--dry-run"]);
    assert!(build.status.success(), "{}", out_text(&build));
    let plan = out_text(&build);
    assert!(plan.contains("workspace-prefix"), "{plan}");
    assert!(plan.contains("== build dependency schema =="), "{plan}");
    assert!(plan.contains("cmake --install"), "{plan}");
    assert!(plan.contains("-DCMAKE_INSTALL_PREFIX="), "{plan}");
    assert!(
        !plan.contains("add_subdirectory") && !plan.contains("OPENSTRATA_SCHEMA_SOURCES_FILE"),
        "composition uses installed package discovery and plain bundles render no stale schema fragment: {plan}"
    );

    // No --with: the selected command finds the containing source workspace
    // and injects the provider from requires.bundles.
    let out = sb.ost(&["--json", "plugin", "doctor", "consumer"]);
    assert!(out.status.success(), "{}", out_text(&out));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let plugin_paths = value["data"]["environment"]["vars"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["key"] == "PXR_PLUGINPATH_NAME")
        .filter_map(|item| item["value"].as_str())
        .collect::<Vec<_>>();
    assert!(plugin_paths.iter().any(|path| path.contains("consumer")));
    assert!(plugin_paths.iter().any(|path| path.contains("schema")));

    // The whole-workspace path computes a distinct closure for each bundle and
    // still needs no duplicated --with declaration.
    let out = sb.ost(&["--json", "plugin", "test", "--workspace", "--up-to", "1"]);
    assert!(out.status.success(), "{}", out_text(&out));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["data"]["graph"]["edges"][0]["from"], "consumer");
    assert_eq!(value["data"]["graph"]["edges"][0]["to"], "schema");

    // A clean-install run that points at some other package is almost always a
    // mistaken experiment. Warn before runtime launch; a matching extracted
    // identity is the intended --no-inject shape and stays quiet.
    let out = sb.ost(&[
        "plugin",
        "run",
        "consumer",
        "--no-inject",
        "--plugin-path",
        "schema",
        "--",
        "unused-command",
    ]);
    assert!(
        !out.status.success(),
        "the sandbox runtime is intentionally mock"
    );
    assert!(
        out_text(&out).contains("PLUGIN_RUN_PLUGIN_PATH_MISMATCH"),
        "a mismatched clean-install root must be called out: {}",
        out_text(&out)
    );
    let out = sb.ost(&[
        "plugin",
        "run",
        "consumer",
        "--no-inject",
        "--plugin-path",
        "consumer",
        "--",
        "unused-command",
    ]);
    assert!(
        !out.status.success(),
        "the sandbox runtime is intentionally mock"
    );
    assert!(
        !out_text(&out).contains("PLUGIN_RUN_PLUGIN_PATH_MISMATCH"),
        "the matching extracted identity is the intended clean-install shape: {}",
        out_text(&out)
    );

    // A broken sibling only matters to bundles that declare dependencies: the
    // schema (empty closure) skips workspace discovery entirely, while the
    // consumer legitimately fails closed on the unloadable workspace.
    let broken = sb.work_file("broken/openstrata.plugin.yaml");
    std::fs::create_dir_all(broken.parent().unwrap()).unwrap();
    std::fs::write(&broken, "plugin: [not: a manifest").unwrap();
    let out = sb.ost(&["--json", "plugin", "doctor", "schema"]);
    assert!(
        out.status.success(),
        "a dependency-free bundle must ignore unrelated siblings:\n{}",
        out_text(&out)
    );
    let out = sb.ost(&["--json", "plugin", "doctor", "consumer"]);
    assert!(
        !out.status.success(),
        "a declared closure fails closed on an unloadable workspace:\n{}",
        out_text(&out)
    );
}

/// A bundle that depends on another used to ship with nothing saying so: the
/// omission surfaced on the consumer's machine as a schema-application failure.
/// The resolved bundle closure now travels with the artifact, and `--workspace`
/// packages providers before the bundles whose closure names them.
#[test]
fn workspace_packaging_records_the_bundle_closure_in_dependency_order() {
    let sb = Sandbox::new("wspackage");
    init_and_pull(&sb);
    for args in [
        vec![
            "plugin",
            "new",
            "usd-schema",
            "schema",
            "--template",
            "usd-schema-cpp",
        ],
        vec![
            "plugin",
            "new",
            "usd-fileformat",
            "consumer",
            "--extension",
            "toy",
        ],
    ] {
        let out = sb.ost(&args);
        assert!(out.status.success(), "scaffold failed:\n{}", out_text(&out));
    }
    // Both bundles need a library artifact present to package.
    for (bundle, stem) in [("schema", "Schema"), ("consumer", "ConsumerFileFormat")] {
        let lib = sb.work_file(&format!(
            "{bundle}/lib/lib{stem}{}",
            std::env::consts::DLL_SUFFIX
        ));
        std::fs::create_dir_all(lib.parent().unwrap()).unwrap();
        std::fs::write(lib, b"test library marker").unwrap();
    }

    let path = sb.work_file("consumer/openstrata.plugin.yaml");
    let source = std::fs::read_to_string(&path).unwrap();
    let source = source.replace(
        "requires:\n  capabilities: [usd-stage-read]\n",
        "requires:\n  capabilities: [usd-stage-read]\n  runtime_libs: [third_party/bin]\n  bundles:\n    - { id: schema, version: '>=0.1,<0.2', contract: 1 }\n",
    );
    std::fs::write(
        &path,
        format!("manifest:\n  schema: openstrata.plugin/v1alpha1\n{source}"),
    )
    .unwrap();
    std::fs::create_dir_all(sb.work_file("consumer/third_party/bin")).unwrap();
    std::fs::write(
        sb.work_file("consumer/third_party/bin/runtime-dependency.marker"),
        b"runtime dependency",
    )
    .unwrap();

    let out = sb.ost(&["--json", "plugin", "package", "--workspace", "--product"]);
    assert!(
        out.status.success(),
        "workspace package failed:\n{}",
        out_text(&out)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let order: Vec<&str> = value["data"]["order"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry.as_str())
        .collect();
    // The provider is packaged first: the consumer's recorded closure names an
    // artifact that must already exist.
    assert_eq!(
        order,
        vec!["schema", "consumer"],
        "providers must be packaged before their dependents"
    );
    assert_eq!(value["data"]["packages"].as_array().unwrap().len(), 2);
    for package in value["data"]["packages"].as_array().unwrap() {
        assert_eq!(package["debug_archive"], serde_json::Value::Null);
        assert_eq!(package["debug_package"]["mode"], "not-produced");
        assert!(package["debug_package"]["reason"]
            .as_str()
            .unwrap()
            .contains(".pdb"));
    }
    assert_eq!(value["data"]["product"]["members"], 2);
    assert!(value["data"]["product"]["archive"]
        .as_str()
        .unwrap()
        .ends_with("-plugin-product.tar.zst"));

    // The consumer's artifact carries the bundle it needs, so a consumer can
    // detect a missing provider from the manifest instead of at load time.
    let manifest = find_first(&sb.work_file("consumer/dist"), "manifest.json").unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest).unwrap()).unwrap();
    let bundles = value["dependencies"]["bundles"].as_array().unwrap();
    assert_eq!(bundles.len(), 1, "the declared provider is recorded");
    assert_eq!(bundles[0]["id"], "schema");
    assert_eq!(bundles[0]["kind"], "usd-schema");
    // The schema contract is what a dependent actually binds to.
    assert_eq!(bundles[0]["contract"], 1);
    assert_eq!(bundles[0]["provenance"], "source-workspace");

    // …and it carries the provider's USD *registration* half, not just the
    // record and the link half. v0.18.0 shipped `libSchemaLib` plus a resolved
    // `bundles` entry while leaving `plugInfo.json` out, so the package asserted
    // a closure it did not have and still failed at `Usd.Stage.Open()`
    // (usd-vrm-plugins report 23 §2).
    let staged: Vec<&str> = value["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["path"].as_str())
        .collect();
    assert!(
        staged.contains(&"runtime/bundles/schema/plugin/resources/schema/plugInfo.json"),
        "the provider's registration half must be staged; got: {staged:?}"
    );
    let provider_library = format!(
        "runtime/bundles/schema/lib/libSchema{}",
        std::env::consts::DLL_SUFFIX
    );
    assert!(
        staged.contains(&provider_library.as_str()),
        "the provider's link half must stay beside its copied plugInfo tree; got: {staged:?}"
    );
    for activation in [
        "openstrata.activation.json",
        "activate.ps1",
        "activate.sh",
        "openstrata_activate.py",
    ] {
        assert!(
            staged.contains(&activation),
            "consumer-facing activation entrypoint '{activation}' must ship"
        );
    }
    assert_eq!(value["debug_package"]["mode"], "not-produced");
    let activation_path = find_first(
        &sb.work_file("consumer/.strata"),
        "openstrata.activation.json",
    )
    .unwrap();
    let activation: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(activation_path).unwrap()).unwrap();
    assert_eq!(activation["schema"], "openstrata.activation/v1alpha1");
    assert!(activation["library_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path == "third_party/bin"));
    assert!(activation["library_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path == "runtime/bundles/schema/lib"));
    assert_eq!(
        activation["environment"]["loader"],
        if cfg!(windows) {
            "PATH"
        } else {
            "LD_LIBRARY_PATH"
        }
    );
    // Every staged path is portable: this list is read on hosts that never saw
    // the producer's separators.
    for path in &staged {
        assert!(!path.contains('\\'), "staged path kept separators: {path}");
    }

    // A provider with no dependencies records an empty closure, not a missing
    // one — "nothing required" and "unknown" must not look the same.
    let schema_manifest = find_first(&sb.work_file("schema/dist"), "manifest.json").unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(schema_manifest).unwrap()).unwrap();
    if let Some(dependencies) = value.get("dependencies") {
        assert!(
            dependencies["bundles"]
                .as_array()
                .is_none_or(|items| items.is_empty()),
            "a dependency-free bundle records no providers"
        );
    }

    // The aggregate is one download containing the exact package bytes and all
    // member provenance sidecars. Its order is the validated graph order, not a
    // second hand-maintained list.
    let product_manifest = find_first(&sb.work_file("dist/products"), "manifest.json").unwrap();
    let product: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&product_manifest).unwrap()).unwrap();
    assert_eq!(product["kind"], "openstrata.plugin-product");
    assert_eq!(
        product["install_order"],
        serde_json::json!(["schema", "consumer"])
    );
    let members = product["members"].as_array().unwrap();
    assert_eq!(members.len(), 2);
    assert_eq!(members[0]["id"], "schema");
    assert_eq!(members[1]["id"], "consumer");
    for member in members {
        assert!(member["archive_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
        assert!(member["manifest"].as_str().unwrap().starts_with("members/"));
        assert!(member["checksums"]
            .as_str()
            .unwrap()
            .starts_with("members/"));
    }
    let product_files: Vec<_> = product["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["path"].as_str())
        .collect();
    assert!(product_files.contains(&"openstrata.product.json"));
    assert!(product_files
        .iter()
        .any(|path| path.starts_with("members/schema/") && path.ends_with(".tar.zst")));
    assert!(product_files
        .iter()
        .any(|path| path.starts_with("members/consumer/") && path.ends_with("manifest.json")));

    let product_dist = product_manifest.parent().unwrap().to_str().unwrap();
    let imported = sb.ost(&["--json", "artifact", "import", product_dist]);
    assert!(
        imported.status.success(),
        "aggregate must be a first-class registry artifact: {}",
        out_text(&imported)
    );
    let imported: serde_json::Value = serde_json::from_slice(&imported.stdout).unwrap();
    assert_eq!(imported["data"]["artifact"]["kind"], "product");
}

/// Documents that no path bakes host separators into a portable artifact.
#[test]
fn packaged_dependency_paths_are_forward_slashed() {
    let sb = Sandbox::new("wspaths");
    init_and_pull(&sb);
    let scaffold = sb.ost(&[
        "plugin",
        "new",
        "usd-fileformat",
        "consumer",
        "--extension",
        "toy",
    ]);
    assert!(scaffold.status.success(), "{}", out_text(&scaffold));

    let lib = sb.work_file(&format!(
        "consumer/lib/libConsumerFileFormat{}",
        std::env::consts::DLL_SUFFIX
    ));
    std::fs::create_dir_all(lib.parent().unwrap()).unwrap();
    std::fs::write(lib, b"test library marker").unwrap();

    let manifest_path = sb.work_file("consumer/openstrata.plugin.yaml");
    let source = std::fs::read_to_string(&manifest_path).unwrap();
    let source = source.replace(
        "requires:\n  capabilities: [usd-stage-read]\n",
        "requires:\n  capabilities: [usd-stage-read]\n  libraries:\n    - { id: container, version: '>=1.0,<2.0' }\n",
    );
    std::fs::write(
        &manifest_path,
        format!("manifest:\n  schema: openstrata.plugin/v1alpha1\n{source}"),
    )
    .unwrap();
    std::fs::create_dir_all(sb.work_file("container")).unwrap();
    std::fs::write(
        sb.work_file("container/openstrata.library.yaml"),
        "schema: openstrata.library/v1alpha1\nlibrary:\n  id: container\n  version: 1.2.0\ncmake:\n  package: Container\n  target: Container::container\nruntime:\n  directories: [bin]\n",
    )
    .unwrap();

    let out = sb.ost(&["--json", "plugin", "test", "--workspace", "--up-to", "1"]);
    assert!(out.status.success(), "{}", out_text(&out));

    // Every recorded path in the evidence document uses `/`, whatever host
    // produced it: this file is read on machines that never saw `\`.
    if let Some(evidence) = find_first(&sb.work, "dependencies.json") {
        let text = std::fs::read_to_string(evidence).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        for library in value["libraries"].as_array().into_iter().flatten() {
            for key in ["descriptor", "prefix"] {
                if let Some(path) = library[key].as_str() {
                    assert!(!path.contains('\\'), "{key} kept host separators: {path}");
                }
            }
            for directory in library["runtime_directories"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let path = directory.as_str().unwrap_or_default();
                assert!(!path.contains('\\'), "runtime dir kept separators: {path}");
            }
        }
    }
}

#[test]
fn plain_library_dependencies_validate_inspect_and_render_build_order() {
    let sb = Sandbox::new("wslibrary");
    init_and_pull(&sb);
    let scaffold = sb.ost(&[
        "plugin",
        "new",
        "usd-fileformat",
        "consumer",
        "--extension",
        "toy",
    ]);
    assert!(scaffold.status.success(), "{}", out_text(&scaffold));

    let manifest_path = sb.work_file("consumer/openstrata.plugin.yaml");
    let source = std::fs::read_to_string(&manifest_path).unwrap();
    let source = source.replace(
        "requires:\n  capabilities: [usd-stage-read]\n",
        "requires:\n  capabilities: [usd-stage-read]\n  libraries:\n    - { id: container, version: '>=1.0,<2.0' }\n",
    );
    std::fs::write(
        &manifest_path,
        format!("manifest:\n  schema: openstrata.plugin/v1alpha1\n{source}"),
    )
    .unwrap();

    let library_root = sb.work_file("libs/container");
    std::fs::create_dir_all(&library_root).unwrap();
    std::fs::write(
        library_root.join("openstrata.library.yaml"),
        "schema: openstrata.library/v1alpha1\nlibrary: { id: container, version: 1.2.0 }\ncmake: { package: container, target: 'container::container' }\nruntime: { directories: [bin, lib] }\n",
    )
    .unwrap();
    std::fs::write(
        library_root.join("CMakeLists.txt"),
        "cmake_minimum_required(VERSION 3.23)\nproject(container LANGUAGES CXX)\nadd_library(container SHARED container.cpp)\ninstall(TARGETS container EXPORT containerTargets)\n",
    )
    .unwrap();
    std::fs::write(
        library_root.join("container.cpp"),
        "int container() { return 1; }\n",
    )
    .unwrap();
    let plugin_library = sb.work_file(&format!(
        "consumer/lib/{}ConsumerFileFormat{}",
        "lib",
        std::env::consts::DLL_SUFFIX
    ));
    std::fs::create_dir_all(plugin_library.parent().unwrap()).unwrap();
    std::fs::write(plugin_library, b"test library marker").unwrap();

    let inspect = sb.ost(&["--json", "plugin", "inspect", "consumer"]);
    assert!(inspect.status.success(), "{}", out_text(&inspect));
    let value: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(value["data"]["libraries"][0]["id"], "container");
    assert_eq!(value["data"]["libraries"][0]["version"], "1.2.0");
    assert_eq!(
        value["data"]["libraries"][0]["cmake_target"],
        "container::container"
    );

    let build = sb.ost(&["plugin", "build", "consumer", "--dry-run"]);
    assert!(build.status.success(), "{}", out_text(&build));
    let plan = out_text(&build);
    assert!(plan.contains("== build library container =="), "{plan}");
    assert!(plan.contains("libs/container"), "{plan}");
    assert!(plan.contains("workspace-prefix"), "{plan}");
    assert!(
        plan.find("== build library container ==") < plan.find("== build primary consumer =="),
        "library must install before the consumer configures: {plan}"
    );

    let workspace = sb.ost(&["--json", "plugin", "test", "--workspace", "--up-to", "1"]);
    assert!(workspace.status.success(), "{}", out_text(&workspace));
    let value: serde_json::Value = serde_json::from_slice(&workspace.stdout).unwrap();
    assert_eq!(value["data"]["graph"]["libraries"][0]["id"], "container");
    assert_eq!(
        value["data"]["graph"]["library_edges"][0]["from"],
        "consumer"
    );

    // Model the install result of the dry-run plan and prove packaging carries
    // the declared runtime closure instead of relying on a mutable sibling.
    let consumer_target = single_target_dir(&sb.work_file("consumer"));
    let target_id = consumer_target.file_name().unwrap();
    let installed_bin = sb
        .work_file(".strata/targets")
        .join(target_id)
        .join("workspace-prefix/bin");
    std::fs::create_dir_all(&installed_bin).unwrap();
    std::fs::write(
        installed_bin.join(format!("container{}", std::env::consts::DLL_SUFFIX)),
        b"plain library runtime marker",
    )
    .unwrap();
    let package = sb.ost(&["plugin", "package", "consumer"]);
    assert!(package.status.success(), "{}", out_text(&package));
    let package_manifest = find_first(&sb.work_file("consumer/dist"), "manifest.json").unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(package_manifest).unwrap()).unwrap();
    assert_eq!(value["dependencies"]["libraries"][0]["id"], "container");
    assert!(value["files"].as_array().unwrap().iter().any(|file| {
        file["path"]
            .as_str()
            .is_some_and(|path| path.contains("runtime/libraries/bin/container"))
    }));
    let packaged_test = sb.ost(&[
        "--json",
        "plugin",
        "test",
        "consumer",
        "--from-package",
        "--up-to",
        "1",
    ]);
    assert!(
        packaged_test.status.success(),
        "{}",
        out_text(&packaged_test)
    );
}

/// A packaged manifest keeps its `requires.bundles` edges, so a bundle
/// standing alone (an extracted package, a single-bundle checkout) must not
/// fail workspace graph validation for siblings that are not there.
#[test]
fn bundle_with_dependencies_but_no_siblings_stays_standalone() {
    let sb = Sandbox::new("standalone-deps");
    init_and_pull(&sb);
    let scaffold = sb.ost(&[
        "plugin",
        "new",
        "usd-fileformat",
        "consumer",
        "--extension",
        "toy",
    ]);
    assert!(scaffold.status.success(), "{}", out_text(&scaffold));

    let manifest_path = sb.work_file("consumer/openstrata.plugin.yaml");
    let source = std::fs::read_to_string(&manifest_path).unwrap();
    let source = source.replace(
        "requires:\n  capabilities: [usd-stage-read]\n",
        "requires:\n  capabilities: [usd-stage-read]\n  bundles:\n    - { id: companion, version: '>=1.0,<2.0' }\n",
    );
    std::fs::write(
        &manifest_path,
        format!("manifest:\n  schema: openstrata.plugin/v1alpha1\n{source}"),
    )
    .unwrap();
    let plugin_library = sb.work_file(&format!(
        "consumer/lib/{}ConsumerFileFormat{}",
        "lib",
        std::env::consts::DLL_SUFFIX
    ));
    std::fs::create_dir_all(plugin_library.parent().unwrap()).unwrap();
    std::fs::write(plugin_library, b"test library marker").unwrap();

    let inspect = sb.ost(&["--json", "plugin", "inspect", "consumer"]);
    assert!(
        inspect.status.success(),
        "a lone bundle with declared edges must stay inspectable:\n{}",
        out_text(&inspect)
    );
    let value: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert!(value["data"]["libraries"].is_null());
}

/// Inside a generated CI job the `OST_CI_*` variables travel into every
/// written report as a `ci` evidence block, so the report records which
/// support cell it proves; outside CI the block is absent.
#[test]
fn report_records_ci_evidence_from_the_env_contract() {
    let sb = Sandbox::new("cievidence");
    init_and_pull(&sb);
    let scaffold = sb.ost(&["plugin", "new", "usd-schema", "alpha"]);
    assert!(
        scaffold.status.success(),
        "scaffold failed:\n{}",
        out_text(&scaffold)
    );

    let digest = format!("sha256:{}", "ab".repeat(32));
    let out = sb.ost_env(
        &["--json", "plugin", "test", "alpha", "--up-to", "1"],
        &[
            ("OST_CI_CELL", "alpha-pr-windows"),
            ("OST_CI_LANE", "pull_request"),
            ("OST_CI_RUNNER_PROFILE", "windows-hosted"),
            ("OST_CI_RUNS_ON", "windows-2022"),
            ("OST_CI_RUNTIME_ARTIFACT", digest.as_str()),
            ("OST_CI_MINIMUM_TRUST", "attested"),
        ],
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true, "test passes: {v}");
    let ci = &v["data"]["ci"];
    assert_eq!(ci["cell"], "alpha-pr-windows");
    assert_eq!(ci["lane"], "pull_request");
    assert_eq!(ci["runner_profile"], "windows-hosted");
    assert_eq!(ci["runtime_artifact"], digest);
    assert_eq!(ci["minimum_trust"], "attested");
    // The unset variable records as null, not a missing key.
    assert!(ci["plugin_artifact"].is_null());

    // The on-disk report.json carries the same evidence.
    let report_dir = v["data"]["report_dir"].as_str().unwrap();
    let report: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(report_dir).join("report.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["ci"]["cell"], "alpha-pr-windows");

    // Outside CI (no OST_CI_CELL) the block is absent entirely.
    let out = sb.ost(&["--json", "plugin", "test", "alpha", "--up-to", "1"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["data"].get("ci").is_none(), "no ci block outside CI: {v}");
}

/// `ost lock` pins extensions from the pulled runtime's manifest — the same
/// source of truth `runtime show` reports — so an install whose recorded
/// extension version drifts (e.g. a re-adopted newer OpenUSD) makes
/// `lock --check` fail instead of silently agreeing with the catalog's
/// certified point (dogfooding report #8: runtime 26.08, lock 25.05.01,
/// `--check` still `up_to_date: true`).
#[test]
fn lock_pins_extensions_from_the_runtime_manifest() {
    let sb = Sandbox::new("lockext");
    init_and_pull(&sb);

    let lock = sb.ost(&["lock"]);
    assert!(lock.status.success(), "lock failed:\n{}", out_text(&lock));
    let check = sb.ost(&["lock", "--check"]);
    assert!(
        check.status.success(),
        "fresh lock is up to date:\n{}",
        out_text(&check)
    );

    // Simulate install drift: rewrite the manifest's openusd version the way
    // a re-adopt of a newer install would record it.
    let manifest_path = find_first(&sb.home, "runtime.json").expect("pulled runtime manifest");
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    let ext = doc["extensions"]
        .as_array_mut()
        .expect("manifest records extensions")
        .iter_mut()
        .find(|e| e["id"] == "openusd")
        .expect("openusd extension recorded");
    ext["version"] = "99.99".into();
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();

    // The drift is visible to --check ...
    let check = sb.ost(&["lock", "--check"]);
    assert!(
        !check.status.success(),
        "stale lock must fail --check after manifest drift:\n{}",
        out_text(&check)
    );

    // ... and a refresh pins the manifest's version, not the catalog's.
    let relock = sb.ost(&["lock"]);
    assert!(
        relock.status.success(),
        "relock failed:\n{}",
        out_text(&relock)
    );
    let lock_text = std::fs::read_to_string(sb.work_file("strata.lock")).unwrap();
    assert!(
        lock_text.contains("99.99"),
        "lock records the runtime manifest's extension version:\n{lock_text}"
    );
}

/// Find the first file under `dir` whose name ends with `suffix`.
fn find_first(dir: &Path, suffix: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p
                .file_name()
                .map(|n| n.to_string_lossy().ends_with(suffix))
                .unwrap_or(false)
            {
                return Some(p);
            }
        }
    }
    None
}
