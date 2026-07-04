// SPDX-License-Identifier: Apache-2.0
//! Loading a plugin bundle from disk.
//!
//! A bundle is a directory containing an `openstrata.plugin.yaml` at its root.
//! [`Bundle`] pairs the parsed manifest with the root path so the verification
//! levels can resolve the manifest's relative paths (`plug_info`, fixtures, the
//! built shared library) against the filesystem.

use camino::{Utf8Path, Utf8PathBuf};

use ost_core::{Error, Result};

use crate::model::{PluginManifest, PLUGIN_MANIFEST};

/// A loaded plugin bundle: its manifest plus the root it was loaded from.
#[derive(Debug, Clone)]
pub struct Bundle {
    pub root: Utf8PathBuf,
    pub manifest: PluginManifest,
}

impl Bundle {
    /// Load the bundle rooted at `root`.
    pub fn load(root: &Utf8Path) -> Result<Bundle> {
        let manifest_path = root.join(PLUGIN_MANIFEST);
        if !manifest_path.as_std_path().is_file() {
            return Err(Error::Operation(format!(
                "no {PLUGIN_MANIFEST} in '{root}' (is this a plugin bundle? try `ost plugin new`)"
            )));
        }

        // Canonicalize the root *once*, here, so every path derived from it is
        // absolute: the CMake `-S`/`-B`/`CMAKE_TOOLCHAIN_FILE` args (CMake
        // resolves a relative toolchain against the build dir, not the cwd) and
        // the session env (USD anchors a relative `PXR_PLUGINPATH_NAME` at its
        // own lib dir, not the cwd). A relative `<bundle>` CLI arg otherwise
        // makes both fail silently. One canonicalize at the boundary removes the
        // whole class of bug — and is the prerequisite for `--with <bundle>`.
        let root = canonicalize_root(root)?;

        let src = std::fs::read_to_string(manifest_path.as_std_path())
            .map_err(|e| Error::io(manifest_path.to_string(), e))?;
        let manifest = PluginManifest::parse(&src)
            .map_err(|e| Error::parse(PLUGIN_MANIFEST, anyhow::Error::new(e)))?;

        // Every filesystem path a manifest can name must stay inside the bundle.
        // Validate them once, here, so all downstream resolution (`plug_info`,
        // fixtures, the verification levels) is operating on trusted input.
        check_safe_relative("usd.plug_info", &manifest.usd.plug_info)?;
        for fixture in manifest.all_fixtures() {
            check_safe_relative("test fixture", fixture)?;
        }
        for dir in &manifest.requires.runtime_libs {
            check_safe_relative("requires.runtime_libs", dir)?;
        }
        // Notices are copied verbatim into a package, so they must stay in-bundle.
        for notice in &manifest.notices {
            check_safe_relative("notices", notice)?;
        }
        // The schema source is fed to usdGenSchema by the build step.
        if let Some(src) = manifest.schema.as_ref().and_then(|s| s.source.as_ref()) {
            check_safe_relative("schema.source", src)?;
        }

        Ok(Bundle { root, manifest })
    }

    /// Resolve a bundle-relative path against the root.
    ///
    /// This is a bare `join`; it does **not** confine the result to the bundle.
    /// Manifest-derived inputs (`plug_info`, fixtures) are kept safe by the
    /// [`check_safe_relative`] check `load` runs up front, and the `lib`/`python`
    /// callers pass literals. The exception is an operator-supplied fixture on
    /// the `ost plugin view` / `test-view` command line, which is trusted user
    /// input and may be absolute. Do not route any *untrusted* (e.g. new
    /// manifest-declared) path through here without validating it first.
    pub fn path(&self, rel: &str) -> Utf8PathBuf {
        self.root.join(rel)
    }

    /// Absolute path to the declared `plugInfo.json`.
    pub fn plug_info(&self) -> Utf8PathBuf {
        self.path(&self.manifest.usd.plug_info)
    }

    /// The directory a USD `PXR_PLUGINPATH_NAME` entry should point at: the
    /// directory *containing* the `plugInfo.json`.
    pub fn plug_info_root(&self) -> Utf8PathBuf {
        let p = self.plug_info();
        p.parent().map(Utf8Path::to_path_buf).unwrap_or(p)
    }

