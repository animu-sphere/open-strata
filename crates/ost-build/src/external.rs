// SPDX-License-Identifier: Apache-2.0
//! Provenance for a build tree OpenStrata did not configure.
//!
//! Plenty of real work happens in build trees `ost build` never touched: a
//! renderer configured by hand against an adopted runtime, a vendor's CMake
//! invocation, a CI job with its own configure step. `ost validate --build-dir`
//! exists for exactly those, and until now it could only shrug — it skipped the
//! `configured`, `built` and `runtime-compatible` checks, because nothing tied
//! the external tree to a runtime OpenStrata knows.
//!
//! This module supplies that tie, without ever pretending OpenStrata performed
//! the build. An import inspects the tree's own `CMakeCache.txt` — the artifact
//! CMake itself wrote — and records the identity it finds there: where the
//! sources are, where the tree is, which runtime it resolved `pxr` from, the
//! generator, configuration, compiler and Python it used.
//!
//! The record is only as good as its binding to the tree, so it carries a digest
//! over the exact cache entries it was derived from ([`IDENTITY_KEYS`]). A tree
//! reconfigured against a different runtime, generator or configuration produces
//! a different digest, and the record stops verifying rather than quietly
//! describing a build that no longer exists.
//!
//! What this buys is narrow and deliberate: on a *full* identity match,
//! `validate --build-dir` may report `runtime-compatible`. It never reports
//! `configured` or `built` — those claim OpenStrata did the work, and it did
//! not.

use std::collections::BTreeMap;

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

pub const EXTERNAL_BUILD_FILE: &str = ".ost-external-build.json";
pub const EXTERNAL_BUILD_SCHEMA: &str = "openstrata.external-build/v1";

/// The CMake cache entries an external build's identity is derived from.
///
/// The set is fixed and listed rather than "every entry": a cache holds hundreds
/// of incidental values (timestamps, per-find-module scratch, absolute paths of
/// tools that do not affect the result) and digesting all of them would make the
/// record fail on changes that mean nothing. These are the entries that decide
/// what was actually produced.
pub const IDENTITY_KEYS: &[&str] = &[
    "CMAKE_HOME_DIRECTORY",
    "CMAKE_CACHEFILE_DIR",
    "CMAKE_GENERATOR",
    "CMAKE_BUILD_TYPE",
    "CMAKE_CXX_COMPILER",
    "CMAKE_CXX_STANDARD",
    "CMAKE_MSVC_RUNTIME_LIBRARY",
    "pxr_DIR",
    "_Python3_INCLUDE_DIR",
];

/// A parsed `CMakeCache.txt`.
#[derive(Debug, Clone, Default)]
pub struct CMakeCache {
    entries: BTreeMap<String, String>,
}

impl CMakeCache {
    /// Parse `KEY:TYPE=VALUE` lines, ignoring comments and blanks.
    pub fn parse(text: &str) -> CMakeCache {
        let mut entries = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
                continue;
            }
            let Some((decl, value)) = line.split_once('=') else {
                continue;
            };
            // `KEY:TYPE` — the type is not part of the identity, only the name.
            let key = decl.split_once(':').map(|(k, _)| k).unwrap_or(decl);
            entries.insert(key.trim().to_string(), value.trim().to_string());
        }
        CMakeCache { entries }
    }

    pub fn load(path: &Utf8Path) -> std::io::Result<CMakeCache> {
        Ok(CMakeCache::parse(&std::fs::read_to_string(
            path.as_std_path(),
        )?))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .get(key)
            .map(|value| value.as_str())
            .filter(|value| !value.is_empty() && !value.ends_with("-NOTFOUND"))
    }

    /// A digest over [`IDENTITY_KEYS`] as this cache holds them.
    ///
    /// Absent keys are encoded as absent rather than skipped, so a tree that
    /// gains or loses one (say, a build that starts pinning the MSVC runtime)
    /// does not collide with the tree that did not.
    pub fn identity_digest(&self) -> String {
        let mut material = String::new();
        for key in IDENTITY_KEYS {
            match self.get(key) {
                Some(value) => material.push_str(&format!("{key}={value}\n")),
                None => material.push_str(&format!("{key}=<absent>\n")),
            }
        }
        ost_core::digest::sha256_hex(material.as_bytes())
    }

    /// The Python version CMake reported, dug out of its find-package details.
    ///
    /// `Python3_VERSION` is not always cached, but the details line that
    /// `find_package` leaves behind carries `v3.13.14(3.13.14)`.
    pub fn python_version(&self) -> Option<String> {
        if let Some(version) = self.get("Python3_VERSION") {
            return Some(version.to_string());
        }
        let details = self.get("FIND_PACKAGE_MESSAGE_DETAILS_Python3")?;
        let start = details.find("[v")? + 2;
        let rest = &details[start..];
        let end = rest.find('(')?;
        Some(rest[..end].to_string())
    }
}

