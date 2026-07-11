// SPDX-License-Identifier: Apache-2.0
//! The local content-addressed artifact store + registry index (Phase 6 MVP).
//!
//! Layout under `~/.ost/artifacts/` (§10.4):
//!
//! ```text
//! artifacts/
//!   index.json                    # {schema, artifacts:[ArtifactRecord]} sorted by digest
//!   objects/sha256/<hex>/         # one directory per artifact, keyed by digest
//!     record.json                 # the registry's identity record
//!     manifest.json               # the producer manifest, byte-for-byte
//!     <name>-<version>-<target>.tar.zst
//!     SHA256SUMS
//! ```
//!
//! The digest is the identity: importing the same bytes twice is a no-op, and
//! every read path (`export`, `verify`, `RuntimeSource::Artifact`) addresses an
//! artifact by digest, never by mutable name. The index is a convenience for
//! `list`/prefix resolution and can always be rebuilt from the object dirs.

use std::fs::File;
use std::time::{SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use ost_core::paths::Store;
use ost_core::{digest, fs::write_atomic, Category, Error, Result};

use crate::record::{
    manifest_files, ArtifactRecord, ArtifactSource, MANIFEST_FILE, RECORD_FILE, RECORD_SCHEMA,
};

/// Filename of the registry index at the store root.
pub const INDEX_FILE: &str = "index.json";

/// The registry index: every known artifact record, sorted by digest so the
/// serialized index is deterministic.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Index {
    pub schema: u32,
    pub artifacts: Vec<ArtifactRecord>,
}

/// Outcome of an import: the record plus whether the bytes were already stored.
#[derive(Debug, Clone)]
pub struct ImportOutcome {
    pub record: ArtifactRecord,
    pub already_present: bool,
}

/// Integrity verification result for one stored artifact.
#[derive(Debug, Clone, Serialize)]
pub struct VerifyReport {
    pub digest: String,
    /// The recomputed archive digest matches the record.
    pub archive_digest_ok: bool,
    /// Files whose archived bytes hash to the producer manifest's value.
    pub files_matched: u64,
    /// Files present in both but hashing differently (path list).
    pub files_mismatched: Vec<String>,
    /// Files the manifest lists but the archive lacks.
    pub files_missing: Vec<String>,
    /// Files the archive carries but the manifest does not list.
    pub files_extra: Vec<String>,
}

impl VerifyReport {
    pub fn passed(&self) -> bool {
        self.archive_digest_ok
            && self.files_mismatched.is_empty()
            && self.files_missing.is_empty()
            && self.files_extra.is_empty()
    }
}

/// Handle on the local artifact store.
pub struct ArtifactStore {
    root: Utf8PathBuf,
}

impl ArtifactStore {
    /// The store under the user store root (`$OST_HOME`-aware).
    pub fn discover() -> ArtifactStore {
        ArtifactStore {
            root: Store::discover().artifacts(),
        }
    }

    /// A store rooted at an explicit path (tests, CI handoff dirs).
    pub fn at(root: Utf8PathBuf) -> ArtifactStore {
        ArtifactStore { root }
    }

    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    /// Object directory for a digest (bare hex).
    pub fn object_dir(&self, digest_hex: &str) -> Utf8PathBuf {
        self.root.join("objects").join("sha256").join(digest_hex)
    }

    /// Absolute path of a stored artifact's archive.
    pub fn archive_path(&self, record: &ArtifactRecord) -> Utf8PathBuf {
        self.object_dir(record.digest_hex()).join(&record.archive)
    }

    /// The stored producer manifest for a record, parsed.
    pub fn producer_manifest(&self, record: &ArtifactRecord) -> Result<serde_json::Value> {
        let path = self.object_dir(record.digest_hex()).join(MANIFEST_FILE);
        let bytes =
            std::fs::read(path.as_std_path()).map_err(|e| Error::io(path.to_string(), e))?;
        serde_json::from_slice(&bytes)
            .map_err(|e| Error::parse(path.to_string(), anyhow::Error::new(e)))
    }

    /// Import the producer output at `path` (a dist directory containing
    /// `manifest.json`, or the `manifest.json` itself) into the store.
    ///
    /// The archive is re-hashed on the way in and must match the manifest's
    /// `archive_digest`; a mismatch is a hard validation error, never a warning
    /// — a wrong digest means the bytes are not what the manifest describes.
    pub fn import(&self, path: &Utf8Path, source: ArtifactSource) -> Result<ImportOutcome> {
        let (dist_dir, manifest_path) = locate_manifest(path)?;
        let manifest_bytes = std::fs::read(manifest_path.as_std_path())
            .map_err(|e| Error::io(manifest_path.to_string(), e))?;
        let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| Error::parse(manifest_path.to_string(), anyhow::Error::new(e)))?;

        let created = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let producer = format!("ost {}", env!("CARGO_PKG_VERSION"));
        let record = ArtifactRecord::from_producer_manifest(&manifest, source, created, &producer)?;

        // Re-hash the archive: the manifest's digest is a claim, the bytes are
        // the truth. Import refuses to register bytes it cannot vouch for.
        let archive_src = dist_dir.join(&record.archive);
        let mut f = File::open(archive_src.as_std_path())
            .map_err(|e| Error::io(archive_src.to_string(), e))?;
        let (actual, actual_size) =
            digest::sha256_hex_reader(&mut f).map_err(|e| Error::io(archive_src.to_string(), e))?;
        if actual != record.digest {
            return Err(Error::coded(
                "ARTIFACT_DIGEST_MISMATCH",
                Category::Validation,
                format!(
                    "archive '{}' hashes to {actual} but its manifest records {} — \
                     the artifact was modified after packaging",
                    record.archive, record.digest
                ),
            )
            .with_hint("re-run the package step to produce a consistent archive + manifest"));
        }
        if actual_size != record.archive_size {
            return Err(Error::coded(
                "ARTIFACT_DIGEST_MISMATCH",
                Category::Validation,
                format!(
                    "archive '{}' is {actual_size} bytes but its manifest records {}",
                    record.archive, record.archive_size
                ),
            ));
        }

        let hex = record.digest_hex().to_string();
        let object_dir = self.object_dir(&hex);
        if object_dir.join(RECORD_FILE).as_std_path().is_file() {
            // Same digest ⇒ same bytes: nothing to copy. Keep the existing
            // record (its provenance reflects the first entry) but make sure the
            // index knows it.
            let existing = self.read_record(&object_dir)?;
            self.index_upsert(&existing)?;
            return Ok(ImportOutcome {
                record: existing,
                already_present: true,
            });
        }

