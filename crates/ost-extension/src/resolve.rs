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
    /// Chosen certified build point, if the extension declares any (§5.3).
    pub certified: Option<Certified>,
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
            resolved.certified = choose_certified(ext, &resolved.features);
        }
    }

    Resolution {
        edges,
        extensions: acc.into_values().collect(),
        runtime_provided,
    }
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
    acc.entry(ext.id.clone()).or_insert_with(|| ResolvedExtension {
        id: ext.id.clone(),
        version: ext.version.clone(),
        features: BTreeSet::new(),
        packages: BTreeSet::new(),
        allowed_range: ext.allowed_range.clone(),
        certified: None,
    });
}

/// Pick the first certified point whose feature set covers the enabled
/// features; fall back to the last (latest) declared point.
fn choose_certified(ext: &Extension, enabled: &BTreeSet<String>) -> Option<Certified> {
    if ext.certified.is_empty() {
        return None;
    }
    ext.certified
        .iter()
        .find(|c| {
            let have: BTreeSet<&str> = c.features.iter().map(String::as_str).collect();
            enabled.iter().all(|f| have.contains(f.as_str()))
        })
        .or_else(|| ext.certified.last())
        .cloned()
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
    fn unknown_capabilities_are_runtime_provided() {
        let catalog = load_all().expect("built-in extensions load");
        let caps = vec!["python-tooling".to_string(), "color-management".to_string()];
        let res = resolve(&catalog, &caps);
        assert!(res.extensions.is_empty());
        assert_eq!(res.runtime_provided.len(), 2);
    }
}
