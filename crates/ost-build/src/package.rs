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

/// Pack every file under `stage` into a `tar.zst` at `archive`.
///
/// Files are added in sorted order for a deterministic archive layout, each
/// hashed as it is written. Returns per-file entries and the archive digest.
pub fn pack_dir(stage: &Utf8Path, archive: &Utf8Path) -> io::Result<PackResult> {
    let mut paths = Vec::new();
    collect_files(stage, &mut paths)?;
    paths.sort();

    if let Some(parent) = archive.parent() {
        std::fs::create_dir_all(parent.as_std_path())?;
    }

    let out = File::create(archive.as_std_path())?;
    let encoder = zstd::stream::write::Encoder::new(out, ZSTD_LEVEL)?.auto_finish();
    let mut builder = tar::Builder::new(encoder);

    let mut files = Vec::new();
    let mut total_size = 0u64;
    for abs in &paths {
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
        files.push(FileEntry {
            path: rel,
            sha256: digest::sha256_hex(&data),
            size: data.len() as u64,
        });
    }

    builder.finish()?;
    drop(builder); // flush and close the zstd encoder + file

    let archive_bytes = std::fs::read(archive.as_std_path())?;
    Ok(PackResult {
        files,
        archive_digest: digest::sha256_hex(&archive_bytes),
        total_size,
        archive_size: archive_bytes.len() as u64,
    })
}

/// Count the regular files under `stage` (recursively).
///
/// Used to reject an empty install tree *before* writing an archive, so an
/// empty `ost package` produces no side effects unless explicitly allowed.
/// Returns `Ok(0)` if `stage` does not exist.
pub fn count_files(stage: &Utf8Path) -> io::Result<usize> {
    if !stage.as_std_path().exists() {
        return Ok(0);
    }
    let mut paths = Vec::new();
    collect_files(stage, &mut paths)?;
    Ok(paths.len())
}

/// Recursively collect regular files under `dir`.
fn collect_files(dir: &Utf8Path, out: &mut Vec<Utf8PathBuf>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir.as_std_path())? {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|_| io::Error::other("non-UTF-8 path in stage tree"))?;
        if path.as_std_path().is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
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
    fn count_files_handles_missing_empty_and_nested() {
        let root = tmp("count");
        // Missing → 0.
        assert_eq!(count_files(&root).unwrap(), 0);

        // Exists but empty (only subdirs) → 0.
        std::fs::create_dir_all(root.join("lib").as_std_path()).unwrap();
        assert_eq!(count_files(&root).unwrap(), 0);

        // Nested regular files are counted.
        std::fs::write(root.join("lib/libfoo.so").as_std_path(), b"x").unwrap();
        std::fs::write(root.join("plugInfo.json").as_std_path(), b"{}").unwrap();
        assert_eq!(count_files(&root).unwrap(), 2);

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }
}
