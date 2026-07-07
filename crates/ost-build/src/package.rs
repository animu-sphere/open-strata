// SPDX-License-Identifier: Apache-2.0
//! Artifact packaging (§10): `tar.zst` + per-file checksums.
//!
//! The MVP artifact format is a zstd-compressed tar of the install/stage tree,
//! plus a manifest and checksums (written by the CLI). Every file is hashed and
//! the archive itself is content-addressed (§10.3), so an artifact has a stable
//! digest identity.

use std::fs::File;
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use std::collections::BTreeSet;

use ost_core::digest;

/// One packaged file and its integrity data.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Path within the archive, forward-slashed and relative to the stage root.
    pub path: String,
    /// `sha256:<hex>` of the file contents.
    pub sha256: String,
    pub size: u64,
}

/// The result of packing a directory.
pub struct PackResult {
    pub files: Vec<FileEntry>,
    /// `sha256:<hex>` of the finished archive.
    pub archive_digest: String,
    /// Total uncompressed bytes packed.
    pub total_size: u64,
    /// Size of the compressed archive on disk.
    pub archive_size: u64,
}

/// Files selected for a slim SDK export, plus top-level entries that were
/// intentionally excluded without being walked.
#[derive(Debug, Clone, Default)]
pub struct SdkStageFiles {
    pub files: Vec<Utf8PathBuf>,
    pub excluded_top_level: Vec<String>,
}

/// Default zstd compression level for artifacts (high ratio; artifacts are
/// written once and pulled many times).
pub const ZSTD_LEVEL: i32 = 19;

/// How [`pack_dir_with`] compresses the archive.
#[derive(Debug, Clone, Copy)]
pub struct PackOptions {
    /// zstd compression level (1..=22). Higher trades speed for a smaller
    /// archive.
    pub level: i32,
    /// zstd worker threads. `0` keeps the single-threaded encoder (byte-stable
    /// output); `N > 0` spreads compression across `N` background workers, which
    /// is much faster for a multi-GB runtime but produces a different — still
    /// valid — archive whose bytes depend on the worker count.
    pub workers: u32,
}

impl Default for PackOptions {
    /// Level 19, single-threaded: the historical [`pack_dir`] behavior, so a
    /// small artifact (`ost package`/`ost plugin`) keeps a byte-stable digest.
    fn default() -> Self {
        Self {
            level: ZSTD_LEVEL,
            workers: 0,
        }
    }
}

/// Progress emitted after each file is written into the archive. Lets a caller
/// show liveness during a long pack (a `tar.zst` reports 0 bytes on disk until
/// the encoder flushes, so an in-flight export otherwise looks hung).
#[derive(Debug, Clone, Copy)]
pub struct PackProgress<'a> {
    pub files_done: usize,
    pub files_total: usize,
    /// Cumulative uncompressed bytes read so far.
    pub bytes_done: u64,
    /// The file just written (archive-relative, forward-slashed).
    pub path: &'a str,
}

/// Pack the given `files` (absolute paths under `stage`) into a `tar.zst` at
/// `archive`, single-threaded at the default level.
///
/// `files` is packed in the given order, each hashed as it is written; pass a
/// sorted list (e.g. from [`stage_files`]) for a deterministic archive layout.
/// Returns per-file entries and the archive digest. See [`pack_dir_with`] for
/// the compression level, worker count, and progress reporting.
pub fn pack_dir(
    stage: &Utf8Path,
    archive: &Utf8Path,
    files: &[Utf8PathBuf],
) -> io::Result<PackResult> {
    pack_dir_with(stage, archive, files, PackOptions::default(), &mut |_| {})
}

