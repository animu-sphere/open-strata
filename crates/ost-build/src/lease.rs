// SPDX-License-Identifier: Apache-2.0
//! An OS-backed exclusive lease over a managed build target.
//!
//! A managed target is a single writable resource: configure regenerates its
//! `.strata/` files, build writes `build/<id>`, output verification reads what
//! that build produced, and completion publication asserts the whole sequence
//! succeeded. Two invocations doing that concurrently interleave into a tree
//! neither of them describes — the hdMerlin v0.17.0 dogfooding pass hit exactly
//! that, with two runs writing one managed target and a completion record that
//! belonged to neither.
//!
//! The lease makes the target's writer explicit. It is held for the whole
//! sequence, not per phase: a lease released between configure and build would
//! let a second writer reconfigure the tree the first is about to compile.
//!
//! ## Why the operating system holds it
//!
//! The exclusion is an OS file lock, not a "is this PID alive?" heuristic over a
//! PID file. The distinction matters on the path that actually breaks: a builder
//! killed by CI, a laptop that lost power, a `taskkill /F`. A PID file outlives
//! all three and wedges the target until someone deletes it by hand; an OS lock
//! is released by the kernel as the process dies, however it dies.
//!
//! * **Unix** — `flock(LOCK_EX | LOCK_NB)`. Advisory, so the record stays
//!   readable by a waiter that wants to name the holder.
//! * **Windows** — an exclusive `CreateFile` share mode. The handle admits
//!   `FILE_SHARE_READ` and nothing more: a second *writer* is refused with a
//!   sharing violation, while a reader that asks for read access only still gets
//!   in to read the owner record.
//!
//! Neither call needs a crate outside `std`; the Unix side declares `flock`
//! the same way [`crate::msvc`] and the CLI's process-group handling declare the
//! few other libc entry points this tree uses.
//!
//! ## Stale owners
//!
//! Because the kernel releases the lock, acquiring it is itself the proof that
//! no live owner holds it. What can still be stale is the *record inside* the
//! file, left by an owner that died before it could clean up. Taking over is
//! therefore always safe with respect to the lock, but the previous owner is
//! still verified — [`StaleReason`] distinguishes an owner that plainly exited
//! from a PID that has since been reused and from a record written by another
//! host, where this host's kernel cannot vouch for the lock at all. The takeover
//! is reported rather than performed in silence, so a build log can say whose
//! interrupted run it inherited.

use std::collections::hash_map::RandomState;
use std::fs::{File, OpenOptions};
use std::hash::{BuildHasher, Hasher};
use std::io::{Read, Seek, SeekFrom, Write};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use ost_core::{Category, Error, Result};

/// Lease file name, written beside the target's other generated state.
pub const TARGET_LEASE_FILE: &str = "target.lease.json";

/// Schema id of the owner record inside the lease file.
pub const TARGET_LEASE_SCHEMA: &str = "openstrata.target-lease/v1";

/// Stable machine code for "another writer holds this target" (design §14.4).
pub const TARGET_BUSY_CODE: &str = "TARGET_BUSY";

/// How long to pause between attempts while waiting for a busy target.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// What to do when another writer already holds the target.
///
/// There is deliberately no "figure it out" variant. A build that silently
/// waited would look hung, and one that silently proceeded would corrupt the
/// tree; the caller states which it wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseMode {
    /// Fail immediately with [`TARGET_BUSY_CODE`], naming the holder.
    Fail,
    /// Retry until the lease is free or the timeout elapses, then fail busy.
    Wait(Duration),
    /// Do not take the lease. The caller promises not to write the target; it
    /// may still read it, and learns who owns it if anyone does.
    ReadOnly,
}

impl LeaseMode {
    /// Parse the CLI spelling: `fail`, `wait`, or `read-only`. `wait` takes its
    /// timeout from `timeout_secs`, where 0 means "wait indefinitely".
    pub fn parse(value: &str, timeout_secs: u64) -> Result<LeaseMode> {
        match value {
            "fail" => Ok(LeaseMode::Fail),
            "wait" => Ok(LeaseMode::Wait(if timeout_secs == 0 {
                Duration::MAX
            } else {
                Duration::from_secs(timeout_secs)
            })),
            "read-only" | "readonly" => Ok(LeaseMode::ReadOnly),
            other => Err(Error::usage(format!(
                "unknown busy policy '{other}' (expected fail, wait, or read-only)"
            ))),
        }
    }
}

