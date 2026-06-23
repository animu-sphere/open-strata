// SPDX-License-Identifier: Apache-2.0
//! Capability resolution (§3.5, §5.3, §6.2).
//!
//! Given the capabilities a profile requests, derive the extensions that must
//! be present, the features to enable on each, the packages those features
//! require, and — for an extension with certified build points — the certified
//! version chosen within its allowed range.

use std::collections::{BTreeMap, BTreeSet};

use crate::loader::Catalog;
use crate::model::{Certified, Extension};

/// One `capability -> provider` edge in the resolution.
#[derive(Debug, Clone)]
pub struct ProviderEdge {
    pub capability: String,
    /// Providing extension id, or `None` when the base runtime provides it.
    pub extension: Option<String>,
    /// Feature that had to be enabled, if any.
    pub feature: Option<String>,
}

/// An extension as resolved: which features are on and what they pull in.
#[derive(Debug, Clone)]
pub struct ResolvedExtension {
    pub id: String,
    pub version: String,
    pub features: BTreeSet<String>,
    pub packages: BTreeSet<String>,
    pub allowed_range: Option<String>,
    /// Chosen certified build point covering the enabled features (§5.3).
    /// `None` if the extension declares none, or if none covers — see
    /// [`ResolvedExtension::uncertified`].
    pub certified: Option<Certified>,
    /// True when the extension declares certified build points but none covers
    /// the enabled feature set: the resolved combination is *not* certified.
    pub uncertified: bool,
}

/// The full resolution result.
#[derive(Debug, Clone)]
pub struct Resolution {
    pub edges: Vec<ProviderEdge>,
    pub extensions: Vec<ResolvedExtension>,
    /// Capabilities no extension provides — satisfied by the base runtime.
    pub runtime_provided: Vec<String>,
}

/// Resolve a set of requested capabilities against the extension catalog.
pub fn resolve(catalog: &Catalog, capabilities: &[String]) -> Resolution {
    let mut acc: BTreeMap<String, ResolvedExtension> = BTreeMap::new();
    let mut edges = Vec::new();
    let mut runtime_provided = Vec::new();

    for cap in capabilities {
        match find_provider(catalog, cap) {
            Some((ext, feature)) => {
                edges.push(ProviderEdge {
                    capability: cap.clone(),
                    extension: Some(ext.id.clone()),
                    feature: feature.clone(),
                });
                enable(&mut acc, catalog, ext, feature.as_deref());
            }
            None => {
                runtime_provided.push(cap.clone());
                edges.push(ProviderEdge {
                    capability: cap.clone(),
                    extension: None,
                    feature: None,
                });
            }
        }
    }

    // Choose a certified build point per extension that declares them.
    for resolved in acc.values_mut() {
        if let Some(ext) = catalog.get(&resolved.id) {
            let (certified, uncertified) = choose_certified(ext, &resolved.features);
            resolved.certified = certified;
            resolved.uncertified = uncertified;
        }
    }

    Resolution {
        edges,
        extensions: acc.into_values().collect(),
        runtime_provided,
    }
}

/// Why a given extension ends up in a [`Resolution`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequirementReason {
    /// A requested capability is directly provided by this extension.
    Direct {
        capability: String,
        /// Feature that had to be enabled for it, if any.
        feature: Option<String>,
    },
    /// Pulled in transitively by another extension's enabled feature
    /// (e.g. `openusd[materialx] -> materialx`).
    Transitive {
        /// Extension whose feature requires this one.
        extension: String,
        /// The feature that pulls it in.
        feature: String,
        /// Capability that enabled that feature, if known.
        capability: Option<String>,
    },
}

/// Explain why `name` is part of `resolution`, as direct and transitive reasons.
///
/// Returns an empty vec when the extension is not required by the resolution.
/// `resolution` must have been produced from `catalog`.
pub fn why(catalog: &Catalog, resolution: &Resolution, name: &str) -> Vec<RequirementReason> {
    let mut reasons = Vec::new();

    // Direct: a requested capability is provided by this extension.
    for edge in &resolution.edges {
        if edge.extension.as_deref() == Some(name) {
            reasons.push(RequirementReason::Direct {
                capability: edge.capability.clone(),
                feature: edge.feature.clone(),
            });
        }
    }

    // Transitive: another resolved extension's enabled feature requires this one.
    for ext in &resolution.extensions {
        if ext.id == name {
            continue;
        }
        let Some(src) = catalog.get(&ext.id) else {
            continue;
        };
        for feature in &ext.features {
            let Some(spec) = src.feature(feature) else {
                continue;
            };
            if spec.requires_extensions.iter().any(|d| d == name) {
                let capability = resolution
                    .edges
                    .iter()
                    .find(|e| {
                        e.extension.as_deref() == Some(ext.id.as_str())
                            && e.feature.as_deref() == Some(feature.as_str())
                    })
                    .map(|e| e.capability.clone());
                reasons.push(RequirementReason::Transitive {
                    extension: ext.id.clone(),
                    feature: feature.clone(),
                    capability,
                });
            }
        }
    }

    reasons
}

/// First extension (by id order) that provides `cap`, with the feature it needs.
fn find_provider<'a>(catalog: &'a Catalog, cap: &str) -> Option<(&'a Extension, Option<String>)> {
    for ext in catalog.iter() {
        if let Some(provide) = ext.provides.get(cap) {
            return Some((ext, provide.feature.clone()));
        }
    }
    None
}

