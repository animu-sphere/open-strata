// SPDX-License-Identifier: Apache-2.0
//! Discovery over fixture install trees.
//!
//! Every fixture is built with the same layout facts the validators encode, and
//! the target OS is injected, so these run identically on a machine with no DCC
//! installed — which is the point: discovery has to be provably correct before
//! anyone can trust it against a real site.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};

use camino::{Utf8Path, Utf8PathBuf};
use ost_core::host::Os;
use ost_host::{
    discover, DiscoveryRequest, DiscoverySource, FingerprintMode, HostFamily, HostInventory,
    HostRecord, HostStatus, Selection, ValidateOptions, VersionSource,
};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn scratch(tag: &str) -> Utf8PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ost-host-{tag}-{}-{unique}", std::process::id()));
    let dir = Utf8PathBuf::from_path_buf(dir).unwrap();
    std::fs::remove_dir_all(dir.as_std_path()).ok();
    std::fs::create_dir_all(dir.as_std_path()).unwrap();
    dir
}

fn write(path: &Utf8Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap().as_std_path()).unwrap();
    std::fs::write(path.as_std_path(), contents).unwrap();
}

fn exe_name(name: &str, os: Os) -> String {
    if os == Os::Windows {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// A Maya-shaped install. `devkit_version` writes the devkit header recent
/// releases ship separately, which is the difference between a metadata answer
/// and a directory-name guess.
fn maya_install(root: &Utf8Path, os: Os, devkit_version: Option<&str>) {
    for name in ["maya", "mayapy"] {
        write(&root.join("bin").join(exe_name(name, os)), "fixture");
    }
    std::fs::create_dir_all(root.join("modules").as_std_path()).unwrap();
    std::fs::create_dir_all(root.join("plug-ins").as_std_path()).unwrap();
    std::fs::create_dir_all(root.join("lib/python3.11").as_std_path()).unwrap();
    if let Some(version) = devkit_version {
        write(
            &root.join("include/maya/MTypes.h"),
            &format!("#define MAYA_API_VERSION {version}0000 // release\n#define MAYA_APP_VERSION {version}\n"),
        );
    }
}

/// A Houdini-shaped install, optionally carrying the HDK version header.
fn houdini_install(root: &Utf8Path, os: Os, toolkit_version: Option<&str>) {
    for name in ["houdini", "hython", "husk"] {
        write(&root.join("bin").join(exe_name(name, os)), "fixture");
    }
    std::fs::create_dir_all(root.join("houdini/python3.11libs").as_std_path()).unwrap();
    std::fs::create_dir_all(root.join("dsolib").as_std_path()).unwrap();
    if let Some(version) = toolkit_version {
        write(
            &root.join("toolkit/include/SYS/SYS_Version.h"),
            &format!("#define SYS_VERSION_FULL \"{version}\"\n"),
        );
    }
}

/// Alias a directory the way a site aliases a version
/// (`/tools/maya/current -> .../maya2025.3`).
///
/// A junction on Windows rather than a symlink: it is what a site actually uses
/// there, and unlike `symlink_dir` it needs no elevation — so the real layout is
/// covered on every platform CI runs rather than on unix only.
fn link_dir(target: &Utf8Path, link: &Utf8Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target.as_std_path(), link.as_std_path()).unwrap();
    #[cfg(windows)]
    {
        let backslashed = |path: &Utf8Path| path.as_str().replace('/', "\\");
        let status = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                &backslashed(link),
                &backslashed(target),
            ])
            .stdout(std::process::Stdio::null())
            .status()
            .expect("spawn mklink");
        assert!(status.success(), "mklink /J {link} {target}");
    }
}

/// A hermetic request: no environment, no known roots, no `PATH`. Providers a
/// test does not name must not be able to reach into the machine running it.
fn request(os: Os) -> DiscoveryRequest {
    DiscoveryRequest {
        use_environment: false,
        use_known_roots: false,
        use_executable_path: false,
        validate: ValidateOptions {
            os,
            ..ValidateOptions::default()
        },
        ..DiscoveryRequest::default()
    }
}