    /// The bundle's `lib/` directory (built shared libraries land here).
    pub fn lib_dir(&self) -> Utf8PathBuf {
        self.path("lib")
    }

    /// Bundle-relative runtime library directories declared by the manifest.
    pub fn runtime_lib_dirs(&self) -> Vec<Utf8PathBuf> {
        self.manifest
            .requires
            .runtime_libs
            .iter()
            .map(|dir| self.path(dir))
            .collect()
    }

    /// The bundle's `python/` directory (Python modules, if any).
    pub fn python_dir(&self) -> Utf8PathBuf {
        self.path("python")
    }

    /// Bundle-relative third-party notice files declared by the manifest.
    pub fn notices(&self) -> &[String] {
        &self.manifest.notices
    }

    /// The schema source (`schema.usda`) the schema build step regenerates
    /// from: the manifest-declared `schema.source` when present, else the
    /// conventional `schema.usda` at the bundle root. The bool says whether it
    /// was explicitly declared — a declared-but-missing source is a
    /// configuration error, while a missing conventional file just means
    /// "no schema here".
    pub fn schema_source(&self) -> (Utf8PathBuf, bool) {
        match self
            .manifest
            .schema
            .as_ref()
            .and_then(|s| s.source.as_ref())
        {
            Some(src) => (self.path(src), true),
            None => (self.path("schema.usda"), false),
        }
    }
}

/// Reject a manifest-declared path that is not a safe, bundle-relative path.
///
/// A bundle records the locations of its `plugInfo.json` and test fixtures
/// relative to its own root. Anything else — an absolute path, a `..` segment,
/// a Windows drive (`C:\…`) or UNC (`\\…`) prefix — could steer `ost plugin`
/// into reading or probing files a malicious `openstrata.plugin.yaml` was never
/// meant to reach (harness §SEC-002).
///
/// The check is host-independent: it splits on both `/` and `\` and inspects
/// the raw string for drive/UNC prefixes, so a `..\` written for Windows is
/// still caught when the bundle is validated on Linux CI (and vice versa) —
/// `Path::components()` alone would miss it, since the host's separator rules
/// differ.
pub(crate) fn check_safe_relative(field: &str, rel: &str) -> Result<()> {
    let reject = |why: &str| {
        Err(Error::config(format!(
            "{field}: '{rel}' is not a safe bundle-relative path — {why}"
        )))
    };

    if rel.is_empty() {
        return reject("it is empty");
    }
    // Windows drive prefix (`C:` / `C:\…`), independent of the host OS.
    let b = rel.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return reject("it looks like a drive path");
    }
    // Unix-absolute (`/…`) or UNC / backslash-absolute (`\\…`, `\…`).
    if rel.starts_with('/') || rel.starts_with('\\') {
        return reject("it is absolute");
    }
    // Inspect each segment under either separator so a `..` is caught regardless
    // of which slash was used to write it.
    for seg in rel.split(['/', '\\']) {
        if seg == ".." {
            return reject("it escapes the bundle with '..'");
        }
    }
    Ok(())
}

/// Canonicalize the bundle root to an absolute path, stripping Windows'
/// `\\?\` *verbatim* prefix that `std::fs::canonicalize` adds.
///
/// CMake and USD both mishandle the verbatim form (CMake treats `\\?\C:\…` as a
/// UNC path; USD's plugin loader fails to match it), so we hand them the plain
/// drive path. On non-Windows hosts `canonicalize` already returns a clean
/// absolute path, so [`strip_verbatim`] is a no-op there.
fn canonicalize_root(root: &Utf8Path) -> Result<Utf8PathBuf> {
    let canon =
        std::fs::canonicalize(root.as_std_path()).map_err(|e| Error::io(root.to_string(), e))?;
    let utf8 = Utf8PathBuf::from_path_buf(canon)
        .map_err(|p| Error::config(format!("bundle path is not valid UTF-8: {}", p.display())))?;
    Ok(strip_verbatim(utf8))
}