        // Stage into a sibling temp dir, then rename into place so a crashed
        // import never leaves a half-populated object directory.
        let staging = self.root.join("objects").join("sha256").join(format!(
            ".tmp-{}-{}",
            &hex[..12],
            std::process::id()
        ));
        if staging.as_std_path().exists() {
            std::fs::remove_dir_all(staging.as_std_path())
                .map_err(|e| Error::io(staging.to_string(), e))?;
        }
        std::fs::create_dir_all(staging.as_std_path())
            .map_err(|e| Error::io(staging.to_string(), e))?;

        let stage_result = (|| -> Result<()> {
            std::fs::copy(
                archive_src.as_std_path(),
                staging.join(&record.archive).as_std_path(),
            )
            .map_err(|e| Error::io(archive_src.to_string(), e))?;
            // The producer manifest is stored byte-for-byte: it is the
            // provenance document, not ours to normalize.
            write_atomic(staging.join(MANIFEST_FILE).as_std_path(), &manifest_bytes)?;
            let bare = &hex;
            write_atomic(
                staging.join("SHA256SUMS").as_std_path(),
                format!("{bare}  {}\n", record.archive).as_bytes(),
            )?;
            let record_json = serde_json::to_string_pretty(&record)
                .map_err(|e| Error::parse("artifact record", anyhow::Error::new(e)))?;
            write_atomic(
                staging.join(RECORD_FILE).as_std_path(),
                format!("{record_json}\n").as_bytes(),
            )?;
            Ok(())
        })();
        if let Err(e) = stage_result {
            let _ = std::fs::remove_dir_all(staging.as_std_path());
            return Err(e);
        }

        match std::fs::rename(staging.as_std_path(), object_dir.as_std_path()) {
            Ok(()) => {}
            Err(e) => {
                let _ = std::fs::remove_dir_all(staging.as_std_path());
                // A concurrent import of the same digest landed first: same
                // bytes, so losing the race is success.
                if !object_dir.join(RECORD_FILE).as_std_path().is_file() {
                    return Err(Error::io(object_dir.to_string(), e));
                }
            }
        }

        self.index_upsert(&record)?;
        Ok(ImportOutcome {
            record,
            already_present: false,
        })
    }

    /// All records in the registry, sorted by digest (empty store ⇒ empty list).
    pub fn list(&self) -> Result<Vec<ArtifactRecord>> {
        Ok(self.read_index()?.artifacts)
    }

    /// Resolve a digest reference — `sha256:<hex>`, bare hex, or a unique hex
    /// prefix of at least 6 chars — to its record.
    pub fn resolve(&self, digest_ref: &str) -> Result<ArtifactRecord> {
        let needle = digest_ref.strip_prefix("sha256:").unwrap_or(digest_ref);
        if needle.len() < 6 || !needle.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::usage(format!(
                "'{digest_ref}' is not a digest reference (expected sha256:<hex> or a hex prefix of >= 6 chars)"
            )));
        }
        let needle = needle.to_ascii_lowercase();

        let index = self.read_index()?;
        let mut matches = index
            .artifacts
            .into_iter()
            .filter(|r| r.digest_hex().starts_with(&needle));
        match (matches.next(), matches.next()) {
            (Some(one), None) => Ok(one),
            (Some(a), Some(b)) => Err(Error::usage(format!(
                "digest prefix '{digest_ref}' is ambiguous (matches at least {} and {})",
                a.short_digest(),
                b.short_digest()
            ))),
            (None, _) => Err(Error::coded(
                "ARTIFACT_NOT_FOUND",
                Category::Precondition,
                format!("no artifact matches '{digest_ref}' in the local registry"),
            )
            .with_hint("run `ost artifact list` to see what is stored")),
        }
    }

    /// Copy an artifact's files (archive, producer manifest, SHA256SUMS,
    /// record) into `dest`, re-verifying the archive digest on the way out.
    /// Returns the record and the paths written.
    pub fn export(
        &self,
        digest_ref: &str,
        dest: &Utf8Path,
    ) -> Result<(ArtifactRecord, Vec<Utf8PathBuf>)> {
        let record = self.resolve(digest_ref)?;
        let object_dir = self.object_dir(record.digest_hex());

        std::fs::create_dir_all(dest.as_std_path()).map_err(|e| Error::io(dest.to_string(), e))?;

        let mut written = Vec::new();
        for name in [
            record.archive.as_str(),
            MANIFEST_FILE,
            "SHA256SUMS",
            RECORD_FILE,
        ] {
            let src = object_dir.join(name);
            let dst = dest.join(name);
            if dst.as_std_path().exists() {
                return Err(Error::usage(format!(
                    "refusing to overwrite existing '{dst}' — export into an empty directory"
                )));
            }
            std::fs::copy(src.as_std_path(), dst.as_std_path())
                .map_err(|e| Error::io(src.to_string(), e))?;
            written.push(dst);
        }

        // Verify the exported archive against the record: a store corrupted at
        // rest must not silently propagate into a CI handoff.
        let exported = dest.join(&record.archive);
        let mut f =
            File::open(exported.as_std_path()).map_err(|e| Error::io(exported.to_string(), e))?;
        let (actual, _) =
            digest::sha256_hex_reader(&mut f).map_err(|e| Error::io(exported.to_string(), e))?;
        if actual != record.digest {
            return Err(Error::coded(
                "ARTIFACT_DIGEST_MISMATCH",
                Category::Validation,
                format!(
                    "stored archive for {} hashes to {actual} — the local store is corrupted",
                    record.short_digest()
                ),
            )
            .with_hint("re-import the artifact from its original producer output"));
        }

        Ok((record, written))
    }

    /// Extract a stored artifact's archive into `dest`, re-verifying the
    /// archive digest before trusting the bytes. Returns the record.
    ///
    /// This is the "use" edge of the registry: a runtime fetch or a CI job
    /// unpacking a plugin bundle under test. Extraction requires an empty
    /// destination and refuses entries that would escape it (tar unpack
    /// sanitization).
    pub fn extract(&self, digest_ref: &str, dest: &Utf8Path) -> Result<ArtifactRecord> {
        let record = self.resolve(digest_ref)?;
        let archive = self.archive_path(&record);
        extract_archive(&archive, &record.digest, dest)?;
        Ok(record)
    }

    /// Verify a stored artifact: recompute the archive digest, then hash every
    /// tar entry and compare it against the producer manifest's `files` list.
    pub fn verify(&self, digest_ref: &str) -> Result<VerifyReport> {
        let record = self.resolve(digest_ref)?;
        let object_dir = self.object_dir(record.digest_hex());

        let archive = object_dir.join(&record.archive);
        let mut f =
            File::open(archive.as_std_path()).map_err(|e| Error::io(archive.to_string(), e))?;
        let (actual, _) =
            digest::sha256_hex_reader(&mut f).map_err(|e| Error::io(archive.to_string(), e))?;
        let archive_digest_ok = actual == record.digest;
        if !archive_digest_ok {
            // The bytes are not the recorded bytes; decoding them as tar.zst
            // would fail (or worse, "succeed") on corrupted input, so the
            // per-file comparison is meaningless — report the digest failure.
            return Ok(VerifyReport {
                digest: record.digest,
                archive_digest_ok: false,
                files_matched: 0,
                files_mismatched: Vec::new(),
                files_missing: Vec::new(),
                files_extra: Vec::new(),
            });
        }

        let manifest_path = object_dir.join(MANIFEST_FILE);
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(manifest_path.as_std_path())
                .map_err(|e| Error::io(manifest_path.to_string(), e))?,
        )
        .map_err(|e| Error::parse(manifest_path.to_string(), anyhow::Error::new(e)))?;
        let expected = manifest_files(&manifest)?;

        // Hash each archived entry. Even when the archive digest already
        // matches, this proves the *manifest's* per-file claims — the digest
        // covers the bytes, the file list is what consumers trust per-file.
        let walk = walk_archive(&archive)?;
        let comparison = compare_archive_files(&walk.files, &expected);

        Ok(VerifyReport {
            digest: record.digest,
            archive_digest_ok,
            files_matched: comparison.matched,
            files_mismatched: comparison.mismatched,
            files_missing: comparison.missing,
            files_extra: comparison.extra,
        })
    }

    fn read_record(&self, object_dir: &Utf8Path) -> Result<ArtifactRecord> {
        let path = object_dir.join(RECORD_FILE);
        let bytes =
            std::fs::read(path.as_std_path()).map_err(|e| Error::io(path.to_string(), e))?;
        serde_json::from_slice(&bytes)
            .map_err(|e| Error::parse(path.to_string(), anyhow::Error::new(e)))
    }

    fn read_index(&self) -> Result<Index> {
        let path = self.root.join(INDEX_FILE);
        match std::fs::read(path.as_std_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| Error::parse(path.to_string(), anyhow::Error::new(e))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Index {
                schema: RECORD_SCHEMA,
                artifacts: Vec::new(),
            }),
            Err(e) => Err(Error::io(path.to_string(), e)),
        }
    }

    /// Insert or replace the record for its digest, keeping the index sorted.
    fn index_upsert(&self, record: &ArtifactRecord) -> Result<()> {
        let mut index = self.read_index()?;
        index.schema = RECORD_SCHEMA;
        index.artifacts.retain(|r| r.digest != record.digest);
        index.artifacts.push(record.clone());
        index.artifacts.sort_by(|a, b| a.digest.cmp(&b.digest));

        std::fs::create_dir_all(self.root.as_std_path())
            .map_err(|e| Error::io(self.root.to_string(), e))?;
        let json = serde_json::to_string_pretty(&index)
            .map_err(|e| Error::parse("artifact index", anyhow::Error::new(e)))?;
        write_atomic(
            self.root.join(INDEX_FILE).as_std_path(),
            format!("{json}\n").as_bytes(),
        )
    }
}