#[test]
fn an_explicit_maya_path_validates_with_metadata_version_and_python_abi() {
    let dir = scratch("maya-explicit");
    let root = dir.join("maya2026");
    maya_install(&root, Os::Linux, Some("2026"));

    let outcome = discover(&DiscoveryRequest {
        explicit_paths: vec![root.clone()],
        ..request(Os::Linux)
    })
    .unwrap();

    assert_eq!(outcome.records.len(), 1, "{:?}", outcome.records);
    let record = &outcome.records[0];
    assert_eq!(record.status, HostStatus::Validated);
    assert_eq!(record.family, HostFamily::Maya);
    assert_eq!(record.product, "Autodesk Maya");

    let version = record.version.as_ref().expect("version");
    assert_eq!(version.raw, "2026");
    assert_eq!(version.source, VersionSource::Metadata);
    assert_eq!(version.confidence, "high");

    let python = record.python.as_ref().expect("python");
    assert_eq!(python.version, "3.11");
    assert_eq!(python.abi_tag, "cp311");

    assert!(record.executables.contains_key("interpreter"));
    assert!(record.executables.contains_key("interactive"));
    assert!(record.markers.contains(&"modules".to_string()));
    // A record states the OS whose layout rules produced it. Without that, this
    // fixture pass would validate Linux paths and stamp them with whichever
    // machine happened to run the suite.
    assert_eq!(record.platform.os, Os::Linux);
    assert_eq!(record.evidence.len(), 1);
    assert_eq!(record.evidence[0].source, DiscoverySource::ExplicitPath);
    assert!(record
        .fingerprint
        .as_ref()
        .unwrap()
        .digest
        .starts_with("sha256:"));

    std::fs::remove_dir_all(dir.as_std_path()).ok();
}

#[test]
fn shipped_metadata_beats_the_directory_name_for_houdini() {
    let dir = scratch("houdini-metadata");
    // The directory says one thing and the HDK header says another; the header
    // is the install talking about itself, so it wins.
    let root = dir.join("hfs20.5");
    houdini_install(&root, Os::Linux, Some("20.5.550"));

    let outcome = discover(&DiscoveryRequest {
        explicit_paths: vec![root.clone()],
        ..request(Os::Linux)
    })
    .unwrap();

    let record = &outcome.records[0];
    let version = record.version.as_ref().expect("version");
    assert_eq!(version.raw, "20.5.550");
    assert_eq!(version.source, VersionSource::Metadata);
    assert_eq!(version.build.as_deref(), Some("550"));
    assert_eq!(record.python.as_ref().unwrap().version, "3.11");

    std::fs::remove_dir_all(dir.as_std_path()).ok();
}

#[test]
fn without_metadata_the_directory_name_answers_but_says_it_is_a_guess() {
    let dir = scratch("houdini-dirname");
    let root = dir.join("hfs20.5.550");
    houdini_install(&root, Os::Linux, None);

    let outcome = discover(&DiscoveryRequest {
        explicit_paths: vec![root],
        ..request(Os::Linux)
    })
    .unwrap();

    let version = outcome.records[0].version.as_ref().expect("version");
    assert_eq!(version.raw, "20.5.550");
    assert_eq!(version.source, VersionSource::DirectoryName);
    assert_eq!(version.confidence, "low");

    std::fs::remove_dir_all(dir.as_std_path()).ok();
}

#[test]
fn a_named_path_is_refused_with_a_reason_but_a_scanned_one_is_skipped() {
    let dir = scratch("rejection");
    let empty = dir.join("not-a-host");
    std::fs::create_dir_all(empty.join("bin").as_std_path()).unwrap();

    // Named directly: the operator is owed an explanation.
    let named = discover(&DiscoveryRequest {
        explicit_paths: vec![empty.clone()],
        families: vec![HostFamily::Maya],
        ..request(Os::Linux)
    })
    .unwrap();
    assert_eq!(named.records.len(), 1);
    assert_eq!(named.records[0].status, HostStatus::Rejected);
    let rejection = named.records[0].rejection.as_ref().expect("rejection");
    assert_eq!(rejection.code, "HOST_MARKERS_MISSING");
    assert!(
        rejection.message.contains("bin/mayapy"),
        "the refusal names what it looked for: {}",
        rejection.message
    );
    assert!(named.records[0].fingerprint.is_none());

    // Merely scanned: reporting every directory under a site root as "not Maya"
    // would bury the answer in noise.
    let scanned = discover(&DiscoveryRequest {
        configured_roots: vec![dir.clone()],
        families: vec![HostFamily::Maya],
        ..request(Os::Linux)
    })
    .unwrap();
    assert!(scanned.records.is_empty(), "{:?}", scanned.records);

    std::fs::remove_dir_all(dir.as_std_path()).ok();
}

