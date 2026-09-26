// SPDX-License-Identifier: Apache-2.0
//! UsdImaging adapter ownership and registration contracts.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Bundle, Diagnostic, PluginKind};

/// Singleton registration keys, in the same vocabulary as component provides.
pub fn imaging_keys(bundle: &Bundle) -> Vec<&str> {
    bundle
        .manifest
        .provides
        .iter()
        .map(String::as_str)
        .filter(|key| key.starts_with("usd-imaging:") || key.starts_with("usd-imaging-prim:"))
        .collect()
}

/// Compare declarations with actual metadata before trusting runtime discovery.
pub fn validate_imaging_metadata(bundle: &Bundle, json: &serde_json::Value) -> Diagnostic {
    const ID: &str = "imaging.metadata";
    let validate = || -> Result<usize, String> {
        let mut actual = BTreeSet::new();
        for plugin in json
            .get("Plugins")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            for (name, details) in plugin
                .pointer("/Info/Types")
                .and_then(serde_json::Value::as_object)
                .into_iter()
                .flatten()
            {
                let bases = details.get("bases").and_then(serde_json::Value::as_array);
                for (base, field, prefix) in [
                    (
                        "UsdImagingAPISchemaAdapter",
                        "apiSchemaName",
                        "usd-imaging:",
                    ),
                    ("UsdImagingPrimAdapter", "primTypeName", "usd-imaging-prim:"),
                ] {
                    if !bases.is_some_and(|bases| bases.iter().any(|b| b.as_str() == Some(base))) {
                        continue;
                    }
                    if details
                        .get("isInternal")
                        .is_some_and(|v| v != &serde_json::Value::Bool(false))
                    {
                        return Err(format!(
                            "external adapter '{name}' must not declare isInternal"
                        ));
                    }
                    let key = details
                        .get(field)
                        .and_then(serde_json::Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                        .ok_or_else(|| format!("adapter '{name}' needs a non-empty {field}"))?;
                    let provision = format!("{prefix}{key}");
                    if !actual.insert(provision.clone()) {
                        return Err(format!("multiple adapter types register '{provision}'"));
                    }
                }
            }
        }
        let keys = imaging_keys(bundle);
        let declared: BTreeSet<_> = keys.iter().map(|s| (*s).to_string()).collect();
        if actual.is_empty() || actual != declared || keys.len() != declared.len() {
            return Err(format!("imaging provides must exactly match unique adapter keys: declared {declared:?}, plugInfo.json {actual:?}"));
        }
        Ok(actual.len())
    };
    match validate() {
        Ok(count) => Diagnostic::pass(ID, 0, format!("{count} declared adapter key(s) match plugInfo.json")),
        Err(message) => Diagnostic::fail(ID, 0, message, vec!["declare usd-imaging:<apiSchemaName> or usd-imaging-prim:<primTypeName> for every external adapter".into()]),
    }
}

/// Reject ambiguous adapter ownership before constructing a session.
pub fn imaging_conflicts(bundles: &[&Bundle]) -> Vec<String> {
    let mut owners = BTreeMap::<&str, BTreeSet<&str>>::new();
    for bundle in bundles {
        for key in imaging_keys(bundle) {
            owners
                .entry(key)
                .or_default()
                .insert(bundle.manifest.name());
        }
    }
    owners
        .into_iter()
        .filter(|(_, owners)| owners.len() > 1)
        .map(|(key, owners)| {
            format!(
                "multiple bundles provide '{key}': {}",
                owners.into_iter().collect::<Vec<_>>().join(", ")
            )
        })
        .collect()
}

