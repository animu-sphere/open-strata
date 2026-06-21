//! Host descriptor — what machine are we on?
//!
//! OpenStrata keeps workflows portable but records the concrete variant in
//! lockfiles and diagnostics (§3.2). This is the minimal, dependency-free
//! detection used by `ost doctor` and target resolution.

use serde::{Deserialize, Serialize};

/// Operating-system family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Linux,
    Macos,
    Windows,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "linux") {
            Os::Linux
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Windows
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::Macos => "macos",
            Os::Windows => "windows",
        }
    }
}

/// CPU architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    X86_64,
    Arm64,
}

impl Arch {
    pub fn current() -> Arch {
        if cfg!(target_arch = "aarch64") {
            Arch::Arm64
        } else {
            Arch::X86_64
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::Arm64 => "arm64",
        }
    }
}

/// A description of the host the CLI is running on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Host {
    pub os: Os,
    pub arch: Arch,
}

impl Host {
    pub fn detect() -> Host {
        Host {
            os: Os::current(),
            arch: Arch::current(),
        }
    }

    /// `linux-x86_64`, `macos-arm64`, ... — the portable variant prefix.
    pub fn slug(&self) -> String {
        format!("{}-{}", self.os.as_str(), self.arch.as_str())
    }

    /// Whether this host is the first-class implementation target (§23).
    pub fn is_primary(&self) -> bool {
        matches!((self.os, self.arch), (Os::Linux, Arch::X86_64))
    }
}
