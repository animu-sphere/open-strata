// SPDX-License-Identifier: Apache-2.0
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
        checks.push(Check::fail("usdcat-present", format!("no usdcat in {bin}")));
    }

    let py_dir = crate::env::usd_python_dir(prefix);
    let pxr = py_dir.join("pxr");
    if pxr.as_std_path().is_dir() {
        checks.push(Check::pass("pxr-package"));
    } else {
        checks.push(Check::fail(
            "pxr-package",
            format!(
                "no pxr package under {}/lib (looked for python/ and site-packages/)",
                prefix
            ),
        ));
    }

    // A runtime that bundles `usdGenSchema` must also carry its schema-gen Python
    // deps (`jinja2`, and `MarkupSafe` transitively). `build_usd.py` installs
    // them only on the build host, so a published image is otherwise silently
    // incomplete for `ost plugin build`'s schema-generate phase — the first
    // hosted CI run dies with a bare `ModuleNotFoundError: No module named
    // 'jinja2'` (report Finding D). Gate it here so `export` (which requires a
    // passing validation) refuses to publish such a runtime.
    if bundles_usdgenschema(&bin) {
        if py_dir.join("jinja2").as_std_path().is_dir() {
            checks.push(Check::pass("schema-gen-deps"));
        } else {
            checks.push(Check::fail(
                "schema-gen-deps",
                format!(
                    "runtime bundles usdGenSchema but jinja2 is missing under {py_dir}; \
                     provision it with `pip install --target {py_dir} jinja2`"
                ),
            ));
        }
    }

    checks
}

/// Whether `bin` holds the `usdGenSchema` schema tool under any common name.
fn bundles_usdgenschema(bin: &Utf8Path) -> bool {
    [
        "usdGenSchema",
        "usdGenSchema.cmd",
        "usdGenSchema.exe",
        "usdGenSchema.bat",
        "usdGenSchema.py",
    ]
    .iter()
    .any(|n| bin.join(n).as_std_path().is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ExtensionRecord, RuntimeSource};
    use crate::Runtime;
    use camino::Utf8PathBuf;
    use ost_core::host::{Arch, Os};
    use ost_core::Host;

    fn manifest(source: RuntimeSource, layout: Vec<String>) -> RuntimeManifest {
        let host = Host {
            os: Os::Linux,
            arch: Arch::X86_64,
        };
        let rt = Runtime::resolve("cy2026", "usd", &host, "3.13.x");
        RuntimeManifest::build(
            &rt,
            "3.13.x",
            vec!["usd-stage-read".into()],
            layout,
            vec![ExtensionRecord {
                id: "openusd".into(),
                version: "25.05.01".into(),
                features: vec!["core".into()],
            }],
            1_700_000_000,
            source,
        )
    }

    fn tmp_dir(tag: &str) -> Utf8PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut dir = Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
        dir.push(format!("ost-validate-{tag}-{}-{nanos}", std::process::id()));
        dir
    }

    fn named(report: &ValidationReport, name: &str) -> Option<bool> {
        report
            .checks
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.passed)
    }

    #[test]
    fn mock_skips_real_runtime_checks() {
        let prefix = tmp_dir("mock");
        std::fs::create_dir_all(prefix.join("bin").as_std_path()).unwrap();
        std::fs::create_dir_all(prefix.join("lib").as_std_path()).unwrap();
        let m = manifest(RuntimeSource::Mock, vec!["bin".into(), "lib".into()]);

        let report = validate(&prefix, &m);
        // The real-runtime checks must not even be emitted for a mock backend.
        assert_eq!(named(&report, "usdcat-present"), None);
        assert_eq!(named(&report, "pxr-package"), None);

        std::fs::remove_dir_all(prefix.as_std_path()).ok();
    }

    #[test]
    fn real_runtime_with_usdcat_and_pxr_passes() {
        let prefix = tmp_dir("real-ok");
        let bin = prefix.join("bin");
        std::fs::create_dir_all(bin.as_std_path()).unwrap();
        std::fs::create_dir_all(prefix.join("lib").as_std_path()).unwrap();
        std::fs::create_dir_all(prefix.join("lib/python/pxr").as_std_path()).unwrap();
        // Cover the .exe fallback on non-Windows too: either name satisfies it.
        std::fs::write(bin.join("usdcat").as_std_path(), b"").unwrap();
        let m = manifest(RuntimeSource::Local, vec!["bin".into(), "lib".into()]);

        let report = validate(&prefix, &m);
        assert_eq!(named(&report, "usdcat-present"), Some(true));
        assert_eq!(named(&report, "pxr-package"), Some(true));

        std::fs::remove_dir_all(prefix.as_std_path()).ok();
    }

    #[test]
    fn usdgenschema_without_jinja2_fails_schema_gen_deps() {
        let prefix = tmp_dir("schema-deps");
        let bin = prefix.join("bin");
        std::fs::create_dir_all(bin.as_std_path()).unwrap();
        std::fs::create_dir_all(prefix.join("lib").as_std_path()).unwrap();
        std::fs::create_dir_all(prefix.join("lib/python/pxr").as_std_path()).unwrap();
        std::fs::write(bin.join("usdcat").as_std_path(), b"").unwrap();
        // Bundles usdGenSchema but no jinja2 on the runtime PYTHONPATH.
        std::fs::write(bin.join("usdGenSchema").as_std_path(), b"").unwrap();
        let m = manifest(RuntimeSource::Build, vec!["bin".into(), "lib".into()]);

        let report = validate(&prefix, &m);
        assert_eq!(named(&report, "schema-gen-deps"), Some(false));
        assert!(
            !report.passed(),
            "a schema runtime missing jinja2 must not validate"
        );

        // Provision jinja2 and it passes — the export gate is now satisfiable.
        std::fs::create_dir_all(prefix.join("lib/python/jinja2").as_std_path()).unwrap();
        let report = validate(&prefix, &m);
        assert_eq!(named(&report, "schema-gen-deps"), Some(true));

        std::fs::remove_dir_all(prefix.as_std_path()).ok();
    }

    #[test]
    fn no_usdgenschema_skips_schema_gen_deps_check() {
        let prefix = tmp_dir("no-schema");
        let bin = prefix.join("bin");
        std::fs::create_dir_all(bin.as_std_path()).unwrap();
        std::fs::create_dir_all(prefix.join("lib/python/pxr").as_std_path()).unwrap();
        std::fs::write(bin.join("usdcat").as_std_path(), b"").unwrap();
        let m = manifest(RuntimeSource::Local, vec!["bin".into()]);

        // A runtime that does not bundle usdGenSchema never needs the deps, so
        // the check is not emitted at all (no false failure).
        let report = validate(&prefix, &m);
        assert_eq!(named(&report, "schema-gen-deps"), None);

        std::fs::remove_dir_all(prefix.as_std_path()).ok();
    }

    #[test]
    fn real_runtime_missing_artifacts_fails() {
        let prefix = tmp_dir("real-missing");
        std::fs::create_dir_all(prefix.join("bin").as_std_path()).unwrap();
        std::fs::create_dir_all(prefix.join("lib").as_std_path()).unwrap();
        // No usdcat, no pxr package.
        let m = manifest(RuntimeSource::Local, vec!["bin".into(), "lib".into()]);

        let report = validate(&prefix, &m);
        assert_eq!(named(&report, "usdcat-present"), Some(false));
        assert_eq!(named(&report, "pxr-package"), Some(false));
        assert!(!report.passed());

        std::fs::remove_dir_all(prefix.as_std_path()).ok();
    }
}
