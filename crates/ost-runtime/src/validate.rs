//! Runtime validation (§18.1).
//!
//! "Installed is not enough" (§3.4): a pulled runtime is only *certified* once
//! it passes validation. This module runs the minimal structural checks that
//! apply to the local backend — schema, digest integrity, and layout — and
//! produces a report. Richer checks (Python import, native library load, USD
//! stage open) arrive with the real artifact backend and the USD phase.

use camino::Utf8Path;

use crate::manifest::RuntimeManifest;

/// One named validation check and its outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    pub passed: bool,
    /// Context shown on failure (or extra info on success).
    pub detail: Option<String>,
}

impl Check {
    fn pass(name: &'static str) -> Check {
        Check {
            name,
            passed: true,
            detail: None,
        }
    }
    fn fail(name: &'static str, detail: impl Into<String>) -> Check {
        Check {
            name,
            passed: false,
            detail: Some(detail.into()),
        }
    }
}

/// The result of validating a runtime.
#[derive(Debug, Clone)]
pub struct ValidationReport {
    pub checks: Vec<Check>,
}

impl ValidationReport {
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| c.passed)
    }
}

/// Validate a pulled runtime at `prefix` against its `manifest`.
pub fn validate(prefix: &Utf8Path, manifest: &RuntimeManifest) -> ValidationReport {
    let mut checks = Vec::new();

    // 1. Schema version is one we understand.
    if manifest.schema == RuntimeManifest::SCHEMA_VERSION {
        checks.push(Check::pass("manifest-schema"));
    } else {
        checks.push(Check::fail(
            "manifest-schema",
            format!(
                "manifest schema {} != expected {}",
                manifest.schema,
                RuntimeManifest::SCHEMA_VERSION
            ),
        ));
    }

    // 2. The stored digest matches a fresh recomputation.
    let recomputed = manifest.compute_digest();
    if recomputed == manifest.digest {
        checks.push(Check::pass("digest-integrity"));
    } else {
        checks.push(Check::fail(
            "digest-integrity",
            format!("recomputed {recomputed} != stored {}", manifest.digest),
        ));
    }

    // 3. Every declared layout directory exists on disk.
    let missing: Vec<&str> = manifest
        .layout
        .iter()
        .filter(|sub| !prefix.join(sub).as_std_path().is_dir())
        .map(|s| s.as_str())
        .collect();
    if missing.is_empty() {
        checks.push(Check::pass("layout-complete"));
    } else {
        checks.push(Check::fail(
            "layout-complete",
            format!("missing directories: {}", missing.join(", ")),
        ));
    }

    // 4. Real runtimes carry actual OpenUSD: assert the tools and bindings are
    //    present. Skipped for the mock backend, whose layout is empty stubs.
    if manifest.source.is_real() {
        checks.extend(real_runtime_checks(prefix));
    }

    ValidationReport { checks }
}

/// Structural checks that only make sense against a real OpenUSD install
/// (`local`/`build`/`artifact`): the `usdcat` tool and the `pxr` Python package.
fn real_runtime_checks(prefix: &Utf8Path) -> Vec<Check> {
    let mut checks = Vec::new();

    let bin = prefix.join("bin");
    let has_usdcat = bin.join("usdcat").as_std_path().is_file()
        || bin.join("usdcat.exe").as_std_path().is_file();
    if has_usdcat {
        checks.push(Check::pass("usdcat-present"));
    } else {
        checks.push(Check::fail(
            "usdcat-present",
            format!("no usdcat in {bin}"),
        ));
    }

    let pxr = prefix.join("lib").join("python").join("pxr");
    if pxr.as_std_path().is_dir() {
        checks.push(Check::pass("pxr-package"));
    } else {
        checks.push(Check::fail(
            "pxr-package",
            format!("no pxr package at {pxr}"),
        ));
    }

    checks
}