/// [`pack_dir`] with an explicit [`PackOptions`] and a `progress` callback fired
/// once per file written.
pub fn pack_dir_with(
    stage: &Utf8Path,
    archive: &Utf8Path,
    files: &[Utf8PathBuf],
    opts: PackOptions,
    progress: &mut dyn FnMut(PackProgress),
) -> io::Result<PackResult> {
    if let Some(parent) = archive.parent() {
        std::fs::create_dir_all(parent.as_std_path())?;
    }

    let out = File::create(archive.as_std_path())?;
    let mut encoder = zstd::stream::write::Encoder::new(out, opts.level)?;
    if opts.workers > 0 {
        // Spread compression across worker threads. Requires the `zstdmt`
        // feature (enabled in the workspace); the archive stays valid either
        // way, only its exact bytes change with the worker count.
        encoder.multithread(opts.workers)?;
    }
    let encoder = encoder.auto_finish();
    let mut builder = tar::Builder::new(encoder);

    let total = files.len();
    let mut entries = Vec::with_capacity(total);
    let mut total_size = 0u64;
    for (i, abs) in files.iter().enumerate() {
        let rel = abs
            .strip_prefix(stage)
            .map(|p| p.as_str().replace('\\', "/"))
            .unwrap_or_else(|_| abs.as_str().to_string());
        let data = std::fs::read(abs.as_std_path())?;

        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, &rel, data.as_slice())?;

        total_size += data.len() as u64;
        let sha256 = digest::sha256_hex(&data);
        progress(PackProgress {
            files_done: i + 1,
            files_total: total,
            bytes_done: total_size,
            path: &rel,
        });
        entries.push(FileEntry {
            path: rel,
            sha256,
            size: data.len() as u64,
        });
    }

    builder.finish()?;
    drop(builder); // flush and close the zstd encoder + file

    // Stream-hash the finished archive rather than `fs::read`-ing it whole: a
    // real runtime archive is many GB, and holding it in memory right after the
    // pack would spike RSS (and stall, looking hung).
    let mut f = File::open(archive.as_std_path())?;
    let (archive_digest, archive_size) = digest::sha256_hex_reader(&mut f)?;
    Ok(PackResult {
        files: entries,
        archive_digest,
        total_size,
        archive_size,
    })
}

/// List the regular files under `stage` (recursive, sorted).
///
/// Walked once and reused: the caller can reject an empty install tree *before*
/// writing an archive (so an empty `ost package` has no side effects unless
/// explicitly allowed) and then hand the same list to [`pack_dir`]. Returns an
/// empty list if `stage` does not exist.
///
/// Only regular files and directories are accepted. A symlink, FIFO, socket, or
/// device node anywhere in the tree (including the stage root itself) is a hard
/// error: following a symlink would copy the *link target's* bytes into the
/// artifact — SSH keys, CI credentials, environment files reached via a planted
/// link — or recurse outside the tree entirely (harness §SEC-001). Type is
/// judged by the entry itself, never by what a link points at.
pub fn stage_files(stage: &Utf8Path) -> io::Result<Vec<Utf8PathBuf>> {
    match std::fs::symlink_metadata(stage.as_std_path()) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(unsupported_entry("symlink", stage));
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    }
    let mut paths = Vec::new();
    collect_files(stage, &mut paths)?;
    paths.sort();
    Ok(paths)
}

/// Top-level directories of the **SDK layout**: the subtrees needed to *build*
/// and *run* an OpenUSD plugin against a runtime. A runtime adopted from a full
/// USD build tree also carries the source (`src/`) and build (`build/`) trees —
/// gigabytes that no consumer of the runtime needs. A slim export keeps only
/// this set (see [`is_sdk_path`]).
///
/// - `include`/`lib` — headers, import libs, shared libs, and the `pxr` Python
///   bindings a build links and a session loads.
/// - `bin` — the runtime tools (`usdcat`, `usdview`) and their DLLs.
/// - `plugin` — USD plugins discovered via `PXR_PLUGINPATH_NAME`.
/// - `cmake` — the exported `pxrTargets.cmake` etc. `find_package(pxr)` loads.
/// - `libraries` — MaterialX's standard data libraries, loaded at runtime for
///   shading (kept; distinct from `resources/`, which is sample geometry/images
///   a plugin build/test never needs).
const SDK_DIRS: &[&str] = &["include", "lib", "bin", "plugin", "cmake", "libraries"];

/// Whether `rel` (a path relative to the runtime prefix, forward- or
/// back-slashed) belongs in the SDK layout: under an [`SDK_DIRS`] subtree, or a
/// top-level CMake package config (`*.cmake`) or attribution file
/// (`LICENSE*`/`NOTICE*`/`THIRD*`). Everything else — notably `build/` and
/// `src/` — is excluded from a slim export.
pub fn is_sdk_path(rel: &Utf8Path) -> bool {
    let mut comps = rel.as_str().split(['/', '\\']).filter(|c| !c.is_empty());
    let Some(first) = comps.next() else {
        return false;
    };
    // A nested path (has a second component) keeps only the SDK subtrees.
    if comps.next().is_some() {
        return SDK_DIRS.contains(&first);
    }
    // A top-level file: keep build-config and attribution, drop the rest.
    let lower = first.to_ascii_lowercase();
    lower.ends_with(".cmake")
        || lower.starts_with("license")
        || lower.starts_with("notice")
        || lower.starts_with("third")
}