/// The invocation that holds — or last held — a target lease.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaseOwner {
    pub schema: String,
    /// Unique per invocation. Build logs and completion records carry this, so
    /// an artifact can be traced back to the run that produced it.
    pub invocation: String,
    /// The command that took the lease, e.g. `ost build`.
    pub command: String,
    /// The managed target id this lease covers.
    pub target: String,
    pub pid: u32,
    pub host: String,
    pub acquired_unix: u64,
}

impl LeaseOwner {
    fn new(invocation: String, command: &str, target: &str) -> LeaseOwner {
        LeaseOwner {
            schema: TARGET_LEASE_SCHEMA.into(),
            invocation,
            command: command.into(),
            target: target.into(),
            pid: std::process::id(),
            host: host_name(),
            acquired_unix: now_unix(),
        }
    }

    /// A one-line identification for error messages and logs.
    pub fn describe(&self) -> String {
        format!(
            "{} (invocation {}, pid {} on {})",
            self.command, self.invocation, self.pid, self.host
        )
    }
}

/// Why a previous owner's record was still in the lease file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleReason {
    /// The recorded process is gone. The ordinary case: an interrupted run.
    OwnerExited,
    /// A live process carries the recorded PID, but it does not hold the lock —
    /// so the PID was recycled and the record describes a different process.
    /// Worth saying out loud, because the PID in an old log now means someone
    /// else.
    PidReused,
    /// The record came from another host. This host's kernel cannot arbitrate a
    /// lock held elsewhere, so on a shared filesystem the exclusion is only as
    /// good as that filesystem's locking.
    ForeignHost,
}

impl StaleReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OwnerExited => "owner-exited",
            Self::PidReused => "pid-reused",
            Self::ForeignHost => "foreign-host",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::OwnerExited => "the previous owner is no longer running",
            Self::PidReused => {
                "a live process now carries the previous owner's pid; the pid was recycled"
            }
            Self::ForeignHost => {
                "the previous owner ran on another host, where this host cannot verify process \
                 identity"
            }
        }
    }
}

/// A previous owner's record, and what verifying it concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleTakeover {
    pub previous: LeaseOwner,
    pub reason: StaleReason,
}

impl StaleTakeover {
    pub fn describe(&self) -> String {
        format!(
            "took over the target lease from {} — {}",
            self.previous.describe(),
            self.reason.describe()
        )
    }
}

/// A held lease. Dropping it releases the lock; the kernel does the same if the
/// process dies first.
#[derive(Debug)]
pub struct TargetLease {
    /// `None` in [`LeaseMode::ReadOnly`], where no lock is taken.
    file: Option<File>,
    path: Utf8PathBuf,
    owner: Option<LeaseOwner>,
    takeover: Option<StaleTakeover>,
    read_only: bool,
}

