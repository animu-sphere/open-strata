// SPDX-License-Identifier: Apache-2.0
//! `CMakePresets.json` generation (§8.3).
//!
//! Each target gets its own presets file under `.strata/targets/<id>/`. The
//! project-root `CMakePresets.json` simply `include`s those per-target files, so
//! adding a target never rewrites another target's presets and the root stays a
//! small, stable index.

use serde_json::{json, Value};

use crate::target::Target;

/// CMakePresets schema version. v4 is the first with `include`; CMake 3.23+.
const PRESETS_VERSION: u64 = 4;

/// Render the per-target presets file content.
pub fn render_target_presets(target: &Target) -> Value {
    let id = target.id();
    json!({
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

/// Produce the root `CMakePresets.json`, ensuring `include_path` is present.
///
/// Pass the existing root document (if any) so includes for other targets are
/// preserved; the result is deterministic (includes are sorted and de-duped).
pub fn root_presets_with_include(existing: Option<&Value>, include_path: &str) -> Value {
    let mut includes: Vec<String> = existing
        .and_then(|v| v.get("include"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    if !includes.iter().any(|p| p == include_path) {
        includes.push(include_path.to_string());
    }
    includes.sort();
    includes.dedup();

    json!({
        "version": PRESETS_VERSION,
        "include": includes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn include_is_idempotent_and_sorted() {
        let path_a = ".strata/targets/cy2026-linux-x86_64-py313-usd/CMakePresets.json";
        let path_b = ".strata/targets/cy2025-linux-x86_64-py311-core/CMakePresets.json";

        let first = root_presets_with_include(None, path_a);
        let second = root_presets_with_include(Some(&first), path_a); // re-add same
        let third = root_presets_with_include(Some(&second), path_b); // add another

        let includes = third["include"].as_array().unwrap();
        assert_eq!(includes.len(), 2);
        // Sorted: cy2025 before cy2026.
        assert_eq!(includes[0].as_str().unwrap(), path_b);
        assert_eq!(includes[1].as_str().unwrap(), path_a);
    }
}
