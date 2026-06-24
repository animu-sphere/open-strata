// SPDX-License-Identifier: Apache-2.0
//! Filesystem helpers shared across OpenStrata.
//!
//! The build tools must never leave a user's file half-written: a crash or a
//! full disk during a naive `write` truncates the original. [`write_atomic`]
//! writes to a sibling temp file, fsyncs it, then renames it over the
//! destination, so a reader sees either the old or the new content — never a
//! torn file.

use std::io::Write;
use std::path::Path;

use crate::{Error, Result};

/// Write `contents` to `path` atomically.
///
/// A temp file in the same directory is written, flushed, and fsynced, then
/// renamed over `path` (rename within a directory is atomic on every supported
/// platform; on Windows it also replaces an existing destination). On failure
/// the temp file is cleaned up and `path` is left untouched.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let dir = dir.unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("tmp");
    let tmp = dir.join(format!(".{name}.ost-{}.tmp", std::process::id()));

    let write = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
        Ok(())
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&tmp);
        return Err(Error::io(tmp.display().to_string(), e));
    }

    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        Error::io(path.display().to_string(), e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomic_replaces_existing_content() {
        let dir = std::env::temp_dir().join(format!("ost-fs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("file.json");

        std::fs::write(&path, b"old").unwrap();
        write_atomic(&path, b"new contents").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new contents");

        // No stray temp files left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
