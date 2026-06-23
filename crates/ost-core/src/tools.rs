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

    for dir in std::env::split_paths(&path) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let base = dir.join(program);
        if base.is_file() {
            return Some(base);
        }
        // On Windows the program is usually named with an extension from PATHEXT.
        if cfg!(windows) {
            for ext in windows_path_exts() {
                let candidate = PathBuf::from(format!("{}{}", base.display(), ext));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
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
