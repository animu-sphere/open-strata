// SPDX-License-Identifier: Apache-2.0
//! The project manifest: `openstrata.toml`.
//!
//! Capabilities are requested by *what they do*, not by package name (§3.5).
//! A project pins a platform year and a profile, and may request additional
//! capabilities and named extensions on top of that profile.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use camino::Utf8Path;
use ost_core::paths::PROJECT_MANIFEST;
use ost_core::{Error, Result};

/// `[project]` table — identity and metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectMeta {
    pub name: String,
    /// Inline project version. Exactly one of this and `version_file` is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Repo-relative, single-line authoritative version file.
    #[serde(
        default,
        alias = "version-file",
        skip_serializing_if = "Option::is_none"
    )]
    pub version_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn default_version() -> String {
    "0.1.0".into()
}

/// `[requires]` table — the runtime contract this project builds against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requires {
    /// Platform calendar-year id, e.g. `cy2026`.
    pub platform: String,
    /// Profile name, e.g. `usd` or `lookdev`.
    #[serde(default = "default_profile")]
    pub profile: String,
    /// Extra capabilities beyond those implied by the profile (§4.5).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// Named certified extensions to include (§4.4).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
}

fn default_profile() -> String {
    "core".into()
}

/// `[build]` table — how the project compiles (§ runtime/compiler split).
///
/// The runtime supplies the SDK/ABI/prefix; the compiler is chosen separately so
/// an adopted OpenUSD install can build with the host compiler. Defaults to the
/// `host` policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildConfig {
    /// Compiler policy: `host` (default), `runtime`, or `explicit`.
    #[serde(default = "default_compiler")]
    pub compiler: String,
    /// C compiler absolute path (required when `compiler = "explicit"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cc: Option<String>,
    /// C++ compiler absolute path (required when `compiler = "explicit"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cxx: Option<String>,
    /// Project-owned, named build configurations selected with `ost build
    /// --intent <name>`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub intents: BTreeMap<String, BuildIntentConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildIntentConfig {
    #[serde(default)]
    pub cache: BTreeMap<String, BuildCacheEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BuildCacheType {
    Bool,
    String,
    Path,
    Filepath,
}

impl BuildCacheType {
    pub fn is_path(self) -> bool {
        matches!(self, Self::Path | Self::Filepath)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuildPathPortability {
    Portable,
    LocalOverride,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BuildCacheValue {
    Bool(bool),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildCacheEntry {
    #[serde(rename = "type")]
    pub kind: BuildCacheType,
    pub value: BuildCacheValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portability: Option<BuildPathPortability>,
}

fn default_compiler() -> String {
    "host".into()
}

/// `[host]` table — third-party DCC host support.
///
/// "Host" here is a *DCC install already on the machine* (Maya, Houdini), not
/// the machine itself ([`ost_core::Host`]). OpenStrata never installs, mutates,
/// or licenses one; this table only says where a site keeps them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<HostDiscoveryConfig>,
}

/// `[host.discovery]` table — declarative, bounded discovery roots.
///
/// Declarative only: no globs are executed, no shell is sourced, and no rule
/// may name a filesystem root. Depth is bounded so a mistyped root cannot turn
/// into a whole-disk walk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostDiscoveryConfig {
    /// Absolute site directories holding host installs, e.g. `/tools/maya`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<String>,
    /// How far below each root an install may sit (1..=[`MAX_DISCOVERY_DEPTH`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u8>,
    /// Restrict discovery to these host families; empty means every family this
    /// `ost` version knows. Family names are validated by `ost-host`, which owns
    /// the list, so an unknown name fails there naming the supported ones.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub families: Vec<String>,
}

/// The deepest a configured discovery root may be walked.
///
/// Site layouts nest a version below a product below a root
/// (`/tools/maya/2025.3`, `/tools/dcc/maya/2025.3`); beyond that a root is
/// almost certainly wrong, and the cost of finding out is a filesystem walk.
pub const MAX_DISCOVERY_DEPTH: u8 = 4;

/// The depth used when `[host.discovery]` declares roots but no `max_depth`.
pub const DEFAULT_DISCOVERY_DEPTH: u8 = 2;

/// `[workspace]` table — the source members OpenStrata composes.
///
/// Member patterns are portable, project-relative directory globs. `*` and
/// `?` match within one path component; recursive `**` patterns are forbidden
/// so discovery remains bounded by the declaration itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfig {
    pub members: Vec<String>,
}

/// Maximum member nesting accepted by explicit and fallback workspace discovery.
pub const MAX_WORKSPACE_MEMBER_DEPTH: usize = 8;

impl Default for BuildConfig {
    fn default() -> Self {
        BuildConfig {
            compiler: default_compiler(),
            cc: None,
            cxx: None,
            intents: BTreeMap::new(),
        }
    }
}

/// The whole `openstrata.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub project: ProjectMeta,
    pub requires: Requires,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<BuildConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<HostConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceConfig>,
}

