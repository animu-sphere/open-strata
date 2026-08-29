// SPDX-License-Identifier: Apache-2.0
//! Host tool discovery.
//!
//! OpenStrata detects host capabilities (§12.2 "Detect: yes") but never installs
//! them. This is a dependency-free `which`: it searches `PATH`, honoring
//! `PATHEXT` on Windows so `cmake` resolves to `cmake.exe`.

use std::path::PathBuf;

/// Locate an executable on `PATH`, returning its full path if found.
pub fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let extensions = if cfg!(windows) {
        windows_path_exts()
    } else {
        Vec::new()
    };

    for dir in std::env::split_paths(&path) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        if let Some(candidate) = candidate_in_dir(&dir, program, cfg!(windows), &extensions) {
            return Some(candidate);
        }
    }
    None
}

fn candidate_in_dir(
    dir: &std::path::Path,
    program: &str,
    windows: bool,
    extensions: &[String],
) -> Option<PathBuf> {
    let base = dir.join(program);
    // On Windows the program is usually named with an extension from PATHEXT.
    // Prefer those executable shims before an extensionless sibling: Node.js
    // installations carry both a Unix `npm` script and `npm.cmd`, but only the
    // latter can be launched through CreateProcess.
    if windows {
        for ext in extensions {
            let candidate = PathBuf::from(format!("{}{}", base.display(), ext));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    base.is_file().then_some(base)
}

/// Whether an executable is present on `PATH`.
pub fn has(program: &str) -> bool {
    which(program).is_some()
}

fn windows_path_exts() -> Vec<String> {
    std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string())
        .split(';')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_prefers_a_launchable_shim_over_an_extensionless_sibling() {
        let root = std::env::temp_dir().join(format!("ost-which-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("npm"), b"#!/bin/sh\n").unwrap();
        std::fs::write(root.join("npm.cmd"), b"@echo off\r\n").unwrap();
        assert_eq!(
            candidate_in_dir(&root, "npm", true, &[".exe".into(), ".cmd".into()]),
            Some(root.join("npm.cmd"))
        );
        std::fs::remove_dir_all(root).ok();
    }
}
