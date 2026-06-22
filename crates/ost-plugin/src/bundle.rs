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
        let src = std::fs::read_to_string(manifest_path.as_std_path())
            .map_err(|e| Error::io(manifest_path.to_string(), e))?;
        let manifest = PluginManifest::parse(&src)
            .map_err(|e| Error::parse(PLUGIN_MANIFEST, anyhow::Error::new(e)))?;
        Ok(Bundle {
            root: root.to_path_buf(),
            manifest,
        })
    }

    /// Resolve a bundle-relative path against the root.
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

    /// The bundle's `python/` directory (Python modules, if any).
    pub fn python_dir(&self) -> Utf8PathBuf {
        self.path("python")
    }
}