/// Remove a Windows `\\?\` verbatim prefix: `\\?\C:\x` -> `C:\x`,
/// `\\?\UNC\srv\share` -> `\\srv\share`. Any other path is returned unchanged.
fn strip_verbatim(p: Utf8PathBuf) -> Utf8PathBuf {
    let s = p.as_str();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return Utf8PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return Utf8PathBuf::from(rest);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_bundle_relative_paths() {
        for ok in [
            "plugInfo.json",
            "plugin/resources/usdluma/plugInfo.json",
            "tests/fixtures/basic.lumagraph",
            "./resources/plugInfo.json",
            "a.b/c-d_e/f",
        ] {
            assert!(check_safe_relative("f", ok).is_ok(), "should accept {ok:?}");
        }
    }

    #[test]
    fn rejects_escaping_and_absolute_paths() {
        for bad in [
            "",
            "..",
            "../outside/plugInfo.json",
            "plugin/../../etc/passwd",
            "..\\windows\\escape",
            "/etc/passwd",
            "\\\\server\\share\\x",
            "\\windows\\x",
            "C:\\Windows\\System32",
            "d:relative",
        ] {
            assert!(
                check_safe_relative("f", bad).is_err(),
                "should reject {bad:?}"
            );
        }
    }

    fn write_bundle(plug_info: &str) -> Utf8PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut root = Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
        root.push(format!("ost-bundle-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(root.as_std_path()).unwrap();
        let manifest = format!(
            "plugin:\n  name: x\n  version: 0.1.0\n  kind: usd-fileformat\n\
             runtime:\n  openusd: \">=24.11,<25.0\"\nusd:\n  plug_info: {plug_info}\n"
        );
        std::fs::write(root.join(PLUGIN_MANIFEST).as_std_path(), manifest).unwrap();
        root
    }

    #[test]
    fn load_rejects_a_manifest_that_escapes_the_bundle() {
        let root = write_bundle("../../../etc/passwd");
        let err = Bundle::load(&root).expect_err("escaping plug_info must be rejected");
        assert_eq!(err.code(), "INVALID_CONFIG");
        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn load_accepts_a_well_formed_bundle() {
        let root = write_bundle("resources/plugInfo.json");
        let bundle = Bundle::load(&root).expect("a safe manifest loads");
        // Compare against the *loaded* (canonicalized) root: `load` absolutizes
        // the root, which may differ from `root` here (symlinked temp dirs, the
        // `\\?\` strip on Windows).
        assert_eq!(
            bundle.plug_info(),
            bundle.root.join("resources/plugInfo.json")
        );
        assert!(bundle.root.is_absolute(), "load must absolutize the root");
        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn load_rejects_runtime_libs_that_escape_the_bundle() {
        let root = write_bundle("resources/plugInfo.json");
        let manifest_path = root.join(PLUGIN_MANIFEST);
        let manifest = std::fs::read_to_string(manifest_path.as_std_path()).unwrap();
        std::fs::write(
            manifest_path.as_std_path(),
            format!("{manifest}requires:\n  runtime_libs: [../outside]\n"),
        )
        .unwrap();
        let err = Bundle::load(&root).expect_err("escaping runtime_libs must be rejected");
        assert_eq!(err.code(), "INVALID_CONFIG");
        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn load_rejects_notices_that_escape_the_bundle() {
        let root = write_bundle("resources/plugInfo.json");
        let manifest_path = root.join(PLUGIN_MANIFEST);
        let manifest = std::fs::read_to_string(manifest_path.as_std_path()).unwrap();
        std::fs::write(
            manifest_path.as_std_path(),
            format!("{manifest}notices: [../../etc/passwd]\n"),
        )
        .unwrap();
        let err = Bundle::load(&root).expect_err("escaping notices must be rejected");
        assert_eq!(err.code(), "INVALID_CONFIG");
        std::fs::remove_dir_all(root.as_std_path()).ok();
    }

    #[test]
    fn strip_verbatim_removes_windows_prefixes() {
        assert_eq!(
            strip_verbatim(Utf8PathBuf::from(r"\\?\C:\dev\bundle")),
            Utf8PathBuf::from(r"C:\dev\bundle")
        );
        assert_eq!(
            strip_verbatim(Utf8PathBuf::from(r"\\?\UNC\srv\share\bundle")),
            Utf8PathBuf::from(r"\\srv\share\bundle")
        );
        // A plain absolute path is untouched.
        assert_eq!(
            strip_verbatim(Utf8PathBuf::from("/home/u/bundle")),
            Utf8PathBuf::from("/home/u/bundle")
        );
    }
}