pub(crate) fn needs_imaging(bundle: &Bundle) -> bool {
    bundle.manifest.kind() == PluginKind::UsdImaging
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PluginManifest, Probe, RuntimeContext, Session, Status, ToolOutput};

    fn bundle(id: &str, key: &str) -> Bundle {
        Bundle {
            root: format!("/imaging-tests/{id}").into(), output_root: None,
            manifest: PluginManifest::parse(&format!(
                "plugin: {{ name: {id}, version: 0.1.0, kind: usd-imaging }}\nruntime: {{ openusd: '>=26.05,<27.0' }}\nprovides: ['{key}']\nusd: {{ plug_info: plugInfo.json }}\n"
            )).unwrap(),
        }
    }

    fn metadata() -> serde_json::Value {
        serde_json::json!({"Plugins":[{"Type":"library","Info":{"Types":{
            "MToonAdapter":{"bases":["UsdImagingAPISchemaAdapter"],"apiSchemaName":"VrmMToonAPI"}
        }}}]})
    }

    #[test]
    fn imaging_metadata_rejects_lies_and_duplicate_adapter_keys() {
        let b = bundle("vrmImaging", "usd-imaging:VrmMToonAPI");
        assert_eq!(
            validate_imaging_metadata(&b, &metadata()).status,
            Status::Pass
        );
        let mut bad = metadata();
        bad["Plugins"][0]["Info"]["Types"]["Other"] =
            bad["Plugins"][0]["Info"]["Types"]["MToonAdapter"].clone();
        assert_eq!(validate_imaging_metadata(&b, &bad).status, Status::Fail);
        let mut internal = metadata();
        internal["Plugins"][0]["Info"]["Types"]["MToonAdapter"]["isInternal"] = true.into();
        assert_eq!(
            validate_imaging_metadata(&b, &internal).status,
            Status::Fail
        );
        assert_eq!(
            validate_imaging_metadata(&bundle("wrong", "usd-imaging:Absent"), &metadata()).status,
            Status::Fail
        );
        let mut missing = metadata();
        missing["Plugins"][0]["Info"]["Types"]["MToonAdapter"]["apiSchemaName"] = "".into();
        assert_eq!(validate_imaging_metadata(&b, &missing).status, Status::Fail);
    }

    #[test]
    fn prim_adapters_have_a_separate_registration_namespace() {
        let prim = bundle("prim", "usd-imaging-prim:VrmMToonAPI");
        let api = bundle("api", "usd-imaging:VrmMToonAPI");
        let mut json = metadata();
        json["Plugins"][0]["Info"]["Types"] = serde_json::json!({
            "PrimAdapter":{"bases":["UsdImagingPrimAdapter"],"primTypeName":"VrmMToonAPI"}
        });
        assert_eq!(validate_imaging_metadata(&prim, &json).status, Status::Pass);
        assert!(imaging_conflicts(&[&prim, &api]).is_empty());
        let other = bundle("other", "usd-imaging:VrmMToonAPI");
        assert_eq!(imaging_conflicts(&[&api, &other]).len(), 1);
        let graph = crate::validate_workspace(&[api, other]);
        assert!(!graph.passed);
        assert!(graph
            .issues
            .iter()
            .any(|i| i.code == "WORKSPACE_IMAGING_CONFLICT"));
    }

    #[test]
    fn imaging_is_intrinsic_and_core_runtime_fails_l1() {
        let b = bundle("vrmImaging", "usd-imaging:VrmMToonAPI");
        assert!(b
            .manifest
            .required_capabilities()
            .contains(&"hydra-preview".into()));
        for (pulled, real, imaging, expected) in [
            (false, false, false, Status::Skip),
            (true, false, true, Status::Fail),
            (true, true, false, Status::Fail),
            (true, true, true, Status::Pass),
        ] {
            let report = crate::diagnose(
                &b,
                &RuntimeContext {
                    pulled,
                    real,
                    imaging,
                    ..Default::default()
                },
                1,
            );
            assert_eq!(
                report
                    .diagnostics
                    .iter()
                    .find(|d| d.id == "runtime.usd_imaging")
                    .unwrap()
                    .status,
                expected
            );
        }
    }

    #[test]
    fn imaging_l2_runs_the_native_registry_and_retains_failure_evidence() {
        struct NativeProbe(bool);
        impl Probe for NativeProbe {
            fn run(&self, program: &str, args: &[&str]) -> ToolOutput {
                assert_eq!(program, "ost-usd-imaging-probe");
                assert_eq!(args[1], "usd-imaging:VrmMToonAPI");
                ToolOutput {
                    code: Some(if self.0 { 0 } else { 3 }),
                    stdout: String::new(),
                    stderr: if self.0 {
                        String::new()
                    } else {
                        "USDIMAGING_ENABLE_PLUGINS=0 disables external adapters".into()
                    },
                }
            }
        }
        let b = bundle("vrmImaging", "usd-imaging:VrmMToonAPI");
        for pass in [true, false] {
            let probe = NativeProbe(pass);
            let session = Session {
                probe: &probe,
                python: None,
                usdcat: None,
                usdview: None,
                has_display: false,
            };
            let checks = crate::run_levels(&b, &session, 2);
            assert_eq!(checks.len(), 1);
            assert_eq!(checks[0].id, "imaging.registration");
            assert_eq!(
                checks[0].status,
                if pass { Status::Pass } else { Status::Fail }
            );
            if !pass {
                assert!(checks[0]
                    .probe
                    .as_ref()
                    .unwrap()
                    .stderr
                    .contains("USDIMAGING_ENABLE_PLUGINS=0"));
            }
        }
    }
}
