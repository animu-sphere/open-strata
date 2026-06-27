// SPDX-License-Identifier: Apache-2.0
//! Session environment preview (harness §9).
//!
//! `ost plugin run` will compose an ephemeral session by taking the runtime's
//! [`EnvSet`] and adding the bundle's discovery/lib/python roots. In Phase 4a we
//! cannot *launch* (no real runtime), but we can show exactly the environment we
//! *would* set — the doctor's "session env preview" — so the contract is visible
//! and testable before the execution levels light up.

use camino::Utf8Path;

use ost_core::host::Os;
use ost_runtime::{EnvOp, EnvSet, EnvVar};

use crate::bundle::Bundle;

/// The dynamic-library environment variable for an OS (mirrors `EnvSet`).
fn lib_key(os: Os) -> &'static str {
    match os {
        Os::Linux => "LD_LIBRARY_PATH",
        Os::Macos => "DYLD_LIBRARY_PATH",
        Os::Windows => "PATH",
    }
}

/// Render a path with forward slashes (portable, matches `EnvSet::for_runtime`).
fn portable(p: &Utf8Path) -> String {
    p.to_string().replace('\\', "/")
}

/// The environment additions a bundle contributes on top of a runtime session:
/// its `plugInfo` root on `PXR_PLUGINPATH_NAME`, declared runtime library dirs
/// plus `lib/` on the dynamic-lib path, and `python/` on `PYTHONPATH`.
pub fn bundle_vars(bundle: &Bundle, os: Os) -> Vec<EnvVar> {
    let mut vars = vec![EnvVar {
        key: "PXR_PLUGINPATH_NAME".into(),
        op: EnvOp::Prepend(portable(&bundle.plug_info_root())),
    }];
    for dir in bundle.runtime_lib_dirs() {
        vars.push(EnvVar {
            key: lib_key(os).into(),
            op: EnvOp::Prepend(portable(&dir)),
        });
    }
    vars.extend([
        EnvVar {
            key: lib_key(os).into(),
            op: EnvOp::Prepend(portable(&bundle.lib_dir())),
        },
        EnvVar {
            key: "PYTHONPATH".into(),
            op: EnvOp::Prepend(portable(&bundle.python_dir())),
        },
    ]);
    vars
}

/// Compose the full session env: the runtime's `EnvSet` followed by the bundle's
/// additions. `Prepend` semantics compose, so the bundle's roots take priority
/// while the runtime's remain present.
pub fn session_env(runtime_env: &EnvSet, bundle: &Bundle, os: Os) -> EnvSet {
    session_env_with(runtime_env, bundle, &[], os)
}

/// Compose a session env for a primary bundle plus additional plugin bundles.
///
/// `EnvSet::Prepend` means later entries for the same key win priority. We add
/// `with` bundles in reverse order, then the primary bundle last, so the final
/// search order is primary first, followed by `--with` bundles in CLI order,
/// followed by the runtime.
pub fn session_env_with(runtime_env: &EnvSet, bundle: &Bundle, with: &[Bundle], os: Os) -> EnvSet {
    let mut vars = runtime_env.vars.clone();
    for extra in with.iter().rev() {
        vars.extend(bundle_vars(extra, os));
    }
    vars.extend(bundle_vars(bundle, os));
    EnvSet {
        sep: runtime_env.sep,
        vars,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PluginManifest;
    use camino::Utf8PathBuf;

    fn bundle() -> Bundle {
        let manifest = PluginManifest::parse(
            r#"
plugin: { name: usdluma, version: 0.1.0, kind: usd-fileformat }
runtime: { openusd: ">=24.11,<25.0" }
usd: { plug_info: plugin/resources/usdluma/plugInfo.json }
"#,
        )
        .unwrap();
        Bundle {
            root: Utf8PathBuf::from("/bundles/usdluma"),
            manifest,
        }
    }

    #[test]
    fn bundle_vars_point_at_the_plug_info_root_not_the_file() {
        let vars = bundle_vars(&bundle(), Os::Linux);
        let pxr = vars
            .iter()
            .find(|v| v.key == "PXR_PLUGINPATH_NAME")
            .expect("has PXR_PLUGINPATH_NAME");
        match &pxr.op {
            EnvOp::Prepend(p) => {
                assert!(p.ends_with("plugin/resources/usdluma"), "got {p}");
                assert!(!p.ends_with("plugInfo.json"));
            }
            _ => panic!("expected prepend"),
        }
    }

    #[test]
    fn windows_routes_lib_through_path() {
        let vars = bundle_vars(&bundle(), Os::Windows);
        assert!(vars.iter().any(|v| v.key == "PATH"));
        assert!(!vars.iter().any(|v| v.key == "LD_LIBRARY_PATH"));
    }

    #[test]
    fn runtime_libs_are_added_to_loader_path() {
        let mut b = bundle();
        b.manifest.requires.runtime_libs = vec!["third_party/zlib/bin".into()];
        let vars = bundle_vars(&b, Os::Linux);
        let libs: Vec<_> = vars
            .iter()
            .filter_map(|v| match (&v.key[..], &v.op) {
                ("LD_LIBRARY_PATH", EnvOp::Prepend(p)) => Some(p.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            libs,
            vec![
                "/bundles/usdluma/third_party/zlib/bin",
                "/bundles/usdluma/lib"
            ]
        );
    }

    #[test]
    fn multi_bundle_session_keeps_primary_first_then_with_order() {
        let runtime = EnvSet {
            sep: ':',
            vars: vec![EnvVar {
                key: "PXR_PLUGINPATH_NAME".into(),
                op: EnvOp::Prepend("/runtime/plugin/usd".into()),
            }],
        };
        let primary = bundle();
        let with_a = Bundle {
            root: Utf8PathBuf::from("/bundles/a"),
            ..bundle()
        };
        let with_b = Bundle {
            root: Utf8PathBuf::from("/bundles/b"),
            ..bundle()
        };
        let env = session_env_with(&runtime, &primary, &[with_a, with_b], Os::Linux);
        let resolved = env.resolve_over(&std::collections::HashMap::new());
        let pxr = resolved
            .iter()
            .find(|(k, _)| k == "PXR_PLUGINPATH_NAME")
            .map(|(_, v)| v.as_str())
            .unwrap();
        assert_eq!(
            pxr,
            "/bundles/usdluma/plugin/resources/usdluma:/bundles/a/plugin/resources/usdluma:/bundles/b/plugin/resources/usdluma:/runtime/plugin/usd"
        );
    }

    #[test]
    fn paths_are_forward_slashed() {
        let win = Bundle {
            root: Utf8PathBuf::from(r"C:\bundles\usdluma"),
            ..bundle()
        };
        for v in bundle_vars(&win, Os::Windows) {
            if let EnvOp::Prepend(p) = &v.op {
                assert!(!p.contains('\\'), "{}: {p}", v.key);
            }
        }
    }
}
