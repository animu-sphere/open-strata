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

/// Zstd compression level for artifacts (high ratio; artifacts are written once).
const ZSTD_LEVEL: i32 = 19;

/// Pack the given `files` (absolute paths under `stage`) into a `tar.zst` at
/// `archive`.
///
/// `files` is packed in the given order, each hashed as it is written; pass a
/// sorted list (e.g. from [`stage_files`]) for a deterministic archive layout.
/// Returns per-file entries and the archive digest.
pub fn pack_dir(
    stage: &Utf8Path,
    archive: &Utf8Path,
    files: &[Utf8PathBuf],
) -> io::Result<PackResult> {
    if let Some(parent) = archive.parent() {
        std::fs::create_dir_all(parent.as_std_path())?;
    }

    let out = File::create(archive.as_std_path())?;
    let encoder = zstd::stream::write::Encoder::new(out, ZSTD_LEVEL)?.auto_finish();
    let mut builder = tar::Builder::new(encoder);

    let mut entries = Vec::new();
    let mut total_size = 0u64;
    for abs in files {
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
        entries.push(FileEntry {
            path: rel,
            sha256: digest::sha256_hex(&data),
            size: data.len() as u64,
        });
    }

    builder.finish()?;
    drop(builder); // flush and close the zstd encoder + file

    let archive_bytes = std::fs::read(archive.as_std_path())?;
    Ok(PackResult {
        files: entries,
        archive_digest: digest::sha256_hex(&archive_bytes),
        total_size,
        archive_size: archive_bytes.len() as u64,
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
