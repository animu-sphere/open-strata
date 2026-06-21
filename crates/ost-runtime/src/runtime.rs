//! Runtime identity (§4.2).
//!
//! A [`Runtime`] is the concrete thing you activate: a platform year, resolved
//! to a host variant, narrowed to a profile. Its id is the stable handle used in
//! the store layout and lockfile, e.g.
//! `openstrata-cy2026-linux-x86_64-py313-usd`.

use camino::Utf8PathBuf;

use ost_core::paths::Store;
use ost_core::variant::Abi;
use ost_core::{Host, Variant};

/// A resolved runtime selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtime {
    pub platform: String,
    pub variant: Variant,
    pub profile: String,
}

impl Runtime {
    /// Resolve a runtime for the given host, deriving the variant from the
    /// platform's Python version and the host's default ABI.
    pub fn resolve(
        platform: impl Into<String>,
        profile: impl Into<String>,
        host: &Host,
        python_version: &str,
    ) -> Runtime {
        let abi = Abi::default_for(host.os);
        let variant = Variant::new(host, abi, python_abi_tag(python_version));
        Runtime {
            platform: platform.into(),
            variant,
            profile: profile.into(),
        }
    }

    /// Stable runtime id, e.g. `openstrata-cy2026-linux-x86_64-py313-usd`.
    pub fn id(&self) -> String {
        format!(
            "openstrata-{}-{}-{}",
            self.platform,
            self.variant.short_slug(),
            self.profile
        )
    }

    /// Install prefix under the user store: `~/.ost/runtimes/<id>`.
    pub fn prefix(&self, store: &Store) -> Utf8PathBuf {
        store.runtimes().join(self.id())
    }
}

/// Convert a platform Python version to a CPython ABI tag.
///
/// `"3.13.x"` -> `"313"`, `"3.11.4"` -> `"311"`.
pub fn python_abi_tag(version: &str) -> String {
    let mut parts = version.split('.');
    let major = parts.next().filter(|s| !s.is_empty()).unwrap_or("3");
    let minor = parts.next().filter(|s| is_numeric(s)).unwrap_or("0");
    format!("{major}{minor}")
}

/// Extract `major.minor`, e.g. `"3.13.x"` -> `"3.13"`. Used for site-packages.
pub fn python_minor(version: &str) -> String {
    let mut parts = version.split('.');
    let major = parts.next().filter(|s| !s.is_empty()).unwrap_or("3");
    let minor = parts.next().filter(|s| is_numeric(s)).unwrap_or("0");
    format!("{major}.{minor}")
}

fn is_numeric(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ost_core::host::{Arch, Os};

    #[test]
    fn abi_tag_parsing() {
        assert_eq!(python_abi_tag("3.13.x"), "313");
        assert_eq!(python_abi_tag("3.11.4"), "311");
        assert_eq!(python_minor("3.13.x"), "3.13");
    }

    #[test]
    fn runtime_id_matches_design() {
        let host = Host {
            os: Os::Linux,
            arch: Arch::X86_64,
        };
        let rt = Runtime::resolve("cy2026", "usd", &host, "3.13.x");
        assert_eq!(rt.id(), "openstrata-cy2026-linux-x86_64-py313-usd");
    }
}
