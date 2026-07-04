// SPDX-License-Identifier: Apache-2.0
//! `ost artifact` — local artifact registry operations (Phase 6 MVP).
//!
//! - `import` register a package output (dist dir / manifest.json) by digest.
//! - `list`   show what the registry holds.
//! - `show`   full identity record for one digest.
//! - `verify` recompute the archive digest and re-hash every archived file
//!   against the producer manifest.
//! - `export` copy an artifact (archive + manifest + checksums + record) out
//!   for CI handoff; the exported directory is re-importable.
//!
//! Artifacts are addressed by digest (full `sha256:<hex>` or a unique prefix),
//! never by mutable name — the registry is the source of truth CI pins.

use camino::Utf8PathBuf;
use clap::Subcommand;

use ost_artifact::{ArtifactKind, ArtifactRecord, ArtifactSource, ArtifactStore, VerifyReport};
use ost_core::{Error, Result};

use crate::output::{self, Format};

#[derive(Debug, Subcommand)]
pub enum ArtifactCmd {
    /// Import a package output into the local registry, keyed by digest.
    Import {
        /// A dist directory containing manifest.json (+ archive), or the
        /// manifest.json itself.
        path: String,
    },
    /// List artifacts in the local registry.
    List {
        /// Only show artifacts of this kind: runtime | plugin | package.
        #[arg(long)]
        kind: Option<String>,
    },
    /// Show the full identity record for one artifact.
    Show {
        /// Digest reference: sha256:<hex> or a unique hex prefix (>= 6 chars).
        digest: String,
    },
    /// Verify a stored artifact's integrity (archive digest + per-file hashes).
    Verify {
        /// Digest reference: sha256:<hex> or a unique hex prefix (>= 6 chars).
        digest: String,
    },
    /// Export an artifact's files into a directory (CI handoff).
    Export {
        /// Digest reference: sha256:<hex> or a unique hex prefix (>= 6 chars).
        digest: String,
        /// Destination directory (created if missing; files must not exist).
        dest: String,
    },
    /// Unpack an artifact's archive into a directory (digest re-verified).
    Extract {
        /// Digest reference: sha256:<hex> or a unique hex prefix (>= 6 chars).
        digest: String,
        /// Destination directory (created if missing).
        dest: String,
    },
}

pub fn run(cmd: ArtifactCmd, fmt: Format) -> Result<()> {
    let store = ArtifactStore::discover();
    match cmd {
        ArtifactCmd::Import { path } => import(&store, &path, fmt),
        ArtifactCmd::List { kind } => list(&store, kind.as_deref(), fmt),
        ArtifactCmd::Show { digest } => show(&store, &digest, fmt),
        ArtifactCmd::Verify { digest } => verify(&store, &digest, fmt),
        ArtifactCmd::Export { digest, dest } => export(&store, &digest, &dest, fmt),
        ArtifactCmd::Extract { digest, dest } => extract(&store, &digest, &dest, fmt),
    }
}

fn import(store: &ArtifactStore, path: &str, fmt: Format) -> Result<()> {
    let out = store.import(Utf8PathBuf::from(path).as_path(), ArtifactSource::Imported)?;
    if fmt.is_json() {
        output::success(&serde_json::json!({
            "imported": true,
            "already_present": out.already_present,
            "artifact": record_json(&out.record),
        }));
        return Ok(());
    }
    if out.already_present {
        println!(
            "Already in the registry as {} ({} {} {})",
            out.record.short_digest(),
            out.record.kind.as_str(),
            out.record.name,
            out.record.version
        );
    } else {
        println!(
            "Imported {} {} {} for {}",
            out.record.kind.as_str(),
            out.record.name,
            out.record.version,
            out.record.target
        );
        println!("  digest: {}", out.record.digest);
    }
    Ok(())
}

fn list(store: &ArtifactStore, kind: Option<&str>, fmt: Format) -> Result<()> {
    let kind = kind
        .map(|k| {
            ArtifactKind::from_tag(k).ok_or_else(|| {
                Error::usage(format!(
                    "unknown artifact kind '{k}' (expected runtime, plugin, or package)"
                ))
            })
        })
        .transpose()?;
    let records: Vec<ArtifactRecord> = store
        .list()?
        .into_iter()
        .filter(|r| kind.is_none_or(|k| r.kind == k))
        .collect();

    if fmt.is_json() {
        output::success(&serde_json::json!({
            "artifacts": records.iter().map(record_json).collect::<Vec<_>>(),
        }));
        return Ok(());
    }
    if records.is_empty() {
        println!("No artifacts in the local registry ({})", store.root());
        println!("  import one with `ost artifact import <dist-dir>` or `ost plugin publish`");
        return Ok(());
    }
    println!("Artifacts in {} :", store.root());
    for r in &records {
        println!(
            "  {}  {:<7} {:<20} {:<10} {}  [{}]",
            r.short_digest(),
            r.kind.as_str(),
            r.name,
            r.version,
            r.target,
            r.validation
        );
    }
    Ok(())
}

