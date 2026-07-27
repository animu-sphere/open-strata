// SPDX-License-Identifier: Apache-2.0
//! Artifact variant identity.
//!
//! Workflows are portable, but every lockfile and diagnostic must record the
//! concrete artifact variant (§3.2):
//!
//! ```text
//! linux-x86_64-glibc228-py313
//! macos-arm64-py313
//! windows-x86_64-msvc143-py313
//! ```

use serde::{Deserialize, Serialize};

use crate::host::{Arch, Host, Os};

/// The C library / ABI flavor that distinguishes otherwise-identical builds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Abi {
    /// glibc with a minimum version, e.g. `glibc228`.
    Glibc { version: String },
    /// MSVC toolset, e.g. `msvc143`.
    Msvc { toolset: String },
    /// macOS deployment floor, e.g. `macos145` for a runtime that cannot load
    /// below macOS 14.5. Measured from the binaries' own load commands, the way
    /// the glibc floor is measured from ELF symbol versions — a declared floor
    /// that nothing checked is what made `--require-target macos-arm64-py313`
    /// pass for an artifact the host could not load.
    Macos { version: String },
    /// Generic — no extra ABI token beyond os+arch. A macOS runtime whose
    /// binaries record no deployment target still lands here, because inventing
    /// a floor would be worse than reporting none.
    Native,
}

impl Abi {
    /// The short token used in a variant slug, if any.
    fn token(&self) -> Option<String> {
        match self {
            Abi::Glibc { version } => Some(format!("glibc{}", version.replace('.', ""))),
            Abi::Msvc { toolset } => Some(format!("msvc{toolset}")),
            Abi::Macos { version } => Some(format!("macos{}", version.replace('.', ""))),
            Abi::Native => None,
        }
    }

    /// Human-readable description, e.g. `glibc 2.28`, `msvc 143`, `native`.
    pub fn describe(&self) -> String {
        match self {
            Abi::Glibc { version } => format!("glibc {version}"),
            Abi::Msvc { toolset } => format!("msvc {toolset}"),
            Abi::Macos { version } => format!("macos {version}"),
            Abi::Native => "native".to_string(),
        }
    }

    /// Pick a sensible default ABI for a host OS.
    pub fn default_for(os: Os) -> Abi {
        match os {
            Os::Linux => Abi::Glibc {
                version: "2.28".into(),
            },
            Os::Windows => Abi::Msvc {
                toolset: "143".into(),
            },
            Os::Macos => Abi::Native,
        }
    }
}

/// A fully-qualified build/runtime variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Variant {
    pub os: Os,
    pub arch: Arch,
    pub abi: Abi,
    /// Python ABI tag without the `py` prefix, e.g. `313` for CPython 3.13.
    pub python: String,
}

impl Variant {
    pub fn new(host: &Host, abi: Abi, python: impl Into<String>) -> Variant {
        Variant {
            os: host.os,
            arch: host.arch,
            abi,
            python: python.into(),
        }
    }

    /// Render the canonical slug, e.g. `linux-x86_64-glibc228-py313`.
    ///
    /// This is the full artifact variant recorded in lockfiles and diagnostics.
    pub fn slug(&self) -> String {
        let mut parts = vec![self.os.as_str().to_string(), self.arch.as_str().to_string()];
        if let Some(tok) = self.abi.token() {
            parts.push(tok);
        }
        parts.push(format!("py{}", self.python));
        parts.join("-")
    }

    /// Render the short slug without the ABI token, e.g. `linux-x86_64-py313`.
    ///
    /// Used in human-facing runtime ids (§4.2), where the ABI is implied by the
    /// platform and need not clutter the identifier.
    pub fn short_slug(&self) -> String {
        format!(
            "{}-{}-py{}",
            self.os.as_str(),
            self.arch.as_str(),
            self.python
        )
    }

    /// CPython ABI tag in OpenStrata's normalized form, e.g.
    /// `cpython-313-x86_64-linux-gnu`. Recorded in `strata.lock` (§9.4).
    pub fn python_abi(&self) -> String {
        let os_suffix = match self.os {
            Os::Linux => "linux-gnu",
            Os::Macos => "darwin",
            Os::Windows => "windows-msvc",
        };
        format!("cpython-{}-{}-{os_suffix}", self.python, self.arch.as_str())
    }
}

impl std::fmt::Display for Variant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.slug())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_slug_matches_design() {
        let host = Host {
            os: Os::Linux,
            arch: Arch::X86_64,
        };
        let v = Variant::new(&host, Abi::default_for(Os::Linux), "313");
        assert_eq!(v.slug(), "linux-x86_64-glibc228-py313");
        assert_eq!(v.short_slug(), "linux-x86_64-py313");
    }

    #[test]
    fn macos_slug_has_no_abi_token_until_a_floor_is_measured() {
        let host = Host {
            os: Os::Macos,
            arch: Arch::Arm64,
        };
        let v = Variant::new(&host, Abi::default_for(Os::Macos), "313");
        assert_eq!(v.slug(), "macos-arm64-py313");

        // A measured deployment floor is a real ABI bound and appears in the
        // slug the way glibc228 and msvc143 do.
        let floored = Variant::new(
            &host,
            Abi::Macos {
                version: "14.5".into(),
            },
            "313",
        );
        assert_eq!(floored.slug(), "macos-arm64-macos145-py313");
        assert_eq!(floored.abi.describe(), "macos 14.5");
        // The short slug stays the human-facing runtime id, ABI-free.
        assert_eq!(floored.short_slug(), "macos-arm64-py313");
    }

    #[test]
    fn python_abi_tag_per_os() {
        let linux = Variant::new(
            &Host {
                os: Os::Linux,
                arch: Arch::X86_64,
            },
            Abi::default_for(Os::Linux),
            "313",
        );
        assert_eq!(linux.python_abi(), "cpython-313-x86_64-linux-gnu");

        let win = Variant::new(
            &Host {
                os: Os::Windows,
                arch: Arch::X86_64,
            },
            Abi::default_for(Os::Windows),
            "313",
        );
        assert_eq!(win.python_abi(), "cpython-313-x86_64-windows-msvc");

        let mac = Variant::new(
            &Host {
                os: Os::Macos,
                arch: Arch::Arm64,
            },
            Abi::default_for(Os::Macos),
            "313",
        );
        assert_eq!(mac.python_abi(), "cpython-313-arm64-darwin");
    }
}
