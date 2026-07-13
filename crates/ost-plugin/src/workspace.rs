// SPDX-License-Identifier: Apache-2.0
//! Read-only validation of bundle dependency graphs in plugin workspaces.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::{satisfies, Bundle, PluginKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceNode {
    pub id: String,
    pub version: String,
    pub kind: PluginKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct WorkspaceEdge {
    pub from: String,
    pub to: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct WorkspaceIssue {
    pub code: String,
    pub bundle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceValidation {
    pub passed: bool,
    pub nodes: Vec<WorkspaceNode>,
    pub edges: Vec<WorkspaceEdge>,
    pub issues: Vec<WorkspaceIssue>,
}

impl WorkspaceValidation {
    /// Return the selected bundle's transitive dependency closure in
    /// deterministic build order (deepest dependencies first).
    ///
    /// The result excludes `bundle_id` itself. A closure is only meaningful for
    /// a graph which passed validation; callers must not compose a graph with a
    /// missing, duplicate, incompatible, or cyclic provider.
    pub fn dependency_order(&self, bundle_id: &str) -> Option<Vec<String>> {
        if !self.passed || !self.nodes.iter().any(|node| node.id == bundle_id) {
            return None;
        }

        let mut adjacency: BTreeMap<&str, Vec<&str>> = self
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), Vec::new()))
            .collect();
        for edge in &self.edges {
            adjacency
                .entry(edge.from.as_str())
                .or_default()
                .push(edge.to.as_str());
        }
        for dependencies in adjacency.values_mut() {
            dependencies.sort_unstable();
            dependencies.dedup();
        }

        fn visit<'a>(
            id: &'a str,
            adjacency: &BTreeMap<&'a str, Vec<&'a str>>,
            visited: &mut BTreeSet<&'a str>,
            ordered: &mut Vec<String>,
        ) {
            if let Some(dependencies) = adjacency.get(id) {
                for dependency in dependencies {
                    if visited.insert(dependency) {
                        visit(dependency, adjacency, visited, ordered);
                        ordered.push((*dependency).to_string());
                    }
                }
            }
        }

        let mut visited = BTreeSet::new();
        let mut ordered = Vec::new();
        visit(bundle_id, &adjacency, &mut visited, &mut ordered);
        Some(ordered)
    }
}