impl TargetLease {
    /// Take the lease for `target`, creating `path` and its parent if needed.
    ///
    /// `command` names the caller (`ost build`) and rides in the owner record.
    pub fn acquire(
        path: &Utf8Path,
        target: &str,
        command: &str,
        mode: LeaseMode,
    ) -> Result<TargetLease> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent.as_std_path())
                .map_err(|error| Error::io(parent.to_string(), error))?;
        }

        if mode == LeaseMode::ReadOnly {
            return Ok(TargetLease {
                file: None,
                path: path.to_owned(),
                owner: read_owner(path),
                takeover: None,
                read_only: true,
            });
        }

        let deadline = match mode {
            LeaseMode::Wait(timeout) => Some(Instant::now().checked_add(timeout)),
            _ => None,
        };

        let file = loop {
            match try_lock(path) {
                Ok(Some(file)) => break file,
                Ok(None) => {
                    // Busy. Wait only if asked, and only while time remains.
                    let waiting = match deadline {
                        // `checked_add` overflowed, i.e. Duration::MAX: wait forever.
                        Some(None) => true,
                        Some(Some(deadline)) => Instant::now() < deadline,
                        None => false,
                    };
                    if !waiting {
                        return Err(busy_error(path, target, mode));
                    }
                    thread::sleep(POLL_INTERVAL);
                }
                Err(error) => return Err(error),
            }
        };

        // The lock is ours. Anything already in the file is a record the previous
        // owner never got to clear, so verify who it was before overwriting it.
        let takeover = read_owner_from(&file).map(|previous| {
            let reason = classify_stale(&previous);
            StaleTakeover { previous, reason }
        });

        let owner = LeaseOwner::new(invocation_id(), command, target);
        write_owner(&file, path, &owner)?;

        Ok(TargetLease {
            file: Some(file),
            path: path.to_owned(),
            owner: Some(owner),
            takeover,
            read_only: false,
        })
    }

    /// The invocation holding this lease, or in read-only mode the invocation
    /// observed to hold it (`None` when the target was unowned).
    pub fn owner(&self) -> Option<&LeaseOwner> {
        self.owner.as_ref()
    }

    /// The invocation id to stamp into logs and completion records.
    pub fn invocation(&self) -> Option<&str> {
        self.owner.as_ref().map(|owner| owner.invocation.as_str())
    }

    /// The previous owner this lease displaced, if it displaced one.
    pub fn takeover(&self) -> Option<&StaleTakeover> {
        self.takeover.as_ref()
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn path(&self) -> &Utf8Path {
        &self.path
    }

    /// Release the lease and clear the record.
    ///
    /// Dropping does the same, minus the clearing; the explicit call is what a
    /// clean exit uses so the next run finds no stale record to reason about.
    ///
    /// The file itself is deliberately *not* unlinked. On Unix the lock lives on
    /// the inode, not the path, so a release that dropped the lock and then
    /// removed the file would leave a window in which a second writer acquires
    /// the doomed inode, the unlink lands, and a third writer creates a fresh
    /// file at the same path and acquires that — two live owners of one target,
    /// neither able to see the other. Truncating in place keeps one inode behind
    /// the path for the target's whole life, and an empty record already reads
    /// as "no previous owner" ([`read_owner_from`]).
    pub fn release(mut self) {
        if let Some(file) = self.file.take() {
            // Clear the record *before* dropping: the lock must still be held
            // while the file is modified, or the truncation can land on top of
            // the next owner's freshly written record.
            let _ = file.set_len(0);
            let _ = file.sync_all();
            drop(file);
        }
    }
}

/// Build the busy error, naming the holder when the record can be read.
fn busy_error(path: &Utf8Path, target: &str, mode: LeaseMode) -> Error {
    let held_by = read_owner(path)
        .map(|owner| format!(" held by {}", owner.describe()))
        .unwrap_or_default();
    let waited = match mode {
        LeaseMode::Wait(timeout) if timeout != Duration::MAX => {
            format!(" after waiting {}s", timeout.as_secs())
        }
        _ => String::new(),
    };
    Error::coded(
        TARGET_BUSY_CODE,
        Category::Precondition,
        format!("target '{target}' is being written by another invocation{held_by}{waited}"),
    )
    .with_hint(
        "wait for it to finish, or choose a policy explicitly: `--on-busy wait` (with \
         `--busy-timeout <secs>`) to queue behind it, or `--on-busy read-only` to proceed without \
         writing the target",
    )
}

/// Decide what a leftover record means, now that the lock is ours.
fn classify_stale(previous: &LeaseOwner) -> StaleReason {
    if previous.host != host_name() {
        return StaleReason::ForeignHost;
    }
    // We hold the lock, so this PID cannot be the recorded owner still running:
    // if something answers to it, the number was reused.
    if process_is_live(previous.pid) {
        return StaleReason::PidReused;
    }
    StaleReason::OwnerExited
}

/// Read the owner record without taking the lock (a waiter naming the holder).
fn read_owner(path: &Utf8Path) -> Option<LeaseOwner> {
    let file = open_shared_read(path).ok()?;
    read_owner_from(&file)
}

fn read_owner_from(file: &File) -> Option<LeaseOwner> {
    let mut file = file;
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut body = String::new();
    file.read_to_string(&mut body).ok()?;
    if body.trim().is_empty() {
        return None;
    }
    // A record we cannot parse is a record we cannot verify. Treat it as absent
    // rather than failing the build: the lock, not the JSON, is the exclusion.
    serde_json::from_str(&body).ok()
}