/// List regular files that belong to the SDK layout, pruning excluded
/// top-level trees before validating their contents.
///
/// This is the slim-export counterpart to [`stage_files`]: kept SDK subtrees
/// still reject symlinks and special files, but excluded build/source trees are
/// not walked at all.
pub fn sdk_stage_files(stage: &Utf8Path) -> io::Result<SdkStageFiles> {
    match std::fs::symlink_metadata(stage.as_std_path()) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(unsupported_entry("symlink", stage));
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(SdkStageFiles::default()),
        Err(e) => return Err(e),
    }

    let mut files = Vec::new();
    let mut excluded = BTreeSet::new();
    collect_sdk_files(stage, stage, &mut files, &mut excluded)?;
    files.sort();
    Ok(SdkStageFiles {
        files,
        excluded_top_level: excluded.into_iter().collect(),
    })
}

/// Recursively collect regular files under `dir`, rejecting any non-regular,
/// non-directory entry.
fn collect_files(dir: &Utf8Path, out: &mut Vec<Utf8PathBuf>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir.as_std_path())? {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|_| io::Error::other("non-UTF-8 path in stage tree"))?;
        // `DirEntry::file_type` does not follow symlinks (unlike `is_dir`/
        // `metadata`), so a link is classified as a link, not as its target.
        let ty = entry.file_type()?;
        if ty.is_symlink() {
            return Err(unsupported_entry("symlink", &path));
        } else if ty.is_dir() {
            collect_files(&path, out)?;
        } else if ty.is_file() {
            out.push(path);
        } else {
            // FIFO, socket, block/character device — nothing that belongs in a
            // portable, content-addressed artifact.
            return Err(unsupported_entry(
                "special file (FIFO/socket/device)",
                &path,
            ));
        }
    }
    Ok(())
}

fn collect_sdk_files(
    stage: &Utf8Path,
    dir: &Utf8Path,
    out: &mut Vec<Utf8PathBuf>,
    excluded: &mut BTreeSet<String>,
) -> io::Result<()> {
    for entry in std::fs::read_dir(dir.as_std_path())? {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|_| io::Error::other("non-UTF-8 path in stage tree"))?;
        let rel = path.strip_prefix(stage).unwrap_or(&path);
        let top = top_component(rel);
        let in_sdk_subtree = top.is_some_and(|c| SDK_DIRS.contains(&c));
        let ty = entry.file_type()?;

        if ty.is_dir() {
            if in_sdk_subtree {
                collect_sdk_files(stage, &path, out, excluded)?;
            } else if let Some(top) = top {
                excluded.insert(top.to_string());
            }
        } else if ty.is_file() {
            if is_sdk_path(rel) {
                out.push(path);
            } else if let Some(top) = top {
                excluded.insert(top.to_string());
            }
        } else if in_sdk_subtree || is_sdk_path(rel) {
            let kind = if ty.is_symlink() {
                "symlink"
            } else {
                "special file (FIFO/socket/device)"
            };
            return Err(unsupported_entry(kind, &path));
        } else if let Some(top) = top {
            excluded.insert(top.to_string());
        }
    }
    Ok(())
}

fn top_component(rel: &Utf8Path) -> Option<&str> {
    rel.as_str()
        .split(['/', '\\'])
        .find(|component| !component.is_empty())
}

