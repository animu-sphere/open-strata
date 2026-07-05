// SPDX-License-Identifier: Apache-2.0
//! Filesystem helpers shared across OpenStrata.
//!
//! The build tools must never leave a user's file half-written: a crash or a
//! full disk during a naive `write` truncates the original. [`write_atomic`]
//! writes to a sibling temp file, fsyncs it, then renames it over the
//! destination, so a reader sees either the old or the new content — never a
//! torn file.
//!
//! The temp file is created with an unpredictable name and `create_new`
//! (`O_EXCL`), so a process sharing the directory cannot pre-create or
//! symlink-hijack the path we are about to write (harness §SEC-003).
//!
//! [`prepare_staging_dir`] / [`remove_dir_all_robust`] give packaging its
//! staging trees: reruns must survive a previous stage that Windows briefly
//! refuses to delete (read-only staged copies, scanner-held handles).

use std::collections::hash_map::RandomState;
use std::fs::OpenOptions;
use std::hash::{BuildHasher, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// How many times to retry if the random temp name happens to collide.
const MAX_TEMP_ATTEMPTS: u32 = 16;

/// Delays between removal retries for [`remove_dir_all_robust`]. Short and
/// bounded (~0.4s total): the transient holders this waits out (antivirus and
/// indexer scans of a previous run's fresh output) usually release within
/// milliseconds, and callers that can stage elsewhere fall back rather than
/// block longer.
const REMOVE_RETRY_DELAYS: &[std::time::Duration] = &[
    std::time::Duration::from_millis(10),
    std::time::Duration::from_millis(50),
    std::time::Duration::from_millis(100),
    std::time::Duration::from_millis(250),
];

/// `remove_dir_all` for build staging trees; a missing tree is success.
///
/// Windows makes a freshly written tree undeletable in two ways this smooths
/// over. Staged copies keep the source's read-only attribute (`fs::copy`
/// preserves it) and Windows refuses to delete read-only files — clear the
/// attribute between attempts. And scanners (Defender, search indexers) hold
/// just-created files open briefly without `FILE_SHARE_DELETE`, which surfaces
/// as a transient `access denied (os error 5)` on a rerun (dogfooding reports
/// #8/#9) — retry over [`REMOVE_RETRY_DELAYS`], then give up so callers can
/// fall back instead of blocking.
pub fn remove_dir_all_robust(dir: &Path) -> std::io::Result<()> {
    let mut result = std::fs::remove_dir_all(dir);
    for delay in REMOVE_RETRY_DELAYS {
        match &result {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => {
                #[cfg(windows)]
                {
                    let _ = clear_readonly_recursive(dir);
                }
                std::thread::sleep(*delay);
                result = std::fs::remove_dir_all(dir);
            }
        }
    }
    match result {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(windows)]
fn clear_readonly_recursive(path: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    let mut perms = meta.permissions();
    if perms.readonly() {
        // Windows-only code: this clears FILE_ATTRIBUTE_READONLY; the lint's
        // unix world-writable concern cannot apply under cfg(windows).
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(path, perms)?;
    }
    if meta.is_dir() {
        for entry in std::fs::read_dir(path)? {
            clear_readonly_recursive(&entry?.path())?;
        }
    }
    Ok(())
}

/// Prepare an empty staging directory, preferring `preferred`.
///
/// A rerun must not fail just because the previous run's stage is temporarily
/// undeletable — on Windows a scanner can still hold the last run's files open,
/// so the reset hits `access denied (os error 5)` (dogfooding report #9). After
/// [`remove_dir_all_robust`]'s bounded retries this falls back to a fresh
/// sibling `<name>-<16 hex>`, leaving the stuck tree behind; every call
/// best-effort sweeps such leftovers, so a later run removes it once the
/// handles close. Callers must stage into the returned path, which is only
/// `preferred` when the reset succeeded.
pub fn prepare_staging_dir(preferred: &Path) -> Result<PathBuf> {
    let parent = preferred
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = preferred
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("stage");
    sweep_stale_staging(parent, name);

    if remove_dir_all_robust(preferred).is_ok() {
        // The recreate can still fail: a removed directory lingers in
        // delete-pending state until the last open handle closes, and creating
        // over it is access-denied. Treat that like the removal failing.
        if std::fs::create_dir_all(preferred).is_ok() {
            return Ok(preferred.to_path_buf());
        }
    }

    std::fs::create_dir_all(parent).map_err(|e| Error::io(parent.display().to_string(), e))?;
    let mut last_err = None;
    for _ in 0..MAX_TEMP_ATTEMPTS {
        let alt = parent.join(format!("{name}-{:016x}", random_token()));
        match std::fs::create_dir(&alt) {
            Ok(()) => return Ok(alt),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last_err = Some(e),
            Err(e) => return Err(Error::io(alt.display().to_string(), e)),
        }
    }
    let e = last_err.unwrap_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::AlreadyExists, "staging name collision")
    });
    Err(Error::io(preferred.display().to_string(), e))
}

