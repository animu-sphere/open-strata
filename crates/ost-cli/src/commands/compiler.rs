// SPDX-License-Identifier: Apache-2.0
//! Compiler-policy resolution shared by `configure`, `build`, and `plugin build`.
//!
//! The runtime supplies the SDK/ABI/prefix; the *compiler* is chosen separately
//! (§ runtime/compiler split) so an adopted OpenUSD install can build with the
//! host toolchain rather than being forced onto a compiler the runtime may not
//! ship. Precedence: CLI flags > `[build]` in `openstrata.toml` > default
//! (`host`).

use std::process::Command;

use camino::Utf8Path;
use clap::Args;

use ost_build::{Compiler, LockCompiler};
use ost_core::host::Os;
use ost_core::{Error, Result};
use ost_manifest::BuildConfig;

/// Shared `--compiler/--cc/--cxx` flags, flattened into each command.
#[derive(Debug, Clone, Default, Args)]
pub struct CompilerOpts {
    /// Compiler policy: `host` (default), `runtime`, or `explicit`.
    #[arg(long)]
    pub compiler: Option<String>,

    /// C compiler path (implies `--compiler explicit`).
    #[arg(long)]
    pub cc: Option<String>,

    /// C++ compiler path (implies `--compiler explicit`).
    #[arg(long)]
    pub cxx: Option<String>,
}

/// Resolve the effective [`Compiler`] from CLI flags layered over the manifest's
/// `[build]` table. Validates that an `explicit` policy has both compilers and
/// that they exist on disk.
pub fn resolve(opts: &CompilerOpts, manifest: Option<&BuildConfig>) -> Result<Compiler> {
    // Explicit paths on the CLI take precedence and imply the explicit policy.
    if opts.cc.is_some() || opts.cxx.is_some() {
        return explicit(opts.cc.clone(), opts.cxx.clone(), "--cc/--cxx");
    }

    // Otherwise a policy may be named on the CLI, else in the manifest.
    let policy = opts
        .compiler
        .clone()
        .or_else(|| manifest.map(|b| b.compiler.clone()))
        .unwrap_or_else(|| "host".to_string());

    match policy.as_str() {
        "host" => Ok(Compiler::Host),
        "runtime" => Ok(Compiler::Runtime),
        "explicit" => {
            // Explicit via manifest: take cc/cxx from `[build]`.
            let (cc, cxx) = manifest
                .map(|b| (b.cc.clone(), b.cxx.clone()))
                .unwrap_or((None, None));
            explicit(cc, cxx, "[build].cc/[build].cxx")
        }
        other => Err(Error::Operation(format!(
            "unknown compiler policy '{other}' (expected: host, runtime, explicit)"
        ))),
    }
}

fn explicit(cc: Option<String>, cxx: Option<String>, source: &str) -> Result<Compiler> {
    let (cc, cxx) = match (cc, cxx) {
        (Some(cc), Some(cxx)) => (cc, cxx),
        _ => {
            return Err(Error::Operation(format!(
                "explicit compiler policy requires both a C and C++ compiler (set {source})"
            )))
        }
    };
    for (label, path) in [("cc", &cc), ("cxx", &cxx)] {
        let p = std::path::Path::new(path);
        // Absolute so the path resolves the same from whatever build directory
        // CMake is invoked in, not relative to the caller's cwd.
        if !p.is_absolute() {
            return Err(Error::Operation(format!(
                "{label} compiler must be an absolute path: {path}"
            )));
        }
        if !p.is_file() {
            return Err(Error::Operation(format!(
                "{label} compiler not found: {path}"
            )));
        }
    }
    Ok(Compiler::Explicit { cc, cxx })
}

/// Build the lock record for a resolved compiler, capturing `--version` output
/// best-effort. `prefix` is the runtime artifact prefix (for the runtime policy).
pub fn to_lock(compiler: &Compiler, prefix: &Utf8Path, os: Os) -> LockCompiler {
    let (cc, cxx) = compiler.resolved_paths(prefix, os);
    LockCompiler {
        policy: compiler.policy().to_string(),
        cc_version: cc.as_deref().and_then(version_of),
        cxx_version: cxx.as_deref().and_then(version_of),
        cc,
        cxx,
    }
}

/// First line of `<compiler> --version`, or `None` if it cannot be run.
fn version_of(path: &str) -> Option<String> {
    // Skip the spawn for prospective paths (e.g. a runtime not yet pulled) that
    // don't exist on disk — the process launch would just fail anyway.
    if !std::path::Path::new(path).is_file() {
        return None;
    }
    let out = Command::new(path).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().next().map(|l| l.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(compiler: Option<&str>, cc: Option<&str>, cxx: Option<&str>) -> CompilerOpts {
        CompilerOpts {
            compiler: compiler.map(String::from),
            cc: cc.map(String::from),
            cxx: cxx.map(String::from),
        }
    }

    #[test]
    fn defaults_to_host() {
        assert_eq!(resolve(&opts(None, None, None), None).unwrap(), Compiler::Host);
    }

    #[test]
    fn cli_overrides_manifest() {
        let manifest = BuildConfig {
            compiler: "runtime".into(),
            cc: None,
            cxx: None,
        };
        // Manifest says runtime; CLI says host → host wins.
        let c = resolve(&opts(Some("host"), None, None), Some(&manifest)).unwrap();
        assert_eq!(c, Compiler::Host);
        // No CLI override → manifest policy applies.
        let c = resolve(&opts(None, None, None), Some(&manifest)).unwrap();
        assert_eq!(c, Compiler::Runtime);
    }

    #[test]
    fn explicit_requires_both_compilers() {
        // Only --cc given → error.
        assert!(resolve(&opts(None, Some("/usr/bin/clang"), None), None).is_err());
        // Manifest explicit without paths → error.
        let manifest = BuildConfig {
            compiler: "explicit".into(),
            cc: None,
            cxx: None,
        };
        assert!(resolve(&opts(None, None, None), Some(&manifest)).is_err());
    }

    #[test]
    fn unknown_policy_errors() {
        assert!(resolve(&opts(Some("clang-only"), None, None), None).is_err());
    }
}