/// A uniform "this does not belong in a package" error.
fn unsupported_entry(kind: &str, path: &Utf8Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{kind} is not allowed in the package staging area: {path}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> Utf8PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut d = Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
        d.push(format!("ost-pack-{tag}-{}-{nanos}", std::process::id()));
        d
    }

    #[test]
    fn is_sdk_path_keeps_layout_and_config_drops_build_tree() {
        // SDK subtrees are kept.
        for keep in [
            "include/pxr/base/tf/tf.h",
            "lib/usd_tf.dll",
            "lib/site-packages/pxr/Tf/_tf.pyd",
            "bin/usdcat.exe",
            "plugin/usd/plugInfo.json",
            "cmake/pxrTargets.cmake",
            "libraries/stdlib/stdlib_defs.mtlx",
        ] {
            assert!(is_sdk_path(Utf8Path::new(keep)), "should keep {keep}");
        }
        // Top-level config/attribution files are kept.
        for keep in ["pxrConfig.cmake", "LICENSE", "NOTICE", "THIRD-PARTY.md"] {
            assert!(is_sdk_path(Utf8Path::new(keep)), "should keep {keep}");
        }
        // The build/source tree and other top-level junk are dropped.
        for drop in [
            "build/OpenUSD/pxr/base/tf/tf.obj",
            "src/MaterialX-1.39.4/README.md",
            "resources/Geometry/shaderball.usda",
            "CHANGELOG.md",
            "README.md",
        ] {
            assert!(!is_sdk_path(Utf8Path::new(drop)), "should drop {drop}");
        }
        // Backslash separators (Windows-staged relative paths) work too.
        assert!(is_sdk_path(Utf8Path::new("lib\\usd_tf.dll")));
        assert!(!is_sdk_path(Utf8Path::new("build\\x.obj")));
    }

    #[test]
    fn sdk_stage_files_prunes_to_sdk_layout() {
        let root = tmp("sdk-prune");
        std::fs::create_dir_all(root.join("include/pxr").as_std_path()).unwrap();
        std::fs::create_dir_all(root.join("lib").as_std_path()).unwrap();
        std::fs::create_dir_all(root.join("build/tmp").as_std_path()).unwrap();
        std::fs::create_dir_all(root.join("src/OpenUSD").as_std_path()).unwrap();
        std::fs::write(root.join("include/pxr/pxr.h").as_std_path(), b"h").unwrap();
        std::fs::write(root.join("lib/usd_tf.dll").as_std_path(), b"dll").unwrap();
        std::fs::write(root.join("pxrConfig.cmake").as_std_path(), b"cmake").unwrap();
        std::fs::write(root.join("build/tmp/object.obj").as_std_path(), b"obj").unwrap();
        std::fs::write(root.join("src/OpenUSD/README.md").as_std_path(), b"src").unwrap();
        std::fs::write(root.join("README.md").as_std_path(), b"readme").unwrap();

        let selected = sdk_stage_files(&root).unwrap();
        let rels: Vec<String> = selected
            .files
            .iter()
            .map(|p| p.strip_prefix(&root).unwrap().as_str().replace('\\', "/"))
            .collect();
        assert_eq!(
            rels,
            vec!["include/pxr/pxr.h", "lib/usd_tf.dll", "pxrConfig.cmake"]
        );
        assert_eq!(
            selected.excluded_top_level,
            vec!["README.md", "build", "src"]
        );

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[cfg(unix)]
    #[test]
    fn sdk_stage_files_prunes_excluded_symlinks() {
        let root = tmp("sdk-prune-symlink");
        std::fs::create_dir_all(root.join("include").as_std_path()).unwrap();
        std::fs::create_dir_all(root.join("build").as_std_path()).unwrap();
        std::fs::write(root.join("include/pxr.h").as_std_path(), b"h").unwrap();
        std::os::unix::fs::symlink("/etc/hostname", root.join("build/link").as_std_path()).unwrap();

        let selected = sdk_stage_files(&root).expect("excluded build tree symlinks are not walked");
        assert_eq!(selected.files.len(), 1);
        assert_eq!(selected.excluded_top_level, vec!["build"]);

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[cfg(unix)]
    #[test]
    fn sdk_stage_files_rejects_kept_symlinks() {
        let root = tmp("sdk-kept-symlink");
        std::fs::create_dir_all(root.join("lib").as_std_path()).unwrap();
        std::os::unix::fs::symlink("/etc/hostname", root.join("lib/link").as_std_path()).unwrap();

        let err = sdk_stage_files(&root).expect_err("kept SDK symlinks must be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("symlink"), "got: {err}");

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn pack_dir_with_workers_and_level_roundtrips_and_reports_progress() {
        let root = tmp("pack-mt");
        std::fs::create_dir_all(root.join("lib").as_std_path()).unwrap();
        std::fs::write(root.join("lib/a.bin").as_std_path(), vec![7u8; 4096]).unwrap();
        std::fs::write(root.join("lib/b.bin").as_std_path(), vec![9u8; 8192]).unwrap();
        std::fs::write(root.join("top.txt").as_std_path(), b"hello").unwrap();
        let files = stage_files(&root).unwrap();

        let archive = root.join("out.tar.zst");
        let mut seen = Vec::new();
        let packed = pack_dir_with(
            &root,
            &archive,
            &files,
            PackOptions {
                level: 3,
                workers: 2,
            },
            &mut |p| seen.push((p.files_done, p.files_total, p.bytes_done)),
        )
        .unwrap();

        // Progress fired once per file, monotonically, ending at the total.
        assert_eq!(seen.len(), files.len());
        assert_eq!(seen.last().unwrap().0, files.len());
        assert_eq!(seen.last().unwrap().2, packed.total_size);
        assert_eq!(packed.total_size, 4096 + 8192 + 5);

        // The digest re-hashes the bytes actually on disk.
        let mut f = File::open(archive.as_std_path()).unwrap();
        let (digest_on_disk, size_on_disk) = digest::sha256_hex_reader(&mut f).unwrap();
        assert_eq!(digest_on_disk, packed.archive_digest);
        assert_eq!(size_on_disk, packed.archive_size);

        // The multithreaded archive unpacks back to the original contents.
        let reader =
            zstd::stream::read::Decoder::new(File::open(archive.as_std_path()).unwrap()).unwrap();
        let mut names = Vec::new();
        for entry in tar::Archive::new(reader).entries().unwrap() {
            let entry = entry.unwrap();
            names.push(entry.path().unwrap().to_string_lossy().replace('\\', "/"));
        }
        names.sort();
        assert_eq!(names, vec!["lib/a.bin", "lib/b.bin", "top.txt"]);

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn stage_files_handles_missing_empty_and_nested() {
        let root = tmp("count");
        // Missing → empty.
        assert!(stage_files(&root).unwrap().is_empty());

        // Exists but empty (only subdirs) → empty.
        std::fs::create_dir_all(root.join("lib").as_std_path()).unwrap();
        assert!(stage_files(&root).unwrap().is_empty());

        // Nested regular files are collected, sorted.
        std::fs::write(root.join("lib/libfoo.so").as_std_path(), b"x").unwrap();
        std::fs::write(root.join("plugInfo.json").as_std_path(), b"{}").unwrap();
        let files = stage_files(&root).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.windows(2).all(|w| w[0] <= w[1]), "paths are sorted");

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[cfg(unix)]
    #[test]
    fn stage_files_rejects_symlinks(/* §SEC-001 */) {
        let root = tmp("symlink");
        std::fs::create_dir_all(root.as_std_path()).unwrap();
        std::fs::write(root.join("real.txt").as_std_path(), b"ok").unwrap();
        // A link pointing at a sensitive file outside the tree.
        std::os::unix::fs::symlink("/etc/hostname", root.join("leak").as_std_path()).unwrap();

        let err = stage_files(&root).expect_err("a symlink in the stage must be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("symlink"), "got: {err}");

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[cfg(unix)]
    #[test]
    fn stage_files_rejects_a_symlinked_root() {
        let base = tmp("symlink-root");
        std::fs::create_dir_all(base.as_std_path()).unwrap();
        let real = base.join("real-stage");
        std::fs::create_dir_all(real.as_std_path()).unwrap();
        let link = base.join("stage");
        std::os::unix::fs::symlink(real.as_std_path(), link.as_std_path()).unwrap();

        let err = stage_files(&link).expect_err("a symlinked stage root must be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        std::fs::remove_dir_all(base.as_std_path()).ok();
    }

    #[cfg(unix)]
    #[test]
    fn stage_files_rejects_special_files() {
        // A unix-domain socket is a special file (not regular, not a dir, not a
        // symlink), creatable from std alone — no extra dependency for the test.
        let root = tmp("socket");
        std::fs::create_dir_all(root.as_std_path()).unwrap();
        let sock = root.join("s.sock");
        let _listener = std::os::unix::net::UnixListener::bind(sock.as_std_path()).unwrap();

        let err = stage_files(&root).expect_err("a socket in the stage must be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("special file"), "got: {err}");

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }
}