#[test]
fn a_scan_resolves_a_version_alias_link_to_the_install_behind_it() {
    // A link is not a directory to `DirEntry::file_type`, so every scanning
    // provider used to skip `current -> maya2026` while `--path` on the same
    // link validated — and a version alias is a standard site layout.
    let dir = scratch("alias-link");
    let real = dir.join("installs/maya2026");
    maya_install(&real, Os::Linux, Some("2026"));
    let site = dir.join("site");
    std::fs::create_dir_all(site.as_std_path()).unwrap();
    link_dir(&real, &site.join("current"));

    let outcome = discover(&DiscoveryRequest {
        configured_roots: vec![site],
        max_depth: 1,
        families: vec![HostFamily::Maya],
        ..request(Os::Linux)
    })
    .unwrap();

    let validated: Vec<&HostRecord> = outcome.validated().collect();
    assert_eq!(validated.len(), 1, "{:?}", outcome.records);
    assert_eq!(validated[0].version.as_ref().unwrap().raw, "2026");
    // And the record names the install rather than the alias: canonicalizing is
    // what makes one install reached two ways one record.
    assert!(
        validated[0].root.as_str().ends_with("installs/maya2026"),
        "{}",
        validated[0].root
    );

    std::fs::remove_dir_all(dir.as_std_path()).ok();
}

#[test]
fn a_declared_root_that_is_absent_is_named_and_a_refusal_stays_reachable() {
    let dir = scratch("missing-root");
    let named = dir.join("not-a-host");
    std::fs::create_dir_all(named.join("bin").as_std_path()).unwrap();

    let outcome = discover(&DiscoveryRequest {
        configured_roots: vec![dir.join("no-such-site")],
        explicit_paths: vec![named],
        families: vec![HostFamily::Maya],
        ..request(Os::Linux)
    })
    .unwrap();

    // A root the site declared and does not have is a misconfiguration worth a
    // sentence; a documented default location that is absent is not.
    assert!(
        outcome
            .warnings
            .iter()
            .any(|warning| warning.code == "HOST_ROOT_NOT_FOUND"),
        "{:?}",
        outcome.warnings
    );

    // Selection is status-agnostic: `inspect` is the command that explains a
    // refusal, so a family selector has to reach a rejected record.
    assert_eq!(outcome.records.len(), 1);
    assert_eq!(outcome.records[0].status, HostStatus::Rejected);
    match ost_host::select(&outcome.records, "maya") {
        Selection::One(record) => assert_eq!(record.status, HostStatus::Rejected),
        other => panic!("the only maya record must be reachable by family: {other:?}"),
    }

    std::fs::remove_dir_all(dir.as_std_path()).ok();
}

#[test]
fn configured_roots_find_installs_within_the_declared_depth_only() {
    let dir = scratch("configured-depth");
    let shallow = dir.join("maya/2025.3");
    let deep = dir.join("vendor/autodesk/maya/2026");
    maya_install(&shallow, Os::Linux, None);
    maya_install(&deep, Os::Linux, None);

    let outcome = discover(&DiscoveryRequest {
        configured_roots: vec![dir.clone()],
        max_depth: 2,
        families: vec![HostFamily::Maya],
        ..request(Os::Linux)
    })
    .unwrap();

    let roots: Vec<&str> = outcome
        .validated()
        .map(|record| record.root.as_str())
        .collect();
    assert_eq!(roots.len(), 1, "only the in-depth install: {roots:?}");
    assert!(roots[0].replace('\\', "/").ends_with("maya/2025.3"));
    assert_eq!(
        outcome.records[0].evidence[0].source,
        DiscoverySource::ConfiguredRoot
    );

    // Raising the bound reaches the deeper one without changing anything else.
    let deeper = discover(&DiscoveryRequest {
        configured_roots: vec![dir.clone()],
        max_depth: 4,
        families: vec![HostFamily::Maya],
        ..request(Os::Linux)
    })
    .unwrap();
    assert_eq!(deeper.validated().count(), 2);

    std::fs::remove_dir_all(dir.as_std_path()).ok();
}