/// Validate bundle identity and dependency contracts without changing build
/// order or invoking CMake. Input order does not affect the report.
pub fn validate_workspace(bundles: &[Bundle]) -> WorkspaceValidation {
    let mut by_id: BTreeMap<&str, Vec<&Bundle>> = BTreeMap::new();
    for bundle in bundles {
        by_id
            .entry(bundle.manifest.name())
            .or_default()
            .push(bundle);
    }

    let mut nodes = Vec::new();
    let mut issues = Vec::new();
    for (id, matches) in &by_id {
        if matches.len() > 1 {
            issues.push(issue(
                "WORKSPACE_DUPLICATE_BUNDLE_ID",
                id,
                None,
                format!("bundle id '{id}' is declared {} times", matches.len()),
            ));
            continue;
        }
        let manifest = &matches[0].manifest;
        if !is_portable_id(id) {
            issues.push(issue(
                "WORKSPACE_BUNDLE_ID_INVALID",
                id,
                None,
                format!("bundle id '{id}' is not a portable identifier"),
            ));
        }
        nodes.push(WorkspaceNode {
            id: id.to_string(),
            version: manifest.plugin.version.clone(),
            kind: manifest.kind(),
            contract: manifest.schema.as_ref().and_then(|schema| schema.contract),
        });
        if manifest.kind() == PluginKind::UsdSchema
            && manifest.schema.as_ref().and_then(|schema| schema.contract) == Some(0)
        {
            issues.push(issue(
                "WORKSPACE_SCHEMA_CONTRACT_INVALID",
                id,
                None,
                "schema.contract must be greater than zero".into(),
            ));
        }
        if manifest.kind() != PluginKind::UsdSchema
            && manifest
                .schema
                .as_ref()
                .and_then(|schema| schema.contract)
                .is_some()
        {
            issues.push(issue(
                "WORKSPACE_SCHEMA_CONTRACT_NOT_APPLICABLE",
                id,
                None,
                "schema.contract is only valid on a usd-schema bundle".into(),
            ));
        }
    }

    let unique: BTreeMap<&str, &Bundle> = by_id
        .iter()
        .filter_map(|(id, matches)| (matches.len() == 1).then_some((*id, matches[0])))
        .collect();
    let mut edges = Vec::new();

    for (id, bundle) in &unique {
        let mut dependencies = BTreeSet::new();
        for dependency in &bundle.manifest.requires.bundles {
            if !is_portable_id(&dependency.id) {
                issues.push(issue(
                    "WORKSPACE_DEPENDENCY_ID_INVALID",
                    id,
                    Some(&dependency.id),
                    format!(
                        "dependency id '{}' is not a portable identifier",
                        dependency.id
                    ),
                ));
                continue;
            }
            if !dependencies.insert(dependency.id.as_str()) {
                issues.push(issue(
                    "WORKSPACE_DUPLICATE_DEPENDENCY",
                    id,
                    Some(&dependency.id),
                    format!(
                        "bundle '{id}' declares dependency '{}' more than once",
                        dependency.id
                    ),
                ));
                continue;
            }
            let Some(provider) = unique.get(dependency.id.as_str()) else {
                issues.push(issue(
                    "WORKSPACE_DEPENDENCY_MISSING",
                    id,
                    Some(&dependency.id),
                    format!("bundle '{id}' requires missing bundle '{}'", dependency.id),
                ));
                continue;
            };

            edges.push(WorkspaceEdge {
                from: id.to_string(),
                to: dependency.id.clone(),
                version: dependency.version.clone(),
                contract: dependency.contract,
            });

            if dependency.version.trim().is_empty() {
                issues.push(issue(
                    "WORKSPACE_DEPENDENCY_VERSION_INVALID",
                    id,
                    Some(&dependency.id),
                    format!(
                        "bundle '{id}' declares an empty version requirement for '{}'",
                        dependency.id
                    ),
                ));
            } else {
                match satisfies(&provider.manifest.plugin.version, &dependency.version) {
                    Ok(true) => {}
                    Ok(false) => issues.push(issue(
                        "WORKSPACE_DEPENDENCY_VERSION_MISMATCH",
                        id,
                        Some(&dependency.id),
                        format!(
                            "bundle '{id}' requires '{}' at {}, but the workspace has {}",
                            dependency.id, dependency.version, provider.manifest.plugin.version
                        ),
                    )),
                    Err(error) => issues.push(issue(
                        "WORKSPACE_DEPENDENCY_VERSION_INVALID",
                        id,
                        Some(&dependency.id),
                        format!(
                        "bundle '{id}' declares an invalid version requirement for '{}': {error}",
                        dependency.id
                    ),
                    )),
                }
            }

            validate_contract(id, dependency, provider, &mut issues);
            validate_direction(id, bundle.manifest.kind(), provider, &mut issues);
        }
    }

    edges.sort();
    for cycle in find_cycles(&nodes, &edges) {
        let closed = cycle
            .iter()
            .chain(cycle.first())
            .cloned()
            .collect::<Vec<_>>()
            .join(" -> ");
        issues.push(issue(
            "WORKSPACE_DEPENDENCY_CYCLE",
            &cycle[0],
            None,
            format!("bundle dependency cycle: {closed}"),
        ));
    }
    issues.sort();

    WorkspaceValidation {
        passed: issues.is_empty(),
        nodes,
        edges,
        issues,
    }
}

fn is_portable_id(id: &str) -> bool {
    id.chars()
        .next()
        .map(|first| first.is_ascii_alphabetic())
        .unwrap_or(false)
        && id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
}

fn validate_contract(
    consumer_id: &str,
    dependency: &crate::BundleDependency,
    provider: &Bundle,
    issues: &mut Vec<WorkspaceIssue>,
) {
    let provided = provider
        .manifest
        .schema
        .as_ref()
        .and_then(|schema| schema.contract);
    if dependency.contract == Some(0) {
        issues.push(issue(
            "WORKSPACE_SCHEMA_CONTRACT_INVALID",
            consumer_id,
            Some(&dependency.id),
            "required schema contract must be greater than zero".into(),
        ));
        return;
    }
    match (provider.manifest.kind(), dependency.contract, provided) {
        (PluginKind::UsdSchema, None, Some(contract)) => issues.push(issue(
            "WORKSPACE_SCHEMA_CONTRACT_REQUIRED",
            consumer_id,
            Some(&dependency.id),
            format!(
                "bundle '{consumer_id}' must select schema contract {contract} from '{}'",
                dependency.id
            ),
        )),
        (PluginKind::UsdSchema, Some(_), None) => issues.push(issue(
            "WORKSPACE_SCHEMA_CONTRACT_MISSING",
            consumer_id,
            Some(&dependency.id),
            format!(
                "schema bundle '{}' does not provide schema.contract",
                dependency.id
            ),
        )),
        (PluginKind::UsdSchema, Some(required), Some(actual)) if required != actual => {
            issues.push(issue(
                "WORKSPACE_SCHEMA_CONTRACT_MISMATCH",
                consumer_id,
                Some(&dependency.id),
                format!(
                    "bundle '{consumer_id}' requires schema contract {required} from '{}', but it provides {actual}",
                    dependency.id
                ),
            ));
        }
        (kind, Some(_), _) if kind != PluginKind::UsdSchema => issues.push(issue(
            "WORKSPACE_SCHEMA_CONTRACT_NOT_APPLICABLE",
            consumer_id,
            Some(&dependency.id),
            format!("bundle '{}' is not a usd-schema bundle", dependency.id),
        )),
        _ => {}
    }
}