impl Project {
    /// A sensible starter manifest for `ost init`.
    pub fn scaffold(name: impl Into<String>, platform: impl Into<String>) -> Project {
        Project {
            project: ProjectMeta {
                name: name.into(),
                version: Some(default_version()),
                version_file: None,
                description: None,
            },
            requires: Requires {
                platform: platform.into(),
                profile: "usd".into(),
                capabilities: Vec::new(),
                extensions: Vec::new(),
            },
            build: None,
            host: None,
            workspace: None,
        }
    }

    pub fn from_toml(src: &str) -> Result<Project> {
        let mut project: Project = toml::from_str(src).map_err(|error| {
            if error.to_string().contains("unknown field") {
                Error::InvalidManifest(format!(
                    "{PROJECT_MANIFEST} uses a key unknown to this ost version: {error}"
                ))
            } else {
                Error::parse(PROJECT_MANIFEST, anyhow::Error::new(error))
            }
        })?;
        if project.project.version.is_none() && project.project.version_file.is_none() {
            project.project.version = Some(default_version());
        }
        project.validate_version_source()?;
        project.validate_build_intents()?;
        project.validate_host_discovery()?;
        project.validate_workspace()?;
        Ok(project)
    }

    pub fn to_toml(&self) -> Result<String> {
        self.validate_version_source()?;
        self.validate_workspace()?;
        toml::to_string_pretty(self)
            .map_err(|e| Error::parse(PROJECT_MANIFEST, anyhow::Error::new(e)))
    }

    /// Resolve the authoritative project version. A version file avoids
    /// forcing adopted projects to duplicate their existing release source.
    pub fn effective_version(&self, root: &Utf8Path) -> Result<String> {
        self.validate_version_source()?;
        if let Some(version) = &self.project.version {
            return Ok(version.clone());
        }
        let relative = self.project.version_file.as_deref().expect("validated");
        let path = root.join(relative);
        let source = std::fs::read_to_string(path.as_std_path())
            .map_err(|error| Error::io(path.to_string(), error))?;
        let version = source.trim();
        if version.is_empty() || version.lines().count() != 1 {
            return Err(Error::config(format!(
                "project.version_file '{relative}' must contain one non-empty line"
            )));
        }
        Ok(version.to_string())
    }

    fn validate_version_source(&self) -> Result<()> {
        match (&self.project.version, &self.project.version_file) {
            (Some(version), None) if !version.trim().is_empty() => Ok(()),
            (None, Some(path)) if safe_relative_file(path) => Ok(()),
            (Some(_), Some(_)) => Err(Error::config(
                "[project] must declare either version or version_file, not both",
            )),
            (None, Some(path)) => Err(Error::config(format!(
                "project.version_file '{path}' must be a safe repo-relative path"
            ))),
            _ => Err(Error::config(
                "[project] must declare a non-empty version or version_file",
            )),
        }
    }