fn show(store: &ArtifactStore, digest: &str, fmt: Format) -> Result<()> {
    let r = store.resolve(digest)?;
    if fmt.is_json() {
        output::success(&serde_json::json!({ "artifact": record_json(&r) }));
        return Ok(());
    }
    println!("{} {} {}", r.kind.as_str(), r.name, r.version);
    println!("  digest:      {}", r.digest);
    println!("  target:      {}", r.target);
    if let Some(profile) = &r.profile {
        println!("  profile:     {profile}");
    }
    println!("  archive:     {} ({} bytes)", r.archive, r.archive_size);
    println!(
        "  contents:    {} file(s), {} bytes uncompressed",
        r.file_count, r.total_size
    );
    println!("  source:      {}", r.source.as_str());
    println!("  validation:  {}", r.validation);
    if r.licenses.is_empty() {
        println!("  licenses:    (none recorded)");
    } else {
        println!("  licenses:    {}", r.licenses.join(", "));
    }
    if let (Some(id), Some(dg)) = (&r.runtime_id, &r.runtime_digest) {
        println!("  runtime:     {id} ({dg})");
    }
    println!("  producer:    {}", r.producer);
    println!("  store:       {}", store.object_dir(r.digest_hex()));
    Ok(())
}

fn verify(store: &ArtifactStore, digest: &str, fmt: Format) -> Result<()> {
    let report = store.verify(digest)?;
    let passed = report.passed();
    if fmt.is_json() {
        output::report(
            passed,
            &serde_json::json!({
                "digest": report.digest,
                "passed": passed,
                "archive_digest_ok": report.archive_digest_ok,
                "files_matched": report.files_matched,
                "files_mismatched": report.files_mismatched,
                "files_missing": report.files_missing,
                "files_extra": report.files_extra,
            }),
        );
    } else {
        render_verify(&report);
    }
    // The report above is this command's single document (§14.3); a failed
    // verification exits with the validation category code directly.
    if !passed {
        std::process::exit(ost_core::Category::Validation.exit_code() as i32);
    }
    Ok(())
}

fn render_verify(report: &VerifyReport) {
    println!("Verify {}", report.digest);
    println!(
        "  archive digest: {}",
        if report.archive_digest_ok {
            "OK"
        } else {
            "MISMATCH"
        }
    );
    if report.archive_digest_ok {
        println!("  files matched:  {}", report.files_matched);
        for f in &report.files_mismatched {
            println!("  MISMATCH: {f}");
        }
        for f in &report.files_missing {
            println!("  MISSING:  {f}");
        }
        for f in &report.files_extra {
            println!("  EXTRA:    {f}");
        }
    }
    println!(
        "  result: {}",
        if report.passed() { "PASS" } else { "FAIL" }
    );
}

fn export(store: &ArtifactStore, digest: &str, dest: &str, fmt: Format) -> Result<()> {
    let dest = Utf8PathBuf::from(dest);
    let (record, written) = store.export(digest, &dest)?;
    if fmt.is_json() {
        output::success(&serde_json::json!({
            "exported": true,
            "digest": record.digest,
            "dest": dest.to_string(),
            "files": written.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
        }));
        return Ok(());
    }
    println!(
        "Exported {} ({} {} {}) to {dest}",
        record.short_digest(),
        record.kind.as_str(),
        record.name,
        record.version
    );
    for p in &written {
        println!("  {p}");
    }
    Ok(())
}

fn extract(store: &ArtifactStore, digest: &str, dest: &str, fmt: Format) -> Result<()> {
    let dest = Utf8PathBuf::from(dest);
    let record = store.extract(digest, &dest)?;
    if fmt.is_json() {
        output::success(&serde_json::json!({
            "extracted": true,
            "digest": record.digest,
            "dest": dest.to_string(),
            "files": record.file_count,
        }));
        return Ok(());
    }
    println!(
        "Extracted {} ({} {} {}) to {dest} ({} file(s))",
        record.short_digest(),
        record.kind.as_str(),
        record.name,
        record.version,
        record.file_count
    );
    Ok(())
}

/// The record as JSON for envelopes (serde derives the stable field order).
fn record_json(r: &ArtifactRecord) -> serde_json::Value {
    serde_json::to_value(r).unwrap_or_else(|_| serde_json::json!({}))
}