fn validate_direction(
    consumer_id: &str,
    consumer_kind: PluginKind,
    provider: &Bundle,
    issues: &mut Vec<WorkspaceIssue>,
) {
    let forbidden = consumer_kind == PluginKind::UsdSchema
        || (matches!(
            consumer_kind,
            PluginKind::UsdAssetResolver | PluginKind::UsdPackageResolver
        ) && provider.manifest.kind() == PluginKind::UsdFileformat);
    if forbidden {
        issues.push(issue(
            "WORKSPACE_DEPENDENCY_DIRECTION_FORBIDDEN",
            consumer_id,
            Some(provider.manifest.name()),
            format!(
                "dependency direction {} -> {} is forbidden",
                consumer_kind.as_str(),
                provider.manifest.kind().as_str()
            ),
        ));
    }
}

fn issue(code: &str, bundle: &str, dependency: Option<&str>, message: String) -> WorkspaceIssue {
    WorkspaceIssue {
        code: code.into(),
        bundle: bundle.into(),
        dependency: dependency.map(str::to_string),
        message,
    }
}

fn find_cycles(nodes: &[WorkspaceNode], edges: &[WorkspaceEdge]) -> BTreeSet<Vec<String>> {
    let mut adjacency: BTreeMap<String, Vec<String>> = nodes
        .iter()
        .map(|node| (node.id.clone(), Vec::new()))
        .collect();
    for edge in edges {
        adjacency
            .entry(edge.from.clone())
            .or_default()
            .push(edge.to.clone());
    }
    for dependencies in adjacency.values_mut() {
        dependencies.sort();
        dependencies.dedup();
    }

    let mut states: BTreeMap<String, u8> = adjacency.keys().map(|id| (id.clone(), 0)).collect();
    let mut stack = Vec::new();
    let mut cycles = BTreeSet::new();
    for id in adjacency.keys() {
        if states[id] == 0 {
            visit(id, &adjacency, &mut states, &mut stack, &mut cycles);
        }
    }
    cycles
}

