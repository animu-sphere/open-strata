// SPDX-License-Identifier: Apache-2.0
//! On-disk layout for OpenStrata.
//!
//! Two roots matter:
//!
//! * The **user store** (`~/.ost`) holds runtimes, extensions, artifacts and
//!   caches that are shared across projects. See §10.4 / §17.3 of the design.
//! * The **project root** holds the per-project `openstrata.toml` and the
//!   generated `.strata/` directory. See §8.3.

use std::path::{Path, PathBuf};

use camino::Utf8PathBuf;

/// Directory name marking a project's generated/build state (§8.3).
pub const STATE_DIR: &str = ".strata";

/// The project manifest filename.
pub const PROJECT_MANIFEST: &str = "openstrata.toml";

/// Environment override for the user store root (useful in CI and tests).
pub const STORE_ENV: &str = "OST_HOME";

/// Resolve the user store root: `$OST_HOME` if set, otherwise `~/.ost`.
pub fn user_store() -> Utf8PathBuf {
    if let Ok(custom) = std::env::var(STORE_ENV) {
        if !custom.is_empty() {
            return Utf8PathBuf::from(custom);
        }
    }
    let home = directories::UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let mut p = Utf8PathBuf::from_path_buf(home).unwrap_or_else(|_| Utf8PathBuf::from("."));
    p.push(".ost");
    p
}

/// Standard subdirectories of the user store (§10.4).
pub struct Store {
    pub root: Utf8PathBuf,
}

impl Store {
    pub fn discover() -> Self {
        Store { root: user_store() }
    }

    pub fn runtimes(&self) -> Utf8PathBuf {
        self.root.join("runtimes")
    }
    pub fn extensions(&self) -> Utf8PathBuf {
        self.root.join("extensions")
    }
    pub fn artifacts(&self) -> Utf8PathBuf {
        self.root.join("artifacts")
    }
    pub fn cache(&self) -> Utf8PathBuf {
        self.root.join("cache")
    }
    pub fn sessions(&self) -> Utf8PathBuf {
        self.root.join("sessions")
    }
    pub fn logs(&self) -> Utf8PathBuf {
        self.root.join("logs")
    }
    /// User-provided platform manifests, layered over the built-in ones.
    pub fn platforms(&self) -> Utf8PathBuf {
        self.root.join("platforms")
    }
}

/// Walk up from `start` looking for a directory containing `openstrata.toml`.
pub fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut cur = Some(start);
    while let Some(dir) = cur {
        if dir.join(PROJECT_MANIFEST).is_file() {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}