/// One hashed file entry from an artifact archive walk.
#[derive(Debug, Clone)]
pub(crate) struct ArchiveFile {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    /// Set for a symlink entry: its (validated, in-tree) target. `sha256`/`size`
    /// then cover the target string. `None` for a regular file.
    pub link_target: Option<String>,
    /// `true` if the tar entry's mode carries a Unix execute bit — the packed
    /// runnable-tool invariant, checked against the manifest so a runtime whose
    /// tools lost `+x` fails verification, not just at runtime.
    pub executable: bool,
}

/// The result of decoding and hashing every entry of a `tar.zst` archive.
#[derive(Debug, Default)]
pub(crate) struct ArchiveWalk {
    /// Regular files, in archive order, with their content digests.
    pub files: Vec<ArchiveFile>,
    /// Entries that would be unsafe to extract, as `<why>: <path>` strings:
    /// absolute or traversal paths, links, and special file types. The walk
    /// itself never extracts, so listing them is safe.
    pub unsafe_entries: Vec<String>,
}

/// Verify `archive` hashes to `expected_digest`, refuse any entry unsafe to
/// extract, then unpack it into `dest`.
///
/// The shared core of [`ArtifactStore::extract`] and callers that unpack a
/// producer archive not (yet) in the store — e.g. `ost plugin test
/// --from-package`, which extracts a freshly built dist archive to run
/// discovery against the *shipped* layout. `dest` must be absent or an empty
/// directory. Fails with `ARTIFACT_DIGEST_MISMATCH` on a byte mismatch and
/// `ARTIFACT_UNSAFE_ENTRY` on an absolute/escaping symlink, a `..` path, a
/// hardlink, or a special file (harness §SEC-001) — scanned before a single
/// byte is unpacked.
pub fn extract_archive(archive: &Utf8Path, expected_digest: &str, dest: &Utf8Path) -> Result<()> {
    let mut f = File::open(archive.as_std_path()).map_err(|e| Error::io(archive.to_string(), e))?;
    let (actual, _) =
        digest::sha256_hex_reader(&mut f).map_err(|e| Error::io(archive.to_string(), e))?;
    if actual != expected_digest {
        return Err(Error::coded(
            "ARTIFACT_DIGEST_MISMATCH",
            Category::Validation,
            format!("archive '{archive}' hashes to {actual}, expected {expected_digest}"),
        )
        .with_hint("the archive is corrupt or was modified after it was built — repackage it"));
    }

    if dest.as_std_path().exists() {
        if !dest.as_std_path().is_dir() {
            return Err(Error::usage(format!(
                "extract destination '{dest}' exists but is not a directory"
            )));
        }
        let mut entries =
            std::fs::read_dir(dest.as_std_path()).map_err(|e| Error::io(dest.to_string(), e))?;
        if let Some(entry) = entries.next() {
            entry.map_err(|e| Error::io(dest.to_string(), e))?;
            return Err(Error::usage(format!(
                "refusing to extract into non-empty directory '{dest}'"
            )));
        }
    } else {
        std::fs::create_dir_all(dest.as_std_path()).map_err(|e| Error::io(dest.to_string(), e))?;
    }

    // Pre-extraction safety gate: scan the (digest-verified) archive and refuse
    // before unpacking a byte if any entry is unsafe. A local artifact reaches
    // here without the transport verify gate, so extraction enforces it itself.
    let unsafe_entries = scan_unsafe_entries(archive)?;
    if !unsafe_entries.is_empty() {
        return Err(Error::coded(
            "ARTIFACT_UNSAFE_ENTRY",
            Category::Validation,
            format!(
                "archive '{archive}' contains {} entr{} unsafe to extract: {}",
                unsafe_entries.len(),
                if unsafe_entries.len() == 1 {
                    "y"
                } else {
                    "ies"
                },
                unsafe_entries.join("; "),
            ),
        ));
    }

    let file = File::open(archive.as_std_path()).map_err(|e| Error::io(archive.to_string(), e))?;
    let decoder =
        zstd::stream::read::Decoder::new(file).map_err(|e| Error::io(archive.to_string(), e))?;
    let mut tar = tar::Archive::new(decoder);
    tar.unpack(dest.as_std_path())
        .map_err(|e| Error::io(dest.to_string(), e))?;
    Ok(())
}

