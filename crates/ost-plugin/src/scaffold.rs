// SPDX-License-Identifier: Apache-2.0
//! `ost plugin new` — scaffold a plugin bundle from an embedded template.
//!
//! Templates live under `templates/<kind>-<lang>/` and are compiled into the
//! binary. Files (and path components) carry `{{token}}` placeholders that are
//! substituted per invocation. Today `usd-fileformat-cpp` and
//! `usd-schema-codeless` exist; `usd-asset-resolver` slots in alongside them.

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

/// The `usd-schema-codeless` template: a resource-only (codeless) API schema —
/// no C++, no shared library; usdGenSchema turns `schema.usda` into the bundle's
/// `plugInfo.json` `Types` block.
const USD_SCHEMA_CODELESS: &[TemplateFile] = &[
    tf(
        "openstrata.plugin.yaml",
        include_str!("../../../templates/usd-schema-codeless/openstrata.plugin.yaml"),
    ),
    tf(
        "CMakeLists.txt",
        include_str!("../../../templates/usd-schema-codeless/CMakeLists.txt"),
    ),
    tf(
        "README.md",
        include_str!("../../../templates/usd-schema-codeless/README.md"),
    ),
    tf(
        ".gitignore",
        include_str!("../../../templates/usd-schema-codeless/.gitignore"),
    ),
    tf(
        "schema.usda",
        include_str!("../../../templates/usd-schema-codeless/schema.usda"),
    ),
    tf(
        "plugin/resources/{{name}}/plugInfo.json",
        include_str!(
            "../../../templates/usd-schema-codeless/plugin/resources/{{name}}/plugInfo.json"
        ),
    ),
    tf(
        "plugin/resources/{{name}}/generatedSchema.usda",
        include_str!(
            "../../../templates/usd-schema-codeless/plugin/resources/{{name}}/generatedSchema.usda"
        ),
    ),
    tf(
        "tests/fixtures/basic.usda",
        include_str!("../../../templates/usd-schema-codeless/tests/fixtures/basic.usda"),
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
    /// `name` as a valid USD identifier (hyphens/spaces → `_`). USD prim and
    /// property names — including schema attribute namespaces like
    /// `{{ident}}:example` — must match `[A-Za-z_][A-Za-z0-9_]*`, so a hyphenated
    /// plugin name (`vrm-schema`) cannot be used there verbatim.
    ident: String,
    extension: String,
}

impl Vars {
    fn apply(&self, s: &str) -> String {
        s.replace("{{name}}", &self.name)
            .replace("{{Name}}", &self.pascal)
            .replace("{{NAME}}", &self.upper)
            .replace("{{ident}}", &self.ident)
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

/// Convert a plugin name to a valid USD identifier base: replace each `-`/space
/// with `_` (USD prim/property names allow only `[A-Za-z_][A-Za-z0-9_]*`).
/// `validate_name` guarantees the name already starts with a letter.
fn to_ident(name: &str) -> String {
    name.chars()
        .map(|c| if c == '-' || c == ' ' { '_' } else { c })
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
        PluginKind::UsdSchema => USD_SCHEMA_CODELESS,
        other => {
            return Err(Error::Operation(format!(
                "no template yet for kind '{}' (ships usd-fileformat + usd-schema; others follow)",
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
        ident: to_ident(name),
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
        std::fs::write(abs.as_std_path(), &contents).map_err(|e| Error::io(abs.to_string(), e))?;
        written.push(rel.clone());

        // A `*.in` is a CMake `configure_file` source. Emit a ready-to-use
        // concrete file next to it (with the host's shared-library name) so
        // `ost plugin doctor`/`test` can inspect it before the first build — the
        // build's `configure_file` then regenerates it for the target being
        // built.
        if let Some(concrete) = rel.as_str().strip_suffix(".in") {
            let concrete_rel = Utf8PathBuf::from(concrete);
            let concrete_abs = dest.join(&concrete_rel);
            let resolved = contents
                .replace("@OPENSTRATA_PLUGIN_LIBRARY_PREFIX@", "lib")
                .replace(
                    "@CMAKE_SHARED_LIBRARY_SUFFIX@",
                    std::env::consts::DLL_SUFFIX,
                );
            std::fs::write(concrete_abs.as_std_path(), resolved)
                .map_err(|e| Error::io(concrete_abs.to_string(), e))?;
            written.push(concrete_rel);
        }
    }

    Ok(written)
}

/// What [`add_cohosted_schema`] created/changed in the bundle.
#[derive(Debug)]
pub struct AddedSchema {
    /// The public schema type usdGenSchema will generate (`libraryPrefix` +
    /// source class), e.g. `ToyMetadataAPI` — also the new `provides` entry.
    pub schema_type: String,
    /// Bundle-relative path of the written `schema.usda` source.
    pub source: Utf8PathBuf,
    pub codeless: bool,
}

/// Add a co-located schema to an *existing* non-schema bundle: write a starter
/// `schema.usda` at `source` and wire the manifest (`provides:
/// usd-schema:<Type>` + `schema.source`) so the next `ost plugin build` runs
/// usdGenSchema, links the generated typed C++ API into the same plugin
/// library, merges the schema `Types` into the bundle's `plugInfo.json`, and
/// stages `generatedSchema.usda` beside it.
///
/// `class` is the *source* class name (default `API`); the public type is
/// `libraryPrefix` (the PascalCase bundle name) + `class`, matching how
/// usdGenSchema composes type names — which is also why the class must not
/// repeat the bundle-name prefix (the `schema.library_prefix` footgun).
/// `codeless: true` writes a `skipCodeGeneration` schema instead: the build
/// then falls back to the resource-only `Types` merge, generating no C++.
pub fn add_cohosted_schema(
    bundle_root: &Utf8Path,
    class: &str,
    source: &str,
    codeless: bool,
) -> Result<AddedSchema> {
    let bundle = crate::Bundle::load(bundle_root)?;
    let manifest = &bundle.manifest;

    if manifest.kind() == PluginKind::UsdSchema {
        return Err(Error::config(format!(
            "'{}' already is a usd-schema bundle — edit its schema.usda directly \
             (schema add co-locates a schema in a *non-schema* bundle)",
            manifest.name()
        )));
    }
    validate_class(class)?;
    crate::bundle::check_safe_relative("schema source", source)?;
    if !source.ends_with(".usda") {
        return Err(Error::config(format!(
            "schema source '{source}' must be a .usda file (usdGenSchema input)"
        )));
    }

    let pascal = to_pascal(manifest.name());
    let schema_type = format!("{pascal}{class}");
    if manifest.schema_provides().iter().any(|t| *t == schema_type) {
        return Err(Error::config(format!(
            "'{}' already provides usd-schema:{schema_type}",
            manifest.name()
        )));
    }
    if let Some(existing) = manifest.schema.as_ref().and_then(|s| s.source.as_ref()) {
        return Err(Error::config(format!(
            "'{}' already declares schema.source: {existing} — one schema.usda per bundle; \
             add further classes to that file",
            manifest.name()
        ))
        .with_hint("usdGenSchema generates every class in the file into the same library"));
    }
    let schema_abs = bundle.path(source);
    if schema_abs.as_std_path().exists() {
        return Err(Error::config(format!(
            "'{source}' already exists — declare it with `schema.source: {source}` \
             instead of re-scaffolding"
        )));
    }

    // 1. The starter schema source.
    let vars = Vars {
        name: manifest.name().to_string(),
        pascal: pascal.clone(),
        upper: pascal.to_ascii_uppercase(),
        ident: to_ident(manifest.name()),
        extension: String::new(),
    };
    if let Some(parent) = schema_abs.parent() {
        std::fs::create_dir_all(parent.as_std_path())
            .map_err(|e| Error::io(parent.to_string(), e))?;
    }
    let schema_src = cohosted_schema_starter(&vars, class, codeless);
    std::fs::write(schema_abs.as_std_path(), schema_src)
        .map_err(|e| Error::io(schema_abs.to_string(), e))?;

    // 2. Wire the manifest, textually (manifests carry the user's comments, so
    // a parse→re-serialize round-trip would destroy them). The edited text is
    // re-parsed and cross-checked before anything is written back.
    let manifest_path = bundle.path(crate::PLUGIN_MANIFEST);
    let original = std::fs::read_to_string(manifest_path.as_std_path())
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;
    let provides_entry = format!("usd-schema:{schema_type}");
    let edited = append_schema_source(&insert_provides_entry(&original, &provides_entry), source);

    let reparsed = crate::PluginManifest::parse(&edited).map_err(|e| {
        Error::config(format!(
            "could not update {} automatically ({e}); add by hand:\n\
             provides:\n  - {provides_entry}\nschema:\n  source: {source}",
            crate::PLUGIN_MANIFEST
        ))
    })?;
    let wired = reparsed.schema_provides().iter().any(|t| *t == schema_type)
        && reparsed.schema.as_ref().and_then(|s| s.source.as_deref()) == Some(source);
    if !wired {
        return Err(Error::config(format!(
            "could not update {} automatically; add by hand:\n\
             provides:\n  - {provides_entry}\nschema:\n  source: {source}",
            crate::PLUGIN_MANIFEST
        )));
    }
    std::fs::write(manifest_path.as_std_path(), edited)
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;

    Ok(AddedSchema {
        schema_type,
        source: Utf8PathBuf::from(source),
        codeless,
    })
}

/// A source class name: a C++-identifier-shaped PascalCase token, e.g.
/// `API` or `MetadataAPI`. usdGenSchema turns it into a class name, so path
/// or namespace characters must be rejected.
fn validate_class(class: &str) -> Result<()> {
    let ok = class
        .chars()
        .next()
        .map(|c| c.is_ascii_uppercase())
        .unwrap_or(false)
        && class.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(Error::config(format!(
            "invalid schema class '{class}': use a PascalCase identifier (letters, digits, '_'), \
             e.g. API or MetadataAPI"
        )))
    }
}

/// The starter `schema.usda` for a co-located schema.
fn cohosted_schema_starter(vars: &Vars, class: &str, codeless: bool) -> String {
    let skip = if codeless { "true" } else { "false" };
    let mode = if codeless {
        "Because skipCodeGeneration = true, the build merges only the generated\n    \
         resources (plugInfo.json Types + generatedSchema.usda); no C++ is added."
    } else {
        "The generated typed C++ API is compiled into the existing plugin\n    \
         library; the schema Types are merged into the bundle's plugInfo.json."
    };
    let s = format!(
        "\
#usda 1.0
(
    \"\"\"
    {{{{Name}}}}{class} - a co-located OpenUSD API schema for the {{{{name}}}}
    plugin, scaffolded by `ost plugin schema add`.

    `ost plugin build` runs usdGenSchema on this file in the composed runtime
    session environment. {mode}
    \"\"\"
    subLayers = [
        @usd/schema.usda@
    ]
)

over \"GLOBAL\" (
    customData = {{
        string libraryName      = \"{{{{name}}}}\"
        string libraryPath      = \".\"
        string libraryPrefix    = \"{{{{Name}}}}\"
        string tokensPrefix     = \"{{{{Name}}}}\"
        bool skipCodeGeneration = {skip}
    }}
)
{{
}}

class \"{class}\" (
    inherits = </APISchemaBase>
    customData = {{
        token apiSchemaType = \"singleApply\"
    }}
    doc = \"\"\"A single-apply API schema. Replace the example property with the
    real data contract this schema defines.\"\"\"
)
{{
    uniform token {{{{ident}}}}:example = \"\" (
        doc = \"Example attribute. Replace with the schema's real properties.\"
    )
}}
"
    );
    vars.apply(&s)
}

/// Insert an entry into the manifest's top-level `provides:` list, preserving
/// the rest of the text (comments included). Creates the block when absent.
fn insert_provides_entry(src: &str, entry: &str) -> String {
    let mut out = String::with_capacity(src.len() + entry.len() + 16);
    let mut inserted = false;
    for line in src.lines() {
        if !inserted && line.starts_with("provides:") {
            let rest = line["provides:".len()..].trim();
            if let Some(inline) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                // Inline list form: `provides: [a, b]` (or empty `[]`).
                let items = inline.trim();
                let joined = if items.is_empty() {
                    entry.to_string()
                } else {
                    format!("{items}, {entry}")
                };
                out.push_str(&format!("provides: [{joined}]\n"));
                inserted = true;
                continue;
            }
            // Block form: prepend the entry as the first list item (order is
            // not semantic), so we never have to find the end of the list.
            out.push_str(line);
            out.push('\n');
            out.push_str(&format!("  - {entry}\n"));
            inserted = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !inserted {
        out.push_str(&format!("provides:\n  - {entry}\n"));
    }
    out
}

/// Declare `schema.source` in the manifest text: extend an existing top-level
/// `schema:` block, or append one.
fn append_schema_source(src: &str, source: &str) -> String {
    let mut out = String::with_capacity(src.len() + source.len() + 24);
    let mut inserted = false;
    for line in src.lines() {
        out.push_str(line);
        out.push('\n');
        if !inserted && line.starts_with("schema:") {
            out.push_str(&format!("  source: {source}\n"));
            inserted = true;
        }
    }
    if !inserted {
        out.push_str(&format!("schema:\n  source: {source}\n"));
    }
    out
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
    fn idents_replace_hyphens_and_spaces() {
        assert_eq!(to_ident("vrm-schema"), "vrm_schema");
        assert_eq!(to_ident("my cool fmt"), "my_cool_fmt");
        assert_eq!(to_ident("toy"), "toy");
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
        assert!(
            !plug_info.contains('@'),
            "configure_file token left in plugInfo.json"
        );
        assert!(plug_info.contains(&format!(
            "../../../lib/libToyFileFormat{}",
            std::env::consts::DLL_SUFFIX
        )));

        // New file-format bundles are schema-cohost ready: when `ost plugin
        // build` generates a compiled schema fragment, CMake includes it into the
        // same shared library. Bundles without a schema simply have no fragment.
        let cmake = std::fs::read_to_string(dir.join("CMakeLists.txt").as_std_path()).unwrap();
        assert!(cmake.contains("OPENSTRATA_SCHEMA_SOURCES_FILE"));

        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    #[test]
    fn scaffolds_a_codeless_schema_bundle() {
        let dir = unique_tmp("scaffold-schema");
        // A schema needs no --extension.
        let files = scaffold(PluginKind::UsdSchema, "vrm-schema", None, &dir).expect("scaffold");

        // The schema source, the resource-only plugInfo.json, and the manifest landed.
        assert!(files.iter().any(|f| f.as_str() == "schema.usda"));
        assert!(files
            .iter()
            .any(|f| f.as_str() == "plugin/resources/vrm-schema/plugInfo.json"));
        // No shared-library source or .in template for a codeless schema.
        assert!(files.iter().all(|f| !f.as_str().ends_with(".in")));

        // Placeholders are substituted and the bundle loads as a codeless schema.
        let bundle = crate::Bundle::load(&dir).expect("loads");
        assert_eq!(bundle.manifest.name(), "vrm-schema");
        assert_eq!(bundle.manifest.kind(), PluginKind::UsdSchema);
        assert!(bundle.manifest.is_codeless_schema());

        // Direct CMake builds of the scaffold should protect usdGenSchema from
        // host locale encodings (notably Japanese Windows cp932) too.
        let cmake = std::fs::read_to_string(dir.join("CMakeLists.txt").as_std_path()).unwrap();
        assert!(cmake.contains("-E env"));
        assert!(cmake.contains("PYTHONUTF8=1"));
        assert!(cmake.contains("PYTHONIOENCODING=utf-8"));
        assert!(cmake.contains("USD_SCHEMA_PYTHON"));
        assert!(cmake.contains("\"${USD_SCHEMA_PYTHON}\" \"${USD_GEN_SCHEMA}\""));

        // The committed plugInfo.json declares the schema type with no token left
        // and no LibraryPath (it is resource-only).
        let plug_info = std::fs::read_to_string(bundle.plug_info().as_std_path()).unwrap();
        assert!(
            !plug_info.contains("{{"),
            "placeholder left in plugInfo.json"
        );
        assert!(plug_info.contains("VrmSchemaAPI"));
        assert!(!plug_info.contains("LibraryPath"));

        // The fixture applies the API and uses a *valid* USD identifier namespace
        // (`vrm_schema:`, not the hyphenated bundle name) so it opens on a real
        // runtime for the L4 apply/round-trip level.
        let fixture =
            std::fs::read_to_string(dir.join("tests/fixtures/basic.usda").as_std_path()).unwrap();
        assert!(fixture.contains("apiSchemas = [\"VrmSchemaAPI\"]"));
        assert!(fixture.contains("vrm_schema:example"));
        assert!(!fixture.contains("vrm-schema:example"));

        // The starter schema avoids non-ASCII prose so a fresh scaffold does not
        // trigger locale-sensitive usdGenSchema failures before users edit it.
        // It also avoids repeating `libraryPrefix` in the class name: usdGenSchema
        // composes those into the public schema type, so this source class still
        // generates `VrmSchemaAPI` without tripping the doctor hint.
        let schema = std::fs::read_to_string(dir.join("schema.usda").as_std_path()).unwrap();
        assert!(schema.is_ascii());
        assert!(schema.contains("string libraryPrefix    = \"VrmSchema\""));
        assert!(schema.contains("class \"API\""));
        assert!(!schema.contains("class \"VrmSchemaAPI\""));

        // The scaffolded bundle passes the static L0 diagnostics.
        let report = crate::diagnose(&bundle, &crate::RuntimeContext::default(), 0);
        assert!(report.passed(), "scaffolded schema should pass L0");
        assert!(
            report
                .diagnostics
                .iter()
                .all(|d| d.id != "schema.library_prefix"),
            "fresh scaffold should not warn about repeated libraryPrefix"
        );

        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    #[test]
    fn provides_entry_insertion_handles_block_inline_and_absent_forms() {
        // Block form: the entry is prepended to the list, comments preserved.
        let block = "plugin:\n  name: toy\n# keep me\nprovides:\n  - usd-fileformat:toy\nusd:\n  plug_info: p\n";
        let edited = insert_provides_entry(block, "usd-schema:ToyAPI");
        assert!(edited.contains("# keep me"));
        assert!(edited.contains("provides:\n  - usd-schema:ToyAPI\n  - usd-fileformat:toy"));

        // Inline form, non-empty and empty.
        let inline = insert_provides_entry("provides: [a, b]\n", "c");
        assert_eq!(inline, "provides: [a, b, c]\n");
        let empty = insert_provides_entry("provides: []\n", "c");
        assert_eq!(empty, "provides: [c]\n");

        // Absent: a new block is appended.
        let absent = insert_provides_entry("plugin:\n  name: toy\n", "c");
        assert!(absent.ends_with("provides:\n  - c\n"));
    }

    #[test]
    fn schema_source_append_extends_or_creates_the_section() {
        let with = append_schema_source("schema:\n  codeless: true\n", "schema/schema.usda");
        assert!(with.contains("schema:\n  source: schema/schema.usda\n  codeless: true"));
        let without = append_schema_source("plugin:\n  name: toy\n", "schema/schema.usda");
        assert!(without.ends_with("schema:\n  source: schema/schema.usda\n"));
    }

    #[test]
    fn schema_add_wires_a_fileformat_bundle() {
        let dir = unique_tmp("schema-add");
        scaffold(PluginKind::UsdFileformat, "toy", Some("toy"), &dir).expect("scaffold");

        let added =
            add_cohosted_schema(&dir, "API", "schema/schema.usda", false).expect("schema add");
        assert_eq!(added.schema_type, "ToyAPI");

        // The bundle reloads with the schema wired: the provides gate that
        // drives the co-hosted build flow, and the declared source path.
        let bundle = crate::Bundle::load(&dir).expect("reload");
        assert_eq!(bundle.manifest.schema_provides(), vec!["ToyAPI"]);
        let (src, declared) = bundle.schema_source();
        assert!(declared);
        assert_eq!(src, bundle.root.join("schema/schema.usda"));
        assert!(src.as_std_path().is_file());

        // Comments in the template manifest survived the textual edit.
        let manifest_src =
            std::fs::read_to_string(dir.join("openstrata.plugin.yaml").as_std_path()).unwrap();
        assert!(manifest_src.contains("# OpenStrata plugin bundle manifest."));

        // Compiled by default; the class avoids the double-prefix footgun and
        // the file stays ASCII (locale-safe for usdGenSchema).
        let schema = std::fs::read_to_string(src.as_std_path()).unwrap();
        assert!(schema.contains("bool skipCodeGeneration = false"));
        assert!(schema.contains("string libraryPrefix    = \"Toy\""));
        assert!(schema.contains("class \"API\""));
        assert!(schema.is_ascii());
        assert!(!schema.contains("{{"), "placeholder left in schema.usda");

        // Idempotence: the same type cannot be added twice.
        let err = add_cohosted_schema(&dir, "API", "other.usda", false).unwrap_err();
        assert!(err.to_string().contains("already provides"), "{err}");

        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    #[test]
    fn schema_add_refusals() {
        // A usd-schema bundle is refused (it IS the schema).
        let dir = unique_tmp("schema-add-kind");
        scaffold(PluginKind::UsdSchema, "vrm", None, &dir).expect("scaffold");
        let err = add_cohosted_schema(&dir, "API", "schema/schema.usda", false).unwrap_err();
        assert!(err.to_string().contains("already is a usd-schema"), "{err}");
        std::fs::remove_dir_all(dir.as_std_path()).ok();

        // Class and source shapes are validated.
        let dir = unique_tmp("schema-add-shape");
        scaffold(PluginKind::UsdFileformat, "toy", Some("toy"), &dir).expect("scaffold");
        assert!(add_cohosted_schema(&dir, "api", "schema/schema.usda", false).is_err());
        assert!(add_cohosted_schema(&dir, "API", "../outside.usda", false).is_err());
        assert!(add_cohosted_schema(&dir, "API", "schema/schema.txt", false).is_err());

        // Codeless writes skipCodeGeneration = true.
        let added =
            add_cohosted_schema(&dir, "API", "schema/schema.usda", true).expect("codeless add");
        assert!(added.codeless);
        let schema = std::fs::read_to_string(dir.join("schema/schema.usda").as_std_path()).unwrap();
        assert!(schema.contains("bool skipCodeGeneration = true"));

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
