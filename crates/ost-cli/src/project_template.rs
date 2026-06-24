// SPDX-License-Identifier: Apache-2.0
//! Project templates for `ost init`.
//!
//! `ost init` defaults to scaffolding a minimal, buildable CMake project so the
//! first-build path (`init` → `runtime pull` → `build`) works without manual
//! steps. Templates live under `templates/<name>/` and are compiled into the
//! binary; path components and file contents carry `{{token}}` placeholders
//! substituted per invocation. `--bare` selects no template, for adopting
//! OpenStrata into an existing CMake project.

use camino::{Utf8Path, Utf8PathBuf};

use ost_core::{Error, Result};

/// The project template to scaffold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    /// A minimal buildable C++ library (default).
    CppLibrary,
    /// A minimal OpenUSD plugin project.
    UsdPlugin,
    /// No files beyond the manifest — for an existing CMake project.
    Bare,
}

impl Template {
    /// Parse the `--template` value.
    pub fn parse(s: &str) -> Result<Template> {
        match s {
            "cpp-library" => Ok(Template::CppLibrary),
            "usd-plugin" => Ok(Template::UsdPlugin),
            "bare" => Ok(Template::Bare),
            other => Err(Error::Operation(format!(
                "unknown template '{other}' (expected: cpp-library, usd-plugin, bare)"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Template::CppLibrary => "cpp-library",
            Template::UsdPlugin => "usd-plugin",
            Template::Bare => "bare",
        }
    }

    fn files(self) -> &'static [TemplateFile] {
        match self {
            Template::CppLibrary => CPP_LIBRARY,
            Template::UsdPlugin => USD_PLUGIN,
            Template::Bare => &[],
        }
    }
}

/// One embedded template file: a path template (with `{{token}}`s) and contents.
struct TemplateFile {
    path: &'static str,
    contents: &'static str,
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
];

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
        Err(Error::Operation(format!(
            "invalid project name '{name}': use letters, digits, '-' or '_', starting with a letter"
        )))
    }
}

/// Template files that already exist under `root` (root-relative). Lets the
/// caller fail before writing anything when `--force` was not given.
pub fn conflicts(template: Template, name: &str, root: &Utf8Path) -> Vec<Utf8PathBuf> {
    let vars = Vars::new(name);
    template
        .files()
        .iter()
        .map(|f| Utf8PathBuf::from(vars.apply(f.path)))
        .filter(|rel| root.join(rel).as_std_path().exists())
        .collect()
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

    // Pre-flight: never clobber an existing file unless --force was given.
    if !force {
        for f in template.files() {
            let rel = vars.apply(f.path);
            let abs = root.join(&rel);
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
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_templates() {
        assert_eq!(Template::parse("cpp-library").unwrap(), Template::CppLibrary);
        assert_eq!(Template::parse("usd-plugin").unwrap(), Template::UsdPlugin);
        assert_eq!(Template::parse("bare").unwrap(), Template::Bare);
        assert!(Template::parse("nope").is_err());
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