fn visit(
    id: &str,
    adjacency: &BTreeMap<String, Vec<String>>,
    states: &mut BTreeMap<String, u8>,
    stack: &mut Vec<String>,
    cycles: &mut BTreeSet<Vec<String>>,
) {
    states.insert(id.into(), 1);
    stack.push(id.into());
    if let Some(dependencies) = adjacency.get(id) {
        for dependency in dependencies {
            match states.get(dependency).copied().unwrap_or(0) {
                0 => visit(dependency, adjacency, states, stack, cycles),
                1 => {
                    if let Some(start) = stack.iter().position(|item| item == dependency) {
                        let mut cycle = stack[start..].to_vec();
                        if let Some((offset, _)) =
                            cycle.iter().enumerate().min_by_key(|(_, id)| *id)
                        {
                            cycle.rotate_left(offset);
                        }
                        cycles.insert(cycle);
                    }
                }
                _ => {}
            }
        }
    }
    stack.pop();
    states.insert(id.into(), 2);
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;

    use super::*;
    use crate::PluginManifest;

    fn bundle(source: &str) -> Bundle {
        Bundle {
            root: Utf8PathBuf::from("unused"),
            manifest: PluginManifest::parse(source).unwrap(),
        }
    }

    fn manifest(name: &str, version: &str, kind: &str, extra: &str) -> String {
        format!(
            "manifest:\n  schema: openstrata.plugin/v1alpha1\nplugin:\n  name: {name}\n  version: {version}\n  kind: {kind}\nruntime:\n  openusd: '>=25.05,<27.0'\nusd:\n  plug_info: resources/plugInfo.json\n{extra}"
        )
    }

    #[test]
    fn accepts_a_compatible_fileformat_schema_graph() {
        let schema = bundle(&manifest(
            "schema",
            "2.1.0",
            "usd-schema",
            "schema:\n  codeless: true\n  contract: 3\n",
        ));
        let format = bundle(&manifest(
            "format",
            "1.0.0",
            "usd-fileformat",
            "requires:\n  bundles:\n    - id: schema\n      version: '>=2.0,<3.0'\n      contract: 3\n",
        ));
        let report = validate_workspace(&[format, schema]);
        assert!(report.passed, "{:?}", report.issues);
        assert_eq!(report.nodes[0].id, "format");
        assert_eq!(report.edges[0].to, "schema");
    }

    #[test]
    fn dependency_order_is_transitive_stable_and_excludes_the_primary() {
        let base = bundle(&manifest("base", "1.0.0", "usd-asset-resolver", ""));
        let schema = bundle(&manifest(
            "schema",
            "1.0.0",
            "usd-schema",
            "schema:\n  contract: 1\n",
        ));
        let middle = bundle(&manifest(
            "middle",
            "1.0.0",
            "usd-fileformat",
            "requires:\n  bundles:\n    - { id: base, version: '>=1.0,<2.0' }\n    - { id: schema, version: '>=1.0,<2.0', contract: 1 }\n",
        ));
        let consumer = bundle(&manifest(
            "consumer",
            "1.0.0",
            "usd-fileformat",
            "requires:\n  bundles:\n    - { id: middle, version: '>=1.0,<2.0' }\n    - { id: schema, version: '>=1.0,<2.0', contract: 1 }\n",
        ));

        let report = validate_workspace(&[consumer, middle, schema, base]);
        assert!(report.passed, "{:?}", report.issues);
        assert_eq!(
            report.dependency_order("consumer").unwrap(),
            vec!["base", "schema", "middle"]
        );
        assert_eq!(
            report.dependency_order("schema").unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(report.dependency_order("missing"), None);
    }

    #[test]
    fn reports_duplicate_missing_version_contract_direction_and_cycles() {
        let schema = bundle(&manifest(
            "schema",
            "2.0.0",
            "usd-schema",
            "schema:\n  contract: 2\nrequires:\n  bundles:\n    - { id: format, version: '>=1.0,<2.0' }\n",
        ));
        let format = bundle(&manifest(
            "format",
            "1.0.0",
            "usd-fileformat",
            "requires:\n  bundles:\n    - { id: schema, version: '>=3.0,<4.0', contract: 1 }\n    - { id: absent, version: '>=1.0,<2.0' }\n",
        ));
        let duplicate = bundle(&manifest("format", "1.1.0", "usd-fileformat", ""));
        let report = validate_workspace(&[schema, format, duplicate]);
        let codes: BTreeSet<_> = report
            .issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect();
        assert!(codes.contains("WORKSPACE_DUPLICATE_BUNDLE_ID"));
        assert!(codes.contains("WORKSPACE_DEPENDENCY_MISSING"));
    }

    #[test]
    fn reports_version_contract_and_cycle_for_unique_nodes() {
        let schema = bundle(&manifest(
            "schema",
            "2.0.0",
            "usd-schema",
            "schema:\n  contract: 2\nrequires:\n  bundles:\n    - { id: format, version: '>=1.0,<2.0' }\n",
        ));
        let format = bundle(&manifest(
            "format",
            "1.0.0",
            "usd-fileformat",
            "requires:\n  bundles:\n    - { id: schema, version: '>=3.0,<4.0', contract: 1 }\n",
        ));
        let report = validate_workspace(&[schema, format]);
        let codes = codes(&report);
        assert!(codes.contains("WORKSPACE_DEPENDENCY_VERSION_MISMATCH"));
        assert!(codes.contains("WORKSPACE_SCHEMA_CONTRACT_MISMATCH"));
        assert!(codes.contains("WORKSPACE_DEPENDENCY_DIRECTION_FORBIDDEN"));
        assert!(codes.contains("WORKSPACE_DEPENDENCY_CYCLE"));
    }

    fn codes(report: &WorkspaceValidation) -> BTreeSet<&str> {
        report
            .issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect()
    }

    #[test]
    fn requires_and_reports_missing_schema_contract_selection() {
        // Provider advertises a contract: the consumer must select it.
        let required = validate_workspace(&[
            bundle(&manifest(
                "schema",
                "1.0.0",
                "usd-schema",
                "schema:\n  contract: 3\n",
            )),
            bundle(&manifest(
                "format",
                "1.0.0",
                "usd-fileformat",
                "requires:\n  bundles:\n    - { id: schema, version: '>=1.0,<2.0' }\n",
            )),
        ]);
        assert!(!required.passed);
        assert!(codes(&required).contains("WORKSPACE_SCHEMA_CONTRACT_REQUIRED"));

        // Consumer selects a contract the schema does not provide.
        let missing = validate_workspace(&[
            bundle(&manifest(
                "schema",
                "1.0.0",
                "usd-schema",
                "schema:\n  codeless: true\n",
            )),
            bundle(&manifest(
                "format",
                "1.0.0",
                "usd-fileformat",
                "requires:\n  bundles:\n    - { id: schema, version: '>=1.0,<2.0', contract: 2 }\n",
            )),
        ]);
        assert!(codes(&missing).contains("WORKSPACE_SCHEMA_CONTRACT_MISSING"));
    }

    #[test]
    fn rejects_inapplicable_and_zero_schema_contracts() {
        // A contract attached to a non-schema dependency, and a non-schema
        // bundle that declares its own schema.contract.
        let not_applicable = validate_workspace(&[
            bundle(&manifest("resolver", "1.0.0", "usd-asset-resolver", "schema:\n  contract: 1\n")),
            bundle(&manifest(
                "format",
                "1.0.0",
                "usd-fileformat",
                "requires:\n  bundles:\n    - { id: resolver, version: '>=1.0,<2.0', contract: 1 }\n",
            )),
        ]);
        assert!(codes(&not_applicable).contains("WORKSPACE_SCHEMA_CONTRACT_NOT_APPLICABLE"));

        // Contract 0 is invalid on both the provider node and the selector.
        let zero = validate_workspace(&[
            bundle(&manifest(
                "schema",
                "1.0.0",
                "usd-schema",
                "schema:\n  contract: 0\n",
            )),
            bundle(&manifest(
                "format",
                "1.0.0",
                "usd-fileformat",
                "requires:\n  bundles:\n    - { id: schema, version: '>=1.0,<2.0', contract: 0 }\n",
            )),
        ]);
        assert!(codes(&zero).contains("WORKSPACE_SCHEMA_CONTRACT_INVALID"));
    }

    #[test]
    fn rejects_empty_and_unparseable_version_ranges() {
        let empty = validate_workspace(&[
            bundle(&manifest(
                "schema",
                "1.0.0",
                "usd-schema",
                "schema:\n  contract: 1\n",
            )),
            bundle(&manifest(
                "format",
                "1.0.0",
                "usd-fileformat",
                "requires:\n  bundles:\n    - { id: schema, version: '', contract: 1 }\n",
            )),
        ]);
        assert!(codes(&empty).contains("WORKSPACE_DEPENDENCY_VERSION_INVALID"));

        let unparseable = validate_workspace(&[
            bundle(&manifest(
                "schema",
                "1.0.0",
                "usd-schema",
                "schema:\n  contract: 1\n",
            )),
            bundle(&manifest(
                "format",
                "1.0.0",
                "usd-fileformat",
                "requires:\n  bundles:\n    - { id: schema, version: '>=abc', contract: 1 }\n",
            )),
        ]);
        assert!(codes(&unparseable).contains("WORKSPACE_DEPENDENCY_VERSION_INVALID"));
    }

    #[test]
    fn rejects_non_portable_ids_and_duplicate_dependencies() {
        let ids = validate_workspace(&[
            bundle(&manifest("1bad", "1.0.0", "usd-fileformat", "")),
            bundle(&manifest(
                "good",
                "1.0.0",
                "usd-fileformat",
                "requires:\n  bundles:\n    - { id: '1dep', version: '>=1.0,<2.0' }\n",
            )),
        ]);
        let ids = codes(&ids);
        assert!(ids.contains("WORKSPACE_BUNDLE_ID_INVALID"));
        assert!(ids.contains("WORKSPACE_DEPENDENCY_ID_INVALID"));

        let duplicate = validate_workspace(&[
            bundle(&manifest("schema", "1.0.0", "usd-schema", "schema:\n  contract: 1\n")),
            bundle(&manifest(
                "format",
                "1.0.0",
                "usd-fileformat",
                "requires:\n  bundles:\n    - { id: schema, version: '>=1.0,<2.0', contract: 1 }\n    - { id: schema, version: '>=1.0,<2.0', contract: 1 }\n",
            )),
        ]);
        assert!(codes(&duplicate).contains("WORKSPACE_DUPLICATE_DEPENDENCY"));
    }
}