    fn validate_build_intents(&self) -> Result<()> {
        let Some(build) = &self.build else {
            return Ok(());
        };
        for (name, intent) in &build.intents {
            if name == "default" {
                return Err(Error::InvalidManifest(
                    "build intent name 'default' is reserved by ost".into(),
                ));
            }
            if !safe_intent_name(name) {
                return Err(Error::InvalidManifest(format!(
                    "build intent name '{name}' must match [A-Za-z0-9][A-Za-z0-9._-]*"
                )));
            }
            for (variable, entry) in &intent.cache {
                if !safe_cache_variable(variable) {
                    return Err(Error::InvalidManifest(format!(
                        "build.intents.{name}.cache key '{variable}' is not a safe CMake cache variable"
                    )));
                }
                if matches!(variable.as_str(), "CMAKE_BUILD_TYPE" | "CMAKE_MAKE_PROGRAM") {
                    return Err(Error::InvalidManifest(format!(
                        "build.intents.{name}.cache.{variable} is owned by ost; use --config or --ninja"
                    )));
                }
                match (entry.kind, &entry.value) {
                    (BuildCacheType::Bool, BuildCacheValue::Bool(_)) => {}
                    (BuildCacheType::Bool, _) => {
                        return Err(Error::InvalidManifest(format!(
                            "build.intents.{name}.cache.{variable}.value must be a TOML boolean for type BOOL"
                        )));
                    }
                    (_, BuildCacheValue::String(value))
                        if !entry.kind.is_path() || !value.is_empty() => {}
                    (_, _) => {
                        return Err(Error::InvalidManifest(format!(
                            "build.intents.{name}.cache.{variable}.value must be a non-empty TOML string for type {:?}",
                            entry.kind
                        )));
                    }
                }
                if entry.kind.is_path() && entry.portability.is_none() {
                    return Err(Error::InvalidManifest(format!(
                        "build.intents.{name}.cache.{variable}.portability is required for PATH/FILEPATH inputs (portable or local-override)"
                    )));
                }
                if !entry.kind.is_path() && entry.portability.is_some() {
                    return Err(Error::InvalidManifest(format!(
                        "build.intents.{name}.cache.{variable}.portability is only valid for PATH/FILEPATH inputs"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Reject a discovery declaration that could turn into an unbounded or
    /// executed scan before any filesystem is touched.
    fn validate_host_discovery(&self) -> Result<()> {
        let Some(discovery) = self.host.as_ref().and_then(|host| host.discovery.as_ref()) else {
            return Ok(());
        };
        for root in &discovery.roots {
            let reason = match discovery_root_problem(root) {
                None => continue,
                Some(reason) => reason,
            };
            return Err(Error::InvalidManifest(format!(
                "host.discovery.roots entry '{root}' {reason}"
            )));
        }
        if let Some(depth) = discovery.max_depth {
            if depth == 0 || depth > MAX_DISCOVERY_DEPTH {
                return Err(Error::InvalidManifest(format!(
                    "host.discovery.max_depth must be between 1 and {MAX_DISCOVERY_DEPTH} (got {depth})"
                )));
            }
        }
        for family in &discovery.families {
            if !safe_family_name(family) {
                return Err(Error::InvalidManifest(format!(
                    "host.discovery.families entry '{family}' must match [a-z][a-z0-9-]*"
                )));
            }
        }
        Ok(())
    }

    fn validate_workspace(&self) -> Result<()> {
        let Some(workspace) = &self.workspace else {
            return Ok(());
        };
        if workspace.members.is_empty() {
            return Err(Error::InvalidManifest(
                "workspace.members must declare at least one member pattern".into(),
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for member in &workspace.members {
            if let Some(reason) = workspace_member_problem(member) {
                return Err(Error::InvalidManifest(format!(
                    "workspace.members entry '{member}' {reason}"
                )));
            }
            if !seen.insert(member) {
                return Err(Error::InvalidManifest(format!(
                    "workspace.members entry '{member}' is duplicated"
                )));
            }
        }
        Ok(())
    }
}

fn workspace_member_problem(member: &str) -> Option<&'static str> {
    if member.is_empty() {
        return Some("must not be empty");
    }
    if member.trim() != member {
        return Some("must not have leading or trailing whitespace");
    }
    if member == "." {
        return None;
    }
    if member.starts_with(['/', '\\']) || member.contains(':') {
        return Some("must be a project-relative path");
    }
    if member.contains('\\') {
        return Some("must use portable '/' separators");
    }
    if member.contains("**") {
        return Some("must not use recursive '**' globs");
    }
    if member.contains(['[', ']', '{', '}']) {
        return Some("may use only '*' and '?' glob metacharacters");
    }
    if member.contains('$') || member.contains('~') || member.contains('%') {
        return Some("must not need shell or environment expansion");
    }
    if member.split('/').count() > MAX_WORKSPACE_MEMBER_DEPTH {
        return Some("exceeds the maximum workspace member depth of 8");
    }
    for component in member.split('/') {
        if component.is_empty() || matches!(component, "." | "..") {
            return Some("must contain only non-empty child path components");
        }
        if component.starts_with('.')
            || matches!(component, "target" | "build" | "out" | "node_modules")
        {
            return Some("must not select a generated or OpenStrata state directory");
        }
    }
    None
}

/// Why a declared discovery root is unusable, or `None` when it is fine.
///
/// Checked as *declared text*, not against this machine's filesystem: a
/// manifest is portable and a Windows root must still be rejected when the
/// check runs on Linux.
fn discovery_root_problem(root: &str) -> Option<&'static str> {
    let trimmed = root.trim();
    if trimmed.is_empty() {
        return Some("must not be empty");
    }
    if trimmed != root {
        return Some("must not have leading or trailing whitespace");
    }
    if trimmed.contains(['*', '?', '[', ']', '{', '}']) {
        return Some("must be a literal directory: discovery roots are declarative, not globs");
    }
    if trimmed.contains('$') || trimmed.contains('~') || trimmed.contains('%') {
        return Some("must not need shell or environment expansion");
    }
    if !is_absolute_declared_path(trimmed) {
        return Some("must be an absolute path");
    }
    if trimmed
        .split(['/', '\\'])
        .any(|component| component == "..")
    {
        return Some("must not contain '..'");
    }
    if is_filesystem_root(trimmed) {
        return Some("must not be a filesystem root: discovery is bounded, never disk-wide");
    }
    None
}

fn is_absolute_declared_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || path.starts_with('\\')
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
}

fn is_filesystem_root(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    let trimmed = normalized.trim_end_matches('/');
    // `/`, `C:` (from `C:/`), and a bare UNC prefix all normalize to nothing
    // meaningful once the trailing separators are gone.
    trimmed.is_empty()
        || (trimmed.len() == 2 && trimmed.as_bytes()[1] == b':')
        || normalized.trim_matches('/').is_empty()
}

fn safe_family_name(name: &str) -> bool {
    !name.is_empty()
        && name.as_bytes()[0].is_ascii_lowercase()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn safe_intent_name(name: &str) -> bool {
    !name.is_empty()
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn safe_cache_variable(name: &str) -> bool {
    !name.is_empty()
        && name.as_bytes()[0].is_ascii_alphabetic()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn safe_relative_file(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with(['/', '\\'])
        && !path.contains(':')
        && !path.split(['/', '\\']).any(|component| component == "..")
}

/// Replace an inline project version with a repo-relative authoritative file,
/// preserving comments, formatting, and unmodelled tables. This is an explicit
/// adoption/migration edit and is idempotent.
pub fn set_version_file(src: &str, path: &str) -> Result<Option<String>> {
    use toml_edit::{value, DocumentMut, Item};

    if !safe_relative_file(path) {
        return Err(Error::config(format!(
            "project.version_file '{path}' must be a safe repo-relative path"
        )));
    }
    let mut doc: DocumentMut = src
        .parse()
        .map_err(|e| Error::parse(PROJECT_MANIFEST, anyhow::Error::new(e)))?;
    let project = doc
        .get_mut("project")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| {
            Error::InvalidManifest(format!("{PROJECT_MANIFEST} is missing the [project] table"))
        })?;
    let current = project
        .get("version_file")
        .or_else(|| project.get("version-file"))
        .and_then(Item::as_str);
    if current == Some(path) && !project.contains_key("version") {
        return Ok(None);
    }
    project.remove("version");
    project.remove("version-file");
    project["version_file"] = value(path);
    let output = doc.to_string();
    Project::from_toml(&output)?;
    Ok(Some(output))
}

/// Add `name` to `[requires].extensions` in raw manifest TOML, preserving the
/// rest of the document (comments, formatting, and any tables this model does
/// not capture). The list is kept sorted. Returns the rewritten TOML, or `None`
/// when the extension is already present (idempotent).
///
/// This edits the source in place rather than round-tripping through [`Project`],
/// which would drop comments and silently delete unmodelled sections.
pub fn add_extension(src: &str, name: &str) -> Result<Option<String>> {
    use toml_edit::{Array, DocumentMut, Item, Value};

    let mut doc: DocumentMut = src
        .parse()
        .map_err(|e| Error::parse(PROJECT_MANIFEST, anyhow::Error::new(e)))?;

    let requires = doc
        .get_mut("requires")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| {
            Error::InvalidManifest(format!(
                "{PROJECT_MANIFEST} is missing the [requires] table"
            ))
        })?;

    let mut names: Vec<String> = requires
        .get("extensions")
        .and_then(Item::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    if names.iter().any(|e| e == name) {
        return Ok(None);
    }
    names.push(name.to_string());
    names.sort();

    let array: Array = names.into_iter().collect();
    requires["extensions"] = Item::Value(Value::Array(array));

    Ok(Some(doc.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# my project
[project]
name = \"demo\"

[requires]
platform = \"cy2026\"  # pinned year
profile = \"lookdev\"
";

    #[test]
    fn build_table_is_optional_and_defaults_to_host() {
        // No [build] table → None; callers fall back to the host policy.
        let p = Project::from_toml(SAMPLE).unwrap();
        assert!(p.build.is_none());
        assert_eq!(p.project.version.as_deref(), Some("0.1.0"));
        assert_eq!(BuildConfig::default().compiler, "host");
    }

    #[test]
    fn build_table_parses_explicit_compiler() {
        let src = format!(
            "{SAMPLE}\n[build]\ncompiler = \"explicit\"\ncc = \"/usr/bin/clang\"\ncxx = \"/usr/bin/clang++\"\n"
        );
        let p = Project::from_toml(&src).unwrap();
        let b = p.build.expect("build table");
        assert_eq!(b.compiler, "explicit");
        assert_eq!(b.cc.as_deref(), Some("/usr/bin/clang"));
        assert_eq!(b.cxx.as_deref(), Some("/usr/bin/clang++"));
    }

    #[test]
    fn build_compiler_defaults_when_table_present_without_field() {
        // `[build]` present but no `compiler` key → defaults to host.
        let src = format!("{SAMPLE}\n[build]\n");
        let p = Project::from_toml(&src).unwrap();
        assert_eq!(p.build.unwrap().compiler, "host");
    }

    #[test]
    fn named_build_intent_parses_typed_cache_entries() {
        let src = format!(
            r#"{SAMPLE}
[build.intents.materialx.cache.MERLIN_ENABLE_MATERIALX]
type = "BOOL"
value = true

[build.intents.materialx.cache.MERLIN_MATERIALX_SOURCE_DIR]
type = "PATH"
value = "../MaterialX"
portability = "local-override"
"#
        );
        let project = Project::from_toml(&src).unwrap();
        let intent = &project.build.unwrap().intents["materialx"];
        assert_eq!(
            intent.cache["MERLIN_ENABLE_MATERIALX"].value,
            BuildCacheValue::Bool(true)
        );
        assert_eq!(
            intent.cache["MERLIN_MATERIALX_SOURCE_DIR"].portability,
            Some(BuildPathPortability::LocalOverride)
        );
    }

    #[test]
    fn project_manifest_fails_closed_on_unknown_or_mistyped_keys() {
        let top_level = format!("{SAMPLE}\n[nonsense_table]\nvalue = true\n");
        let error = Project::from_toml(&top_level).unwrap_err().to_string();
        assert!(error.contains("unknown to this ost version"), "{error}");
        assert!(error.contains("nonsense_table"), "{error}");

        let nested = format!("{SAMPLE}\n[build]\ncompilor = \"host\"\n");
        let error = Project::from_toml(&nested).unwrap_err().to_string();
        assert!(error.contains("compilor"), "{error}");
        assert!(error.contains("compiler"), "{error}");

        let wrong_type = format!(
            "{SAMPLE}\n[build.intents.demo.cache.FEATURE]\ntype = \"BOOL\"\nvalue = \"ON\"\n"
        );
        assert!(Project::from_toml(&wrong_type).is_err());
    }

    #[test]
    fn path_cache_entries_require_explicit_portability() {
        let src = format!(
            "{SAMPLE}\n[build.intents.demo.cache.SDK_ROOT]\ntype = \"PATH\"\nvalue = \"../sdk\"\n"
        );
        let error = Project::from_toml(&src).unwrap_err().to_string();
        assert!(error.contains("portability is required"), "{error}");
    }

    #[test]
    fn host_discovery_declares_bounded_literal_roots() {
        let src = format!(
            "{SAMPLE}\n[host.discovery]\nroots = [\"/tools/maya\"]\nmax_depth = 3\nfamilies = [\"maya\"]\n"
        );
        let discovery = Project::from_toml(&src)
            .unwrap()
            .host
            .expect("host table")
            .discovery
            .expect("discovery table");
        assert_eq!(discovery.roots, vec!["/tools/maya".to_string()]);
        assert_eq!(discovery.max_depth, Some(3));
        assert_eq!(discovery.families, vec!["maya".to_string()]);
    }

    #[test]
    fn host_discovery_refuses_unbounded_expanded_or_executable_roots() {
        // Each of these would turn a declaration into a scan nobody asked for:
        // a whole-disk walk, a glob, a shell expansion, or an escape upward.
        // They are rejected as *text*, so a Windows root fails on Linux too.
        for (root, expected) in [
            ("/", "filesystem root"),
            ("C:\\", "filesystem root"),
            ("/tools/maya*", "globs"),
            ("$STUDIO_TOOLS/maya", "expansion"),
            ("tools/maya", "absolute"),
            ("/tools/../..", "'..'"),
        ] {
            // A TOML literal string so a Windows separator stays a separator.
            let src = format!("{SAMPLE}\n[host.discovery]\nroots = ['{root}']\n");
            let error = Project::from_toml(&src)
                .expect_err(&format!("'{root}' must be rejected"))
                .to_string();
            assert!(error.contains(expected), "'{root}': {error}");
        }
    }

    #[test]
    fn host_discovery_depth_stays_inside_the_bound() {
        for depth in ["0", "5"] {
            let src =
                format!("{SAMPLE}\n[host.discovery]\nroots = [\"/tools\"]\nmax_depth = {depth}\n");
            let error = Project::from_toml(&src).unwrap_err().to_string();
            assert!(
                error.contains("max_depth must be between 1 and 4"),
                "{error}"
            );
        }
    }

    #[test]
    fn workspace_members_parse_portable_bounded_globs() {
        let src = format!(
            "{SAMPLE}\n[workspace]\nmembers = [\".\", \"plugins/*\", \"adapters/*/*\", \"tools/converter\"]\n"
        );
        let workspace = Project::from_toml(&src)
            .unwrap()
            .workspace
            .expect("workspace table");
        assert_eq!(
            workspace.members,
            vec![".", "plugins/*", "adapters/*/*", "tools/converter"]
        );
    }

    #[test]
    fn workspace_members_reject_unbounded_or_nonportable_patterns() {
        for (member, expected) in [
            ("", "must not be empty"),
            ("/plugins/*", "project-relative"),
            ("plugins\\*", "portable '/'"),
            ("plugins/**", "recursive '**'"),
            ("../plugins/*", "child path components"),
            (".cache/plugins", "generated or OpenStrata state"),
            ("target/*", "generated or OpenStrata state"),
        ] {
            let src = format!("{SAMPLE}\n[workspace]\nmembers = ['{member}']\n");
            let error = Project::from_toml(&src).unwrap_err().to_string();
            assert!(error.contains(expected), "'{member}': {error}");
        }

        let empty = format!("{SAMPLE}\n[workspace]\nmembers = []\n");
        assert!(Project::from_toml(&empty).is_err());
    }

    #[test]
    fn add_extension_preserves_comments_and_sorts() {
        let out = add_extension(SAMPLE, "openusd").unwrap().expect("changed");
        assert!(out.contains("# my project"));
        assert!(out.contains("# pinned year"));
        assert!(out.contains("extensions = [\"openusd\"]"));

        // Adding a second one keeps the list sorted.
        let out = add_extension(&out, "materialx").unwrap().expect("changed");
        let idx_mtlx = out.find("materialx").unwrap();
        let idx_usd = out.find("openusd").unwrap();
        assert!(idx_mtlx < idx_usd, "extensions must stay sorted");
    }

    #[test]
    fn add_extension_is_idempotent() {
        let out = add_extension(SAMPLE, "openusd").unwrap().unwrap();
        assert!(add_extension(&out, "openusd").unwrap().is_none());
    }

    #[test]
    fn add_extension_keeps_unmodelled_sections() {
        let src = format!("{SAMPLE}\n[tools.cmake]\ngenerator = \"Ninja\"\n");
        let out = add_extension(&src, "openusd").unwrap().unwrap();
        assert!(out.contains("[tools.cmake]"));
        assert!(out.contains("generator = \"Ninja\""));
    }

    #[test]
    fn version_file_is_an_exclusive_authoritative_source() {
        let src = SAMPLE.replace(
            "name = \"demo\"",
            "name = \"demo\"\nversion_file = \"VERSION\"",
        );
        let project = Project::from_toml(&src).unwrap();
        assert!(project.project.version.is_none());

        let both = src.replace(
            "version_file = \"VERSION\"",
            "version = \"1.0.0\"\nversion_file = \"VERSION\"",
        );
        assert!(Project::from_toml(&both).is_err());
    }

    #[test]
    fn version_file_migration_is_targeted_and_idempotent() {
        let src = SAMPLE.replace("name = \"demo\"", "name = \"demo\"\nversion = \"1.2.3\"");
        let output = set_version_file(&src, "VERSION").unwrap().unwrap();
        assert!(output.contains("# my project"));
        assert!(output.contains("# pinned year"));
        assert!(output.contains("version_file = \"VERSION\""));
        assert!(!output.contains("version = \"1.2.3\""));
        assert!(set_version_file(&output, "VERSION").unwrap().is_none());
        assert!(set_version_file(&src, "../VERSION").is_err());
    }
}
