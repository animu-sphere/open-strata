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

    /// The executable to launch for an interactive devshell, with any leading
    /// arguments. Returns the primary candidate; callers handle "not found".
    pub fn launch_command(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Shell::Bash => ("bash", &["-i"]),
            Shell::Pwsh => ("pwsh", &["-NoLogo"]),
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
        // Emit forward slashes everywhere: they are portable (accepted by
        // Windows APIs and CMake) and avoid the ugly `/`+`\` mix that arises
        // when the store root uses `/` and `Utf8Path::join` adds `\`. On Linux
        // this is already a no-op.
        let path = |p: &Utf8Path| p.to_string().replace('\\', "/");

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
                op: EnvOp::Prepend(path(&prefix.join("bin"))),
            },
            EnvVar {
                key: lib_key.into(),
                op: EnvOp::Prepend(path(&prefix.join("lib"))),
            },
            EnvVar {
                key: "PYTHONPATH".into(),
                op: EnvOp::Prepend(path(&site)),
            },
            EnvVar {
                key: "CMAKE_PREFIX_PATH".into(),
                op: EnvOp::Prepend(path(prefix)),
            },
        ];
        if usd_plugins {
            vars.push(EnvVar {
                key: "PXR_PLUGINPATH_NAME".into(),
                op: EnvOp::Prepend(path(&prefix.join("plugin").join("usd"))),
            });
        }
        EnvSet { sep, vars }
    }

    /// Build the environment that activates an *adopted* OpenUSD install at
    /// `root` (Phase 4b `local` source). USD's own install layout differs from
    /// the OpenStrata prefix: Python bindings live under `lib/python` (not a
    /// versioned `site-packages`), so this maps the install's real directories
    /// rather than the store layout.
    pub fn for_usd_install(root: &Utf8Path, os: Os) -> EnvSet {
        let sep = if os == Os::Windows { ';' } else { ':' };
        let lib_key = match os {
            Os::Linux => "LD_LIBRARY_PATH",
            Os::Macos => "DYLD_LIBRARY_PATH",
            Os::Windows => "PATH",
        };
        let path = |p: &Utf8Path| p.to_string().replace('\\', "/");

        let vars = vec![
            EnvVar {
                key: "PATH".into(),
                op: EnvOp::Prepend(path(&root.join("bin"))),
            },
            // On Windows lib_key is PATH, so USD's lib/ DLLs land on PATH too.
            EnvVar {
                key: lib_key.into(),
                op: EnvOp::Prepend(path(&root.join("lib"))),
            },
            EnvVar {
                // USD installs put the `pxr` package under lib/python.
                key: "PYTHONPATH".into(),
                op: EnvOp::Prepend(path(&root.join("lib").join("python"))),
            },
            EnvVar {
                key: "CMAKE_PREFIX_PATH".into(),
                op: EnvOp::Prepend(path(root)),
            },
            EnvVar {
                key: "PXR_PLUGINPATH_NAME".into(),
                op: EnvOp::Prepend(path(&root.join("plugin").join("usd"))),
            },
        ];
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

    /// Resolve the set against the current process environment, returning the
    /// final `(key, value)` pairs. `Prepend` ops read the existing value (and
    /// any earlier op on the same key) so repeated keys compose correctly.
    pub fn resolve(&self) -> Vec<(String, String)> {
        use std::collections::HashMap;
        let mut overlay: HashMap<String, String> = HashMap::new();
        // Preserve order of first appearance for a stable result.
        let mut order: Vec<String> = Vec::new();
        for v in &self.vars {
            if !overlay.contains_key(&v.key) {
                order.push(v.key.clone());
            }
            match &v.op {
                EnvOp::Set(val) => {
                    overlay.insert(v.key.clone(), val.clone());
                }
                EnvOp::Prepend(val) => {
                    let existing = overlay
                        .get(&v.key)
                        .cloned()
                        .or_else(|| std::env::var(&v.key).ok());
                    let next = match existing {
                        Some(e) if !e.is_empty() => format!("{val}{}{e}", self.sep),
                        _ => val.clone(),
                    };
                    overlay.insert(v.key.clone(), next);
                }
            }
        }
        order
            .into_iter()
            .map(|k| {
                let val = overlay.remove(&k).unwrap_or_default();
                (k, val)
            })
            .collect()
    }

    /// Apply the resolved environment to a child process command.
    pub fn apply(&self, cmd: &mut std::process::Command) {
        for (k, v) in self.resolve() {
            cmd.env(k, v);
        }
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
    fn paths_are_forward_slashed() {
        // Even given a backslash-laden prefix, emitted values use `/` only.
        let win_prefix = Utf8PathBuf::from(r"C:\store\runtimes\rt");
        let set = EnvSet::for_runtime(&win_prefix, Os::Windows, "3.13", true);
        for (key, value) in set.pairs() {
            assert!(!value.contains('\\'), "{key} still has a backslash: {value}");
        }
    }

    #[test]
    fn usd_install_uses_lib_python_for_pythonpath() {
        let set = EnvSet::for_usd_install(&Utf8PathBuf::from("/opt/usd"), Os::Linux);
        let py = set
            .vars
            .iter()
            .find(|v| v.key == "PYTHONPATH")
            .expect("PYTHONPATH present");
        match &py.op {
            EnvOp::Prepend(p) => assert!(p.ends_with("lib/python"), "got {p}"),
            _ => panic!("expected prepend"),
        }
        // The adopted install still exposes USD's plugin discovery root.
        assert!(set.vars.iter().any(|v| v.key == "PXR_PLUGINPATH_NAME"));
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