/// Best-effort removal of fallback staging directories a previous run left
/// behind (`<name>-<16 hex>` siblings of the preferred stage). Failures are
/// ignored: a still-locked tree just waits for a later sweep.
fn sweep_stale_staging(parent: &Path, name: &str) {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    let prefix = format!("{name}-");
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(s) = file_name.to_str() else {
            continue;
        };
        let Some(suffix) = s.strip_prefix(&prefix) else {
            continue;
        };
        if suffix.len() == 16 && suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
            let _ = remove_dir_all_robust(&entry.path());
        }
    }
}

/// Write `contents` to `path` atomically.
///
/// A temp file in the same directory is created with `create_new` and an
/// unpredictable name, written, flushed, and fsynced, then renamed over `path`
/// (rename within a directory is atomic on every supported platform; on Windows
/// it also replaces an existing destination). The parent directory is fsynced
/// afterwards so the rename itself survives a crash. On failure the temp file is
/// cleaned up and `path` is left untouched.
///
/// If `path` already exists and is a symlink, this errors rather than follow it:
/// generated files are written in place, not through a link an attacker (or a
/// stale state directory) may have planted.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("tmp");

    // Refuse to clobber a symlink sitting at the destination.
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(Error::io(
                path.display().to_string(),
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "refusing to write atomically over a symlink",
                ),
            ));
        }
    }

    let (mut file, tmp) = create_temp(dir, name)?;

    let result = file.write_all(contents).and_then(|()| file.sync_all());
    // Close the handle before the rename: Windows cannot rename a file that is
    // still open.
    drop(file);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(Error::io(tmp.display().to_string(), e));
    }

    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        Error::io(path.display().to_string(), e)
    })?;

    // Best-effort: fsync the directory so the rename is durable. Not every
    // platform/filesystem supports this, so failures here are not fatal.
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Create a fresh temp file next to the destination, returning the open handle
/// and its path. The name is unpredictable and the file is created with
/// `O_EXCL`, so it cannot be a pre-existing file or a symlink.
fn create_temp(dir: &Path, name: &str) -> Result<(std::fs::File, PathBuf)> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    // Mode is left to the process umask (as a plain `File::create` would): the
    // current callers write shared project config (`CMakePresets.json`,
    // `CMakeUserPresets.json`) that CMake, IDEs, and other accounts must read, so
    // forcing owner-only here would break a second UID reading the checkout. A
    // future helper can opt sensitive outputs into a tighter mode if one appears.

    let mut last_err = None;
    for _ in 0..MAX_TEMP_ATTEMPTS {
        let tmp = dir.join(format!(".{name}.ost-{:016x}.tmp", random_token()));
        match opts.open(&tmp) {
            Ok(f) => return Ok((f, tmp)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                last_err = Some(e);
                continue;
            }
            Err(e) => return Err(Error::io(tmp.display().to_string(), e)),
        }
    }
    let e = last_err.unwrap_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::AlreadyExists, "temp name collision")
    });
    Err(Error::io(dir.join(name).display().to_string(), e))
}

