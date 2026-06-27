// SPDX-License-Identifier: Apache-2.0
//! `ost plugin new` — scaffold a plugin bundle from an embedded template.
//!
//! Templates live under `templates/<kind>-<lang>/` and are compiled into the
//! binary. Files (and path components) carry `{{token}}` placeholders that are
//! substituted per invocation. Today only the `usd-fileformat-cpp` template
//! exists; `usd-asset-resolver` and `usd-schema` slot in alongside it.

use camino::{Utf8Path, Utf8PathBuf};

use ost_core::{Error, Result};

use crate::model::PluginKind;

/// One embedded template file: a path template (with `{{token}}`s) and contents.
struct TemplateFile {
    path: &'static str,
    contents: &'static str,
}

/// The `usd-fileformat-cpp` template. `include_str!` paths are relative to this
/// source file (`crates/ost-plugin/src/`).
const USD_FILEFORMAT_CPP: &[TemplateFile] = &[
    tf(
        "openstrata.plugin.yaml",
        include_str!("../../../templates/usd-fileformat-cpp/openstrata.plugin.yaml"),
    ),
    tf(
        "CMakeLists.txt",
        include_str!("../../../templates/usd-fileformat-cpp/CMakeLists.txt"),
    ),
    tf(
        "README.md",
        include_str!("../../../templates/usd-fileformat-cpp/README.md"),
    ),
    tf(
        ".gitignore",
        include_str!("../../../templates/usd-fileformat-cpp/.gitignore"),
    ),
    tf(
        "src/{{Name}}FileFormat.h",
        include_str!("../../../templates/usd-fileformat-cpp/src/{{Name}}FileFormat.h"),
    ),
    tf(
        "src/{{Name}}FileFormat.cpp",
        include_str!("../../../templates/usd-fileformat-cpp/src/{{Name}}FileFormat.cpp"),
    ),
    tf(
        "plugin/resources/{{name}}/plugInfo.json.in",
        include_str!(
            "../../../templates/usd-fileformat-cpp/plugin/resources/{{name}}/plugInfo.json.in"
        ),
    ),
    tf(
        "tests/fixtures/basic.{{extension}}",
        include_str!("../../../templates/usd-fileformat-cpp/tests/fixtures/basic.{{extension}}"),
    ),
    tf(
        "tests/fixtures/invalid.{{extension}}",
        include_str!("../../../templates/usd-fileformat-cpp/tests/fixtures/invalid.{{extension}}"),
    ),
];

const fn tf(path: &'static str, contents: &'static str) -> TemplateFile {
    TemplateFile { path, contents }
}

/// Parameters that fill a template's placeholders.
struct Vars {
    name: String,
    pascal: String,
    upper: String,
    extension: String,
}

impl Vars {
    fn apply(&self, s: &str) -> String {
        s.replace("{{name}}", &self.name)
            .replace("{{Name}}", &self.pascal)
            .replace("{{NAME}}", &self.upper)
            .replace("{{extension}}", &self.extension)
    }
}

/// Convert a plugin name to a PascalCase C++ identifier base, e.g.
/// `my-fmt` / `my_fmt` -> `MyFmt`.
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

/// Validate a plugin name: lowercase alphanumerics plus `-`/`_`, starting with a
/// letter. Keeps generated identifiers and filenames sane and portable.
fn validate_name(name: &str) -> Result<()> {
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
            "invalid plugin name '{name}': use letters, digits, '-' or '_', starting with a letter"
        )))
    }
}

/// Validate a file extension: lowercase alphanumerics only. The extension is
/// substituted into generated file *paths* (e.g. `tests/fixtures/basic.<ext>`),
/// so anything path-like (`/`, `\`, `.`, `..`) must be rejected to keep
/// scaffolding confined to the destination directory.
fn validate_extension(ext: &str) -> Result<()> {
    let ok = !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric());
    if ok {
        Ok(())
    } else {
        Err(Error::Operation(format!(
            "invalid extension '{ext}': use lowercase letters and digits only (no '.', '/', or path separators)"
        )))
    }
}

