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
//! CMake itself wrote — and, where the generator requires it, the adjacent
//! `CMakeFiles/<version>/CMakeCXXCompiler.cmake` — and records the identity it
//! finds there: where the sources are, where the tree is, which runtime it
//! resolved `pxr` from, the generator, configuration, compiler and Python it
//! used.
//!
//! The record is only as good as its binding to the tree, so it carries a digest
//! over the applicable cache entries ([`IDENTITY_KEYS`]) and compiler metadata.
//! A tree reconfigured against a different runtime, generator or configuration
//! produces a different digest, and the record stops verifying rather than
//! quietly describing a build that no longer exists.
//!
//! What this buys is narrow and deliberate: on a *full* identity match,
//! `validate --build-dir` may report `runtime-compatible`. It never reports
//! `configured` or `built` — those claim OpenStrata did the work, and it did
//! not.

use std::collections::BTreeMap;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

pub const EXTERNAL_BUILD_FILE: &str = ".ost-external-build.json";
pub const EXTERNAL_BUILD_SCHEMA_V1: &str = "openstrata.external-build/v1";
pub const EXTERNAL_BUILD_SCHEMA: &str = "openstrata.external-build/v2";

/// The CMake cache entries an external build's identity is derived from.
///
/// The set is fixed and listed rather than "every entry": a cache holds hundreds
/// of incidental values (timestamps, per-find-module scratch, absolute paths of
/// tools that do not affect the result) and digesting all of them would make the
/// record fail on changes that mean nothing. These are the entries that decide
/// what was actually produced.
const IDENTITY_KEYS_V1: &[&str] = &[
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

pub const IDENTITY_KEYS: &[&str] = &[
    "CMAKE_HOME_DIRECTORY",
    "CMAKE_CACHEFILE_DIR",
    "CMAKE_GENERATOR",
    "CMAKE_GENERATOR_INSTANCE",
    "CMAKE_GENERATOR_PLATFORM",
    "CMAKE_GENERATOR_TOOLSET",
    "CMAKE_BUILD_TYPE",
    "CMAKE_CONFIGURATION_TYPES",
    "CMAKE_CXX_COMPILER",
    "CMAKE_CXX_STANDARD",
    "CMAKE_MSVC_RUNTIME_LIBRARY",
    "pxr_DIR",
    "_Python3_INCLUDE_DIR",
];

const COMPILER_IDENTITY_KEYS: &[&str] = &[
    "CMAKE_CXX_COMPILER",
    "CMAKE_CXX_COMPILER_ID",
    "CMAKE_CXX_COMPILER_VERSION",
    "CMAKE_CXX_COMPILER_ARCHITECTURE_ID",
    "CMAKE_CXX_SIMULATE_ID",
    "CMAKE_CXX_SIMULATE_VERSION",
    "CMAKE_CXX_COMPILER_FRONTEND_VARIANT",
];

/// A parsed `CMakeCache.txt`.
#[derive(Debug, Clone, Default)]
pub struct CMakeCache {
    entries: BTreeMap<String, String>,
    compiler_identity: Option<CMakeCompilerIdentity>,
}

#[derive(Debug, Clone)]
struct CMakeCompilerIdentity {
    source: String,
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
        CMakeCache {
            entries,
            compiler_identity: None,
        }
    }

    pub fn load(path: &Utf8Path) -> std::io::Result<CMakeCache> {
        let mut cache = CMakeCache::parse(&std::fs::read_to_string(path.as_std_path())?);
        if let Some(build_dir) = path.parent() {
            cache.compiler_identity = load_compiler_identity(build_dir)?;
        }
        Ok(cache)
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
        self.identity_digest_for_scope(true)
    }

    /// Hash the identity entries that apply to this import. A core-only tree
    /// must not go stale merely because an unrelated `pxr_DIR` appeared later.
    pub fn identity_digest_for_scope(&self, requires_openusd: bool) -> String {
        let mut material = String::new();
        for key in IDENTITY_KEYS {
            if !requires_openusd && *key == "pxr_DIR" {
                continue;
            }
            match self.get(key) {
                Some(value) => material.push_str(&format!("{key}={value}\n")),
                None => material.push_str(&format!("{key}=<absent>\n")),
            }
        }
        if let Some(compiler) = &self.compiler_identity {
            material.push_str(&format!("CXX_IDENTITY_SOURCE={}\n", compiler.source));
            for key in COMPILER_IDENTITY_KEYS {
                match compiler.entries.get(*key) {
                    Some(value) => material.push_str(&format!("{key}={value}\n")),
                    None => material.push_str(&format!("{key}=<absent>\n")),
                }
            }
        } else {
            material.push_str("CXX_IDENTITY_SOURCE=<absent>\n");
        }
        ost_core::digest::sha256_hex(material.as_bytes())
    }

    fn identity_digest_v1(&self) -> String {
        let mut material = String::new();
        for key in IDENTITY_KEYS_V1 {
            match self.get(key) {
                Some(value) => material.push_str(&format!("{key}={value}\n")),
                None => material.push_str(&format!("{key}=<absent>\n")),
            }
        }
        ost_core::digest::sha256_hex(material.as_bytes())
    }

    fn cxx_compiler(&self) -> Option<(String, String)> {
        if let Some(value) = self.get("CMAKE_CXX_COMPILER") {
            return Some((normalize(value), "CMakeCache.txt:CMAKE_CXX_COMPILER".into()));
        }
        let compiler = self.compiler_identity.as_ref()?;
        let value = compiler.entries.get("CMAKE_CXX_COMPILER")?;
        Some((normalize(value), compiler.source.clone()))
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

/// The profile and capabilities whose requirements were evaluated at import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalImportScope {
    pub profile: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    pub requires_openusd: bool,
}

impl Default for ExternalImportScope {
    fn default() -> Self {
        // v1 records predate explicit scopes and always required a pxr binding.
        ExternalImportScope {
            profile: String::new(),
            capabilities: Vec::new(),
            requires_openusd: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalRequirementStatus {
    Applied,
    NotApplicable,
}

/// One import precondition and whether the selected scope made it relevant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalRequirement {
    pub name: String,
    pub status: ExternalRequirementStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub detail: String,
}

/// The toolchain identity read out of the cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalToolchain {
    pub generator: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub generator_flavor: String,
    #[serde(default)]
    pub multi_config: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub configurations: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator_instance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator_platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator_toolset: Option<String>,
    pub configuration: String,
    pub cxx_compiler: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cxx_compiler_source: Option<String>,
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
    #[serde(default)]
    pub scope: ExternalImportScope,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requirements: Vec<ExternalRequirement>,
    pub runtime: ExternalRuntime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openusd_version: Option<String>,
    pub toolchain: ExternalToolchain,
    /// Digest over applicable [`IDENTITY_KEYS`] and compiler metadata at import.
    pub cache_digest: String,
    pub imported_unix: u64,
}

/// Why an external tree could not be imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// The cache lacks an entry the identity cannot be established without.
    MissingIdentity {
        field: &'static str,
        generator: String,
        sources: Vec<String>,
    },
    /// The tree resolved `pxr` from somewhere other than the selected runtime.
    ForeignRuntime { found: String, expected: String },
    /// A multi-config generator requires the concrete built configuration.
    ConfigurationRequired {
        generator: String,
        available: Vec<String>,
    },
    /// The requested configuration is not selectable in this build tree.
    ConfigurationUnavailable {
        generator: String,
        requested: String,
        available: Vec<String>,
    },
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::MissingIdentity {
                field,
                generator,
                sources,
            } => write!(
                f,
                "{generator} build tree does not expose {field}; inspected {}",
                sources.join(" and ")
            ),
            ImportError::ForeignRuntime { found, expected } => write!(
                f,
                "the build tree resolved pxr from '{found}', not from the selected runtime at \
                 '{expected}'"
            ),
            ImportError::ConfigurationRequired {
                generator,
                available,
            } => write!(
                f,
                "{generator} is multi-config; select the configuration whose binaries this provenance describes with --config (available: {})",
                available.join(", ")
            ),
            ImportError::ConfigurationUnavailable {
                generator,
                requested,
                available,
            } => write!(
                f,
                "configuration '{requested}' is not available in {generator} (available: {})",
                available.join(", ")
            ),
        }
    }
}