/// An unpredictable 64-bit token for the temp filename.
///
/// `RandomState` is seeded from OS entropy when constructed (it backs `HashMap`'s
/// hash-flooding defense); mixing in the clock and PID gives a fresh value per
/// call. This makes the name unguessable — the hard guarantee against hijacking
/// still comes from the `O_EXCL` create, which refuses an existing path.
fn random_token() -> u64 {
    let mut h = RandomState::new().build_hasher();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    h.write_u128(nanos);
    h.write_u32(std::process::id());
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("ost-fs-{tag}-{}-{nanos}", std::process::id()))
    }

    #[test]
    fn write_atomic_replaces_existing_content() {
        let dir = scratch("replace");
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

    #[test]
    fn temp_names_are_unpredictable() {
        // Two tokens in a row must not match; a predictable PID-only name would.
        assert_ne!(random_token(), random_token());
    }

    #[test]
    fn prepare_staging_dir_resets_existing_contents() {
        let dir = scratch("stage-reset");
        let preferred = dir.join("package-stage");
        let leftover = preferred.join("resources").join("plugInfo.json");
        std::fs::create_dir_all(leftover.parent().unwrap()).unwrap();
        std::fs::write(&leftover, "{}").unwrap();
        // A read-only staged copy (fs::copy preserves the attribute) used to
        // fail the Windows reset with access-denied (dogfooding report #8).
        let mut perms = std::fs::metadata(&leftover).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&leftover, perms).unwrap();

        let stage = prepare_staging_dir(&preferred).unwrap();
        assert_eq!(stage, preferred, "deletable stage is reused in place");
        assert!(stage.is_dir(), "stage was recreated");
        assert!(!leftover.exists(), "old contents were removed");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prepare_staging_dir_creates_missing_parents() {
        let dir = scratch("stage-fresh");
        let preferred = dir.join("targets").join("t").join("package-stage");
        let stage = prepare_staging_dir(&preferred).unwrap();
        assert_eq!(stage, preferred);
        assert!(stage.is_dir());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prepare_staging_dir_sweeps_fallback_leftovers() {
        let dir = scratch("stage-sweep");
        let preferred = dir.join("package-stage");
        let stale = dir.join("package-stage-0123456789abcdef");
        let unrelated = dir.join("package-stage-notahexsuffix!");
        std::fs::create_dir_all(stale.join("lib")).unwrap();
        std::fs::create_dir_all(&unrelated).unwrap();

        let stage = prepare_staging_dir(&preferred).unwrap();
        assert_eq!(stage, preferred);
        assert!(!stale.exists(), "fallback leftover was swept");
        assert!(unrelated.is_dir(), "non-fallback siblings are untouched");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The dogfooding report #9 failure: a scanner-style process holds a file
    /// from the previous stage open without `FILE_SHARE_DELETE`, so the reset
    /// cannot remove the tree. The rerun must proceed in a fresh sibling
    /// instead of failing, and a later run must sweep that sibling.
    #[cfg(windows)]
    #[test]
    fn prepare_staging_dir_falls_back_when_stage_is_locked() {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;

        let dir = scratch("stage-locked");
        let preferred = dir.join("package-stage");
        let locked = preferred.join("lib").join("plugin.dll");
        std::fs::create_dir_all(locked.parent().unwrap()).unwrap();
        std::fs::write(&locked, b"dll bytes").unwrap();

        let holder = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ) // readable, but not deletable
            .open(&locked)
            .unwrap();

        let stage = prepare_staging_dir(&preferred).expect("rerun must not fail on a locked stage");
        assert_ne!(stage, preferred, "locked stage forces a fallback sibling");
        assert!(stage.is_dir(), "fallback stage exists");
        let stage_name = stage.file_name().unwrap().to_str().unwrap();
        assert!(
            stage_name.starts_with("package-stage-"),
            "fallback is a recognizable sibling: {stage_name}"
        );

        drop(holder);
        let swept = prepare_staging_dir(&preferred).unwrap();
        assert_eq!(swept, preferred, "unlocked stage is reclaimed");
        assert!(!stage.exists(), "previous fallback was swept");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_refuses_a_symlinked_destination() {
        let dir = scratch("symlink-dest");
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("outside.txt");
        std::fs::write(&target, b"secret").unwrap();
        let dest = dir.join("link.json");
        std::os::unix::fs::symlink(&target, &dest).unwrap();

        let err =
            write_atomic(&dest, b"payload").expect_err("writing over a symlink must be rejected");
        assert_eq!(err.code(), "IO_ERROR");
        // The link target is untouched — we did not write through it.
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "secret");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_honors_umask_like_a_plain_write() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("perms");
        std::fs::create_dir_all(&dir).unwrap();

        // A reference file written the ordinary way picks up the same umask the
        // atomic write would; the atomic write must not apply a tighter mode of
        // its own (which would lock out other accounts reading the checkout).
        let reference = dir.join("reference.json");
        std::fs::write(&reference, b"x").unwrap();
        let want = std::fs::metadata(&reference).unwrap().permissions().mode() & 0o777;

        let path = dir.join("file.json");
        write_atomic(&path, b"x").unwrap();
        let got = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;

        assert_eq!(
            got, want,
            "atomic write mode {got:o} != plain write {want:o}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
