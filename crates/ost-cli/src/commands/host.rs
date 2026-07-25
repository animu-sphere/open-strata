// SPDX-License-Identifier: Apache-2.0
//! `ost host` — discover, list, and inspect third-party DCC installs.
//!
//! These commands are **diagnostic**: `discover` and `list` report what is on
//! the machine, so finding nothing is an answer and exits `0`, exactly as
//! `doctor` does. Only a real failure — an unusable selector, an unreadable
//! inventory — takes a category exit code.
//!
//! Nothing here launches, configures, or modifies a host. Composing one into a
//! runnable environment is Formation's job.

use std::collections::BTreeSet;

use camino::{Utf8Path, Utf8PathBuf};
use clap::{Args, Subcommand};
use ost_core::paths::{find_project_root, PROJECT_MANIFEST};
use ost_core::{Category, Error, Result};
use ost_host::{
    canonical_root_key, discover, project_inventory_path, select, user_cache_path,
    DiscoveryRequest, FingerprintMode, HostFamily, HostInventory, HostRecord, HostStatus,
    Selection, ValidateOptions,
};
use ost_manifest::{Project, DEFAULT_DISCOVERY_DEPTH};

use crate::output::{self, Format};

#[derive(Debug, Subcommand)]
pub enum HostCmd {
    /// Scan for DCC installs and record what was found.
    Discover(DiscoverArgs),
    /// List the hosts already recorded, re-checked against the filesystem.
    List(ListArgs),
    /// Show one host record in full.
    Inspect(InspectArgs),
}

#[derive(Debug, Args)]
pub struct DiscoverArgs {
    /// Restrict discovery to one family (maya, houdini). Repeatable.
    #[arg(long = "host", value_name = "FAMILY")]
    pub families: Vec<String>,

    /// Validate this exact install root. Repeatable. A path that is not a host
    /// is reported with the reason, rather than silently skipped.
    #[arg(long = "path", value_name = "PATH")]
    pub paths: Vec<Utf8PathBuf>,

    /// Scan this directory in addition to `[host.discovery].roots`. Repeatable.
    #[arg(long = "root", value_name = "PATH")]
    pub roots: Vec<Utf8PathBuf>,

    /// How deep below each scanned root an install may sit.
    #[arg(long)]
    pub depth: Option<u8>,

    /// Skip the vendor environment variables (MAYA_LOCATION, HFS).
    #[arg(long)]
    pub no_environment: bool,

    /// Skip this platform's documented default install locations.
    #[arg(long)]
    pub no_known_roots: bool,

    /// Also accept install roots derived from executables on PATH. Off by
    /// default: PATH reflects the calling shell, not the machine.
    #[arg(long)]
    pub from_path: bool,

    /// Run each host's own `--version` when shipped metadata cannot answer.
    /// Executes the host binary under a bounded timeout.
    #[arg(long)]
    pub probe: bool,

    /// Fingerprint depth: `standard` (identity and layout) or `deep` (also
    /// hashes the selected executables).
    #[arg(long, default_value = "standard")]
    pub fingerprint: String,

    /// Ignore the cached inventory and rescan. Discovery always rescans; this
    /// additionally discards records the scan did not re-find.
    #[arg(long)]
    pub refresh: bool,

    /// Also write the result to the project's reviewable `.strata/hosts/`
    /// inventory.
    #[arg(long)]
    pub register: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Restrict the listing to one family.
    #[arg(long = "host", value_name = "FAMILY")]
    pub families: Vec<String>,

    /// Restrict the listing to one status.
    #[arg(long, value_name = "STATUS")]
    pub status: Option<String>,
}

#[derive(Debug, Args)]
pub struct InspectArgs {
    /// Instance id, install path, family name, or id prefix — the last three
    /// only when they name exactly one install.
    pub selector: String,
}

pub fn run(command: HostCmd, format: Format) -> Result<()> {
    match command {
        HostCmd::Discover(args) => discover_command(&args, format),
        HostCmd::List(args) => list_command(&args, format),
        HostCmd::Inspect(args) => inspect_command(&args, format),
    }
}

/// The project this invocation is running inside, when there is one. `ost host`
/// works fine outside a project — a machine's installs are not a project's
/// property — so a missing project is `None`, not an error.
fn current_project() -> Result<Option<(Utf8PathBuf, Project)>> {
    let cwd = std::env::current_dir().map_err(|error| Error::io(".", error))?;
    let Some(root) = find_project_root(&cwd) else {
        return Ok(None);
    };
    let root = Utf8PathBuf::from_path_buf(root)
        .map_err(|path| Error::config(format!("non-UTF-8 project path: {}", path.display())))?;
    let manifest = root.join(PROJECT_MANIFEST);
    let source = std::fs::read_to_string(manifest.as_std_path())
        .map_err(|error| Error::io(manifest.to_string(), error))?;
    Ok(Some((root.clone(), Project::from_toml(&source)?)))
}

fn parse_families(values: &[String]) -> Result<Vec<HostFamily>> {
    values
        .iter()
        .map(|value| HostFamily::parse(value))
        .collect()
}