/// Decode a `tar.zst` archive and hash every regular file, flagging entries
/// that must never be extracted (pre-extraction safety, transport plan §
/// "Verification order on pull").
pub(crate) fn walk_archive(archive: &Utf8Path) -> Result<ArchiveWalk> {
    let file = File::open(archive.as_std_path()).map_err(|e| Error::io(archive.to_string(), e))?;
    let decoder =
        zstd::stream::read::Decoder::new(file).map_err(|e| Error::io(archive.to_string(), e))?;
    let mut tar = tar::Archive::new(decoder);

    let mut walk = ArchiveWalk::default();
    let mut seen_files = std::collections::HashSet::new();
    let entries = tar
        .entries()
        .map_err(|e| Error::io(archive.to_string(), e))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| Error::io(archive.to_string(), e))?;
        let path = entry
            .path()
            .map_err(|e| Error::io(archive.to_string(), e))?
            .to_string_lossy()
            .replace('\\', "/");

        let entry_type = entry.header().entry_type();
        let executable = entry.header().mode().unwrap_or(0) & 0o111 != 0;
        let link_target = read_link_target(&entry, entry_type, archive)?;
        match classify_entry(&path, entry_type, link_target.as_deref()) {
            EntryClass::Unsafe(reason) => walk.unsafe_entries.push(reason),
            EntryClass::Directory => {}
            EntryClass::Regular => {
                if !seen_files.insert(path.clone()) {
                    walk.unsafe_entries
                        .push(format!("duplicate file path: {path}"));
                    continue;
                }
                let (sha, size) = digest::sha256_hex_reader(&mut entry)
                    .map_err(|e| Error::io(archive.to_string(), e))?;
                walk.files.push(ArchiveFile {
                    path,
                    sha256: sha,
                    size,
                    link_target: None,
                    executable,
                });
            }
            EntryClass::Symlink(target) => {
                if !seen_files.insert(path.clone()) {
                    walk.unsafe_entries
                        .push(format!("duplicate file path: {path}"));
                    continue;
                }
                // A symlink carries no bytes; its identity is its target string,
                // hashed so a tampered target is a manifest mismatch.
                walk.files.push(ArchiveFile {
                    path,
                    sha256: digest::sha256_hex(target.as_bytes()),
                    size: target.len() as u64,
                    link_target: Some(target),
                    executable: false,
                });
            }
        }
    }
    Ok(walk)
}

/// Cheaply scan a `tar.zst` for entries unsafe to extract, **without hashing any
/// file contents**. This is the extract-time safety gate: `extract` needs only
/// the safety verdict, not the per-file digests `walk_archive` computes, and
/// re-hashing every byte of a multi-GB SDK (the unpack pass reads it all again)
/// is wasted work. Classification is shared with `walk_archive` via
/// [`classify_entry`], so the pre-extraction gate can never drift from the walk.
pub(crate) fn scan_unsafe_entries(archive: &Utf8Path) -> Result<Vec<String>> {
    let file = File::open(archive.as_std_path()).map_err(|e| Error::io(archive.to_string(), e))?;
    let decoder =
        zstd::stream::read::Decoder::new(file).map_err(|e| Error::io(archive.to_string(), e))?;
    let mut tar = tar::Archive::new(decoder);

    let mut unsafe_entries = Vec::new();
    let mut seen_files = std::collections::HashSet::new();
    let entries = tar
        .entries()
        .map_err(|e| Error::io(archive.to_string(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(archive.to_string(), e))?;
        let path = entry
            .path()
            .map_err(|e| Error::io(archive.to_string(), e))?
            .to_string_lossy()
            .replace('\\', "/");
        let entry_type = entry.header().entry_type();
        let link_target = read_link_target(&entry, entry_type, archive)?;
        match classify_entry(&path, entry_type, link_target.as_deref()) {
            EntryClass::Unsafe(reason) => unsafe_entries.push(reason),
            EntryClass::Directory => {}
            // A file or safe symlink: only the duplicate check matters here; the
            // entry's bytes are never read, so the reader just skips past them.
            EntryClass::Regular | EntryClass::Symlink(_) => {
                if !seen_files.insert(path.clone()) {
                    unsafe_entries.push(format!("duplicate file path: {path}"));
                }
            }
        }
    }
    Ok(unsafe_entries)
}

/// The safety classification of one tar entry, judged purely from its path,
/// type, and (for a symlink) link target — no contents read. Shared by
/// [`walk_archive`] and [`scan_unsafe_entries`] so the two gates cannot disagree.
enum EntryClass {
    /// A regular file, safe to keep/extract.
    Regular,
    /// A directory: carries no manifest entry and is always safe.
    Directory,
    /// A safe symlink, carrying its validated, normalized in-tree target.
    Symlink(String),
    /// Unsafe to extract; the string is a ready-to-report `<why>: <path>` reason.
    Unsafe(String),
}

/// Classify `path`/`entry_type`/`link_target` (see [`EntryClass`]). A shared-
/// library soname chain is a legitimate symlink, but a symlink can redirect a
/// later write outside the destination, so only a *relative, in-tree* target is
/// kept; hardlinks and special files (device/fifo) have no place in an artifact
/// (harness §SEC-001).
fn classify_entry(path: &str, entry_type: tar::EntryType, link_target: Option<&str>) -> EntryClass {
    if let Some(why) = unsafe_entry_path(path) {
        return EntryClass::Unsafe(format!("{why}: {path}"));
    }
    match entry_type {
        tar::EntryType::Regular => EntryClass::Regular,
        tar::EntryType::Directory => EntryClass::Directory,
        tar::EntryType::Symlink => {
            let target = link_target.unwrap_or_default();
            match unsafe_symlink_target(path, target) {
                Some(why) => EntryClass::Unsafe(format!("{why}: {path}")),
                None => EntryClass::Symlink(target.to_string()),
            }
        }
        other => EntryClass::Unsafe(format!("unsupported entry type {other:?}: {path}")),
    }
}

/// The normalized (forward-slashed) target of a symlink entry, or `None` for a
/// non-symlink. Kept separate from [`classify_entry`] so the latter stays a pure
/// function over already-decoded strings.
fn read_link_target<R: std::io::Read>(
    entry: &tar::Entry<'_, R>,
    entry_type: tar::EntryType,
    archive: &Utf8Path,
) -> Result<Option<String>> {
    if entry_type != tar::EntryType::Symlink {
        return Ok(None);
    }
    Ok(Some(
        entry
            .link_name()
            .map_err(|e| Error::io(archive.to_string(), e))?
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default(),
    ))
}

/// Why an archived path must not be extracted, if any.
fn unsafe_entry_path(path: &str) -> Option<&'static str> {
    if path.is_empty() {
        return Some("empty path");
    }
    if path.starts_with('/') {
        return Some("absolute path");
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Some("drive-letter path");
    }
    if path.split('/').any(|c| c == "..") {
        return Some("parent traversal");
    }
    if path.chars().any(char::is_control) {
        return Some("control character in path");
    }
    None
}

