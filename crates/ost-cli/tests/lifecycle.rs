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
        Command::new(ost_bin())
            .args(args)
            .current_dir(&self.work)
            .env("OST_HOME", &self.home)
            // Don't let a developer's adopt/build env leak into the mock pull.
            .env_remove("OST_USD_ROOT")
            .env_remove("OST_USD_SRC")
            .env_remove("OST_USD_DEPS")
            .output()
            .expect("spawn ost")
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
    assert!(manifest_text.contains("\"kind\": \"openstrata.plugin-bundle\""));
    assert!(manifest_text.contains("\"cxx_abi\""));
    assert!(
        manifest_text.contains("\"license\": \"Apache-2.0\""),
        "package manifest should record the plugin license:\n{manifest_text}"
    );
    assert!(find_first(&dist, "SHA256SUMS").is_some());
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
