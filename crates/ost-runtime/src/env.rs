//! Environment generation and shell rendering (§7.5).
//!
//! OpenStrata never mutates the ambient environment silently (§23). Instead it
//! *describes* the environment a runtime needs, and `ost env` prints it for a
//! shell to evaluate. The OS adapter decides the dynamic-library variable and
//! the path separator; everything else is uniform.

use camino::Utf8Path;

use ost_core::host::Os;

/// Target shell for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Pwsh,
}

impl Shell {
    pub fn from_name(name: &str) -> Option<Shell> {
        match name.to_ascii_lowercase().as_str() {
            "bash" | "sh" | "zsh" => Some(Shell::Bash),
            "pwsh" | "powershell" | "ps" => Some(Shell::Pwsh),
            _ => None,
        }
    }

    /// The conventional default shell for a host OS.
    pub fn default_for(os: Os) -> Shell {
        match os {
            Os::Windows => Shell::Pwsh,
            _ => Shell::Bash,
        }
    }
}

/// How a variable is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvOp {
    /// Replace the variable's value outright.
    Set(String),
    /// Prepend a path entry, preserving any existing value.
    Prepend(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvVar {
    pub key: String,
    pub op: EnvOp,
}

/// An ordered set of environment mutations plus the path separator to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvSet {
    pub sep: char,
    pub vars: Vec<EnvVar>,
}

impl EnvSet {
    /// Build the environment that activates `prefix` for the given OS.
    ///
    /// `usd_plugins` adds `PXR_PLUGINPATH_NAME`; callers pass `true` when the
    /// resolved profile requests any USD capability.
    pub fn for_runtime(prefix: &Utf8Path, os: Os, py_minor: &str, usd_plugins: bool) -> EnvSet {
        let sep = if os == Os::Windows { ';' } else { ':' };
        // On Windows the dynamic loader uses PATH; elsewhere a dedicated var.
        let lib_key = match os {
            Os::Linux => "LD_LIBRARY_PATH",
            Os::Macos => "DYLD_LIBRARY_PATH",
            Os::Windows => "PATH",
        };
        // Join components one at a time so the OS separator stays consistent.
        let site = if os == Os::Windows {
            prefix.join("Lib").join("site-packages")
        } else {
            prefix
                .join("lib")
                .join(format!("python{py_minor}"))
                .join("site-packages")
        };

        let mut vars = vec![
            EnvVar {
                key: "PATH".into(),
                op: EnvOp::Prepend(prefix.join("bin").to_string()),
            },
            EnvVar {
                key: lib_key.into(),
                op: EnvOp::Prepend(prefix.join("lib").to_string()),
            },
            EnvVar {
                key: "PYTHONPATH".into(),
                op: EnvOp::Prepend(site.to_string()),
            },
            EnvVar {
                key: "CMAKE_PREFIX_PATH".into(),
                op: EnvOp::Prepend(prefix.to_string()),
            },
        ];
        if usd_plugins {
            vars.push(EnvVar {
                key: "PXR_PLUGINPATH_NAME".into(),
                op: EnvOp::Prepend(prefix.join("plugin").join("usd").to_string()),
            });
        }
        EnvSet { sep, vars }
    }

    /// Render the set as evaluable shell statements (no trailing prose).
    pub fn render(&self, shell: Shell) -> String {
        let mut out = String::new();
        for v in &self.vars {
            let line = match (shell, &v.op) {
                (Shell::Bash, EnvOp::Set(val)) => format!("export {}=\"{}\"", v.key, val),
                // `${KEY:+sep$KEY}` keeps the result clean when KEY is unset.
                (Shell::Bash, EnvOp::Prepend(val)) => {
                    format!("export {0}=\"{1}${{{0}:+{2}${0}}}\"", v.key, val, self.sep)
                }
                (Shell::Pwsh, EnvOp::Set(val)) => format!("$env:{} = \"{}\"", v.key, val),
                (Shell::Pwsh, EnvOp::Prepend(val)) => {
                    format!("$env:{0} = \"{1}{2}$env:{0}\"", v.key, val, self.sep)
                }
            };
            out.push_str(&line);
            out.push('\n');
        }
        out
    }

    /// The variables as `(key, rendered-value)` pairs, for `--json` output.
    pub fn pairs(&self) -> Vec<(String, String)> {
        self.vars
            .iter()
            .map(|v| {
                let val = match &v.op {
                    EnvOp::Set(s) | EnvOp::Prepend(s) => s.clone(),
                };
                (v.key.clone(), val)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;

    fn prefix() -> Utf8PathBuf {
        Utf8PathBuf::from("/store/runtimes/rt")
    }

    #[test]
    fn linux_uses_ld_library_path_and_colon() {
        let set = EnvSet::for_runtime(&prefix(), Os::Linux, "3.13", true);
        assert_eq!(set.sep, ':');
        let keys: Vec<_> = set.vars.iter().map(|v| v.key.as_str()).collect();
        assert!(keys.contains(&"LD_LIBRARY_PATH"));
        assert!(keys.contains(&"PXR_PLUGINPATH_NAME"));
    }

    #[test]
    fn bash_prepend_is_clean_when_unset() {
        let set = EnvSet::for_runtime(&prefix(), Os::Linux, "3.13", false);
        let rendered = set.render(Shell::Bash);
        // Validate the bash prepend template independent of path separators
        // (camino's join uses the host separator, so don't hard-code one).
        assert!(rendered.starts_with("export PATH=\""));
        assert!(rendered.contains("${PATH:+:$PATH}\""));
        assert!(!rendered.contains("PXR_PLUGINPATH_NAME"));
    }

    #[test]
    fn windows_routes_libs_through_path_with_semicolons() {
        let set = EnvSet::for_runtime(&prefix(), Os::Windows, "3.13", false);
        assert_eq!(set.sep, ';');
        // No dedicated lib var on Windows; both bin and lib land on PATH.
        let path_entries = set.vars.iter().filter(|v| v.key == "PATH").count();
        assert_eq!(path_entries, 2);
    }
}