/// Why a symlink's target makes it unsafe to extract, if any. Purely lexical: no
/// filesystem access, mirroring [`ost_build::validate_symlink`] on the producer
/// side. `entry_path` is the (already path-validated) archive path of the link.
///
/// Safe means a **relative** target that, resolved against the link's own
/// directory, stays inside the archive root. An absolute target or a `..` that
/// climbs above the root is rejected (harness §SEC-001).
fn unsafe_symlink_target(entry_path: &str, target: &str) -> Option<&'static str> {
    if target.is_empty() {
        return Some("symlink with an empty target");
    }
    if target.chars().any(char::is_control) {
        return Some("symlink with a control character in its target");
    }
    if target.starts_with('/') {
        return Some("symlink with an absolute target");
    }
    let tb = target.as_bytes();
    if tb.len() >= 2 && tb[0].is_ascii_alphabetic() && tb[1] == b':' {
        return Some("symlink with a drive-letter target");
    }
    // Resolve `target` against the link's parent directory; a `..` that pops past
    // the archive root escapes the tree.
    let mut stack: Vec<&str> = entry_path.split('/').filter(|c| !c.is_empty()).collect();
    stack.pop(); // the link's own file name
    for comp in target.split('/').filter(|c| !c.is_empty()) {
        match comp {
            "." => {}
            ".." => {
                if stack.pop().is_none() {
                    return Some("symlink whose target escapes the archive root");
                }
            }
            name => stack.push(name),
        }
    }
    None
}

/// Comparison of hashed archive contents against a producer manifest file list.
#[derive(Debug, Default)]
pub(crate) struct FileComparison {
    pub matched: u64,
    pub mismatched: Vec<String>,
    pub missing: Vec<String>,
    pub extra: Vec<String>,
}

impl FileComparison {
    pub fn passed(&self) -> bool {
        self.mismatched.is_empty() && self.missing.is_empty() && self.extra.is_empty()
    }
}

/// Compare hashed archive entries against the manifest's per-file claims.
pub(crate) fn compare_archive_files(
    actual: &[ArchiveFile],
    expected: &[crate::record::ManifestFile],
) -> FileComparison {
    let mut cmp = FileComparison::default();
    let mut seen_expected = std::collections::HashSet::new();
    for want in expected {
        if !seen_expected.insert(want.path.as_str()) {
            cmp.mismatched.push(want.path.clone());
            continue;
        }
        match actual.iter().find(|f| f.path == want.path) {
            // A symlink and a regular file whose contents equal the target string
            // hash alike; `link_target` keeps their manifest identities distinct.
            // `executable` is part of the identity too — a runtime tool that lost
            // its `+x` bit is a mismatch, not a silent pass.
            Some(f)
                if f.sha256 == want.sha256
                    && f.size == want.size
                    && f.link_target == want.link_target
                    && f.executable == want.executable =>
            {
                cmp.matched += 1
            }
            Some(_) => cmp.mismatched.push(want.path.clone()),
            None => cmp.missing.push(want.path.clone()),
        }
    }
    let mut seen_actual = std::collections::HashSet::new();
    for f in actual {
        if !seen_actual.insert(f.path.as_str()) || !expected.iter().any(|w| w.path == f.path) {
            cmp.extra.push(f.path.clone());
        }
    }
    cmp
}

