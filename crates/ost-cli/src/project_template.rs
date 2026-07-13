// SPDX-License-Identifier: Apache-2.0
//! Project templates for `ost init`.
//!
//! `ost init` defaults to scaffolding a minimal, buildable CMake project so the
//! first-build path (`init` → `runtime pull` → `build`) works without manual
//! steps. Templates live under `templates/<name>/` and are compiled into the
//! binary; path components and file contents carry `{{token}}` placeholders
//! substituted per invocation. `--bare` selects no template, for adopting
//! OpenStrata into an existing CMake project.

use std::collections::{BTreeMap, BTreeSet};

use camino::{Utf8Path, Utf8PathBuf};

use ost_core::template::{ScaffoldProvenance, TemplateDescriptor, SCAFFOLD_PROVENANCE};
use ost_core::{Error, Result};

/// The project template to scaffold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    /// A minimal buildable C++ library (default).
    CppLibrary,
    /// A minimal OpenUSD plugin project.
    UsdPlugin,
    /// A dual-mode root for a repository of plugin bundles (`ost plugin new`).
    PluginWorkspace,
    /// No files beyond the manifest — for an existing CMake project.
    Bare,
}

impl Template {
    /// Parse the `--template` value.
    pub fn parse(s: &str) -> Result<Template> {
        match s {
            "cpp-library" => Ok(Template::CppLibrary),
            "usd-plugin" => Ok(Template::UsdPlugin),
            "usd-plugin-workspace" | "plugin-workspace" => Ok(Template::PluginWorkspace),
            "bare" => Ok(Template::Bare),
            other => Err(Error::usage(format!(
                "unknown template '{other}' (expected: cpp-library, usd-plugin, usd-plugin-workspace, bare)"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Template::CppLibrary => "cpp-library",
            Template::UsdPlugin => "usd-plugin",
            Template::PluginWorkspace => "usd-plugin-workspace",
            Template::Bare => "bare",
        }
    }

    fn embedded(self) -> Option<EmbeddedTemplate> {
        match self {
            Template::CppLibrary => Some(EmbeddedTemplate {
                descriptor: CPP_LIBRARY_DESCRIPTOR,
                files: CPP_LIBRARY,
            }),
            Template::UsdPlugin => Some(EmbeddedTemplate {
                descriptor: USD_PLUGIN_DESCRIPTOR,
                files: USD_PLUGIN,
            }),
            Template::PluginWorkspace => Some(EmbeddedTemplate {
                descriptor: PLUGIN_WORKSPACE_DESCRIPTOR,
                files: PLUGIN_WORKSPACE,
            }),
            Template::Bare => None,
        }
    }

    fn files(self) -> &'static [TemplateFile] {
        self.embedded().map(|t| t.files).unwrap_or(&[])
    }
}

/// One embedded template file: a path template (with `{{token}}`s) and contents.
struct TemplateFile {
    path: &'static str,
    contents: &'static str,
}

#[derive(Clone, Copy)]
struct EmbeddedTemplate {
    descriptor: &'static str,
    files: &'static [TemplateFile],
}

const fn tf(path: &'static str, contents: &'static str) -> TemplateFile {
    TemplateFile { path, contents }
}

/// `include_str!` paths are relative to this source file (`crates/ost-cli/src/`).
const CPP_LIBRARY: &[TemplateFile] = &[
    tf(
        "CMakeLists.txt",
        include_str!("../../../templates/cpp-library/CMakeLists.txt"),
    ),
    tf(
        "README.md",
        include_str!("../../../templates/cpp-library/README.md"),
    ),
    tf(
        ".gitignore",
        include_str!("../../../templates/cpp-library/.gitignore"),
    ),
    tf(
        "include/{{name}}/{{name}}.hpp",
        include_str!("../../../templates/cpp-library/include/{{name}}/{{name}}.hpp"),
    ),
    tf(
        "src/{{name}}.cpp",
        include_str!("../../../templates/cpp-library/src/{{name}}.cpp"),
    ),
    tf(
        "cmake/{{Name}}Config.cmake.in",
        include_str!("../../../templates/cpp-library/cmake/{{Name}}Config.cmake.in"),
    ),
    tf(
        "openstrata.library.yaml",
        include_str!("../../../templates/cpp-library/openstrata.library.yaml"),
    ),
];

const CPP_LIBRARY_DESCRIPTOR: &str = include_str!("../../../templates/cpp-library/template.yaml");

const USD_PLUGIN: &[TemplateFile] = &[
    tf(
        "CMakeLists.txt",
        include_str!("../../../templates/usd-plugin/CMakeLists.txt"),
    ),
    tf(
        "README.md",
        include_str!("../../../templates/usd-plugin/README.md"),
    ),
    tf(
        ".gitignore",
        include_str!("../../../templates/usd-plugin/.gitignore"),
    ),
    tf(
        "src/{{name}}.cpp",
        include_str!("../../../templates/usd-plugin/src/{{name}}.cpp"),
    ),
    tf(
        "plugin/resources/plugInfo.json",
        include_str!("../../../templates/usd-plugin/plugin/resources/plugInfo.json"),
    ),
];

const USD_PLUGIN_DESCRIPTOR: &str = include_str!("../../../templates/usd-plugin/template.yaml");

/// A dual-mode workspace root: a CMakeLists.txt that resolves OpenUSD once and
/// add_subdirectory()s every bundle, so the repo is `cmake -S .`-able without
/// `ost`. Carries no per-bundle files — bundles are added with `ost plugin new`.
const PLUGIN_WORKSPACE: &[TemplateFile] = &[
    tf(
        "CMakeLists.txt",
        include_str!("../../../templates/plugin-workspace/CMakeLists.txt"),
    ),
    tf(
        "CMakePresets.json",
        include_str!("../../../templates/plugin-workspace/CMakePresets.json"),
    ),
    tf(
        "README.md",
        include_str!("../../../templates/plugin-workspace/README.md"),
    ),
    tf(
        ".gitignore",
        include_str!("../../../templates/plugin-workspace/.gitignore"),
    ),
];

const PLUGIN_WORKSPACE_DESCRIPTOR: &str =
    include_str!("../../../templates/plugin-workspace/template.yaml");

/// Placeholder substitutions for a template.
struct Vars {
    name: String,
    pascal: String,
    upper: String,
}

impl Vars {
    fn new(name: &str) -> Vars {
        let pascal = to_pascal(name);
        let upper = pascal.to_ascii_uppercase();
        Vars {
            name: name.to_string(),
            pascal,
            upper,
        }
    }

    fn apply(&self, s: &str) -> String {
        s.replace("{{name}}", &self.name)
            .replace("{{Name}}", &self.pascal)
            .replace("{{NAME}}", &self.upper)
    }
}

/// Convert a project name to a PascalCase identifier base: `my-lib` -> `MyLib`.
/// The result has no separators, so it is also safe as a C++ namespace/macro.
fn to_pascal(name: &str) -> String {
    name.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// Validate a project name: lowercase-ish alphanumerics plus `-`/`_`, starting
/// with a letter. Keeps generated identifiers and filenames portable.
pub fn validate_name(name: &str) -> Result<()> {
    let ok = name
        .chars()
        .next()
        .map(|c| c.is_ascii_alphabetic())
        .unwrap_or(false)
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(Error::usage(format!(
            "invalid project name '{name}': use letters, digits, '-' or '_', starting with a letter"
        )))
    }
}

/// Template files that already exist under `root` (root-relative). Lets the
/// caller fail before writing anything when `--force` was not given.
pub fn conflicts(template: Template, name: &str, root: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    let vars = Vars::new(name);
    let planned = planned_outputs(template, &vars)?;
    Ok(planned
        .into_iter()
        .filter(|rel| root.join(rel).as_std_path().exists())
        .collect())
}

/// Write `template`'s files into `root`, returning the files written
/// (root-relative). Refuses to overwrite an existing file unless `force`.
///
/// The manifest and `.strata/` are written separately by `ost init`.
pub fn scaffold(
    template: Template,
    name: &str,
    root: &Utf8Path,
    force: bool,
) -> Result<Vec<Utf8PathBuf>> {
    // The name is only substituted into template files, so it need only be a
    // portable identifier when there are files to write. `Bare` writes none.
    if !template.files().is_empty() {
        validate_name(name)?;
    }
    let vars = Vars::new(name);
    let planned = planned_outputs(template, &vars)?;

    // Pre-flight: never clobber an existing file unless --force was given.
    if !force {
        for rel in &planned {
            let abs = root.join(rel);
            if abs.as_std_path().exists() {
                return Err(Error::Operation(format!(
                    "refusing to overwrite existing '{rel}'; \
                     pass --force, or use --bare to skip template files"
                )));
            }
        }
    }

    let mut written = Vec::new();
    for f in template.files() {
        let rel = Utf8PathBuf::from(vars.apply(f.path));
        let abs = root.join(&rel);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent.as_std_path())
                .map_err(|e| Error::io(parent.to_string(), e))?;
        }
        std::fs::write(abs.as_std_path(), vars.apply(f.contents))
            .map_err(|e| Error::io(abs.to_string(), e))?;
        written.push(rel);
    }
    if let Some(embedded) = template.embedded() {
        let descriptor = TemplateDescriptor::parse(embedded.descriptor)?;
        let inputs: BTreeMap<String, String> = BTreeMap::from([("name".into(), name.to_string())]);
        let provenance =
            ScaffoldProvenance::new(&descriptor, env!("CARGO_PKG_VERSION"), inputs).to_yaml()?;
        let path = root.join(SCAFFOLD_PROVENANCE);
        std::fs::write(path.as_std_path(), provenance)
            .map_err(|e| Error::io(path.to_string(), e))?;
        written.push(SCAFFOLD_PROVENANCE.into());
    }
    Ok(written)
}

