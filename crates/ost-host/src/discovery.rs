// SPDX-License-Identifier: Apache-2.0
//! Candidate discovery: composable, order-stable providers over bounded roots.
//!
//! Discovery is two phases, mirroring the plugin harness's "candidate →
//! validated, never a false PASS". A provider only *suggests* a root; a
//! validator decides. Nothing here mutates a host install, expands a shell, or
//! walks a filesystem it was not pointed at.

use std::collections::BTreeMap;

use camino::{Utf8Path, Utf8PathBuf};
use ost_core::host::Os;
use ost_core::Result;

use crate::record::{
    canonical_root_key, fingerprint, normalize_path, DiscoveryEvidence, DiscoverySource,
    FingerprintMode, HostFamily, HostIdentity, HostPlatform, HostRecord, HostRejection, HostStatus,
    HOST_RECORD_SCHEMA,
};
use crate::validate::{self, ValidateOptions};

/// The most directories one `discover` may look at, across every provider.
///
/// A misconfigured root is the expected failure mode, and the symptom is a scan
/// that never ends. This turns that into a bounded scan and a warning.
pub const SCAN_BUDGET: usize = 4096;

/// What to look at, and how hard.
#[derive(Debug, Clone)]
pub struct DiscoveryRequest {
    /// Families to consider. Empty means every family this `ost` knows.
    pub families: Vec<HostFamily>,
    /// Roots the operator named directly. Each is a candidate on its own.
    pub explicit_paths: Vec<Utf8PathBuf>,
    /// `[host.discovery].roots` from the project manifest, scanned to depth.
    pub configured_roots: Vec<Utf8PathBuf>,
    pub max_depth: u8,
    /// Consult vendor environment variables (`MAYA_LOCATION`, `HFS`).
    pub use_environment: bool,
    /// Consult this platform's documented default install locations.
    pub use_known_roots: bool,
    /// Consult `PATH`. Off by default: an ambient `PATH` entry says which host
    /// a shell happens to prefer, not which hosts a machine has, and letting it
    /// contribute silently would make discovery depend on the caller's shell.
    pub use_executable_path: bool,
    pub fingerprint: FingerprintMode,
    pub validate: ValidateOptions,
}

impl Default for DiscoveryRequest {
    fn default() -> Self {
        DiscoveryRequest {
            families: Vec::new(),
            explicit_paths: Vec::new(),
            configured_roots: Vec::new(),
            max_depth: ost_manifest::DEFAULT_DISCOVERY_DEPTH,
            use_environment: true,
            use_known_roots: true,
            use_executable_path: false,
            fingerprint: FingerprintMode::Standard,
            validate: ValidateOptions::default(),
        }
    }
}

impl DiscoveryRequest {
    /// The families to try, defaulted to all.
    pub fn effective_families(&self) -> Vec<HostFamily> {
        if self.families.is_empty() {
            HostFamily::ALL.to_vec()
        } else {
            self.families.clone()
        }
    }
}

/// A non-fatal condition worth telling the operator about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryWarning {
    pub code: &'static str,
    pub message: String,
}

/// The result of one discovery pass.
#[derive(Debug, Clone, Default)]
pub struct DiscoveryOutcome {
    /// Validated first, then rejected; each group sorted by id. Deterministic
    /// so two runs on an unchanged machine print the same bytes.
    pub records: Vec<HostRecord>,
    pub warnings: Vec<DiscoveryWarning>,
    /// How many directories the scan actually looked at.
    pub scanned: usize,
}

impl DiscoveryOutcome {
    pub fn validated(&self) -> impl Iterator<Item = &HostRecord> {
        self.records
            .iter()
            .filter(|record| record.status == HostStatus::Validated)
    }
}

/// A root some provider suggested, before any validator has looked at it.
#[derive(Debug, Clone)]
struct Candidate {
    /// The family this root is claimed to be, when the provider knows.
    family: Option<HostFamily>,
    root: Utf8PathBuf,
    evidence: Vec<DiscoveryEvidence>,
    /// Whether a refusal is worth reporting. An operator who named a path is
    /// owed a reason; a directory that merely turned up in a scan is not, or
    /// scanning `/opt` would report every directory on the machine.
    report_rejection: bool,
}