/// Accept either a dist directory or a direct path to its `manifest.json`.
pub(crate) fn locate_manifest(path: &Utf8Path) -> Result<(Utf8PathBuf, Utf8PathBuf)> {
    if path.as_std_path().is_dir() {
        let manifest = path.join(MANIFEST_FILE);
        if !manifest.as_std_path().is_file() {
            return Err(Error::precondition(format!(
                "'{path}' has no {MANIFEST_FILE} — point at a package output directory \
                 (e.g. dist/plugins/<name>/<version>/<target>/)"
            )));
        }
        Ok((path.to_owned(), manifest))
    } else if path.file_name() == Some(MANIFEST_FILE) {
        let dir = path
            .parent()
            .ok_or_else(|| Error::usage(format!("'{path}' has no parent directory")))?;
        Ok((dir.to_owned(), path.to_owned()))
    } else {
        Err(Error::usage(format!(
            "'{path}' is neither a package output directory nor a {MANIFEST_FILE}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::ArtifactKind;

    /// Build a real dist dir (archive + manifest) with ost-build-shaped output.
    fn make_dist(root: &Utf8Path, name: &str, content: &[u8]) -> Utf8PathBuf {
        let dist = root.join(format!("dist-{name}"));
        std::fs::create_dir_all(dist.as_std_path()).unwrap();

        // A tiny tar.zst with one file, hashed the way pack_dir does.
        let archive_name = format!("{name}-0.1.0-target.tar.zst");
        let archive = dist.join(&archive_name);
        let out = File::create(archive.as_std_path()).unwrap();
        let enc = zstd::stream::write::Encoder::new(out, 3)
            .unwrap()
            .auto_finish();
        let mut tar = tar::Builder::new(enc);
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "lib/payload.bin", content)
            .unwrap();
        tar.finish().unwrap();
        drop(tar);

        let bytes = std::fs::read(archive.as_std_path()).unwrap();
        let manifest = serde_json::json!({
            "schema": 1,
            "kind": "openstrata.plugin-bundle",
            "plugin": { "name": name, "version": "0.1.0", "kind": "usd-fileformat", "license": "Apache-2.0" },
            "target": "cy2026-linux-x86_64-gcc11-py313-usd",
            "archive": archive_name,
            "archive_digest": digest::sha256_hex(&bytes),
            "archive_size": bytes.len(),
            "total_size": content.len(),
            "created_unix": 1_750_000_000,
            "provenance": {
                "profile": "usd",
                "runtime": { "id": "rt", "digest": "sha256:beef" },
                "validation": { "passed": true },
            },
            "files": [
                { "path": "lib/payload.bin", "sha256": digest::sha256_hex(content), "size": content.len() },
            ],
        });
        std::fs::write(
            dist.join(MANIFEST_FILE).as_std_path(),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        dist
    }

    fn tmp_root(tag: &str) -> Utf8PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut d = Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
        d.push(format!("ost-artifact-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(d.as_std_path()).unwrap();
        d
    }

    #[test]
    fn import_list_show_roundtrip() {
        let root = tmp_root("import");
        let store = ArtifactStore::at(root.join("store"));
        let dist = make_dist(&root, "toy", b"plugin bytes");

        let out = store.import(&dist, ArtifactSource::Published).unwrap();
        assert!(!out.already_present);
        assert_eq!(out.record.kind, ArtifactKind::Plugin);
        assert_eq!(out.record.name, "toy");

        // Idempotent by digest.
        let again = store.import(&dist, ArtifactSource::Imported).unwrap();
        assert!(again.already_present);
        // The first entry's provenance wins.
        assert_eq!(again.record.source, ArtifactSource::Published);

        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);

        // Resolution by prefix and by full reference.
        let hex = out.record.digest_hex().to_string();
        assert_eq!(store.resolve(&hex[..8]).unwrap().digest, out.record.digest);
        assert_eq!(
            store.resolve(&out.record.digest).unwrap().digest,
            out.record.digest
        );
        assert!(store.resolve("deadbeef").is_err());

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn import_rejects_a_tampered_archive() {
        let root = tmp_root("tamper");
        let store = ArtifactStore::at(root.join("store"));
        let dist = make_dist(&root, "toy", b"plugin bytes");

        // Flip the archive after the manifest was written.
        let archive = dist.join("toy-0.1.0-target.tar.zst");
        let mut bytes = std::fs::read(archive.as_std_path()).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(archive.as_std_path(), &bytes).unwrap();

        let err = store
            .import(&dist, ArtifactSource::Imported)
            .expect_err("tampered archive must be refused");
        assert_eq!(err.code(), "ARTIFACT_DIGEST_MISMATCH");
        assert!(store.list().unwrap().is_empty(), "nothing was registered");

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn import_rejects_a_pathy_archive_filename() {
        let root = tmp_root("pathy-archive");
        let store = ArtifactStore::at(root.join("store"));
        let dist = make_dist(&root, "toy", b"plugin bytes");
        let manifest_path = dist.join(MANIFEST_FILE);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(manifest_path.as_std_path()).unwrap()).unwrap();
        manifest["archive"] = serde_json::json!("../toy-0.1.0-target.tar.zst");
        std::fs::write(
            manifest_path.as_std_path(),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let err = store
            .import(&dist, ArtifactSource::Imported)
            .expect_err("archive path traversal must be refused before opening files");
        assert_eq!(err.code(), "MANIFEST_INVALID");
        assert!(store.list().unwrap().is_empty(), "nothing was registered");

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn verify_passes_and_catches_store_corruption() {
        let root = tmp_root("verify");
        let store = ArtifactStore::at(root.join("store"));
        let dist = make_dist(&root, "toy", b"plugin bytes");
        let out = store.import(&dist, ArtifactSource::Published).unwrap();

        let report = store.verify(&out.record.digest).unwrap();
        assert!(report.passed(), "fresh import verifies: {report:?}");
        assert_eq!(report.files_matched, 1);

        // Corrupt the stored archive; verify must fail on the digest.
        let stored = store
            .object_dir(out.record.digest_hex())
            .join(&out.record.archive);
        let mut bytes = std::fs::read(stored.as_std_path()).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(stored.as_std_path(), &bytes).unwrap();

        let report = store.verify(&out.record.digest).unwrap();
        assert!(!report.archive_digest_ok);
        assert!(!report.passed());

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn extract_unpacks_verified_bytes_and_refuses_corruption() {
        let root = tmp_root("extract");
        let store = ArtifactStore::at(root.join("store"));
        let dist = make_dist(&root, "toy", b"plugin bytes");
        let out = store.import(&dist, ArtifactSource::Published).unwrap();

        let dest = root.join("unpacked");
        let record = store.extract(&out.record.digest, &dest).unwrap();
        assert_eq!(record.digest, out.record.digest);
        let payload = dest.join("lib/payload.bin");
        assert_eq!(
            std::fs::read(payload.as_std_path()).unwrap(),
            b"plugin bytes"
        );
        let err = store
            .extract(&out.record.digest, &dest)
            .expect_err("extract must refuse to merge into an existing tree");
        assert_eq!(err.code(), "INVALID_ARGUMENT");

        // Corrupt the stored archive: extract must refuse before unpacking.
        let stored = store.archive_path(&out.record);
        let mut bytes = std::fs::read(stored.as_std_path()).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(stored.as_std_path(), &bytes).unwrap();
        let err = store
            .extract(&out.record.digest, &root.join("unpacked2"))
            .expect_err("corrupted store must be refused");
        assert_eq!(err.code(), "ARTIFACT_DIGEST_MISMATCH");

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn push_guards_store_corruption_before_reaching_the_transport() {
        use crate::reference::RemoteReference;
        use crate::transport::file::FileTransport;

        let root = tmp_root("push-guard");
        let store = ArtifactStore::at(root.join("store"));
        let dist = make_dist(&root, "toy", b"plugin bytes");
        let out = store.import(&dist, ArtifactSource::Published).unwrap();
        let dest = RemoteReference::parse("file:///tmp/whatever").unwrap();

        // A clean store passes the re-hash guard and reaches the transport, which
        // (read-only) refuses — proving the guard let it through.
        let err = crate::transport::push(&FileTransport::new(), &store, &out.record.digest, &dest)
            .expect_err("file backend cannot push");
        assert_eq!(err.code(), "ARTIFACT_PUSH_UNSUPPORTED");

        // Corrupt the stored archive; push must refuse before any transport call.
        let stored = store.archive_path(&out.record);
        let mut bytes = std::fs::read(stored.as_std_path()).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(stored.as_std_path(), &bytes).unwrap();
        let err = crate::transport::push(&FileTransport::new(), &store, &out.record.digest, &dest)
            .expect_err("corrupted store must not publish");
        assert_eq!(err.code(), "ARTIFACT_DIGEST_MISMATCH");

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn export_roundtrips_and_refuses_overwrite() {
        let root = tmp_root("export");
        let store = ArtifactStore::at(root.join("store"));
        let dist = make_dist(&root, "toy", b"plugin bytes");
        let out = store.import(&dist, ArtifactSource::Published).unwrap();

        let dest = root.join("handoff");
        let (record, written) = store.export(&out.record.digest, &dest).unwrap();
        assert_eq!(record.digest, out.record.digest);
        assert_eq!(written.len(), 4);
        assert!(dest.join(&record.archive).as_std_path().is_file());
        assert!(dest.join(MANIFEST_FILE).as_std_path().is_file());

        // An exported dist dir is importable again (registry ⇄ CI handoff).
        let store2 = ArtifactStore::at(root.join("store2"));
        let re = store2.import(&dest, ArtifactSource::Imported).unwrap();
        assert_eq!(re.record.digest, record.digest);

        // Second export into the same dir refuses to clobber.
        assert!(store.export(&out.record.digest, &dest).is_err());

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    /// One tar entry to write into a test archive.
    enum Entry<'a> {
        Reg(&'a str, &'a [u8]),
        /// A regular file packed with the execute bit (mode `0o755`).
        RegExec(&'a str, &'a [u8]),
        Sym(&'a str, &'a str),
    }

    /// Build a `tar.zst` at `archive` from `entries`, returning its digest.
    fn write_archive(archive: &Utf8Path, entries: &[Entry]) -> String {
        let out = File::create(archive.as_std_path()).unwrap();
        let enc = zstd::stream::write::Encoder::new(out, 3)
            .unwrap()
            .auto_finish();
        let mut tar = tar::Builder::new(enc);
        for e in entries {
            match e {
                Entry::Reg(path, bytes) => {
                    let mut h = tar::Header::new_gnu();
                    h.set_size(bytes.len() as u64);
                    h.set_mode(0o644);
                    h.set_cksum();
                    tar.append_data(&mut h, path, *bytes).unwrap();
                }
                Entry::RegExec(path, bytes) => {
                    let mut h = tar::Header::new_gnu();
                    h.set_size(bytes.len() as u64);
                    h.set_mode(0o755);
                    h.set_cksum();
                    tar.append_data(&mut h, path, *bytes).unwrap();
                }
                Entry::Sym(path, target) => {
                    let mut h = tar::Header::new_gnu();
                    h.set_entry_type(tar::EntryType::Symlink);
                    h.set_size(0);
                    h.set_mode(0o777);
                    tar.append_link(&mut h, path, target).unwrap();
                }
            }
        }
        tar.finish().unwrap();
        drop(tar);
        digest::sha256_hex(&std::fs::read(archive.as_std_path()).unwrap())
    }

    #[test]
    fn unsafe_symlink_target_classifies_safe_and_escaping() {
        // A relative, in-tree soname link is safe.
        assert!(unsafe_symlink_target("lib/libFoo.so", "libFoo.so.1").is_none());
        assert!(unsafe_symlink_target("lib/a/b.so", "../c/real.so").is_none());
        // Escapes and absolutes are rejected.
        assert!(unsafe_symlink_target("lib/leak", "/etc/hostname").is_some());
        assert!(unsafe_symlink_target("lib/leak", "../../secret").is_some());
        assert!(unsafe_symlink_target("lib/leak", "C:\\secret").is_some());
        assert!(unsafe_symlink_target("lib/leak", "").is_some());
        assert!(unsafe_symlink_target("lib/leak", "a\nb").is_some());
    }

    #[test]
    fn walk_archive_keeps_safe_symlink_and_flags_escaping() {
        let root = tmp_root("walk-symlink");
        let archive = root.join("a.tar.zst");
        write_archive(
            &archive,
            &[
                Entry::Reg("lib/libFoo.so.1.39.4", b"ELF"),
                Entry::Sym("lib/libFoo.so", "libFoo.so.1.39.4"),
                Entry::Sym("lib/leak", "../../../etc/passwd"),
            ],
        );

        let walk = walk_archive(&archive).unwrap();
        // The real lib and the safe link are hashed files; the escaping link is not.
        let link = walk
            .files
            .iter()
            .find(|f| f.path == "lib/libFoo.so")
            .expect("safe symlink is kept");
        assert_eq!(link.link_target.as_deref(), Some("libFoo.so.1.39.4"));
        assert_eq!(link.sha256, digest::sha256_hex(b"libFoo.so.1.39.4"));
        assert!(!walk.files.iter().any(|f| f.path == "lib/leak"));
        assert_eq!(walk.unsafe_entries.len(), 1);
        assert!(
            walk.unsafe_entries[0].contains("lib/leak"),
            "{:?}",
            walk.unsafe_entries
        );

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn compare_flags_symlink_vs_regular_type_confusion() {
        // The archive holds a regular file whose bytes equal a target string; the
        // manifest claims a symlink with that target. Equal sha256/size must NOT
        // pass — `link_target` distinguishes them.
        let target = "libFoo.so.1";
        let actual = vec![ArchiveFile {
            path: "lib/x".into(),
            sha256: digest::sha256_hex(target.as_bytes()),
            size: target.len() as u64,
            link_target: None,
            executable: false,
        }];
        let expected = vec![crate::record::ManifestFile {
            path: "lib/x".into(),
            sha256: digest::sha256_hex(target.as_bytes()),
            size: target.len() as u64,
            link_target: Some(target.into()),
            executable: false,
        }];
        let cmp = compare_archive_files(&actual, &expected);
        assert!(!cmp.passed());
        assert_eq!(cmp.mismatched, vec!["lib/x".to_string()]);
    }

    #[test]
    fn walk_reads_executable_bit_and_verify_flags_a_stripped_tool() {
        let root = tmp_root("walk-exec");
        std::fs::create_dir_all(root.as_std_path()).unwrap();
        let archive = root.join("a.tar.zst");
        write_archive(
            &archive,
            &[
                Entry::RegExec("bin/usdcat", b"#!/bin/sh\n"),
                Entry::Reg("lib/data.txt", b"data"),
            ],
        );

        let walk = walk_archive(&archive).unwrap();
        let tool = walk.files.iter().find(|f| f.path == "bin/usdcat").unwrap();
        assert!(tool.executable, "the 0o755 entry is read as executable");
        let data = walk
            .files
            .iter()
            .find(|f| f.path == "lib/data.txt")
            .unwrap();
        assert!(!data.executable, "the 0o644 entry is not executable");

        // A manifest that claims the tool is NOT executable must not verify
        // against an archive whose tool is — the runnable invariant is part of
        // the per-file identity, not just content bytes.
        let expected = vec![
            crate::record::ManifestFile {
                path: "bin/usdcat".into(),
                sha256: tool.sha256.clone(),
                size: tool.size,
                link_target: None,
                executable: false,
            },
            crate::record::ManifestFile {
                path: "lib/data.txt".into(),
                sha256: data.sha256.clone(),
                size: data.size,
                link_target: None,
                executable: false,
            },
        ];
        let cmp = compare_archive_files(&walk.files, &expected);
        assert!(!cmp.passed());
        assert_eq!(cmp.mismatched, vec!["bin/usdcat".to_string()]);

        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[cfg(unix)]
    #[test]
    fn extract_recreates_safe_symlink_and_refuses_escaping() {
        // A dist dir whose archive carries a real lib + a safe soname symlink.
        let root = tmp_root("extract-symlink");
        let dist = root.join("dist");
        std::fs::create_dir_all(dist.as_std_path()).unwrap();
        let archive_name = "sym-0.1.0-target.tar.zst";
        let archive = dist.join(archive_name);
        let target = "libFoo.so.1.39.4";
        let dig = write_archive(
            &archive,
            &[
                Entry::Reg("lib/libFoo.so.1.39.4", b"ELF"),
                Entry::Sym("lib/libFoo.so", target),
            ],
        );
        let archive_bytes = std::fs::read(archive.as_std_path()).unwrap();
        let manifest = serde_json::json!({
            "schema": 1,
            "kind": "openstrata.plugin-bundle",
            "plugin": { "name": "sym", "version": "0.1.0", "kind": "usd-fileformat", "license": "Apache-2.0" },
            "target": "cy2026-linux-x86_64-gcc11-py313-usd",
            "archive": archive_name,
            "archive_digest": dig,
            "archive_size": archive_bytes.len(),
            "total_size": 3 + target.len(),
            "created_unix": 1_750_000_000,
            "provenance": { "profile": "usd", "runtime": { "id": "rt", "digest": "sha256:beef" }, "validation": { "passed": true } },
            "files": [
                { "path": "lib/libFoo.so.1.39.4", "sha256": digest::sha256_hex(b"ELF"), "size": 3 },
                { "path": "lib/libFoo.so", "sha256": digest::sha256_hex(target.as_bytes()), "size": target.len(), "link_target": target },
            ],
        });
        std::fs::write(
            dist.join(MANIFEST_FILE).as_std_path(),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let store = ArtifactStore::at(root.join("store"));
        let out = store.import(&dist, ArtifactSource::Published).unwrap();
        // verify agrees the packed symlink matches its manifest claim.
        assert!(store.verify(&out.record.digest).unwrap().passed());

        // extract recreates the symlink as a symlink, not a copied file.
        let dest = root.join("unpacked");
        store.extract(&out.record.digest, &dest).unwrap();
        let link = dest.join("lib/libFoo.so");
        let meta = std::fs::symlink_metadata(link.as_std_path()).unwrap();
        assert!(
            meta.file_type().is_symlink(),
            "extract must restore the link"
        );
        assert_eq!(
            std::fs::read_link(link.as_std_path()).unwrap(),
            std::path::Path::new("libFoo.so.1.39.4")
        );

        // An escaping symlink smuggled into the stored archive is refused before a
        // byte is unpacked, even though the archive digest matches its manifest.
        let root2 = tmp_root("extract-escape");
        let dist2 = root2.join("dist");
        std::fs::create_dir_all(dist2.as_std_path()).unwrap();
        let archive2 = dist2.join(archive_name);
        let target = "../../../etc/passwd";
        let dig = write_archive(
            &archive2,
            &[Entry::Reg("lib/real", b"x"), Entry::Sym("lib/leak", target)],
        );
        let manifest2 = serde_json::json!({
            "schema": 1,
            "kind": "openstrata.plugin-bundle",
            "plugin": { "name": "sym", "version": "0.1.0", "kind": "usd-fileformat", "license": "Apache-2.0" },
            "target": "cy2026-linux-x86_64-gcc11-py313-usd",
            "archive": archive_name,
            "archive_digest": digest::sha256_hex(&std::fs::read(archive2.as_std_path()).unwrap()),
            "archive_size": std::fs::metadata(archive2.as_std_path()).unwrap().len(),
            "total_size": 1,
            "created_unix": 1_750_000_000,
            "provenance": { "profile": "usd", "runtime": { "id": "rt", "digest": "sha256:beef" }, "validation": { "passed": true } },
            "files": [ { "path": "lib/real", "sha256": digest::sha256_hex(b"x"), "size": 1 } ],
        });
        assert_eq!(dig, manifest2["archive_digest"].as_str().unwrap());
        std::fs::write(
            dist2.join(MANIFEST_FILE).as_std_path(),
            serde_json::to_string_pretty(&manifest2).unwrap(),
        )
        .unwrap();
        let store2 = ArtifactStore::at(root2.join("store"));
        let out2 = store2.import(&dist2, ArtifactSource::Imported).unwrap();
        let err = store2
            .extract(&out2.record.digest, &root2.join("unpacked"))
            .expect_err("an escaping symlink must be refused before unpacking");
        assert!(err.to_string().contains("unsafe to extract"), "{err}");
        assert!(!root2.join("unpacked/lib").as_std_path().exists());

        std::fs::remove_dir_all(root.as_std_path()).ok();
        std::fs::remove_dir_all(root2.as_std_path()).ok();
    }
}