/// Ensure `ext` is present, enable `feature`, and pull in its requirements.
fn enable(
    acc: &mut BTreeMap<String, ResolvedExtension>,
    catalog: &Catalog,
    ext: &Extension,
    feature: Option<&str>,
) {
    ensure(acc, ext);

    let feature = match feature {
        Some(f) => f.to_string(),
        None => return, // capability needs no specific feature
    };

    // Enable the feature; if it was already enabled, stop (avoids cycles).
    let newly = acc
        .get_mut(&ext.id)
        .map(|re| re.features.insert(feature.clone()))
        .unwrap_or(false);
    if !newly {
        return;
    }

    if let Some(spec) = ext.feature(&feature) {
        if let Some(re) = acc.get_mut(&ext.id) {
            for pkg in &spec.requires_packages {
                re.packages.insert(pkg.clone());
            }
        }
        // Pull in extensions this feature depends on (e.g. usd[materialx] -> materialx).
        for dep_id in &spec.requires_extensions {
            if let Some(dep) = catalog.get(dep_id) {
                ensure(acc, dep);
            }
        }
    }
}

fn ensure(acc: &mut BTreeMap<String, ResolvedExtension>, ext: &Extension) {
    acc.entry(ext.id.clone())
        .or_insert_with(|| ResolvedExtension {
            id: ext.id.clone(),
            version: ext.version.clone(),
            features: BTreeSet::new(),
            packages: BTreeSet::new(),
            allowed_range: ext.allowed_range.clone(),
            certified: None,
            uncertified: false,
        });
}

/// Pick the first certified point whose feature set covers the enabled features.
///
/// Returns `(chosen, uncertified)`:
/// - `(None, false)` — the extension declares no certified points.
/// - `(Some(point), false)` — a covering certified point was found.
/// - `(None, true)` — certified points exist but none covers the enabled
///   feature set, so the resolved combination is *not* certified. We do **not**
///   fall back to an arbitrary point, since that would misreport an
///   uncertified build as certified (§5.3).
fn choose_certified(ext: &Extension, enabled: &BTreeSet<String>) -> (Option<Certified>, bool) {
    if ext.certified.is_empty() {
        return (None, false);
    }
    match ext.certified.iter().find(|c| {
        let have: BTreeSet<&str> = c.features.iter().map(String::as_str).collect();
        enabled.iter().all(|f| have.contains(f.as_str()))
    }) {
        Some(point) => (Some(point.clone()), false),
        None => (None, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::load_all;

    #[test]
    fn lookdev_pulls_materialx_via_openusd_feature() {
        let catalog = load_all().expect("built-in extensions load");
        let caps: Vec<String> = ["usd-stage-read", "usd-materialx", "hydra-preview"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let res = resolve(&catalog, &caps);

        let openusd = res
            .extensions
            .iter()
            .find(|e| e.id == "openusd")
            .expect("openusd resolved");
        // usd-materialx enables the materialx feature, hydra-preview enables imaging.
        assert!(openusd.features.contains("materialx"));
        assert!(openusd.features.contains("imaging"));
        // imaging pulls in its packages.
        assert!(openusd.packages.contains("openexr"));
        // The materialx feature transitively pulls in the materialx extension.
        assert!(res.extensions.iter().any(|e| e.id == "materialx"));
        // A certified build point is chosen.
        assert!(openusd.certified.is_some());
    }

    #[test]
    fn uncovered_feature_set_is_flagged_not_silently_certified() {
        // An extension whose only certified point covers `core`, while the
        // resolution enables `imaging` too — no point covers the combination.
        let ext: Extension = serde_yaml::from_str(
            r#"
id: demo
type: solution.demo
version: "1.0.0"
certified:
  - version: "1.0.0"
    features: [core]
"#,
        )
        .expect("extension parses");

        let enabled: BTreeSet<String> = ["core", "imaging"].iter().map(|s| s.to_string()).collect();
        let (chosen, uncertified) = choose_certified(&ext, &enabled);
        assert!(chosen.is_none(), "must not pick a non-covering point");
        assert!(uncertified, "uncovered combination must be flagged");

        // A covering subset still resolves cleanly.
        let core_only: BTreeSet<String> = ["core"].iter().map(|s| s.to_string()).collect();
        let (chosen, uncertified) = choose_certified(&ext, &core_only);
        assert_eq!(chosen.map(|c| c.version), Some("1.0.0".to_string()));
        assert!(!uncertified);
    }

    #[test]
    fn why_reports_transitive_materialx_pull() {
        let catalog = load_all().expect("built-in extensions load");
        let caps: Vec<String> = ["usd-stage-read", "usd-materialx", "hydra-preview"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let res = resolve(&catalog, &caps);

        // materialx is not directly requested; it is pulled in by openusd[materialx].
        let reasons = why(&catalog, &res, "materialx");
        assert!(
            reasons.iter().any(|r| matches!(
                r,
                RequirementReason::Transitive { extension, feature, capability }
                    if extension == "openusd"
                        && feature == "materialx"
                        && capability.as_deref() == Some("usd-materialx")
            )),
            "materialx should be pulled in via openusd[materialx]: {reasons:?}"
        );

        // openusd is required directly by the requested capabilities.
        let openusd = why(&catalog, &res, "openusd");
        assert!(openusd.iter().any(|r| matches!(
            r,
            RequirementReason::Direct { capability, .. } if capability == "usd-stage-read"
        )));
    }

    #[test]
    fn why_is_empty_when_extension_not_required() {
        let catalog = load_all().expect("built-in extensions load");
        // core-only resolution does not enable the materialx feature.
        let res = resolve(&catalog, &["usd-stage-read".to_string()]);
        assert!(why(&catalog, &res, "materialx").is_empty());
    }

    #[test]
    fn unknown_capabilities_are_runtime_provided() {
        let catalog = load_all().expect("built-in extensions load");
        let caps = vec!["python-tooling".to_string(), "color-management".to_string()];
        let res = resolve(&catalog, &caps);
        assert!(res.extensions.is_empty());
        assert_eq!(res.runtime_provided.len(), 2);
    }
}