/// Run every enabled provider, then validate what they found.
pub fn discover(request: &DiscoveryRequest) -> Result<DiscoveryOutcome> {
    let families = request.effective_families();
    let mut outcome = DiscoveryOutcome::default();
    let mut budget = SCAN_BUDGET;
    let mut candidates: Vec<Candidate> = Vec::new();

    // Provider order is the resolution order, and it is stable: what an
    // operator names beats what a project configures, which beats what the
    // environment hints, which beats a documented default.
    for path in &request.explicit_paths {
        candidates.push(Candidate {
            family: single_family(&families),
            root: path.clone(),
            evidence: vec![DiscoveryEvidence {
                source: DiscoverySource::ExplicitPath,
                detail: normalize_path(path).into_string(),
            }],
            report_rejection: true,
        });
    }

    for root in &request.configured_roots {
        // A root a site *declared* and does not have is a misconfiguration, and
        // the symptom without this is an empty result that names no cause. A
        // documented default location is the opposite case — most machines have
        // none of them — so only declared roots are reported.
        if !root.as_std_path().is_dir() {
            outcome.warnings.push(DiscoveryWarning {
                code: "HOST_ROOT_NOT_FOUND",
                message: format!(
                    "configured discovery root '{}' is not a readable directory",
                    normalize_path(root)
                ),
            });
            continue;
        }
        scan_root(
            root,
            request.max_depth,
            DiscoverySource::ConfiguredRoot,
            normalize_path(root).as_str(),
            None,
            &mut candidates,
            &mut budget,
            &mut outcome,
        );
    }

    if request.use_environment {
        for (family, variable) in environment_variables(&families) {
            let Some(value) = read_env(variable) else {
                continue;
            };
            candidates.push(Candidate {
                family: Some(family),
                root: Utf8PathBuf::from(value),
                evidence: vec![DiscoveryEvidence {
                    source: DiscoverySource::Environment,
                    detail: variable.to_string(),
                }],
                // A vendor variable pointing at nothing is a real
                // misconfiguration, and silence about it costs a support call.
                report_rejection: true,
            });
        }
    }

    if request.use_known_roots {
        for known in known_roots(request.validate.os, &families) {
            scan_root(
                &known.parent,
                1,
                DiscoverySource::KnownInstallRoot,
                normalize_path(&known.parent).as_str(),
                Some((known.family, known.prefixes)),
                &mut candidates,
                &mut budget,
                &mut outcome,
            );
        }
    }

    if request.use_executable_path {
        candidates.extend(executable_path_candidates(&families, request.validate.os));
    }

    outcome.scanned = SCAN_BUDGET - budget;

    let merged = merge_by_root(candidates);
    for candidate in merged {
        validate_candidate(&candidate, &families, request, &mut outcome);
    }

    // Validated installs first: the answer to "what do I have" comes before the
    // answer to "what did you look at and refuse".
    outcome
        .records
        .sort_by(|left, right| left.listing_order().cmp(&right.listing_order()));
    Ok(outcome)
}

/// When exactly one family was requested, a bare path is claimed to be it.
fn single_family(families: &[HostFamily]) -> Option<HostFamily> {
    match families {
        [family] => Some(*family),
        _ => None,
    }
}

fn read_env(variable: &str) -> Option<String> {
    let value = std::env::var(variable).ok()?;
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn environment_variables(families: &[HostFamily]) -> Vec<(HostFamily, &'static str)> {
    let mut variables = Vec::new();
    for family in families {
        match family {
            HostFamily::Maya => variables.push((HostFamily::Maya, "MAYA_LOCATION")),
            HostFamily::Houdini => variables.push((HostFamily::Houdini, "HFS")),
        }
    }
    variables
}

/// A documented default install location: a parent directory plus the name
/// prefixes its install directories use.
struct KnownRoot {
    family: HostFamily,
    parent: Utf8PathBuf,
    prefixes: &'static [&'static str],
}