fn planned_outputs(template: Template, vars: &Vars) -> Result<Vec<Utf8PathBuf>> {
    let Some(embedded) = template.embedded() else {
        return Ok(Vec::new());
    };
    let descriptor = TemplateDescriptor::parse(embedded.descriptor)?;
    if descriptor.template.id != template.as_str() {
        return Err(Error::InvalidManifest(format!(
            "template descriptor id '{}' does not match catalog id '{}'",
            descriptor.template.id,
            template.as_str()
        )));
    }

    let mut planned: Vec<Utf8PathBuf> = embedded
        .files
        .iter()
        .map(|file| Utf8PathBuf::from(vars.apply(file.path)))
        .collect();
    planned.push(SCAFFOLD_PROVENANCE.into());

    let actual: BTreeSet<String> = planned.iter().map(ToString::to_string).collect();
    if actual.len() != planned.len() {
        return Err(Error::InvalidManifest(format!(
            "template '{}' renders duplicate output paths",
            descriptor.template.id
        )));
    }
    let declared: BTreeSet<String> = descriptor
        .outputs
        .files
        .iter()
        .map(|path| vars.apply(path))
        .collect();
    if declared != actual {
        let missing: Vec<_> = actual.difference(&declared).cloned().collect();
        let extra: Vec<_> = declared.difference(&actual).cloned().collect();
        return Err(Error::InvalidManifest(format!(
            "template '{}' outputs do not match embedded files (undeclared: [{}]; missing: [{}])",
            descriptor.template.id,
            missing.join(", "),
            extra.join(", ")
        )));
    }
    Ok(planned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_templates() {
        assert_eq!(
            Template::parse("cpp-library").unwrap(),
            Template::CppLibrary
        );
        assert_eq!(Template::parse("usd-plugin").unwrap(), Template::UsdPlugin);
        assert_eq!(
            Template::parse("plugin-workspace").unwrap(),
            Template::PluginWorkspace
        );
        assert_eq!(
            Template::parse("usd-plugin-workspace").unwrap(),
            Template::PluginWorkspace
        );
        assert_eq!(Template::parse("bare").unwrap(), Template::Bare);
        assert!(Template::parse("nope").is_err());
    }

    #[test]
    fn every_project_catalog_descriptor_matches_its_embedded_outputs() {
        let vars = Vars::new("catalog-check");
        for template in [
            Template::CppLibrary,
            Template::UsdPlugin,
            Template::PluginWorkspace,
        ] {
            let outputs = planned_outputs(template, &vars).expect("valid catalog entry");
            assert!(outputs
                .iter()
                .any(|path| path.as_str() == SCAFFOLD_PROVENANCE));
        }
    }

    #[test]
    fn plugin_workspace_emits_a_dual_mode_glob_root() {
        let dir = unique_tmp("workspace");
        std::fs::create_dir_all(dir.as_std_path()).unwrap();
        let written = scaffold(Template::PluginWorkspace, "vrm-plugins", &dir, false).unwrap();

        assert!(written.iter().any(|p| p.as_str() == "CMakeLists.txt"));
        assert!(written.iter().any(|p| p.as_str() == "CMakePresets.json"));
        assert!(written.iter().any(|p| p.as_str() == SCAFFOLD_PROVENANCE));

        // The root resolves USD once and add_subdirectory()s discovered bundles,
        // and its project() name is the substituted workspace name.
        let cml = std::fs::read_to_string(dir.join("CMakeLists.txt").as_std_path()).unwrap();
        assert!(!cml.contains("{{"));
        assert!(cml.contains("project(vrm-plugins"));
        assert!(cml.contains("find_package(pxr"));
        assert!(cml.contains("openstrata.plugin.yaml"));
        assert!(cml.contains("add_subdirectory"));
        assert!(cml.contains("${CMAKE_CURRENT_SOURCE_DIR}/plugins"));

        let provenance: ScaffoldProvenance = serde_yaml::from_str(
            &std::fs::read_to_string(dir.join(SCAFFOLD_PROVENANCE).as_std_path()).unwrap(),
        )
        .unwrap();
        assert_eq!(provenance.template.id, "usd-plugin-workspace");
        assert_eq!(provenance.inputs.get("name").unwrap(), "vrm-plugins");

        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    #[test]
    fn pascalizes_names() {
        assert_eq!(to_pascal("toy"), "Toy");
        assert_eq!(to_pascal("my-lib"), "MyLib");
        assert_eq!(to_pascal("my_cool_lib"), "MyCoolLib");
    }

    #[test]
    fn rejects_bad_names() {
        assert!(validate_name("9bad").is_err());
        assert!(validate_name("has space").is_err());
        assert!(validate_name("ok-name_2").is_ok());
    }

    #[test]
    fn bare_writes_nothing() {
        let dir = unique_tmp("bare");
        std::fs::create_dir_all(dir.as_std_path()).unwrap();
        let written = scaffold(Template::Bare, "demo", &dir, false).unwrap();
        assert!(written.is_empty());
        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    #[test]
    fn bare_accepts_non_identifier_names() {
        // `--bare` substitutes the name into no files, so directory names that
        // aren't portable identifiers (a leading digit, a dot) must be accepted.
        let dir = unique_tmp("bare-name");
        std::fs::create_dir_all(dir.as_std_path()).unwrap();
        assert!(scaffold(Template::Bare, "2026_show", &dir, false).is_ok());
        assert!(scaffold(Template::Bare, "show.v2", &dir, false).is_ok());
        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    #[test]
    fn cpp_library_substitutes_tokens_and_has_install_rules() {
        let dir = unique_tmp("cpp");
        std::fs::create_dir_all(dir.as_std_path()).unwrap();
        let written = scaffold(Template::CppLibrary, "my-lib", &dir, false).unwrap();

        // Path tokens are substituted.
        assert!(written.iter().any(|p| p.as_str() == "src/my-lib.cpp"));
        assert!(written
            .iter()
            .any(|p| p.as_str() == "include/my-lib/my-lib.hpp"));

        // No placeholders remain, and the header uses an identifier-safe namespace.
        let header =
            std::fs::read_to_string(dir.join("include/my-lib/my-lib.hpp").as_std_path()).unwrap();
        assert!(!header.contains("{{"));
        assert!(header.contains("namespace MyLib"));
        assert!(header.contains("MYLIB_HPP"));

        // CMake template carries install rules (needed for `ost package`).
        let cml = std::fs::read_to_string(dir.join("CMakeLists.txt").as_std_path()).unwrap();
        assert!(cml.contains("install(TARGETS"));
        assert!(cml.contains("cmake_minimum_required(VERSION 3.23)"));

        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    #[test]
    fn refuses_to_overwrite_without_force() {
        let dir = unique_tmp("conflict");
        std::fs::create_dir_all(dir.as_std_path()).unwrap();
        std::fs::write(dir.join("CMakeLists.txt").as_std_path(), "# existing").unwrap();

        let err = scaffold(Template::CppLibrary, "demo", &dir, false);
        assert!(err.is_err());
        // The existing file is left intact.
        let kept = std::fs::read_to_string(dir.join("CMakeLists.txt").as_std_path()).unwrap();
        assert_eq!(kept, "# existing");

        // With --force it proceeds.
        assert!(scaffold(Template::CppLibrary, "demo", &dir, true).is_ok());
        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    fn unique_tmp(tag: &str) -> Utf8PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut dir = Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
        dir.push(format!("ost-init-{tag}-{}-{nanos}", std::process::id()));
        dir
    }
}