/// The runtime an external tree was built against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalRuntime {
    pub id: String,
    pub digest: String,
    /// The runtime root the tree resolved `pxr` from.
    pub root: String,
}

/// The toolchain identity read out of the cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalToolchain {
    pub generator: String,
    pub configuration: String,
    pub cxx_compiler: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cxx_standard: Option<String>,
    /// The MSVC runtime library (the CRT) when the tree pins one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msvc_runtime: Option<String>,
    /// The Python whose ABI the tree built against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python_include: Option<String>,
}

/// An imported record of a build OpenStrata did not perform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalBuildProvenance {
    pub schema: String,
    pub source_root: String,
    pub build_dir: String,
    pub runtime: ExternalRuntime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openusd_version: Option<String>,
    pub toolchain: ExternalToolchain,
    /// Digest over [`IDENTITY_KEYS`] at import time.
    pub cache_digest: String,
    pub imported_unix: u64,
}

/// Why an external tree could not be imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// The cache lacks an entry the identity cannot be established without.
    MissingEntry(&'static str),
    /// The tree resolved `pxr` from somewhere other than the selected runtime.
    ForeignRuntime { found: String, expected: String },
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::MissingEntry(key) => write!(
                f,
                "CMakeCache.txt has no '{key}' — the build tree's identity cannot be established \
                 from it"
            ),
            ImportError::ForeignRuntime { found, expected } => write!(
                f,
                "the build tree resolved pxr from '{found}', not from the selected runtime at \
                 '{expected}'"
            ),
        }
    }
}