fn write_owner(file: &File, path: &Utf8Path, owner: &LeaseOwner) -> Result<()> {
    let body = serde_json::to_string_pretty(owner)
        .map_err(|error| Error::parse(path.to_string(), anyhow::Error::new(error)))?;
    let mut file = file;
    file.set_len(0)
        .and_then(|()| file.seek(SeekFrom::Start(0)))
        .and_then(|_| file.write_all(format!("{body}\n").as_bytes()))
        .and_then(|()| file.flush())
        .map_err(|error| Error::io(path.to_string(), error))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// An id unique to this invocation, in the same portable alphabet the renderer
/// producer sessions use so the two can be correlated.
fn invocation_id() -> String {
    let mut hasher = RandomState::new().build_hasher();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    hasher.write_u128(nanos);
    hasher.write_u32(std::process::id());
    format!("{:016x}", hasher.finish())
}

// ---------------------------------------------------------------------------
// Platform: exclusive acquisition, shared reads, and process liveness.
// ---------------------------------------------------------------------------

/// Try to take the lock. `Ok(None)` means another writer holds it.
#[cfg(unix)]
fn try_lock(path: &Utf8Path) -> Result<Option<File>> {
    use std::os::unix::io::AsRawFd;

    const LOCK_EX: i32 = 2;
    const LOCK_NB: i32 = 4;

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.as_std_path())
        .map_err(|error| Error::io(path.to_string(), error))?;

    // SAFETY: `fd` is owned by `file` and outlives the call.
    let rc = unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) };
    if rc == 0 {
        return Ok(Some(file));
    }
    let error = std::io::Error::last_os_error();
    match error.kind() {
        std::io::ErrorKind::WouldBlock => Ok(None),
        _ => Err(Error::io(path.to_string(), error)),
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn flock(fd: std::os::raw::c_int, operation: std::os::raw::c_int) -> std::os::raw::c_int;
    fn gethostname(name: *mut std::os::raw::c_char, len: usize) -> std::os::raw::c_int;
    fn kill(pid: std::os::raw::c_int, sig: std::os::raw::c_int) -> std::os::raw::c_int;
}

#[cfg(windows)]
fn try_lock(path: &Utf8Path) -> Result<Option<File>> {
    use std::os::windows::fs::OpenOptionsExt;

    /// Readers may still open the record; writers may not.
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;

    match OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(FILE_SHARE_READ)
        .open(path.as_std_path())
    {
        Ok(file) => Ok(Some(file)),
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(ERROR_SHARING_VIOLATION) | Some(ERROR_LOCK_VIOLATION)
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(Error::io(path.to_string(), error)),
    }
}

/// Open the record for reading while someone else holds the lease.
#[cfg(unix)]
fn open_shared_read(path: &Utf8Path) -> std::io::Result<File> {
    File::open(path.as_std_path())
}

#[cfg(windows)]
fn open_shared_read(path: &Utf8Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;

    // Ask for read access only, and admit the holder's read+write access, or the
    // open is refused for conflicting with the very handle we want to read.
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(path.as_std_path())
}