#[test]
fn one_install_reached_two_ways_is_one_record_carrying_both_provenances() {
    let dir = scratch("merge");
    let root = dir.join("maya2026");
    maya_install(&root, Os::Linux, Some("2026"));

    let outcome = discover(&DiscoveryRequest {
        explicit_paths: vec![root.clone()],
        configured_roots: vec![dir.clone()],
        max_depth: 1,
        families: vec![HostFamily::Maya],
        ..request(Os::Linux)
    })
    .unwrap();

    assert_eq!(outcome.records.len(), 1, "{:?}", outcome.records);
    let sources: Vec<DiscoverySource> = outcome.records[0]
        .evidence
        .iter()
        .map(|evidence| evidence.source)
        .collect();
    assert!(
        sources.contains(&DiscoverySource::ExplicitPath),
        "{sources:?}"
    );
    assert!(
        sources.contains(&DiscoverySource::ConfiguredRoot),
        "a site's own configuration is retained, not overwritten: {sources:?}"
    );

    std::fs::remove_dir_all(dir.as_std_path()).ok();
}

#[test]
fn two_families_under_one_root_are_told_apart_by_their_own_executables() {
    let dir = scratch("mixed");
    maya_install(&dir.join("maya2026"), Os::Linux, Some("2026"));
    houdini_install(&dir.join("hfs20.5.550"), Os::Linux, Some("20.5.550"));

    let outcome = discover(&DiscoveryRequest {
        configured_roots: vec![dir.clone()],
        max_depth: 1,
        ..request(Os::Linux)
    })
    .unwrap();

    let families: Vec<HostFamily> = outcome.validated().map(|record| record.family).collect();
    assert_eq!(families, vec![HostFamily::Maya, HostFamily::Houdini]);

    std::fs::remove_dir_all(dir.as_std_path()).ok();
}

#[test]
fn a_deep_fingerprint_separates_installs_a_standard_one_cannot() {
    let dir = scratch("deep");
    let root = dir.join("maya2026");
    maya_install(&root, Os::Linux, Some("2026"));

    let standard = discover(&DiscoveryRequest {
        explicit_paths: vec![root.clone()],
        families: vec![HostFamily::Maya],
        ..request(Os::Linux)
    })
    .unwrap();
    let deep = discover(&DiscoveryRequest {
        explicit_paths: vec![root.clone()],
        families: vec![HostFamily::Maya],
        fingerprint: FingerprintMode::Deep,
        ..request(Os::Linux)
    })
    .unwrap();

    assert_eq!(
        standard.records[0].id, deep.records[0].id,
        "the same install keeps one identity whatever the fingerprint mode"
    );
    assert_ne!(
        standard.records[0].fingerprint_digest(),
        deep.records[0].fingerprint_digest(),
        "a deeper input set is a different digest, and says so via its mode"
    );
    assert_eq!(
        deep.records[0].fingerprint.as_ref().unwrap().mode,
        FingerprintMode::Deep
    );

    std::fs::remove_dir_all(dir.as_std_path()).ok();
}

#[test]
fn an_inventory_round_trips_and_downgrades_an_install_that_changed() {
    let dir = scratch("inventory");
    let root = dir.join("maya2026");
    maya_install(&root, Os::Linux, Some("2026"));

    let outcome = discover(&DiscoveryRequest {
        explicit_paths: vec![root.clone()],
        families: vec![HostFamily::Maya],
        ..request(Os::Linux)
    })
    .unwrap();
    let path = dir.join("inventory.json");
    HostInventory::new(outcome.records.clone())
        .save(&path)
        .unwrap();

    let mut loaded = HostInventory::load(&path).unwrap().expect("written");
    assert_eq!(
        loaded.records, outcome.records,
        "the record survives the round trip"
    );
    assert_eq!(
        loaded.refresh_statuses(),
        0,
        "an unchanged install stays validated"
    );
    assert_eq!(loaded.records[0].status, HostStatus::Validated);

    // Re-installing over the same root must not keep reading as the same bytes.
    write(&root.join("bin/mayapy"), "a different build entirely");
    let mut reloaded = HostInventory::load(&path).unwrap().unwrap();
    assert_eq!(reloaded.refresh_statuses(), 1);
    assert_eq!(reloaded.records[0].status, HostStatus::Stale);

    // And a root that has gone away is unreachable, not merely stale.
    std::fs::remove_dir_all(root.as_std_path()).unwrap();
    let mut gone = HostInventory::load(&path).unwrap().unwrap();
    gone.refresh_statuses();
    assert_eq!(gone.records[0].status, HostStatus::Unreachable);

    assert!(
        HostInventory::load(&dir.join("absent.json"))
            .unwrap()
            .is_none(),
        "no inventory yet is a state, not an error"
    );

    std::fs::remove_dir_all(dir.as_std_path()).ok();
}