impl ExternalBuildProvenance {
    /// Derive a record from an inspected cache, bound to `runtime`.
    ///
    /// The tree must have resolved `pxr` from the selected runtime's root; a
    /// record that skipped this check would let a tree built against some other
    /// OpenUSD claim compatibility with the one OpenStrata resolves.
    pub fn from_cache(
        cache: &CMakeCache,
        runtime: ExternalRuntime,
        openusd_version: Option<String>,
        imported_unix: u64,
    ) -> Result<ExternalBuildProvenance, ImportError> {
        let source_root = cache
            .get("CMAKE_HOME_DIRECTORY")
            .ok_or(ImportError::MissingEntry("CMAKE_HOME_DIRECTORY"))?;
        let build_dir = cache
            .get("CMAKE_CACHEFILE_DIR")
            .ok_or(ImportError::MissingEntry("CMAKE_CACHEFILE_DIR"))?;
        let generator = cache
            .get("CMAKE_GENERATOR")
            .ok_or(ImportError::MissingEntry("CMAKE_GENERATOR"))?;
        let cxx_compiler = cache
            .get("CMAKE_CXX_COMPILER")
            .ok_or(ImportError::MissingEntry("CMAKE_CXX_COMPILER"))?;
        let pxr_dir = cache
            .get("pxr_DIR")
            .ok_or(ImportError::MissingEntry("pxr_DIR"))?;

        if !same_path(pxr_dir, &runtime.root) {
            return Err(ImportError::ForeignRuntime {
                found: pxr_dir.to_string(),
                expected: runtime.root.clone(),
            });
        }

        Ok(ExternalBuildProvenance {
            schema: EXTERNAL_BUILD_SCHEMA.into(),
            source_root: normalize(source_root),
            build_dir: normalize(build_dir),
            runtime,
            openusd_version,
            toolchain: ExternalToolchain {
                generator: generator.to_string(),
                // A single-config generator with no explicit type is CMake's
                // empty default; record it as such rather than inventing one.
                configuration: cache.get("CMAKE_BUILD_TYPE").unwrap_or("").to_string(),
                cxx_compiler: normalize(cxx_compiler),
                cxx_standard: cache.get("CMAKE_CXX_STANDARD").map(str::to_string),
                msvc_runtime: cache.get("CMAKE_MSVC_RUNTIME_LIBRARY").map(str::to_string),
                python_version: cache.python_version(),
                python_include: cache.get("_Python3_INCLUDE_DIR").map(normalize),
            },
            cache_digest: cache.identity_digest(),
            imported_unix,
        })
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Whether this record still describes the tree and runtime in front of us.
    ///
    /// Every field has to match. A partial match is what makes an external claim
    /// dangerous: a tree reconfigured against a newer runtime looks identical
    /// except in the one place that decides whether its binaries still load.
    pub fn verify_against(
        &self,
        cache: &CMakeCache,
        build_dir: &Utf8Path,
        runtime: &ExternalRuntime,
    ) -> Result<(), String> {
        if self.schema != EXTERNAL_BUILD_SCHEMA {
            return Err(format!(
                "unsupported external build schema '{}'",
                self.schema
            ));
        }
        if !same_path(&self.build_dir, build_dir.as_str()) {
            return Err(format!(
                "provenance describes build directory '{}', not '{build_dir}'",
                self.build_dir
            ));
        }
        if self.cache_digest != cache.identity_digest() {
            return Err(
                "the build tree has been reconfigured since its provenance was imported — \
                 re-run `ost external import`"
                    .into(),
            );
        }
        if self.runtime.id != runtime.id {
            return Err(format!(
                "provenance runtime '{}' != selected runtime '{}'",
                self.runtime.id, runtime.id
            ));
        }
        if self.runtime.digest != runtime.digest {
            return Err(format!(
                "runtime digest drift: imported {} != current {}",
                short(&self.runtime.digest),
                short(&runtime.digest)
            ));
        }
        if !same_path(&self.runtime.root, &runtime.root) {
            return Err(format!(
                "provenance runtime root '{}' != current '{}'",
                self.runtime.root, runtime.root
            ));
        }
        Ok(())
    }

    /// A one-line summary for the `validate` detail column.
    pub fn describe(&self) -> String {
        let configuration = if self.toolchain.configuration.is_empty() {
            "<default>"
        } else {
            &self.toolchain.configuration
        };
        format!(
            "external build imported from {} ({}, {configuration})",
            self.build_dir, self.toolchain.generator
        )
    }
}

fn short(digest: &str) -> String {
    match digest.split_once(':') {
        Some((algo, hex)) => format!("{algo}:{}", &hex[..hex.len().min(12)]),
        None => digest.to_string(),
    }
}

/// CMake writes forward slashes; make ours match so a record round-trips.
fn normalize(path: &str) -> String {
    path.replace('\\', "/")
}

/// Compare two paths as the host's filesystem would.
///
/// CMake is not consistent about drive-letter case even within one cache —
/// `CMAKE_HOME_DIRECTORY` can read `C:/dev/x` while `CMAKE_CACHEFILE_DIR` reads
/// `c:/dev/x/build` — so a case-sensitive comparison would reject a tree that is
/// plainly the same one.
fn same_path(left: &str, right: &str) -> bool {
    let left = normalize(left);
    let right = normalize(right);
    let left = left.trim_end_matches('/');
    let right = right.trim_end_matches('/');
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed but realistic cache: the shapes here (an `INTERNAL` generator,
    /// a lowercase drive letter on one key, a `-NOTFOUND` value) are taken from
    /// a cache CMake actually wrote.
    const CACHE: &str = r#"
# This is the CMakeCache file.
//The directory containing a CMake configuration file for pxr.
pxr_DIR:PATH=C:/Users/x/.ost/runtimes/openstrata-cy2026
CMAKE_BUILD_TYPE:STRING=Release
CMAKE_CXX_COMPILER:FILEPATH=C:/MSVC/bin/cl.exe
CMAKE_CXX_STANDARD:STRING=20
CMAKE_GENERATOR:INTERNAL=Ninja
CMAKE_HOME_DIRECTORY:INTERNAL=C:/dev/project
CMAKE_CACHEFILE_DIR:INTERNAL=c:/dev/project/build
_Python3_INCLUDE_DIR:INTERNAL=C:/Python313/Include
_Python3_CONFIG:INTERNAL=_Python3_CONFIG-NOTFOUND
FIND_PACKAGE_MESSAGE_DETAILS_Python3:INTERNAL=[C:/py.lib][C:/Include][found components: Development ][v3.13.14(3.13.14)]
"#;

    fn runtime() -> ExternalRuntime {
        ExternalRuntime {
            id: "openstrata-cy2026".into(),
            digest: "sha256:abc123def456".into(),
            root: "C:/Users/x/.ost/runtimes/openstrata-cy2026".into(),
        }
    }

    fn imported() -> ExternalBuildProvenance {
        ExternalBuildProvenance::from_cache(
            &CMakeCache::parse(CACHE),
            runtime(),
            Some("26.05".into()),
            100,
        )
        .expect("the cache resolves pxr from the selected runtime")
    }

    #[test]
    fn parses_typed_entries_and_ignores_comments() {
        let cache = CMakeCache::parse(CACHE);
        assert_eq!(cache.get("CMAKE_GENERATOR"), Some("Ninja"));
        assert_eq!(cache.get("CMAKE_BUILD_TYPE"), Some("Release"));
        // A `-NOTFOUND` value is CMake's way of saying "absent", not a value.
        assert_eq!(cache.get("_Python3_CONFIG"), None);
        assert_eq!(cache.get("NOT_IN_CACHE"), None);
    }

    #[test]
    fn python_version_comes_from_the_find_package_details() {
        assert_eq!(
            CMakeCache::parse(CACHE).python_version(),
            Some("3.13.14".into())
        );
    }

    #[test]
    fn import_records_the_toolchain_identity() {
        let record = imported();
        assert_eq!(record.source_root, "C:/dev/project");
        assert_eq!(record.toolchain.generator, "Ninja");
        assert_eq!(record.toolchain.configuration, "Release");
        assert_eq!(record.toolchain.cxx_standard.as_deref(), Some("20"));
        assert_eq!(record.toolchain.python_version.as_deref(), Some("3.13.14"));
        assert_eq!(record.openusd_version.as_deref(), Some("26.05"));
    }

    /// A tree that resolved OpenUSD from somewhere else must not be importable
    /// as evidence about this runtime — that is the whole point of the binding.
    #[test]
    fn a_tree_built_against_another_runtime_is_refused() {
        let mut elsewhere = runtime();
        elsewhere.root = "D:/other/usd".into();
        let error =
            ExternalBuildProvenance::from_cache(&CMakeCache::parse(CACHE), elsewhere, None, 100)
                .expect_err("a foreign pxr root is refused");
        assert!(matches!(error, ImportError::ForeignRuntime { .. }));
    }

    #[test]
    fn a_cache_without_pxr_cannot_be_imported() {
        let error = ExternalBuildProvenance::from_cache(
            &CMakeCache::parse("CMAKE_HOME_DIRECTORY:INTERNAL=/src\n"),
            runtime(),
            None,
            100,
        )
        .expect_err("an incomplete cache is refused");
        assert!(matches!(error, ImportError::MissingEntry(_)));
    }

    #[test]
    fn a_matching_tree_verifies() {
        let record = imported();
        assert!(record
            .verify_against(
                &CMakeCache::parse(CACHE),
                Utf8Path::new("c:/dev/project/build"),
                &runtime()
            )
            .is_ok());
    }

    /// The digest is what makes the record honest over time: reconfiguring the
    /// tree must invalidate it, even though the file on disk is untouched.
    #[test]
    fn reconfiguring_the_tree_invalidates_the_record() {
        let record = imported();
        let reconfigured = CMakeCache::parse(&CACHE.replace("Release", "Debug"));
        let error = record
            .verify_against(
                &reconfigured,
                Utf8Path::new("c:/dev/project/build"),
                &runtime(),
            )
            .expect_err("a reconfigured tree no longer verifies");
        assert!(error.contains("reconfigured"), "{error}");
    }

    /// Runtime drift is the case that decides whether the binaries still load.
    #[test]
    fn runtime_digest_drift_fails_verification() {
        let record = imported();
        let mut current = runtime();
        current.digest = "sha256:999999999999".into();
        let error = record
            .verify_against(
                &CMakeCache::parse(CACHE),
                Utf8Path::new("c:/dev/project/build"),
                &current,
            )
            .expect_err("drift is refused");
        assert!(error.contains("digest drift"), "{error}");
    }

    /// A record copied into a different tree describes neither.
    #[test]
    fn a_record_from_another_build_directory_fails() {
        let record = imported();
        let error = record
            .verify_against(
                &CMakeCache::parse(CACHE),
                Utf8Path::new("c:/dev/project/other-build"),
                &runtime(),
            )
            .expect_err("a foreign build dir is refused");
        assert!(error.contains("build directory"), "{error}");
    }

    /// CMake mixes drive-letter case within one cache, so path comparison has to
    /// tolerate it on Windows without becoming case-blind on Unix.
    #[test]
    fn windows_paths_compare_case_insensitively() {
        if cfg!(windows) {
            assert!(same_path("C:/Dev/Project", "c:/dev/project"));
        } else {
            assert!(!same_path("/Dev/Project", "/dev/project"));
        }
        assert!(same_path("C:/dev/project/", "C:/dev/project"));
        assert!(same_path("C:\\dev\\project", "C:/dev/project"));
    }

    /// Gaining an identity key must change the digest, or a tree that starts
    /// pinning the CRT would verify against a record that predates it.
    #[test]
    fn absent_and_present_identity_keys_digest_differently() {
        let without = CMakeCache::parse(CACHE);
        let with = CMakeCache::parse(&format!(
            "{CACHE}CMAKE_MSVC_RUNTIME_LIBRARY:STRING=MultiThreadedDLL\n"
        ));
        assert_ne!(without.identity_digest(), with.identity_digest());
    }
}