fn known_roots(os: Os, families: &[HostFamily]) -> Vec<KnownRoot> {
    let mut roots = Vec::new();
    let wanted = |family: HostFamily| families.contains(&family);

    match os {
        Os::Windows => {
            // Both program-files variables are consulted because a 32-bit
            // process sees the x86 tree under `ProgramFiles`.
            let program_files: Vec<String> = ["ProgramFiles", "ProgramW6432"]
                .iter()
                .filter_map(|variable| read_env(variable))
                .collect();
            let program_files = if program_files.is_empty() {
                vec!["C:/Program Files".to_string()]
            } else {
                program_files
            };
            for base in program_files {
                let base = Utf8PathBuf::from(base.replace('\\', "/"));
                if wanted(HostFamily::Maya) {
                    roots.push(KnownRoot {
                        family: HostFamily::Maya,
                        parent: base.join("Autodesk"),
                        prefixes: &["maya"],
                    });
                }
                if wanted(HostFamily::Houdini) {
                    roots.push(KnownRoot {
                        family: HostFamily::Houdini,
                        parent: base.join("Side Effects Software"),
                        prefixes: &["houdini", "hfs"],
                    });
                }
            }
        }
        Os::Linux => {
            if wanted(HostFamily::Maya) {
                roots.push(KnownRoot {
                    family: HostFamily::Maya,
                    parent: Utf8PathBuf::from("/usr/autodesk"),
                    prefixes: &["maya"],
                });
            }
            if wanted(HostFamily::Houdini) {
                // SideFX unpacks straight into `/opt` (`/opt/hfs20.5.550`), so
                // the parent is `/opt` with a name filter rather than a walk.
                for parent in ["/opt", "/opt/sidefx", "/usr/local/sidefx"] {
                    roots.push(KnownRoot {
                        family: HostFamily::Houdini,
                        parent: Utf8PathBuf::from(parent),
                        prefixes: &["hfs", "houdini"],
                    });
                }
            }
        }
        Os::Macos => {
            if wanted(HostFamily::Maya) {
                roots.push(KnownRoot {
                    family: HostFamily::Maya,
                    parent: Utf8PathBuf::from("/Applications/Autodesk"),
                    prefixes: &["maya"],
                });
            }
            if wanted(HostFamily::Houdini) {
                roots.push(KnownRoot {
                    family: HostFamily::Houdini,
                    parent: Utf8PathBuf::from("/Applications/Houdini"),
                    prefixes: &["houdini", "hfs"],
                });
            }
        }
    }
    roots
}