fn discover_command(args: &DiscoverArgs, format: Format) -> Result<()> {
    let project = current_project()?;
    let configured = project
        .as_ref()
        .and_then(|(_, project)| project.host.as_ref())
        .and_then(|host| host.discovery.as_ref());

    let mut families = parse_families(&args.families)?;
    if families.is_empty() {
        if let Some(discovery) = configured {
            families = parse_families(&discovery.families)?;
        }
    }

    let mut roots: Vec<Utf8PathBuf> = configured
        .map(|discovery| {
            discovery
                .roots
                .iter()
                .map(Utf8PathBuf::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    roots.extend(args.roots.iter().cloned());

    let depth = args
        .depth
        .or_else(|| configured.and_then(|discovery| discovery.max_depth))
        .unwrap_or(DEFAULT_DISCOVERY_DEPTH);
    if depth == 0 || depth > ost_manifest::MAX_DISCOVERY_DEPTH {
        return Err(Error::usage(format!(
            "--depth must be between 1 and {} (got {depth})",
            ost_manifest::MAX_DISCOVERY_DEPTH
        )));
    }

    let request = DiscoveryRequest {
        families,
        explicit_paths: args.paths.clone(),
        configured_roots: roots,
        max_depth: depth,
        use_environment: !args.no_environment,
        use_known_roots: !args.no_known_roots,
        use_executable_path: args.from_path,
        fingerprint: FingerprintMode::parse(&args.fingerprint)?,
        validate: ValidateOptions {
            probe: args.probe,
            ..ValidateOptions::default()
        },
    };

    let outcome = discover(&request)?;
    let cache_path = user_cache_path();
    let records = if args.refresh {
        outcome.records.clone()
    } else {
        // A scan only sees the roots it was pointed at, so a narrow scan must
        // not silently delete hosts a wider one recorded earlier. `--refresh`
        // is how an operator says "forget what you knew".
        merge_with_cache(&cache_path, outcome.records.clone())?
    };
    HostInventory::new(records.clone()).save(&cache_path)?;

    let mut registered = None;
    if args.register {
        let Some((root, _)) = project.as_ref() else {
            return Err(Error::coded(
                "HOST_PROJECT_REQUIRED",
                Category::Precondition,
                "--register needs an OpenStrata project",
            )
            .with_hint(
                "run it inside a project, or drop --register to update the user cache only",
            ));
        };
        let path = project_inventory_path(root);
        HostInventory::new(records.clone()).save(&path)?;
        registered = Some(path);
    }

    let mut warnings: Vec<serde_json::Value> = outcome
        .warnings
        .iter()
        .map(|warning| serde_json::json!({ "code": warning.code, "message": warning.message }))
        .collect();
    if records.iter().all(|record| !record.status.is_usable()) {
        // Finding nothing is a legitimate answer, but a silent empty result
        // reads like a broken command. Say which providers were consulted.
        warnings.push(serde_json::json!({
            "code": "HOST_NONE_VALIDATED",
            "message": format!(
                "no DCC host validated; consulted {}",
                consulted(&request)
            ),
        }));
    }

    match format {
        Format::Json => output::report_with_warnings(
            true,
            &serde_json::json!({
                "schema_version": ost_host::HOST_RECORD_SCHEMA,
                "scanned_directories": outcome.scanned,
                "cache": cache_path,
                "registered": registered,
                "records": records,
            }),
            &warnings,
        ),
        Format::Human => {
            print_records(&records);
            for warning in &warnings {
                println!(
                    "warning[{}]: {}",
                    warning["code"].as_str().unwrap_or_default(),
                    warning["message"].as_str().unwrap_or_default()
                );
            }
            println!("  cache: {cache_path}");
            if let Some(path) = registered {
                println!("  registered: {path}");
            }
        }
    }
    Ok(())
}

/// Which providers this request actually consulted, so an empty result is
/// explainable without re-running with different flags.
fn consulted(request: &DiscoveryRequest) -> String {
    let mut sources = Vec::new();
    if !request.explicit_paths.is_empty() {
        sources.push(format!("{} explicit path(s)", request.explicit_paths.len()));
    }
    if !request.configured_roots.is_empty() {
        sources.push(format!(
            "{} configured root(s) at depth {}",
            request.configured_roots.len(),
            request.max_depth
        ));
    }
    if request.use_environment {
        sources.push("vendor environment variables".into());
    }
    if request.use_known_roots {
        sources.push("default install locations".into());
    }
    if request.use_executable_path {
        sources.push("PATH".into());
    }
    if sources.is_empty() {
        return "no providers (every provider was disabled)".into();
    }
    sources.join(", ")
}

/// Keep cached records the current scan did not re-find, but let the scan's own
/// view win wherever the two overlap.
///
/// Overlap is decided by **canonical root, not by id**: an id carries the
/// version, so an install upgraded in place mints a new one, and matching on ids
/// would keep the pre-upgrade record as a second usable host at a single root —
/// two answers to "which Maya do I have" for one install. Whatever this scan saw
/// at a root is the whole truth about that root.
fn merge_with_cache(cache_path: &Utf8Path, fresh: Vec<HostRecord>) -> Result<Vec<HostRecord>> {
    let Some(mut cached) = HostInventory::load(cache_path)? else {
        return Ok(fresh);
    };
    cached.refresh_statuses();
    let mut records = fresh;
    let rescanned: BTreeSet<String> = records
        .iter()
        .map(|record| canonical_root_key(&record.root))
        .collect();
    for record in cached.records {
        if !rescanned.contains(&canonical_root_key(&record.root)) {
            records.push(record);
        }
    }
    records.sort_by(|left, right| left.listing_order().cmp(&right.listing_order()));
    Ok(records)
}

fn list_command(args: &ListArgs, format: Format) -> Result<()> {
    let families = parse_families(&args.families)?;
    let status = args.status.as_deref().map(HostStatus::parse).transpose()?;

    let cache_path = user_cache_path();
    let mut inventory =
        HostInventory::load(&cache_path)?.unwrap_or_else(|| HostInventory::new(Vec::new()));
    // The cache is a record of a past observation; re-check it before serving
    // any of it as usable.
    inventory.refresh_statuses();

    let records: Vec<HostRecord> = inventory
        .records
        .into_iter()
        .filter(|record| families.is_empty() || families.contains(&record.family))
        .filter(|record| status.is_none_or(|status| record.status == status))
        .collect();

    let mut warnings = Vec::new();
    if records.is_empty() {
        warnings.push(serde_json::json!({
            "code": "HOST_INVENTORY_EMPTY",
            "message": "no matching host records; run `ost host discover` first",
        }));
    }

    match format {
        Format::Json => output::report_with_warnings(
            true,
            &serde_json::json!({ "cache": cache_path, "records": records }),
            &warnings,
        ),
        Format::Human => {
            print_records(&records);
            if records.is_empty() {
                println!("no host records (run `ost host discover`)");
            }
        }
    }
    Ok(())
}

fn inspect_command(args: &InspectArgs, format: Format) -> Result<()> {
    let cache_path = user_cache_path();
    let mut inventory =
        HostInventory::load(&cache_path)?.unwrap_or_else(|| HostInventory::new(Vec::new()));
    inventory.refresh_statuses();

    let record = match select(&inventory.records, &args.selector) {
        Selection::One(record) => record.clone(),
        Selection::Ambiguous(ids) => {
            return Err(Error::coded(
                "HOST_SELECTOR_AMBIGUOUS",
                Category::Usage,
                format!(
                    "'{}' matches {} hosts: {}",
                    args.selector,
                    ids.len(),
                    ids.join(", ")
                ),
            )
            .with_hint("name one instance id exactly"));
        }
        Selection::None => {
            return Err(Error::coded(
                "HOST_NOT_FOUND",
                Category::Precondition,
                format!("no host matches '{}'", args.selector),
            )
            .with_hint("run `ost host discover`, then `ost host list`"));
        }
    };

    match format {
        Format::Json => output::success(&serde_json::to_value(&record).map_err(|error| {
            Error::Operation(format!("cannot serialize the host record: {error}"))
        })?),
        Format::Human => print_record_detail(&record),
    }
    Ok(())
}

fn print_records(records: &[HostRecord]) {
    for record in records {
        println!("{} {}", record.family.as_str(), record.summary());
        println!("    root: {}", record.root);
        if let Some(python) = &record.python {
            println!("    python: {} ({})", python.version, python.abi_tag);
        }
        if let Some(rejection) = &record.rejection {
            println!("    rejected[{}]: {}", rejection.code, rejection.message);
        }
    }
}

fn print_record_detail(record: &HostRecord) {
    println!("{} ({})", record.id, record.product);
    println!("  status: {}", record.status.as_str());
    println!("  root: {}", record.root);
    match &record.version {
        Some(version) => println!(
            "  version: {} (source: {}, confidence: {})",
            version.raw,
            version.source.as_str(),
            version.confidence
        ),
        None => println!("  version: unknown"),
    }
    match &record.python {
        Some(python) => println!(
            "  python: {} ({}) from {}",
            python.version, python.abi_tag, python.evidence
        ),
        None => println!("  python: not detected"),
    }
    println!(
        "  platform: {}-{}",
        record.platform.os.as_str(),
        record.platform.arch.as_str()
    );
    for (role, executable) in &record.executables {
        println!("  {role}: {} ({} bytes)", executable.path, executable.size);
    }
    if !record.markers.is_empty() {
        println!("  markers: {}", record.markers.join(", "));
    }
    for evidence in &record.evidence {
        println!(
            "  found via {}: {}",
            evidence.source.as_str(),
            evidence.detail
        );
    }
    match &record.fingerprint {
        Some(fingerprint) => println!(
            "  fingerprint: {} (mode: {}, inputs v{})",
            fingerprint.digest,
            fingerprint.mode.as_str(),
            fingerprint.inputs_version
        ),
        None => println!("  fingerprint: not computed"),
    }
    if let Some(rejection) = &record.rejection {
        println!("  rejected[{}]: {}", rejection.code, rejection.message);
    }
}