#[test]
fn selectors_resolve_exactly_or_report_the_candidates() {
    let dir = scratch("select");
    let older = dir.join("maya2025");
    let newer = dir.join("maya2026");
    maya_install(&older, Os::Linux, Some("2025"));
    maya_install(&newer, Os::Linux, Some("2026"));

    let outcome = discover(&DiscoveryRequest {
        configured_roots: vec![dir.clone()],
        max_depth: 1,
        families: vec![HostFamily::Maya],
        ..request(Os::Linux)
    })
    .unwrap();
    let records = outcome.records;
    assert_eq!(records.len(), 2);

    let id = records[0].id.clone();
    assert!(matches!(
        ost_host::select(&records, &id),
        Selection::One(record) if record.id == id
    ));

    // An id prefix that names one install resolves; a family that names two
    // must not be resolved by scan order.
    assert!(matches!(
        ost_host::select(&records, "maya-2026"),
        Selection::One(_)
    ));
    match ost_host::select(&records, "maya") {
        Selection::Ambiguous(ids) => assert_eq!(ids.len(), 2, "{ids:?}"),
        other => panic!("an ambiguous selector must not pick one: {other:?}"),
    }
    assert!(matches!(
        ost_host::select(&records, "houdini"),
        Selection::None
    ));

    std::fs::remove_dir_all(dir.as_std_path()).ok();
}

#[test]
fn recorded_paths_use_one_separator_whatever_found_them() {
    // A record built on Windows must not mix separators — the same directory
    // reached by two providers has to read as the same string, and this tree
    // already made that rule for packaged manifests.
    let dir = scratch("separators");
    let root = dir.join("maya2026");
    maya_install(&root, Os::Linux, Some("2026"));

    let outcome = discover(&DiscoveryRequest {
        explicit_paths: vec![root.clone()],
        configured_roots: vec![dir.clone()],
        max_depth: 1,
        families: vec![HostFamily::Maya],
        ..request(Os::Linux)
    })
    .unwrap();

    let record = &outcome.records[0];
    assert!(!record.root.as_str().contains('\\'), "{}", record.root);
    for evidence in &record.evidence {
        assert!(!evidence.detail.contains('\\'), "{}", evidence.detail);
    }
    for executable in record.executables.values() {
        assert!(!executable.path.contains('\\'), "{}", executable.path);
    }

    std::fs::remove_dir_all(dir.as_std_path()).ok();
}

#[test]
fn discovery_is_deterministic_across_runs() {
    let dir = scratch("determinism");
    maya_install(&dir.join("maya2026"), Os::Linux, Some("2026"));
    houdini_install(&dir.join("hfs20.5.550"), Os::Linux, Some("20.5.550"));

    let run = || {
        discover(&DiscoveryRequest {
            configured_roots: vec![dir.clone()],
            max_depth: 1,
            ..request(Os::Linux)
        })
        .unwrap()
        .records
    };
    let first = run();
    assert_eq!(
        first,
        run(),
        "an unchanged machine reports identical records"
    );

    // The same fact stated twice: identity and fingerprints are pure functions
    // of the install, so nothing wall-clock leaked into them.
    let digests: BTreeMap<String, Option<String>> = first
        .iter()
        .map(|record| {
            (
                record.id.clone(),
                record.fingerprint_digest().map(str::to_string),
            )
        })
        .collect();
    assert_eq!(digests.len(), 2);

    std::fs::remove_dir_all(dir.as_std_path()).ok();
}