/// Walk `root` to `max_depth`, adding every directory as a candidate.
///
/// `filter` restricts the walk to a family and the directory-name prefixes that
/// family's installs use — what a documented default location needs, so a
/// crowded `/opt` costs one `read_dir` rather than a subtree walk.
#[allow(clippy::too_many_arguments)]
fn scan_root(
    root: &Utf8Path,
    max_depth: u8,
    source: DiscoverySource,
    detail: &str,
    filter: Option<(HostFamily, &'static [&'static str])>,
    candidates: &mut Vec<Candidate>,
    budget: &mut usize,
    outcome: &mut DiscoveryOutcome,
) {
    if !root.as_std_path().is_dir() {
        return;
    }
    // The root itself is a candidate: a site may configure the install
    // directory directly rather than its parent.
    if filter.is_none() {
        candidates.push(Candidate {
            family: None,
            root: root.to_owned(),
            evidence: vec![DiscoveryEvidence {
                source,
                detail: detail.to_string(),
            }],
            report_rejection: false,
        });
    }

    let mut frontier = vec![(root.to_owned(), 0u8)];
    while let Some((directory, depth)) = frontier.pop() {
        if depth >= max_depth {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(directory.as_std_path()) else {
            continue;
        };
        for entry in entries.flatten() {
            if *budget == 0 {
                if !outcome
                    .warnings
                    .iter()
                    .any(|warning| warning.code == "HOST_SCAN_BUDGET_EXHAUSTED")
                {
                    outcome.warnings.push(DiscoveryWarning {
                        code: "HOST_SCAN_BUDGET_EXHAUSTED",
                        message: format!(
                            "stopped after {SCAN_BUDGET} directories; narrow \
                             [host.discovery].roots or lower max_depth"
                        ),
                    });
                }
                return;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let child = directory.join(&name);
            if !is_directory(&entry, &child) {
                continue;
            }
            *budget -= 1;
            // Dot-directories are caches and version-control state, never an
            // install; skipping them keeps a site root cheap to scan.
            if name.starts_with('.') {
                continue;
            }
            match filter {
                Some((family, prefixes)) => {
                    let lowered = name.to_lowercase();
                    if !prefixes.iter().any(|prefix| lowered.starts_with(prefix)) {
                        continue;
                    }
                    candidates.push(Candidate {
                        family: Some(family),
                        root: child,
                        evidence: vec![DiscoveryEvidence {
                            source,
                            detail: detail.to_string(),
                        }],
                        report_rejection: false,
                    });
                }
                None => {
                    candidates.push(Candidate {
                        family: None,
                        root: child.clone(),
                        evidence: vec![DiscoveryEvidence {
                            source,
                            detail: detail.to_string(),
                        }],
                        report_rejection: false,
                    });
                    frontier.push((child, depth + 1));
                }
            }
        }
    }
}

/// Whether an entry names a directory worth scanning, **resolving links**.
///
/// [`std::fs::DirEntry::file_type`] deliberately does not traverse a link, so a
/// site that aliases a version — `/tools/maya/current -> 2025.3`, the very
/// layout canonicalization exists to collapse — would be skipped by every
/// scanning provider while `--path` on the same link validated fine. A Windows
/// junction reports as a link here too, and is the same case.
///
/// The extra `stat` is paid only for links, and a link cycle is already bounded
/// by `max_depth` and the scan budget.
fn is_directory(entry: &std::fs::DirEntry, path: &Utf8Path) -> bool {
    match entry.file_type() {
        Ok(kind) if kind.is_dir() => true,
        Ok(kind) if kind.is_symlink() => path.as_std_path().is_dir(),
        _ => false,
    }
}

/// Resolve install roots from anchor executables on `PATH`.
fn executable_path_candidates(families: &[HostFamily], os: Os) -> Vec<Candidate> {
    let suffix = if os == Os::Windows { ".exe" } else { "" };
    let mut candidates = Vec::new();
    for family in families {
        let names: Vec<String> = match family {
            HostFamily::Maya => vec![format!("mayapy{suffix}"), format!("maya{suffix}")],
            HostFamily::Houdini => vec![format!("hython{suffix}"), format!("houdini{suffix}")],
        };
        for name in names {
            let Some(executable) = find_on_path(&name) else {
                continue;
            };
            // `<root>/bin/<exe>` — the root is the bin directory's parent.
            let Some(root) = executable.parent().and_then(Utf8Path::parent) else {
                continue;
            };
            candidates.push(Candidate {
                family: Some(*family),
                root: root.to_owned(),
                evidence: vec![DiscoveryEvidence {
                    source: DiscoverySource::ExecutablePath,
                    detail: normalize_path(&executable).into_string(),
                }],
                report_rejection: true,
            });
        }
    }
    candidates
}

fn find_on_path(name: &str) -> Option<Utf8PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter_map(|entry| Utf8PathBuf::from_path_buf(entry.join(name)).ok())
        .find(|candidate| candidate.as_std_path().is_file())
}

/// Collapse candidates that name the same install, keeping every provenance.
///
/// `current -> 2025.3` and `/tools/maya/2025.3` are one install found twice,
/// and canonicalizing is what makes them collapse. A path that cannot be
/// canonicalized (it does not exist) is kept as declared so the validator can
/// say so rather than the provider dropping it silently.
fn merge_by_root(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut merged: BTreeMap<String, Candidate> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for mut candidate in candidates {
        let canonical = canonicalize(&candidate.root).unwrap_or(candidate.root);
        candidate.root = normalize_path(&canonical);
        let key = canonical_root_key(&candidate.root);
        match merged.get_mut(&key) {
            Some(existing) => {
                for evidence in candidate.evidence {
                    if !existing.evidence.contains(&evidence) {
                        existing.evidence.push(evidence);
                    }
                }
                existing.family = existing.family.or(candidate.family);
                existing.report_rejection |= candidate.report_rejection;
            }
            None => {
                order.push(key.clone());
                merged.insert(key, candidate);
            }
        }
    }
    order
        .into_iter()
        .filter_map(|key| merged.remove(&key))
        .map(|mut candidate| {
            candidate.evidence.sort();
            candidate
        })
        .collect()
}

/// Canonicalize a path, undoing Windows' verbatim (`\\?\`) prefix so records
/// carry the path a person would type.
fn canonicalize(path: &Utf8Path) -> Option<Utf8PathBuf> {
    let canonical = std::fs::canonicalize(path.as_std_path()).ok()?;
    let text = canonical.to_string_lossy().into_owned();
    let text = match text.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => text.strip_prefix(r"\\?\").unwrap_or(&text).to_string(),
    };
    Some(Utf8PathBuf::from(text))
}

/// Validate one candidate against every family it could be, recording the
/// first that confirms — and, when the candidate was named rather than found,
/// a rejection per family that did not.
fn validate_candidate(
    candidate: &Candidate,
    families: &[HostFamily],
    request: &DiscoveryRequest,
    outcome: &mut DiscoveryOutcome,
) {
    let attempts: Vec<HostFamily> = match candidate.family {
        Some(family) if families.contains(&family) => vec![family],
        Some(_) => return,
        None => families.to_vec(),
    };

    let mut rejections: Vec<(HostFamily, HostRejection)> = Vec::new();
    for family in attempts {
        match validate::validate(family, &candidate.root, &request.validate) {
            Ok(validation) => {
                let platform = HostPlatform::for_os(request.validate.os);
                let identity = HostIdentity {
                    family,
                    root: &candidate.root,
                    version: validation.version.as_ref(),
                    executables: &validation.executables,
                    python: validation.python.as_ref(),
                    markers: &validation.markers,
                    platform,
                };
                let (fingerprint, status) = match fingerprint(&identity, request.fingerprint) {
                    Ok(fingerprint) => (Some(fingerprint), HostStatus::Validated),
                    Err(error) => {
                        // A deep fingerprint reads the executables; losing that
                        // read means the install is not fully readable, which
                        // is a candidate, not a validated host.
                        outcome.warnings.push(DiscoveryWarning {
                            code: "HOST_FINGERPRINT_UNAVAILABLE",
                            message: format!("{}: {error}", candidate.root),
                        });
                        (None, HostStatus::Candidate)
                    }
                };
                outcome.records.push(HostRecord {
                    schema_version: HOST_RECORD_SCHEMA.to_string(),
                    id: HostRecord::instance_id(
                        family,
                        validation.version.as_ref(),
                        &candidate.root,
                    ),
                    family,
                    product: family.product().to_string(),
                    status,
                    root: candidate.root.clone(),
                    version: validation.version,
                    executables: validation.executables,
                    python: validation.python,
                    platform,
                    markers: validation.markers,
                    evidence: candidate.evidence.clone(),
                    fingerprint,
                    rejection: None,
                });
                return;
            }
            Err(rejection) => rejections.push((family, rejection)),
        }
    }

    if !candidate.report_rejection {
        return;
    }
    for (family, rejection) in rejections {
        outcome.records.push(HostRecord {
            schema_version: HOST_RECORD_SCHEMA.to_string(),
            id: HostRecord::instance_id(family, None, &candidate.root),
            family,
            product: family.product().to_string(),
            status: HostStatus::Rejected,
            root: candidate.root.clone(),
            version: None,
            executables: BTreeMap::new(),
            python: None,
            platform: HostPlatform::for_os(request.validate.os),
            markers: Vec::new(),
            evidence: candidate.evidence.clone(),
            fingerprint: None,
            rejection: Some(rejection),
        });
    }
}
