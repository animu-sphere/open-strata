// SPDX-License-Identifier: Apache-2.0
//! End-to-end tests for the `artifact` runtime source (Phase 6):
//! `ost runtime export` → registry → CI handoff → `ost runtime pull
//! --from-artifact` on a second store.
//!
//! A real OpenUSD tree is too heavy for CI, so the "real" runtime is a pulled
//! mock whose manifest is promoted to a validated `build` source and whose
//! prefix gets USD-shaped marker files — exactly the shape `export` gates on
//! and `pull --from-artifact` verifies after extraction. Covered contract:
//!
//! - `export` refuses a mock runtime (`EXPORT_REAL_RUNTIME_REQUIRED`, exit 4);
//! - `export` registers a validated real runtime by digest (`published`);
//! - the artifact round-trips through `artifact export`/`import` to a second
//!   store, and `pull --from-artifact` materializes it there with
//!   `source: artifact` + the registry digest in the manifest;
//! - a non-runtime artifact is refused (`ARTIFACT_KIND_MISMATCH`, exit 5).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn ost_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ost")
}

struct Sandbox {
    base: PathBuf,
    home: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let base =
            std::env::temp_dir().join(format!("ost-rta-{tag}-{}-{nanos}", std::process::id()));
        let home = base.join("home");
        std::fs::create_dir_all(&home).unwrap();
        Sandbox { base, home }
    }

    fn ost(&self, args: &[&str]) -> Output {
        Command::new(ost_bin())
            .args(args)
            .current_dir(&self.base)
            .env("OST_HOME", &self.home)
            // A developer's adopt/build env must not leak into the mock pulls.
            .env_remove("OST_USD_ROOT")
            .env_remove("OST_USD_SRC")
            .env_remove("OST_USD_DEPS")
            .output()
            .expect("spawn ost")
    }

    /// The single runtime prefix in this sandbox's store.
    fn runtime_prefix(&self) -> PathBuf {
        let runtimes = self.home.join("runtimes");
        let mut dirs: Vec<_> = std::fs::read_dir(&runtimes)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(dirs.len(), 1, "expected exactly one pulled runtime");
        dirs.remove(0)
    }

    /// Promote the pulled mock runtime to a validated, self-contained `build`
    /// source and give its prefix USD-shaped content. Only provenance fields
    /// change, so the manifest's canonical digest stays valid.
    fn promote_mock_to_build(&self) -> PathBuf {
        let prefix = self.runtime_prefix();
        let manifest_path = prefix.join("runtime.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest["source"] = "build".into();
        manifest["validation"] = "passed".into();
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        // USD markers: a plugin registry, the pxr Python package, and a header
        // naming the same release the catalog records (no version drift).
        for (rel, content) in [
            ("plugin/usd/plugInfo.json", "{}"),
            ("lib/python/pxr/__init__.py", ""),
            ("bin/usdcat", "#!/bin/sh\n"),
            (
                "include/pxr/pxr.h",
                "#define PXR_MAJOR_VERSION 0\n\
                 #define PXR_MINOR_VERSION 25\n\
                 #define PXR_PATCH_VERSION 5\n",
            ),
        ] {
            let path = prefix.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, content).unwrap();
        }
        prefix
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn stdout_json(out: &Output) -> serde_json::Value {
    assert!(
        out.status.success(),
        "expected success\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout is a JSON envelope")
}

fn error_json(out: &Output, exit: i32) -> serde_json::Value {
    assert_eq!(
        out.status.code(),
        Some(exit),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout is a JSON error envelope")
}

fn path_str(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn export_refuses_a_mock_runtime() {
    let sb = Sandbox::new("mock");
    stdout_json(&sb.ost(&["--json", "runtime", "pull", "cy2026", "--profile", "usd"]));

    let out = sb.ost(&["--json", "runtime", "export", "cy2026", "--profile", "usd"]);
    let v = error_json(&out, 4);
    assert_eq!(v["error"]["code"], "EXPORT_REAL_RUNTIME_REQUIRED");
}

#[test]
fn export_dist_refuses_non_empty_directory_without_deleting_it() {
    let sb = Sandbox::new("dist-safe");
    stdout_json(&sb.ost(&["--json", "runtime", "pull", "cy2026", "--profile", "usd"]));
    sb.promote_mock_to_build();

    let dist = sb.base.join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    let sentinel = dist.join("keep.txt");
    std::fs::write(&sentinel, b"do not delete").unwrap();

    let out = sb.ost(&[
        "--json",
        "runtime",
        "export",
        "cy2026",
        "--profile",
        "usd",
        "--dist",
        path_str(&dist),
    ]);
    let v = error_json(&out, 2);
    assert_eq!(v["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"do not delete");
}

#[test]
fn export_handoff_and_pull_from_artifact_roundtrip() {
    let sb1 = Sandbox::new("export");
    stdout_json(&sb1.ost(&["--json", "runtime", "pull", "cy2026", "--profile", "usd"]));
    sb1.promote_mock_to_build();

    // Export registers the runtime in sb1's registry, addressed by digest.
    let v = stdout_json(&sb1.ost(&["--json", "runtime", "export", "cy2026", "--profile", "usd"]));
    assert_eq!(v["data"]["exported"], true);
    let digest = v["data"]["digest"].as_str().unwrap().to_string();
    assert!(digest.starts_with("sha256:"));

    // The registry record is a published runtime artifact.
    let v = stdout_json(&sb1.ost(&["--json", "artifact", "show", &digest]));
    assert_eq!(v["data"]["artifact"]["kind"], "runtime");
    assert_eq!(v["data"]["artifact"]["source"], "published");
    assert_eq!(v["data"]["artifact"]["validation"], "passed");

    // Hand off to a second machine (fresh OST_HOME): artifact export → import.
    let handoff = sb1.base.join("handoff");
    stdout_json(&sb1.ost(&["--json", "artifact", "export", &digest, path_str(&handoff)]));
    let sb2 = Sandbox::new("fetch");
    let v = stdout_json(&sb2.ost(&["--json", "artifact", "import", path_str(&handoff)]));
    assert_eq!(v["data"]["artifact"]["digest"], digest.as_str());

    // Materialize the runtime from the artifact on the second store.
    let v = stdout_json(&sb2.ost(&[
        "--json",
        "runtime",
        "pull",
        "cy2026",
        "--profile",
        "usd",
        "--from-artifact",
        &digest,
    ]));
    assert_eq!(v["data"]["source"], "artifact");

    // The extracted tree and the restored manifest carry the provenance.
    let prefix = sb2.runtime_prefix();
    assert!(prefix.join("plugin/usd/plugInfo.json").is_file());
    assert!(prefix.join("lib/python/pxr/__init__.py").is_file());
    let v = stdout_json(&sb2.ost(&["--json", "runtime", "show", "cy2026", "--profile", "usd"]));
    assert_eq!(v["data"]["source"], "artifact");
    assert_eq!(v["data"]["artifact_digest"], digest.as_str());
    assert_eq!(v["data"]["validation"], "passed");

    // Re-pull without --force is still refused (same contract as other sources).
    let out = sb2.ost(&[
        "--json",
        "runtime",
        "pull",
        "cy2026",
        "--profile",
        "usd",
        "--from-artifact",
        &digest,
    ]);
    error_json(&out, 2);
}

/// A Linux runtime SDK ships shared-library soname chains as in-tree relative
/// symlinks (`libFoo.so → libFoo.so.1.39.4`). Export must pack them as link
/// entries — never dereferencing them into copies — and materialization from the
/// artifact must restore them as symlinks pointing at the same in-tree target
/// (v0.11.0 report ask #2). Symlinks are a unix construct, so this runs on unix
/// hosts (macOS included), where `runtime export`/`pull --from-artifact` exercise
/// the real filesystem link path end to end.
#[cfg(unix)]
#[test]
fn export_preserves_in_tree_symlinks_through_pull_from_artifact() {
    use std::os::unix::fs::symlink;

    let sb1 = Sandbox::new("symlink-export");
    stdout_json(&sb1.ost(&["--json", "runtime", "pull", "cy2026", "--profile", "usd"]));
    let prefix = sb1.promote_mock_to_build();

    // A soname chain in lib/: the real object + two relative in-tree links.
    let lib = prefix.join("lib");
    std::fs::write(lib.join("libFoo.so.1.39.4"), b"\x7fELF real object").unwrap();
    symlink("libFoo.so.1.39.4", lib.join("libFoo.so.1")).unwrap();
    symlink("libFoo.so.1", lib.join("libFoo.so")).unwrap();

    let v = stdout_json(&sb1.ost(&["--json", "runtime", "export", "cy2026", "--profile", "usd"]));
    let digest = v["data"]["digest"].as_str().unwrap().to_string();

    // Hand off to a fresh store and materialize the runtime there.
    let handoff = sb1.base.join("handoff");
    stdout_json(&sb1.ost(&["--json", "artifact", "export", &digest, path_str(&handoff)]));
    let sb2 = Sandbox::new("symlink-fetch");
    stdout_json(&sb2.ost(&["--json", "artifact", "import", path_str(&handoff)]));
    stdout_json(&sb2.ost(&[
        "--json",
        "runtime",
        "pull",
        "cy2026",
        "--profile",
        "usd",
        "--from-artifact",
        &digest,
    ]));

    // The links survive as links (not dereferenced copies) with their targets.
    let out_lib = sb2.runtime_prefix().join("lib");
    let so = out_lib.join("libFoo.so");
    let so1 = out_lib.join("libFoo.so.1");
    assert!(
        std::fs::symlink_metadata(&so)
            .unwrap()
            .file_type()
            .is_symlink(),
        "libFoo.so must be restored as a symlink"
    );
    assert_eq!(std::fs::read_link(&so).unwrap(), Path::new("libFoo.so.1"));
    assert_eq!(
        std::fs::read_link(&so1).unwrap(),
        Path::new("libFoo.so.1.39.4")
    );
    // The link resolves to the real object, and only one real copy exists.
    assert_eq!(std::fs::read(&so).unwrap(), b"\x7fELF real object");
    assert!(!std::fs::symlink_metadata(out_lib.join("libFoo.so.1.39.4"))
        .unwrap()
        .file_type()
        .is_symlink());
}

/// On Linux the runtime variant defaults to the nominal `glibc228`. Export must
/// override that with the real floor measured from the packed ELF binaries, so a
/// runtime that references `GLIBC_2.43` is labeled `glibc243` — the truthful
/// target a support line pins (and which a stale `glibc228` pin no longer
/// string-matches, closing the ask #7 false-pass at its source).
///
/// The runtime variant is host-derived, so the glibc default only appears on
/// Linux; the assertions run there. The test still compiles everywhere (a runtime
/// guard, not `#[cfg]`) so field/API drift is caught on every platform's build.
#[test]
fn export_relabels_target_with_measured_glibc_floor() {
    if !cfg!(target_os = "linux") {
        return;
    }
    let sb = Sandbox::new("glibc-floor");
    stdout_json(&sb.ost(&["--json", "runtime", "pull", "cy2026", "--profile", "usd"]));
    let prefix = sb.promote_mock_to_build();

    // The runtime's nominal (defaulted) ABI before measurement is glibc 2.28.
    let v = stdout_json(&sb.ost(&["--json", "runtime", "show", "cy2026", "--profile", "usd"]));
    assert_eq!(v["data"]["variant"]["abi"]["glibc"]["version"], "2.28");

    // Drop a fake ELF shared library that references GLIBC_2.43 (newer than the
    // nominal floor) plus a lower reference, so the max wins.
    let lib = prefix.join("lib/libfake.so");
    let mut elf = vec![0x7f, b'E', b'L', b'F'];
    elf.extend_from_slice(b"\0\0 needs GLIBC_2.17 and GLIBC_2.43 \0");
    std::fs::write(&lib, elf).unwrap();

    let v = stdout_json(&sb.ost(&["--json", "runtime", "export", "cy2026", "--profile", "usd"]));
    assert_eq!(v["data"]["exported"], true);
    assert_eq!(v["data"]["glibc_floor"], "2.43");
    // The runtime artifact's `target` is the variant slug (no cycle prefix / profile
    // suffix), as `runtime_artifact_manifest` records it — the measured floor moves
    // the ABI token from `glibc228` to `glibc243`.
    assert_eq!(v["data"]["target"], "linux-x86_64-glibc243-py313");
    let digest = v["data"]["digest"].as_str().unwrap().to_string();

    // The registry record carries the corrected, truthful target, and records the
    // measured-vs-recorded drift as evidence.
    let v = stdout_json(&sb.ost(&["--json", "artifact", "show", &digest]));
    assert_eq!(
        v["data"]["artifact"]["target"],
        "linux-x86_64-glibc243-py313"
    );
}

#[test]
fn pull_from_artifact_refuses_non_runtime_kinds() {
    let sb = Sandbox::new("kind");

    // A minimal plugin-bundle dist: enough for `artifact import`.
    let stage = camino::Utf8PathBuf::from_path_buf(sb.base.join("stage")).unwrap();
    std::fs::create_dir_all(stage.join("lib").as_std_path()).unwrap();
    std::fs::write(stage.join("lib/toy.dll").as_std_path(), b"bytes").unwrap();
    let dist = camino::Utf8PathBuf::from_path_buf(sb.base.join("dist")).unwrap();
    let archive = dist.join("toy-0.1.0.tar.zst");
    let files = ost_build::stage_files(&stage).unwrap();
    let packed = ost_build::pack_dir(&stage, &archive, &files).unwrap();
    let files_json: Vec<_> = packed
        .files
        .iter()
        .map(|f| serde_json::json!({ "path": f.path, "sha256": f.sha256, "size": f.size }))
        .collect();
    let manifest = serde_json::json!({
        "schema": 1,
        "kind": "openstrata.plugin-bundle",
        "plugin": { "name": "toy", "version": "0.1.0", "kind": "usd-fileformat", "license": "Apache-2.0" },
        "target": "cy2026-test",
        "archive": "toy-0.1.0.tar.zst",
        "archive_digest": packed.archive_digest,
        "archive_size": packed.archive_size,
        "total_size": packed.total_size,
        "files": files_json,
    });
    std::fs::write(
        dist.join("manifest.json").as_std_path(),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let v = stdout_json(&sb.ost(&["--json", "artifact", "import", dist.as_str()]));
    let digest = v["data"]["artifact"]["digest"]
        .as_str()
        .unwrap()
        .to_string();

    let out = sb.ost(&[
        "--json",
        "runtime",
        "pull",
        "cy2026",
        "--profile",
        "usd",
        "--from-artifact",
        &digest,
    ]);
    let v = error_json(&out, 5);
    assert_eq!(v["error"]["code"], "ARTIFACT_KIND_MISMATCH");
}