#[cfg(unix)]
fn host_name() -> String {
    let mut buf = vec![0u8; 256];
    // SAFETY: `buf` is a live allocation of the length passed.
    let rc = unsafe { gethostname(buf.as_mut_ptr() as *mut std::os::raw::c_char, buf.len()) };
    if rc != 0 {
        return "unknown".into();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

#[cfg(windows)]
fn host_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown".into())
}

#[cfg(not(any(unix, windows)))]
fn host_name() -> String {
    "unknown".into()
}

/// Whether any process currently answers to `pid`.
#[cfg(unix)]
fn process_is_live(pid: u32) -> bool {
    // Signal 0 performs the permission and existence checks without delivering
    // anything. EPERM means it exists and is not ours — still alive.
    // SAFETY: no memory is touched; the call only inspects process tables.
    let rc = unsafe { kill(pid as std::os::raw::c_int, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().kind() == std::io::ErrorKind::PermissionDenied
}

#[cfg(windows)]
fn process_is_live(pid: u32) -> bool {
    use std::process::{Command, Stdio};

    // This runs only on the stale-recovery path, where a process spawn is cheap
    // relative to the build about to start — the same reason the CLI reaches for
    // `taskkill` rather than binding the Win32 process APIs.
    let Ok(output) = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        // Without an answer, assume the PID is gone: we already hold the lock,
        // and claiming reuse we cannot demonstrate would be the louder lie.
        return false;
    };
    // A miss prints an INFO banner rather than failing, so match the PID itself.
    String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
}

#[cfg(not(any(unix, windows)))]
fn process_is_live(_pid: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PID no live process can carry, for exercising the dead-owner path.
    ///
    /// It stays positive as a `c_int` — a negative value would make `kill`
    /// address a process *group* — while sitting far above any platform's
    /// `pid_max`, and it is not a multiple of 4, which Windows PIDs always are.
    /// PID 0 would not do: on Windows it is the System Idle Process, which is
    /// very much alive.
    const UNASSIGNABLE_PID: u32 = 0x7FFF_FFFE;

    fn scratch(tag: &str) -> Utf8PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ost-lease-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        Utf8PathBuf::from_path_buf(dir).unwrap()
    }

    #[test]
    fn acquire_records_the_owning_invocation() {
        let dir = scratch("owner");
        let path = dir.join(TARGET_LEASE_FILE);
        let lease = TargetLease::acquire(&path, "cy2026-linux", "ost build", LeaseMode::Fail)
            .expect("lease is free");

        let owner = lease.owner().expect("an owner");
        assert_eq!(owner.target, "cy2026-linux");
        assert_eq!(owner.command, "ost build");
        assert_eq!(owner.pid, std::process::id());
        assert!(!owner.invocation.is_empty());
        // The record is readable while held, so a waiter can name the holder.
        let observed = read_owner(&path).expect("record is readable while held");
        assert_eq!(observed.invocation, owner.invocation);

        lease.release();
        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    /// The exclusion itself: a second writer must not get in.
    #[test]
    fn a_second_writer_fails_busy_and_names_the_holder() {
        let dir = scratch("busy");
        let path = dir.join(TARGET_LEASE_FILE);
        let held = TargetLease::acquire(&path, "target", "ost build", LeaseMode::Fail)
            .expect("first writer wins");

        let error = TargetLease::acquire(&path, "target", "ost test", LeaseMode::Fail)
            .expect_err("second writer is refused");
        assert_eq!(error.code(), TARGET_BUSY_CODE);
        assert_eq!(error.exit_code(), Category::Precondition.exit_code());
        // Naming the holder is what makes the failure actionable.
        let invocation = held.invocation().expect("an invocation");
        assert!(
            error.to_string().contains(invocation),
            "busy error should name the holder: {error}"
        );

        held.release();
        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    /// `wait` must still fail busy once its timeout elapses, rather than hang.
    #[test]
    fn wait_gives_up_at_the_timeout() {
        let dir = scratch("wait");
        let path = dir.join(TARGET_LEASE_FILE);
        let held = TargetLease::acquire(&path, "target", "ost build", LeaseMode::Fail)
            .expect("first writer wins");

        let started = Instant::now();
        let error = TargetLease::acquire(
            &path,
            "target",
            "ost build",
            LeaseMode::Wait(Duration::from_millis(400)),
        )
        .expect_err("waiting writer eventually gives up");
        assert_eq!(error.code(), TARGET_BUSY_CODE);
        assert!(started.elapsed() >= Duration::from_millis(400));

        held.release();
        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    /// Read-only attaches to a held target without contending for it.
    #[test]
    fn read_only_attaches_and_reports_the_holder() {
        let dir = scratch("readonly");
        let path = dir.join(TARGET_LEASE_FILE);
        let held = TargetLease::acquire(&path, "target", "ost build", LeaseMode::Fail)
            .expect("first writer wins");

        let attached = TargetLease::acquire(&path, "target", "ost validate", LeaseMode::ReadOnly)
            .expect("read-only never contends");
        assert!(attached.is_read_only());
        assert_eq!(
            attached.owner().map(|o| o.invocation.clone()),
            held.owner().map(|o| o.invocation.clone())
        );

        held.release();
        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    /// Releasing must actually free the target for the next writer.
    #[test]
    fn release_frees_the_target() {
        let dir = scratch("release");
        let path = dir.join(TARGET_LEASE_FILE);
        let first = TargetLease::acquire(&path, "target", "ost build", LeaseMode::Fail).unwrap();
        first.release();

        // The path stays, holding one inode for the target's whole life: on Unix
        // the lock is on the inode, so unlinking it would let a later writer
        // create a second file at the same path and acquire a lock that excludes
        // nobody. What a release clears is the record, not the file.
        assert!(
            path.as_std_path().exists(),
            "release must not unlink the lease file"
        );
        assert!(
            std::fs::read_to_string(path.as_std_path())
                .unwrap()
                .trim()
                .is_empty(),
            "release must clear the owner record"
        );

        let second = TargetLease::acquire(&path, "target", "ost build", LeaseMode::Fail)
            .expect("lease is free again");
        // A cleanly released lease leaves nothing to take over.
        assert!(second.takeover().is_none());
        second.release();
        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    /// An owner that died leaves its record behind; the next writer inherits the
    /// target and says so instead of finding a wedged lock.
    #[test]
    fn stale_record_from_a_dead_owner_is_taken_over() {
        let dir = scratch("stale");
        let path = dir.join(TARGET_LEASE_FILE);

        let dead = LeaseOwner {
            schema: TARGET_LEASE_SCHEMA.into(),
            invocation: "deadbeefdeadbeef".into(),
            command: "ost build".into(),
            target: "target".into(),
            pid: UNASSIGNABLE_PID,
            host: host_name(),
            acquired_unix: 1,
        };
        std::fs::write(
            path.as_std_path(),
            serde_json::to_string_pretty(&dead).unwrap(),
        )
        .unwrap();

        let lease = TargetLease::acquire(&path, "target", "ost build", LeaseMode::Fail)
            .expect("a dead owner does not wedge the target");
        let takeover = lease.takeover().expect("the stale record is reported");
        assert_eq!(takeover.previous.invocation, "deadbeefdeadbeef");
        assert_eq!(takeover.reason, StaleReason::OwnerExited);
        // The new owner replaced the record.
        assert_ne!(lease.invocation(), Some("deadbeefdeadbeef"));

        lease.release();
        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    /// A record from another host cannot be verified here, and must not be
    /// quietly treated as an ordinary exited owner.
    #[test]
    fn foreign_host_record_is_classified_as_unverifiable() {
        let owner = LeaseOwner {
            schema: TARGET_LEASE_SCHEMA.into(),
            invocation: "abc".into(),
            command: "ost build".into(),
            target: "target".into(),
            // This process is live, so only the host check can produce the
            // foreign verdict — proving host is tested before liveness.
            pid: std::process::id(),
            host: format!("not-{}", host_name()),
            acquired_unix: 1,
        };
        assert_eq!(classify_stale(&owner), StaleReason::ForeignHost);
    }

    /// A live PID that does not hold the lock means the number was recycled.
    #[test]
    fn live_pid_without_the_lock_is_reported_as_reuse() {
        let owner = LeaseOwner {
            schema: TARGET_LEASE_SCHEMA.into(),
            invocation: "abc".into(),
            command: "ost build".into(),
            target: "target".into(),
            pid: std::process::id(),
            host: host_name(),
            acquired_unix: 1,
        };
        assert_eq!(classify_stale(&owner), StaleReason::PidReused);
    }

    /// A truncated or hand-edited record must not fail the build: the lock is
    /// the exclusion, the JSON is only provenance.
    #[test]
    fn unparseable_record_is_treated_as_absent() {
        let dir = scratch("garbage");
        let path = dir.join(TARGET_LEASE_FILE);
        std::fs::write(path.as_std_path(), b"{ not json").unwrap();

        let lease = TargetLease::acquire(&path, "target", "ost build", LeaseMode::Fail)
            .expect("a corrupt record does not wedge the target");
        assert!(lease.takeover().is_none());
        assert!(lease.owner().is_some());

        lease.release();
        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    #[test]
    fn busy_policy_parses_its_cli_spellings() {
        assert_eq!(LeaseMode::parse("fail", 0).unwrap(), LeaseMode::Fail);
        assert_eq!(
            LeaseMode::parse("read-only", 0).unwrap(),
            LeaseMode::ReadOnly
        );
        assert_eq!(
            LeaseMode::parse("wait", 30).unwrap(),
            LeaseMode::Wait(Duration::from_secs(30))
        );
        // `wait` with no timeout waits indefinitely rather than not at all.
        assert_eq!(
            LeaseMode::parse("wait", 0).unwrap(),
            LeaseMode::Wait(Duration::MAX)
        );
        assert!(LeaseMode::parse("maybe", 0).is_err());
    }
}