/// Scaffold a new bundle of `kind` named `name` into `dest` (the bundle root).
///
/// `extension` is required for file-format plugins. Returns the list of files
/// written (bundle-relative), in creation order. Refuses to overwrite a
/// non-empty destination.
pub fn scaffold(
    kind: PluginKind,
    name: &str,
    extension: Option<&str>,
    dest: &Utf8Path,
) -> Result<Vec<Utf8PathBuf>> {
    validate_name(name)?;

    let files = match kind {
        PluginKind::UsdFileformat => USD_FILEFORMAT_CPP,
        other => {
            return Err(Error::Operation(format!(
                "no template yet for kind '{}' (4a ships usd-fileformat; others follow)",
                other.as_str()
            )))
        }
    };

    let extension = match (kind, extension) {
        (PluginKind::UsdFileformat, Some(e)) => {
            validate_extension(e)?;
            e.to_string()
        }
        (PluginKind::UsdFileformat, None) => {
            return Err(Error::Operation(
                "usd-fileformat needs --extension <ext> (the file extension it reads)".into(),
            ))
        }
        (_, e) => e.unwrap_or("").to_string(),
    };

    if dest.as_std_path().exists() {
        let non_empty = std::fs::read_dir(dest.as_std_path())
            .map(|mut d| d.next().is_some())
            .unwrap_or(false);
        if non_empty {
            return Err(Error::Operation(format!(
                "destination '{dest}' already exists and is not empty"
            )));
        }
    }

    let vars = Vars {
        name: name.to_string(),
        pascal: to_pascal(name),
        upper: to_pascal(name).to_ascii_uppercase(),
        extension,
    };

    let mut written = Vec::new();
    for file in files {
        let rel = Utf8PathBuf::from(vars.apply(file.path));
        let abs = dest.join(&rel);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent.as_std_path())
                .map_err(|e| Error::io(parent.to_string(), e))?;
        }
        let contents = vars.apply(file.contents);
        std::fs::write(abs.as_std_path(), &contents)
            .map_err(|e| Error::io(abs.to_string(), e))?;
        written.push(rel.clone());

        // A `*.in` is a CMake `configure_file` source. Emit a ready-to-use
        // concrete file next to it (with the host's shared-library suffix) so
        // `ost plugin doctor`/`test` work before the first build — the build's
        // `configure_file` then regenerates it for the target being built.
        if let Some(concrete) = rel.as_str().strip_suffix(".in") {
            let concrete_rel = Utf8PathBuf::from(concrete);
            let concrete_abs = dest.join(&concrete_rel);
            let resolved =
                contents.replace("@CMAKE_SHARED_LIBRARY_SUFFIX@", std::env::consts::DLL_SUFFIX);
            std::fs::write(concrete_abs.as_std_path(), resolved)
                .map_err(|e| Error::io(concrete_abs.to_string(), e))?;
            written.push(concrete_rel);
        }
    }

    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pascalizes_separated_names() {
        assert_eq!(to_pascal("toy"), "Toy");
        assert_eq!(to_pascal("my-fmt"), "MyFmt");
        assert_eq!(to_pascal("my_cool_fmt"), "MyCoolFmt");
    }

    #[test]
    fn rejects_bad_names() {
        assert!(validate_name("9bad").is_err());
        assert!(validate_name("has space").is_err());
        assert!(validate_name("ok-name_2").is_ok());
    }

    #[test]
    fn rejects_path_like_extensions() {
        // The extension is substituted into generated file paths, so anything
        // path-like must be rejected to prevent escaping the destination.
        assert!(validate_extension("").is_err());
        assert!(validate_extension("../../etc").is_err());
        assert!(validate_extension("a/b").is_err());
        assert!(validate_extension("a.b").is_err());
        assert!(validate_extension("toy").is_ok());
        // And the scaffold entry point rejects it too.
        let dir = unique_tmp("scaffold-badext");
        assert!(scaffold(PluginKind::UsdFileformat, "toy", Some("../evil"), &dir).is_err());
        assert!(!dir.as_std_path().exists());
    }

    #[test]
    fn scaffolds_a_buildable_bundle() {
        let dir = unique_tmp("scaffold");
        let files =
            scaffold(PluginKind::UsdFileformat, "toy", Some("toy"), &dir).expect("scaffold");

        // The manifest and a token-substituted source file landed.
        assert!(files.iter().any(|f| f.as_str() == "openstrata.plugin.yaml"));
        assert!(files.iter().any(|f| f.as_str() == "src/ToyFileFormat.cpp"));
        // Both the configure_file source (.in) and the ready-to-use concrete
        // plugInfo.json are written.
        assert!(files
            .iter()
            .any(|f| f.as_str() == "plugin/resources/toy/plugInfo.json.in"));
        assert!(files
            .iter()
            .any(|f| f.as_str() == "plugin/resources/toy/plugInfo.json"));

        // Placeholders are gone and the parsed manifest is coherent.
        let manifest_src =
            std::fs::read_to_string(dir.join("openstrata.plugin.yaml").as_std_path()).unwrap();
        assert!(!manifest_src.contains("{{"));
        let bundle = crate::Bundle::load(&dir).expect("loads");
        assert_eq!(bundle.manifest.name(), "toy");
        assert!(bundle.plug_info().as_std_path().is_file());

        // The concrete plugInfo.json has the host's lib suffix resolved (no
        // leftover `@CMAKE_*@` token) and points at the bundle's lib/ — the two
        // things USD needs to dlopen it (it has no PATH fallback for the lib).
        let plug_info = std::fs::read_to_string(bundle.plug_info().as_std_path()).unwrap();
        assert!(!plug_info.contains('@'), "configure_file token left in plugInfo.json");
        assert!(plug_info.contains(&format!(
            "../../../lib/libToyFileFormat{}",
            std::env::consts::DLL_SUFFIX
        )));

        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    #[test]
    fn refuses_non_empty_destination() {
        let dir = unique_tmp("scaffold-existing");
        std::fs::create_dir_all(dir.as_std_path()).unwrap();
        std::fs::write(dir.join("keep.txt").as_std_path(), "x").unwrap();
        let err = scaffold(PluginKind::UsdFileformat, "toy", Some("toy"), &dir);
        assert!(err.is_err());
        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    #[test]
    fn fileformat_requires_extension() {
        let dir = unique_tmp("scaffold-noext");
        assert!(scaffold(PluginKind::UsdFileformat, "toy", None, &dir).is_err());
    }

    fn unique_tmp(tag: &str) -> Utf8PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut dir = Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
        dir.push(format!("ost-plugin-{tag}-{}-{nanos}", std::process::id()));
        dir
    }
}
