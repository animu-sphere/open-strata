// SPDX-License-Identifier: Apache-2.0
//! `CMakePresets.json` generation and safe merging (§8.3).
//!
//! Each target gets its own presets file under `.strata/targets/<id>/`. A root
//! presets file simply `include`s those per-target files, so adding a target
//! never rewrites another target's presets.
//!
//! By default OpenStrata writes its includes to the tool-owned
//! `CMakeUserPresets.json` and never touches the user's `CMakePresets.json`.
//! `ost presets install` can wire them into `CMakePresets.json` on request; the
//! merge primitives here preserve every existing field (including unknown ones)
//! so that operation is non-destructive.

use serde_json::{Map, Value};

use crate::target::Target;

/// CMakePresets schema version. v4 is the first with `include`; CMake 3.23+.
const PRESETS_VERSION: u64 = 4;

/// Render the per-target presets file content.
pub fn render_target_presets(target: &Target) -> Value {
    let id = target.id();
    serde_json::json!({
        "version": PRESETS_VERSION,
        "configurePresets": [
            {
                "name": id,
                "displayName": format!(
                    "OpenStrata {} / {} ({})",
                    target.platform, target.profile, target.variant.short_slug()
                ),
                "generator": target.generator,
                "binaryDir": format!("${{sourceDir}}/build/{id}"),
                "toolchainFile":
                    format!("${{sourceDir}}/.strata/targets/{id}/toolchain.cmake"),
                "cacheVariables": {
                    "CMAKE_BUILD_TYPE": "Release"
                }
            }
        ]
    })
}

/// The OpenStrata-managed include path for a target's per-target presets file,
/// relative to the project root (which is where the including file lives).
pub fn managed_include(id: &str) -> String {
    format!(".strata/targets/{id}/CMakePresets.json")
}

/// Whether an `include` entry is one OpenStrata manages, i.e. points at a
/// per-target presets file under `.strata/targets/`.
pub fn is_managed_include(path: &str) -> bool {
    path.starts_with(".strata/targets/") && path.ends_with("/CMakePresets.json")
}