impl ImportError {
    /// A remediation that can change this exact outcome. Compiler discovery
    /// failures deliberately do not send the user toward an unrelated pxr root.
    pub fn remediation(&self) -> String {
        match self {
            ImportError::MissingIdentity { field, .. } if *field == "C++ compiler identity" => {
                "finish configuring the tree with C++ enabled so CMake writes \
                 CMakeFiles/<version>/CMakeCXXCompiler.cmake, then re-run the import"
                    .into()
            }
            ImportError::MissingIdentity { field, .. } if *field == "OpenUSD runtime binding" => {
                "configure this OpenUSD-dependent tree against the selected runtime \
                 (pass its prefix as pxr_ROOT), then re-run the import"
                    .into()
            }
            ImportError::MissingIdentity { .. } => {
                "point --build-dir at a completed CMake configure tree, then re-run the import"
                    .into()
            }
            ImportError::ForeignRuntime { .. } => {
                "select the exact runtime used by this tree, or reconfigure it against the \
                 selected runtime before importing"
                    .into()
            }
            ImportError::ConfigurationRequired { .. }
            | ImportError::ConfigurationUnavailable { .. } => {
                "pass --config with one configuration listed by the CMake build tree".into()
            }
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
        scope: ExternalImportScope,
        imported_unix: u64,
    ) -> Result<ExternalBuildProvenance, ImportError> {
        Self::from_cache_for_configuration(
            cache,
            runtime,
            openusd_version,
            scope,
            imported_unix,
            None,
        )
    }

    /// Derive provenance for one concrete single- or multi-config output.
    pub fn from_cache_for_configuration(
        cache: &CMakeCache,
        runtime: ExternalRuntime,
        openusd_version: Option<String>,
        scope: ExternalImportScope,
        imported_unix: u64,
        selected_configuration: Option<&str>,
    ) -> Result<ExternalBuildProvenance, ImportError> {
        let detected_generator = cache.get("CMAKE_GENERATOR").unwrap_or("unresolved");
        let source_root = cache.get("CMAKE_HOME_DIRECTORY").ok_or_else(|| {
            missing_identity(
                "source root",
                detected_generator,
                &["CMakeCache.txt:CMAKE_HOME_DIRECTORY"],
            )
        })?;
        let build_dir = cache.get("CMAKE_CACHEFILE_DIR").ok_or_else(|| {
            missing_identity(
                "build directory",
                detected_generator,
                &["CMakeCache.txt:CMAKE_CACHEFILE_DIR"],
            )
        })?;
        let generator = cache.get("CMAKE_GENERATOR").ok_or_else(|| {
            missing_identity(
                "generator identity",
                "unresolved",
                &["CMakeCache.txt:CMAKE_GENERATOR"],
            )
        })?;
        let (generator_flavor, multi_config) = classify_generator(generator, cache);
        let generator_diagnostic = format!("{generator} ({generator_flavor})");
        let (cxx_compiler, cxx_compiler_source) = cache.cxx_compiler().ok_or_else(|| {
            missing_identity(
                "C++ compiler identity",
                &generator_diagnostic,
                &[
                    "CMakeCache.txt:CMAKE_CXX_COMPILER",
                    "CMakeFiles/<version>/CMakeCXXCompiler.cmake",
                ],
            )
        })?;

        let mut requirements = vec![ExternalRequirement {
            name: "cmake.cxx-compiler".into(),
            status: ExternalRequirementStatus::Applied,
            source: Some(cxx_compiler_source.clone()),
            detail: format!("resolved C++ compiler identity for {generator_diagnostic}"),
        }];
        if scope.requires_openusd {
            let pxr_dir = cache.get("pxr_DIR").ok_or_else(|| {
                missing_identity(
                    "OpenUSD runtime binding",
                    &generator_diagnostic,
                    &["CMakeCache.txt:pxr_DIR"],
                )
            })?;
            if !path_is_within(pxr_dir, &runtime.root) {
                return Err(ImportError::ForeignRuntime {
                    found: pxr_dir.to_string(),
                    expected: runtime.root.clone(),
                });
            }
            requirements.push(ExternalRequirement {
                name: "openusd.runtime".into(),
                status: ExternalRequirementStatus::Applied,
                source: Some("CMakeCache.txt:pxr_DIR".into()),
                detail: "selected capabilities require OpenUSD and the tree resolves the selected runtime"
                    .into(),
            });
        } else {
            requirements.push(ExternalRequirement {
                name: "openusd.runtime".into(),
                status: ExternalRequirementStatus::NotApplicable,
                source: None,
                detail: format!(
                    "profile '{}' and requested capabilities exercise no OpenUSD-dependent capability",
                    scope.profile
                ),
            });
        }

        let configurations = if multi_config {
            cache
                .get("CMAKE_CONFIGURATION_TYPES")
                .unwrap_or("")
                .split(';')
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        } else {
            Vec::new()
        };
        let configuration = if multi_config {
            let selected = selected_configuration
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| ImportError::ConfigurationRequired {
                    generator: generator_diagnostic.clone(),
                    available: configurations.clone(),
                })?;
            if !configurations.iter().any(|value| value == selected) {
                return Err(ImportError::ConfigurationUnavailable {
                    generator: generator_diagnostic,
                    requested: selected.to_string(),
                    available: configurations,
                });
            }
            selected.to_string()
        } else {
            let configured = cache.get("CMAKE_BUILD_TYPE").unwrap_or("").to_string();
            if let Some(selected) = selected_configuration.filter(|value| !value.trim().is_empty())
            {
                if !configured.is_empty() && configured != selected {
                    return Err(ImportError::ConfigurationUnavailable {
                        generator: generator_diagnostic,
                        requested: selected.to_string(),
                        available: vec![configured],
                    });
                }
            }
            configured
        };
        let cache_digest = cache.identity_digest_for_scope(scope.requires_openusd);

        Ok(ExternalBuildProvenance {
            schema: EXTERNAL_BUILD_SCHEMA.into(),
            source_root: normalize(source_root),
            build_dir: normalize(build_dir),
            scope,
            requirements,
            runtime,
            openusd_version,
            toolchain: ExternalToolchain {
                generator: generator.to_string(),
                generator_flavor,
                multi_config,
                configurations,
                generator_instance: cache.get("CMAKE_GENERATOR_INSTANCE").map(normalize),
                generator_platform: cache.get("CMAKE_GENERATOR_PLATFORM").map(str::to_string),
                generator_toolset: cache.get("CMAKE_GENERATOR_TOOLSET").map(str::to_string),
                configuration,
                cxx_compiler,
                cxx_compiler_source: Some(cxx_compiler_source),
                cxx_standard: cache.get("CMAKE_CXX_STANDARD").map(str::to_string),
                msvc_runtime: cache.get("CMAKE_MSVC_RUNTIME_LIBRARY").map(str::to_string),
                python_version: cache.python_version(),
                python_include: cache.get("_Python3_INCLUDE_DIR").map(normalize),
            },
            cache_digest,
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
        if self.schema != EXTERNAL_BUILD_SCHEMA && self.schema != EXTERNAL_BUILD_SCHEMA_V1 {
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
        let cache_digest = if self.schema == EXTERNAL_BUILD_SCHEMA_V1 {
            cache.identity_digest_v1()
        } else {
            cache.identity_digest_for_scope(self.scope.requires_openusd)
        };
        if self.cache_digest != cache_digest {
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
        let configuration = if self.toolchain.multi_config {
            if self.toolchain.configurations.is_empty() {
                "multi-config".into()
            } else {
                format!("multi-config: {}", self.toolchain.configurations.join(", "))
            }
        } else if self.toolchain.configuration.is_empty() {
            "<default>".into()
        } else {
            self.toolchain.configuration.clone()
        };
        let openusd = self
            .requirements
            .iter()
            .find(|requirement| requirement.name == "openusd.runtime")
            .map(|requirement| match requirement.status {
                ExternalRequirementStatus::Applied => "OpenUSD binding applied",
                ExternalRequirementStatus::NotApplicable => "OpenUSD binding not applicable",
            })
            .unwrap_or("OpenUSD binding imported by legacy record");
        let flavor = if self.toolchain.generator_flavor.is_empty() {
            self.toolchain.generator.as_str()
        } else {
            self.toolchain.generator_flavor.as_str()
        };
        format!(
            "external build imported from {} ({flavor}, {configuration}; {openusd})",
            self.build_dir
        )
    }
}

fn missing_identity(field: &'static str, generator: &str, sources: &[&str]) -> ImportError {
    ImportError::MissingIdentity {
        field,
        generator: generator.into(),
        sources: sources.iter().map(|source| (*source).into()).collect(),
    }
}

fn classify_generator(generator: &str, cache: &CMakeCache) -> (String, bool) {
    match generator {
        "Ninja" => ("ninja".into(), false),
        "Ninja Multi-Config" => ("ninja-multi-config".into(), true),
        "Xcode" => ("xcode".into(), true),
        value if value.starts_with("Visual Studio ") => ("visual-studio".into(), true),
        _ if cache.get("CMAKE_CONFIGURATION_TYPES").is_some() => {
            ("other-multi-config".into(), true)
        }
        _ => ("other-single-config".into(), false),
    }
}

/// Read compiler identity from the generator-neutral file CMake writes after
/// compiler detection. Visual Studio commonly omits the same value from the
/// top-level cache, while Ninja usually provides both sources.
fn load_compiler_identity(build_dir: &Utf8Path) -> std::io::Result<Option<CMakeCompilerIdentity>> {
    let cmake_files = build_dir.join("CMakeFiles");
    let entries = match std::fs::read_dir(cmake_files.as_std_path()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path().join("CMakeCXXCompiler.cmake");
        if path.is_file() {
            if let Ok(path) = Utf8PathBuf::from_path_buf(path) {
                candidates.push(path);
            }
        }
    }
    candidates.sort();
    let Some(path) = candidates.pop() else {
        return Ok(None);
    };
    let contents = std::fs::read_to_string(path.as_std_path())?;
    let source = path
        .strip_prefix(build_dir)
        .unwrap_or(&path)
        .as_str()
        .replace('\\', "/");
    Ok(Some(CMakeCompilerIdentity {
        source: format!("{source}:CMAKE_CXX_COMPILER"),
        entries: parse_cmake_sets(&contents),
    }))
}

fn parse_cmake_sets(contents: &str) -> BTreeMap<String, String> {
    let mut entries = BTreeMap::new();
    for line in contents.lines() {
        let line = line.trim();
        let Some(inner) = line
            .strip_prefix("set(")
            .and_then(|line| line.strip_suffix(')'))
        else {
            continue;
        };
        let Some(split) = inner.find(char::is_whitespace) else {
            continue;
        };
        let key = inner[..split].trim();
        let mut value = inner[split..].trim();
        if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
            value = &value[1..value.len() - 1];
        }
        if !key.is_empty() && !value.is_empty() {
            entries.insert(key.into(), value.into());
        }
    }
    entries
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

/// `pxr_DIR` normally names `<runtime>/lib/cmake/pxr`, while callers select the
/// install prefix itself. Require component-aware containment so a sibling such
/// as `/usd-old` cannot masquerade as `/usd`.
fn path_is_within(path: &str, root: &str) -> bool {
    let path = normalize(path);
    let root = normalize(root);
    let path = path.trim_end_matches('/');
    let root = root.trim_end_matches('/');
    if same_path(path, root) {
        return true;
    }
    if path.len() <= root.len() || path.as_bytes().get(root.len()) != Some(&b'/') {
        return false;
    }
    let prefix = &path[..root.len()];
    if cfg!(windows) {
        prefix.eq_ignore_ascii_case(root)
    } else {
        prefix == root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A trimmed but realistic cache: the shapes here (an `INTERNAL` generator,
    /// a lowercase drive letter on one key, a `-NOTFOUND` value) are taken from
    /// a cache CMake actually wrote.
    const CACHE: &str = r#"
# This is the CMakeCache file.
//The directory containing a CMake configuration file for pxr.
pxr_DIR:PATH=C:/Users/x/.ost/runtimes/openstrata-cy2026/lib/cmake/pxr
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

    fn usd_scope() -> ExternalImportScope {
        ExternalImportScope {
            profile: "usd".into(),
            capabilities: vec!["usd-stage-read".into()],
            requires_openusd: true,
        }
    }

    fn core_scope() -> ExternalImportScope {
        ExternalImportScope {
            profile: "core".into(),
            capabilities: vec!["build-cxx".into()],
            requires_openusd: false,
        }
    }

    fn temporary_tree(tag: &str) -> Utf8PathBuf {
        static SEQUENCE: AtomicU32 = AtomicU32::new(0);
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ost-external-{tag}-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Utf8PathBuf::from_path_buf(path).unwrap()
    }

    fn imported() -> ExternalBuildProvenance {
        ExternalBuildProvenance::from_cache(
            &CMakeCache::parse(CACHE),
            runtime(),
            Some("26.05".into()),
            usd_scope(),
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
        assert_eq!(record.toolchain.generator_flavor, "ninja");
        assert!(!record.toolchain.multi_config);
        assert_eq!(record.toolchain.configuration, "Release");
        assert_eq!(record.toolchain.cxx_standard.as_deref(), Some("20"));
        assert_eq!(record.toolchain.python_version.as_deref(), Some("3.13.14"));
        assert_eq!(record.openusd_version.as_deref(), Some("26.05"));
    }

    #[test]
    fn multi_config_generators_record_selectable_configurations() {
        for (generator, flavor) in [
            ("Ninja Multi-Config", "ninja-multi-config"),
            ("Xcode", "xcode"),
        ] {
            let cache = CMakeCache::parse(&format!(
                "CMAKE_HOME_DIRECTORY:INTERNAL=/src\n\
                 CMAKE_CACHEFILE_DIR:INTERNAL=/build\n\
                 CMAKE_GENERATOR:INTERNAL={generator}\n\
                 CMAKE_CONFIGURATION_TYPES:STRING=Debug;Release;RelWithDebInfo\n\
                 CMAKE_CXX_COMPILER:FILEPATH=/usr/bin/c++\n\
                 pxr_DIR:PATH={}\n",
                runtime().root
            ));
            let record = ExternalBuildProvenance::from_cache_for_configuration(
                &cache,
                runtime(),
                None,
                usd_scope(),
                100,
                Some("Release"),
            )
            .unwrap();
            assert_eq!(record.toolchain.generator_flavor, flavor);
            assert!(record.toolchain.multi_config);
            assert_eq!(record.toolchain.configuration, "Release");
            assert_eq!(
                record.toolchain.configurations,
                ["Debug", "Release", "RelWithDebInfo"]
            );
        }
    }

    #[test]
    fn multi_config_import_requires_one_available_configuration() {
        let cache = CMakeCache::parse(&format!(
            "CMAKE_HOME_DIRECTORY:INTERNAL=/src\n\
             CMAKE_CACHEFILE_DIR:INTERNAL=/build\n\
             CMAKE_GENERATOR:INTERNAL=Ninja Multi-Config\n\
             CMAKE_CONFIGURATION_TYPES:STRING=Debug;Release\n\
             CMAKE_CXX_COMPILER:FILEPATH=/usr/bin/c++\n\
             pxr_DIR:PATH={}\n",
            runtime().root
        ));
        assert!(matches!(
            ExternalBuildProvenance::from_cache(&cache, runtime(), None, usd_scope(), 100),
            Err(ImportError::ConfigurationRequired { .. })
        ));
        assert!(matches!(
            ExternalBuildProvenance::from_cache_for_configuration(
                &cache,
                runtime(),
                None,
                usd_scope(),
                100,
                Some("MinSizeRel"),
            ),
            Err(ImportError::ConfigurationUnavailable { .. })
        ));
    }

    #[test]
    fn visual_studio_compiler_identity_comes_from_cmakefiles() {
        let build_dir = temporary_tree("visual-studio");
        let portable_build = normalize(build_dir.as_str());
        std::fs::write(
            build_dir.join("CMakeCache.txt").as_std_path(),
            format!(
                "CMAKE_HOME_DIRECTORY:INTERNAL=C:/dev/project\n\
                 CMAKE_CACHEFILE_DIR:INTERNAL={portable_build}\n\
                 CMAKE_GENERATOR:INTERNAL=Visual Studio 17 2022\n\
                 CMAKE_GENERATOR_INSTANCE:INTERNAL=C:/Program Files/Microsoft Visual Studio/2022/Community\n\
                 CMAKE_GENERATOR_PLATFORM:INTERNAL=x64\n\
                 CMAKE_GENERATOR_TOOLSET:INTERNAL=v143\n\
                 CMAKE_CONFIGURATION_TYPES:STRING=Debug;Release\n\
                 pxr_DIR:PATH={}\n",
                runtime().root
            ),
        )
        .unwrap();
        let compiler_dir = build_dir.join("CMakeFiles/3.31.0");
        std::fs::create_dir_all(compiler_dir.as_std_path()).unwrap();
        let compiler_path = compiler_dir.join("CMakeCXXCompiler.cmake");
        std::fs::write(
            compiler_path.as_std_path(),
            "set(CMAKE_CXX_COMPILER \"C:/MSVC/bin/cl.exe\")\n\
             set(CMAKE_CXX_COMPILER_ID \"MSVC\")\n\
             set(CMAKE_CXX_COMPILER_VERSION \"19.43\")\n",
        )
        .unwrap();

        let cache = CMakeCache::load(&build_dir.join("CMakeCache.txt")).unwrap();
        let digest_before = cache.identity_digest();
        let record = ExternalBuildProvenance::from_cache_for_configuration(
            &cache,
            runtime(),
            None,
            usd_scope(),
            100,
            Some("Release"),
        )
        .unwrap();
        assert_eq!(record.toolchain.generator_flavor, "visual-studio");
        assert!(record.toolchain.multi_config);
        assert_eq!(record.toolchain.cxx_compiler, "C:/MSVC/bin/cl.exe");
        assert_eq!(
            record.toolchain.cxx_compiler_source.as_deref(),
            Some("CMakeFiles/3.31.0/CMakeCXXCompiler.cmake:CMAKE_CXX_COMPILER")
        );
        assert_eq!(record.toolchain.generator_platform.as_deref(), Some("x64"));
        assert_eq!(record.toolchain.generator_toolset.as_deref(), Some("v143"));

        std::fs::write(
            compiler_path.as_std_path(),
            "set(CMAKE_CXX_COMPILER \"C:/MSVC/bin/cl.exe\")\n\
             set(CMAKE_CXX_COMPILER_ID \"MSVC\")\n\
             set(CMAKE_CXX_COMPILER_VERSION \"19.44\")\n",
        )
        .unwrap();
        let changed = CMakeCache::load(&build_dir.join("CMakeCache.txt")).unwrap();
        assert_ne!(digest_before, changed.identity_digest());
        std::fs::remove_dir_all(build_dir.as_std_path()).unwrap();
    }

    #[test]
    fn core_scope_records_openusd_as_not_applicable() {
        let cache = CMakeCache::parse(
            "CMAKE_HOME_DIRECTORY:INTERNAL=/src\n\
             CMAKE_CACHEFILE_DIR:INTERNAL=/build\n\
             CMAKE_GENERATOR:INTERNAL=Ninja\n\
             CMAKE_CXX_COMPILER:FILEPATH=/usr/bin/c++\n",
        );
        let record =
            ExternalBuildProvenance::from_cache(&cache, runtime(), None, core_scope(), 100)
                .expect("core does not require a pxr cache entry");
        let openusd = record
            .requirements
            .iter()
            .find(|requirement| requirement.name == "openusd.runtime")
            .unwrap();
        assert_eq!(openusd.status, ExternalRequirementStatus::NotApplicable);
        assert!(record.describe().contains("not applicable"));
    }

    #[test]
    fn missing_compiler_diagnostic_names_generator_and_sources() {
        let cache = CMakeCache::parse(
            "CMAKE_HOME_DIRECTORY:INTERNAL=/src\n\
             CMAKE_CACHEFILE_DIR:INTERNAL=/build\n\
             CMAKE_GENERATOR:INTERNAL=Visual Studio 17 2022\n",
        );
        let error = ExternalBuildProvenance::from_cache(&cache, runtime(), None, core_scope(), 100)
            .unwrap_err();
        let detail = error.to_string();
        assert!(detail.contains("Visual Studio 17 2022 (visual-studio)"));
        assert!(detail.contains("CMakeFiles/<version>/CMakeCXXCompiler.cmake"));
        assert!(!error.remediation().contains("pxr"));
    }

    /// A tree that resolved OpenUSD from somewhere else must not be importable
    /// as evidence about this runtime — that is the whole point of the binding.
    #[test]
    fn a_tree_built_against_another_runtime_is_refused() {
        let mut elsewhere = runtime();
        elsewhere.root = "D:/other/usd".into();
        let error = ExternalBuildProvenance::from_cache(
            &CMakeCache::parse(CACHE),
            elsewhere,
            None,
            usd_scope(),
            100,
        )
        .expect_err("a foreign pxr root is refused");
        assert!(matches!(error, ImportError::ForeignRuntime { .. }));
    }

    #[test]
    fn a_cache_without_pxr_cannot_be_imported() {
        let error = ExternalBuildProvenance::from_cache(
            &CMakeCache::parse(
                "CMAKE_HOME_DIRECTORY:INTERNAL=/src\n\
                 CMAKE_CACHEFILE_DIR:INTERNAL=/build\n\
                 CMAKE_GENERATOR:INTERNAL=Ninja\n\
                 CMAKE_CXX_COMPILER:FILEPATH=/usr/bin/c++\n",
            ),
            runtime(),
            None,
            usd_scope(),
            100,
        )
        .expect_err("an incomplete cache is refused");
        assert!(matches!(error, ImportError::MissingIdentity { .. }));
        assert!(error.to_string().contains("OpenUSD runtime binding"));
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

    #[test]
    fn legacy_v1_records_remain_readable_and_verifiable() {
        let cache = CMakeCache::parse(CACHE);
        let mut record = imported();
        record.schema = EXTERNAL_BUILD_SCHEMA_V1.into();
        record.cache_digest = cache.identity_digest_v1();
        let mut value = serde_json::to_value(record).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("scope");
        object.remove("requirements");
        let toolchain = object["toolchain"].as_object_mut().unwrap();
        for field in [
            "generator_flavor",
            "multi_config",
            "configurations",
            "generator_instance",
            "generator_platform",
            "generator_toolset",
            "cxx_compiler_source",
        ] {
            toolchain.remove(field);
        }
        let decoded: ExternalBuildProvenance = serde_json::from_value(value).unwrap();
        assert!(decoded.scope.requires_openusd);
        assert!(decoded
            .verify_against(&cache, Utf8Path::new("c:/dev/project/build"), &runtime())
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

    #[test]
    fn pxr_config_directory_must_be_inside_the_runtime_prefix() {
        assert!(path_is_within("C:/runtime/lib/cmake/pxr", "C:/runtime"));
        assert!(path_is_within("C:/runtime", "C:/runtime"));
        assert!(!path_is_within(
            "C:/runtime-old/lib/cmake/pxr",
            "C:/runtime"
        ));
        assert!(!path_is_within("C:/other/lib/cmake/pxr", "C:/runtime"));
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