/// The `include` array of a presets document, as owned strings.
pub fn includes_of(doc: &Value) -> Vec<String> {
    doc.get("include")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Ensure each path in `want` is present in `root`'s `include` array, preserving
/// every other field (configure/build/test/workflow presets, `vendor`, and any
/// unknown keys). Includes are de-duplicated and sorted for a stable result.
///
/// `version` is set to the minimum that supports `include` (4) only when it is
/// missing or lower; an existing higher version is left as-is.
///
/// Returns whether the document was modified.
pub fn ensure_includes(root: &mut Map<String, Value>, want: &[String]) -> bool {
    let version_changed = ensure_version(root);

    let mut includes: Vec<String> = root
        .get("include")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    for w in want {
        if !includes.iter().any(|p| p == w) {
            includes.push(w.clone());
        }
    }
    includes.sort();
    includes.dedup();

    let new_value = Value::Array(includes.into_iter().map(Value::from).collect());
    let include_changed = root.get("include") != Some(&new_value);
    if include_changed {
        root.insert("include".to_string(), new_value);
    }

    version_changed || include_changed
}

/// Remove every OpenStrata-managed include from `root`, preserving all other
/// fields and any non-managed includes. Returns the removed paths.
///
/// If the `include` array becomes empty it is dropped entirely.
pub fn remove_managed_includes(root: &mut Map<String, Value>) -> Vec<String> {
    let Some(arr) = root.get("include").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut kept: Vec<Value> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    for entry in arr {
        match entry.as_str() {
            Some(s) if is_managed_include(s) => removed.push(s.to_string()),
            // Preserve non-managed string entries and any non-string entries.
            _ => kept.push(entry.clone()),
        }
    }

    if removed.is_empty() {
        return removed;
    }
    if kept.is_empty() {
        root.remove("include");
    } else {
        root.insert("include".to_string(), Value::Array(kept));
    }
    removed
}

/// Set `version` to at least [`PRESETS_VERSION`]; returns whether it changed.
fn ensure_version(root: &mut Map<String, Value>) -> bool {
    match root.get("version").and_then(Value::as_u64) {
        Some(v) if v >= PRESETS_VERSION => false,
        _ => {
            root.insert("version".to_string(), Value::from(PRESETS_VERSION));
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn ensure_includes_preserves_all_existing_fields() {
        // A realistic user file: configure/build/test presets, vendor, unknown.
        let mut root = obj(json!({
            "version": 6,
            "configurePresets": [{ "name": "dev" }],
            "buildPresets": [{ "name": "dev", "configurePreset": "dev" }],
            "testPresets": [{ "name": "dev", "configurePreset": "dev" }],
            "workflowPresets": [{ "name": "ci" }],
            "vendor": { "acme/ide": { "open": true } },
            "futureField": { "anything": [1, 2, 3] }
        }));

        let changed = ensure_includes(
            &mut root,
            &[managed_include("cy2026-linux-x86_64-py313-usd")],
        );

        assert!(changed);
        // Existing fields are all intact.
        assert!(root.contains_key("configurePresets"));
        assert!(root.contains_key("buildPresets"));
        assert!(root.contains_key("testPresets"));
        assert!(root.contains_key("workflowPresets"));
        assert_eq!(root["vendor"]["acme/ide"]["open"], json!(true));
        assert_eq!(root["futureField"]["anything"], json!([1, 2, 3]));
        // Higher existing version is kept (not downgraded to 4).
        assert_eq!(root["version"], json!(6));
        // The include was added.
        assert_eq!(
            includes_of(&Value::Object(root.clone())),
            vec![".strata/targets/cy2026-linux-x86_64-py313-usd/CMakePresets.json"]
        );
    }

    #[test]
    fn ensure_includes_is_idempotent_and_sorts() {
        let a = managed_include("cy2026-linux-x86_64-py313-usd");
        let b = managed_include("cy2025-linux-x86_64-py311-core");

        let mut root = Map::new();
        assert!(ensure_includes(&mut root, std::slice::from_ref(&a)));
        // Re-adding the same include is a no-op.
        assert!(!ensure_includes(&mut root, std::slice::from_ref(&a)));
        // Adding another sorts them.
        assert!(ensure_includes(&mut root, std::slice::from_ref(&b)));

        assert_eq!(includes_of(&Value::Object(root.clone())), vec![b, a]);
        assert_eq!(root["version"], json!(PRESETS_VERSION));
    }

    #[test]
    fn ensure_version_bumps_only_when_too_low() {
        let mut low = obj(json!({ "version": 2 }));
        assert!(ensure_version(&mut low));
        assert_eq!(low["version"], json!(PRESETS_VERSION));

        let mut high = obj(json!({ "version": 8 }));
        assert!(!ensure_version(&mut high));
        assert_eq!(high["version"], json!(8));
    }

    #[test]
    fn remove_managed_includes_keeps_user_includes() {
        let mut root = obj(json!({
            "version": 4,
            "include": [
                "vendor/Presets.json",
                ".strata/targets/cy2026-linux-x86_64-py313-usd/CMakePresets.json"
            ],
            "configurePresets": [{ "name": "dev" }]
        }));

        let removed = remove_managed_includes(&mut root);
        assert_eq!(
            removed,
            vec![".strata/targets/cy2026-linux-x86_64-py313-usd/CMakePresets.json"]
        );
        // The user's own include and presets survive.
        assert_eq!(includes_of(&Value::Object(root.clone())), vec!["vendor/Presets.json"]);
        assert!(root.contains_key("configurePresets"));
    }

    #[test]
    fn remove_managed_includes_drops_empty_array() {
        let mut root = obj(json!({
            "version": 4,
            "include": [".strata/targets/x/CMakePresets.json"]
        }));
        remove_managed_includes(&mut root);
        assert!(!root.contains_key("include"));
    }

    #[test]
    fn is_managed_include_matches_only_target_presets() {
        assert!(is_managed_include(
            ".strata/targets/cy2026-linux-x86_64-py313-usd/CMakePresets.json"
        ));
        assert!(!is_managed_include("vendor/CMakePresets.json"));
        assert!(!is_managed_include(".strata/targets/x/toolchain.cmake"));
    }
}
