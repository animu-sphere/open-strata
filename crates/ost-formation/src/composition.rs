// SPDX-License-Identifier: Apache-2.0
//! Deterministic, pre-materialization runtime component resolution.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use ost_artifact::{
    ArtifactRecord, CapabilityProvision, CapabilityRequirement, ComponentContract,
    EnvironmentOperation,
};
use ost_core::{digest, Category, Error, Result};
use serde::{Deserialize, Serialize};

use crate::validate_full_digest;

pub const COMPOSITION_SCHEMA: &str = "openstrata.runtime-composition/v1alpha1";
pub const RESOLVED_COMPOSITION_SCHEMA: &str = "openstrata.runtime-composition-resolved/v1alpha1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCompositionManifest {
    pub schema: String,
    pub composition: CompositionHeader,
    #[serde(default)]
    pub requirements: Vec<CapabilityRequirement>,
    #[serde(default)]
    pub artifacts: Vec<CompositionArtifactRef>,
    #[serde(default)]
    pub providers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionHeader {
    pub name: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionArtifactRef {
    pub artifact: String,
    /// Optional digest-pinned transport locator; never part of runtime identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl RuntimeCompositionManifest {
    pub fn parse(source: &str) -> Result<Self> {
        let manifest: Self = toml::from_str(source).map_err(|error| {
            Error::parse("runtime composition manifest", anyhow::Error::new(error))
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != COMPOSITION_SCHEMA {
            return Err(Error::config(format!(
                "unsupported runtime composition schema '{}' (expected '{COMPOSITION_SCHEMA}')",
                self.schema
            )));
        }
        portable_id("composition.name", &self.composition.name)?;
        if self.composition.target.trim().is_empty() {
            return Err(Error::config("composition.target must not be empty"));
        }
        if self.requirements.is_empty() {
            return Err(Error::config(
                "runtime composition must declare at least one requirement",
            ));
        }
        if self.artifacts.is_empty() {
            return Err(Error::config(
                "runtime composition must declare at least one artifact candidate",
            ));
        }
        let mut digests = BTreeSet::new();
        for artifact in &self.artifacts {
            validate_full_digest("composition artifact", &artifact.artifact)?;
            if let Some(source) = &artifact.source {
                let reference = ost_artifact::RemoteReference::parse(source)?;
                if !reference.is_pinned() {
                    return Err(Error::coded(
                        "COMPOSITION_SOURCE_MUTABLE",
                        Category::Configuration,
                        "composition sources must be digest-pinned",
                    ));
                }
            }
            if !digests.insert(&artifact.artifact) {
                return Err(Error::coded(
                    "COMPOSITION_DUPLICATE_ARTIFACT",
                    Category::Configuration,
                    format!("artifact '{}' is listed more than once", artifact.artifact),
                ));
            }
        }
        for requirement in &self.requirements {
            validate_requirement(requirement)?;
        }
        for (capability, provider) in &self.providers {
            portable_capability("providers capability", capability)?;
            portable_id("providers component id", provider)?;
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String> {
        let bytes = serde_json::to_vec(&self.canonical()).map_err(|error| {
            Error::Operation(format!(
                "cannot serialize runtime composition manifest: {error}"
            ))
        })?;
        Ok(digest::sha256_hex(&bytes))
    }

    /// Set-like inputs have one ordering; transport locations are not identity.
    pub fn canonical(&self) -> Self {
        let mut manifest = self.clone();
        for artifact in &mut manifest.artifacts {
            artifact.source = None;
        }
        manifest
            .artifacts
            .sort_by(|a, b| a.artifact.cmp(&b.artifact));
        manifest
            .requirements
            .sort_by(|a, b| requirement_key(a).cmp(&requirement_key(b)));
        manifest.requirements.dedup();
        manifest
    }
}

#[derive(Debug, Clone)]
pub struct CompositionInput {
    pub records: Vec<ArtifactRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRuntimeComposition {
    pub schema: String,
    pub name: String,
    pub target: String,
    pub manifest_digest: String,
    pub composition_digest: String,
    pub components: Vec<ResolvedRuntimeComponent>,
    pub providers: Vec<ResolvedProvider>,
    pub environment: Vec<ResolvedEnvironmentContribution>,
    pub install: Vec<ResolvedInstallMapping>,
    pub conflicts: Vec<CompositionConflict>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRuntimeComponent {
    pub id: String,
    pub kind: String,
    pub version: String,
    pub digest: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedProvider {
    pub capability: String,
    pub component: String,
    pub version: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedEnvironmentContribution {
    pub variable: String,
    pub operation: String,
    pub source: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedInstallMapping {
    pub destination: String,
    pub component: String,
    pub artifact: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompositionConflict {
    pub code: String,
    pub subject: String,
    pub sources: Vec<String>,
    pub reason: String,
}

struct Candidate<'a> {
    record: &'a ArtifactRecord,
    contract: &'a ComponentContract,
}

pub fn resolve_runtime_composition(
    declared: &RuntimeCompositionManifest,
    input: CompositionInput,
) -> Result<ResolvedRuntimeComposition> {
    declared.validate()?;
    if input.records.len() != declared.artifacts.len() {
        return Err(Error::coded(
            "COMPOSITION_ARTIFACT_SET_MISMATCH",
            Category::Validation,
            format!(
                "composition declares {} artifact candidate(s), but {} record(s) were supplied",
                declared.artifacts.len(),
                input.records.len()
            ),
        ));
    }

    let declared_digests = declared
        .artifacts
        .iter()
        .map(|artifact| artifact.artifact.as_str())
        .collect::<BTreeSet<_>>();
    let mut ids = BTreeSet::new();
    let mut candidates = Vec::new();
    for record in &input.records {
        if !declared_digests.contains(record.digest.as_str()) {
            return Err(Error::coded(
                "COMPOSITION_ARTIFACT_SET_MISMATCH",
                Category::Validation,
                format!(
                    "resolved artifact {} is not pinned by the composition manifest",
                    record.digest
                ),
            ));
        }
        validate_full_digest("resolved component digest", &record.digest)?;
        let contract = record.component.as_ref().ok_or_else(|| {
            Error::coded(
                "COMPOSITION_COMPONENT_CONTRACT_REQUIRED",
                Category::Validation,
                format!(
                    "artifact {} ({}) has no versioned component contract",
                    record.short_digest(),
                    record.name
                ),
            )
            .with_hint("republish the artifact with OpenStrata v0.22.4 or newer")
        })?;
        contract.validate()?;
        if contract.id != record.name || contract.version != record.version {
            return Err(Error::coded(
                "COMPOSITION_COMPONENT_IDENTITY_MISMATCH",
                Category::Validation,
                format!(
                    "artifact {} records {} {}, but its component contract declares {} {}",
                    record.short_digest(),
                    record.name,
                    record.version,
                    contract.id,
                    contract.version
                ),
            ));
        }
        if !ids.insert(contract.id.clone()) {
            return Err(Error::coded(
                "COMPOSITION_COMPONENT_ID_COLLISION",
                Category::Validation,
                format!(
                    "component id '{}' is provided by multiple artifacts",
                    contract.id
                ),
            ));
        }
        candidates.push(Candidate { record, contract });
    }
    candidates.sort_by(|left, right| candidate_key(left).cmp(&candidate_key(right)));

    let mut by_capability: BTreeMap<&str, Vec<(&Candidate<'_>, &CapabilityProvision)>> =
        BTreeMap::new();
    for candidate in &candidates {
        for provision in &candidate.contract.provides {
            by_capability
                .entry(&provision.capability)
                .or_default()
                .push((candidate, provision));
        }
    }

    let mut queue = declared
        .requirements
        .iter()
        .cloned()
        .map(|requirement| (None, requirement))
        .collect::<Vec<_>>();
    queue.sort_by(|left, right| requirement_key(&left.1).cmp(&requirement_key(&right.1)));
    let mut queue = VecDeque::from(queue);
    let mut selected = BTreeMap::<String, &Candidate<'_>>::new();
    let mut selected_provider = BTreeMap::<String, (&Candidate<'_>, String)>::new();
    let mut edges = BTreeSet::<(String, String)>::new();

    while let Some((consumer, requirement)) = queue.pop_front() {
        validate_requirement(&requirement)?;
        let manifest_pin = declared.providers.get(&requirement.capability);
        if let (Some(local), Some(global)) = (requirement.provider.as_ref(), manifest_pin) {
            if local != global {
                return Err(Error::coded(
                    "COMPOSITION_PROVIDER_PIN_CONFLICT",
                    Category::Configuration,
                    format!(
                        "requirement '{}' pins '{}', but providers pins '{}'",
                        requirement.capability, local, global
                    ),
                ));
            }
        }
        let pin = requirement.provider.as_ref().or(manifest_pin);

        if let Some((provider, version)) = selected_provider.get(&requirement.capability) {
            if pin.is_some_and(|pin| pin != &provider.contract.id) {
                return Err(Error::coded(
                    "COMPOSITION_PROVIDER_PIN_CONFLICT",
                    Category::Validation,
                    format!(
                        "capability '{}' already resolves to '{}', not pinned '{}'",
                        requirement.capability,
                        provider.contract.id,
                        pin.unwrap_or(&provider.contract.id)
                    ),
                ));
            }
            require_version(&requirement, version, &provider.contract.id)?;
            if let Some(consumer) = consumer {
                edges.insert((provider.contract.id.clone(), consumer));
            }
            continue;
        }

        let providers = by_capability
            .get(requirement.capability.as_str())
            .cloned()
            .unwrap_or_default();
        if providers.is_empty() {
            return Err(Error::coded(
                "COMPOSITION_MISSING_PROVIDER",
                Category::Validation,
                format!(
                    "no artifact provides capability '{}'",
                    requirement.capability
                ),
            ));
        }
        let target_compatible = providers
            .into_iter()
            .filter(|(candidate, _)| target_matches(candidate, &declared.composition.target))
            .collect::<Vec<_>>();
        if target_compatible.is_empty() {
            return Err(Error::coded(
                "COMPOSITION_TARGET_CONFLICT",
                Category::Validation,
                format!(
                    "providers for capability '{}' do not support target '{}'",
                    requirement.capability, declared.composition.target
                ),
            ));
        }
        if let Some(pin) = pin {
            if !target_compatible
                .iter()
                .any(|(candidate, _)| candidate.contract.id == *pin)
            {
                return Err(Error::coded(
                    "COMPOSITION_PROVIDER_PIN_MISMATCH",
                    Category::Validation,
                    format!(
                        "provider pin '{pin}' does not name a target-compatible provider for '{}'",
                        requirement.capability
                    ),
                ));
            }
        }
        let mut compatible = Vec::new();
        for (candidate, provision) in target_compatible {
            if pin.is_some_and(|pin| pin != &candidate.contract.id) {
                continue;
            }
            if requirement
                .version
                .as_deref()
                .map(|range| version_satisfies(&provision.version, range))
                .transpose()?
                .unwrap_or(true)
            {
                compatible.push((candidate, provision));
            }
        }
        if compatible.is_empty() {
            let detail = match pin {
                Some(pin) => format!(
                    "provider pin '{pin}' cannot satisfy capability '{}'{}",
                    requirement.capability,
                    rendered_constraint(&requirement)
                ),
                None => format!(
                    "providers for capability '{}' do not satisfy{}",
                    requirement.capability,
                    rendered_constraint(&requirement)
                ),
            };
            return Err(Error::coded(
                "COMPOSITION_VERSION_CONFLICT",
                Category::Validation,
                detail,
            ));
        }
        if compatible.len() > 1
            && pin.is_none()
            && compatible.iter().any(|(_, provision)| provision.singleton)
        {
            return Err(Error::coded(
                "COMPOSITION_SINGLETON_COLLISION",
                Category::Validation,
                format!(
                    "singleton capability '{}' has multiple compatible providers: {}",
                    requirement.capability,
                    compatible
                        .iter()
                        .map(|(candidate, _)| candidate.contract.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
            .with_hint(format!(
                "pin one provider in [providers], for example '{}' = '{}'",
                requirement.capability, compatible[0].0.contract.id
            )));
        }
        compatible.sort_by(|left, right| candidate_key(left.0).cmp(&candidate_key(right.0)));
        let (provider, provision) = compatible[0];
        selected_provider.insert(
            requirement.capability.clone(),
            (provider, provision.version.clone()),
        );
        if let Some(consumer) = consumer {
            edges.insert((provider.contract.id.clone(), consumer));
        }
        if selected
            .insert(provider.contract.id.clone(), provider)
            .is_none()
        {
            let mut requirements = provider.contract.requires.clone();
            requirements.sort_by(|left, right| requirement_key(left).cmp(&requirement_key(right)));
            for dependency in requirements {
                queue.push_back((Some(provider.contract.id.clone()), dependency));
            }
        }
    }

    for (capability, provider) in &declared.providers {
        if !selected_provider.contains_key(capability) {
            return Err(Error::coded(
                "COMPOSITION_UNUSED_PROVIDER_PIN",
                Category::Configuration,
                format!(
                    "provider pin '{capability}' = '{provider}' does not select a declared or transitive requirement"
                ),
            ));
        }
    }

    check_singletons(&selected)?;
    check_abi(&selected)?;
    check_openusd(&selected_provider, &selected)?;
    let ordered_ids = topological_order(&selected, &edges)?;
    let components = ordered_ids
        .iter()
        .map(|id| {
            let candidate = selected[id];
            ResolvedRuntimeComponent {
                id: id.clone(),
                kind: candidate.contract.kind.as_str().into(),
                version: candidate.contract.version.clone(),
                digest: candidate.record.digest.clone(),
                target: candidate.record.target.clone(),
            }
        })
        .collect::<Vec<_>>();
    let providers = selected_provider
        .iter()
        .map(|(capability, (candidate, version))| ResolvedProvider {
            capability: capability.clone(),
            component: candidate.contract.id.clone(),
            version: version.clone(),
            digest: candidate.record.digest.clone(),
        })
        .collect::<Vec<_>>();
    let environment = resolve_environment(&selected)?;
    let install = resolve_install(&selected)?;
    let mut resolved = ResolvedRuntimeComposition {
        schema: RESOLVED_COMPOSITION_SCHEMA.into(),
        name: declared.composition.name.clone(),
        target: declared.composition.target.clone(),
        manifest_digest: declared.digest()?,
        composition_digest: String::new(),
        components,
        providers,
        environment,
        install,
        conflicts: Vec::new(),
    };
    let bytes = serde_json::to_vec(&resolved).map_err(|error| {
        Error::Operation(format!("cannot serialize resolved composition: {error}"))
    })?;
    resolved.composition_digest = digest::sha256_hex(&bytes);
    Ok(resolved)
}

fn candidate_key<'a>(candidate: &'a Candidate<'_>) -> (&'a str, &'a str, &'a str) {
    (
        &candidate.contract.id,
        &candidate.contract.version,
        &candidate.record.digest,
    )
}

fn requirement_key(requirement: &CapabilityRequirement) -> (&str, &str, &str) {
    (
        &requirement.capability,
        requirement.version.as_deref().unwrap_or(""),
        requirement.provider.as_deref().unwrap_or(""),
    )
}

fn target_matches(candidate: &Candidate<'_>, target: &str) -> bool {
    let targets = &candidate.contract.compatibility.targets;
    if targets.is_empty() {
        candidate.record.target == target
    } else {
        targets.iter().any(|accepted| accepted == target)
    }
}

fn require_version(
    requirement: &CapabilityRequirement,
    version: &str,
    provider: &str,
) -> Result<()> {
    let Some(range) = requirement.version.as_deref() else {
        return Ok(());
    };
    if version_satisfies(version, range)? {
        return Ok(());
    }
    Err(Error::coded(
        "COMPOSITION_VERSION_CONFLICT",
        Category::Validation,
        format!(
            "provider '{provider}' exposes {} {version}, which does not satisfy '{range}'",
            requirement.capability
        ),
    ))
}

fn version_satisfies(version: &str, range: &str) -> Result<bool> {
    ost_plugin::satisfies(version, range).map_err(|error| {
        Error::coded(
            "COMPOSITION_INVALID_VERSION_CONSTRAINT",
            Category::Configuration,
            error.to_string(),
        )
    })
}

fn rendered_constraint(requirement: &CapabilityRequirement) -> String {
    requirement
        .version
        .as_ref()
        .map(|version| format!(" version '{version}'"))
        .unwrap_or_default()
}

fn check_singletons(selected: &BTreeMap<String, &Candidate<'_>>) -> Result<()> {
    let mut providers = BTreeMap::<&str, (bool, Vec<&str>)>::new();
    for candidate in selected.values() {
        for provision in &candidate.contract.provides {
            let (has_singleton, sources) = providers
                .entry(&provision.capability)
                .or_insert_with(|| (false, Vec::new()));
            *has_singleton |= provision.singleton;
            sources.push(&candidate.contract.id);
        }
    }
    for (capability, (has_singleton, sources)) in providers {
        if has_singleton && sources.len() > 1 {
            return Err(Error::coded(
                "COMPOSITION_SINGLETON_COLLISION",
                Category::Validation,
                format!(
                    "selected components {} all provide singleton capability '{capability}'",
                    sources.join(", ")
                ),
            ));
        }
    }
    Ok(())
}

fn check_abi(selected: &BTreeMap<String, &Candidate<'_>>) -> Result<()> {
    let by_abi = selected
        .values()
        .filter_map(|candidate| {
            candidate
                .contract
                .compatibility
                .abi
                .as_deref()
                .map(|abi| (abi, candidate.contract.id.as_str()))
        })
        .fold(BTreeMap::<&str, Vec<&str>>::new(), |mut map, (abi, id)| {
            map.entry(abi).or_default().push(id);
            map
        });
    if by_abi.len() <= 1 {
        return Ok(());
    }
    Err(Error::coded(
        "COMPOSITION_ABI_CONFLICT",
        Category::Validation,
        format!(
            "selected components require incompatible ABIs: {}",
            by_abi
                .iter()
                .map(|(abi, ids)| format!("{abi} ({})", ids.join(", ")))
                .collect::<Vec<_>>()
                .join("; ")
        ),
    ))
}

fn check_openusd(
    providers: &BTreeMap<String, (&Candidate<'_>, String)>,
    selected: &BTreeMap<String, &Candidate<'_>>,
) -> Result<()> {
    let Some((_, version)) = providers.get("usd") else {
        return Ok(());
    };
    for candidate in selected.values() {
        let Some(range) = candidate.contract.compatibility.openusd.as_deref() else {
            continue;
        };
        if !version_satisfies(version, range)? {
            return Err(Error::coded(
                "COMPOSITION_OPENUSD_CONFLICT",
                Category::Validation,
                format!(
                    "component '{}' requires OpenUSD '{}', but selected provider is {version}",
                    candidate.contract.id, range
                ),
            ));
        }
    }
    Ok(())
}

fn resolve_environment(
    selected: &BTreeMap<String, &Candidate<'_>>,
) -> Result<Vec<ResolvedEnvironmentContribution>> {
    let mut by_variable =
        BTreeMap::<&str, Vec<(&str, &ost_artifact::EnvironmentContribution)>>::new();
    for candidate in selected.values() {
        for contribution in &candidate.contract.environment {
            by_variable
                .entry(&contribution.variable)
                .or_default()
                .push((&candidate.contract.id, contribution));
        }
    }
    let mut resolved = Vec::new();
    for (variable, mut contributions) in by_variable {
        contributions.sort_by(|left, right| left.0.cmp(right.0));
        let sets = contributions
            .iter()
            .filter(|(_, contribution)| contribution.operation == EnvironmentOperation::Set)
            .collect::<Vec<_>>();
        if !sets.is_empty() {
            let distinct = sets
                .iter()
                .map(|(_, contribution)| &contribution.values)
                .collect::<BTreeSet<_>>();
            if sets.len() != contributions.len() || distinct.len() != 1 {
                return Err(Error::coded(
                    "COMPOSITION_ENVIRONMENT_CONFLICT",
                    Category::Validation,
                    format!(
                        "environment variable '{variable}' has incompatible set/list contributions from {}",
                        contributions
                            .iter()
                            .map(|(source, _)| *source)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
        }
        for (source, contribution) in contributions {
            resolved.push(ResolvedEnvironmentContribution {
                variable: variable.into(),
                operation: contribution.operation.as_str().into(),
                source: source.into(),
                values: contribution.values.clone(),
            });
        }
    }
    Ok(resolved)
}

fn resolve_install(
    selected: &BTreeMap<String, &Candidate<'_>>,
) -> Result<Vec<ResolvedInstallMapping>> {
    let mut owners = BTreeMap::<String, (&str, &str)>::new();
    let mut resolved = Vec::new();
    for candidate in selected.values() {
        for mapping in &candidate.contract.install {
            let normalized = normalize_relative_path(&mapping.destination);
            let key = normalized.to_ascii_lowercase();
            if let Some((owner, destination)) =
                owners.insert(key, (&candidate.contract.id, mapping.destination.as_str()))
            {
                return Err(Error::coded(
                    "COMPOSITION_INSTALL_PATH_COLLISION",
                    Category::Validation,
                    format!(
                        "components '{owner}' and '{}' both install '{}'/ '{}'",
                        candidate.contract.id, destination, mapping.destination
                    ),
                ));
            }
            resolved.push(ResolvedInstallMapping {
                destination: normalized,
                component: candidate.contract.id.clone(),
                artifact: candidate.record.digest.clone(),
                source: normalize_relative_path(&mapping.source),
            });
        }
    }
    resolved.sort_by(|left, right| {
        (&left.destination, &left.component, &left.source).cmp(&(
            &right.destination,
            &right.component,
            &right.source,
        ))
    });
    Ok(resolved)
}

fn normalize_relative_path(path: &str) -> String {
    path.split(['/', '\\'])
        .filter(|component| !component.is_empty() && *component != ".")
        .collect::<Vec<_>>()
        .join("/")
}

fn topological_order(
    selected: &BTreeMap<String, &Candidate<'_>>,
    edges: &BTreeSet<(String, String)>,
) -> Result<Vec<String>> {
    let mut indegree = selected
        .keys()
        .map(|id| (id.clone(), 0usize))
        .collect::<BTreeMap<_, _>>();
    let mut outgoing = BTreeMap::<String, BTreeSet<String>>::new();
    for (provider, consumer) in edges {
        if provider == consumer {
            continue;
        }
        if outgoing
            .entry(provider.clone())
            .or_default()
            .insert(consumer.clone())
        {
            *indegree.entry(consumer.clone()).or_default() += 1;
        }
    }
    let mut ready = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| id.clone())
        .collect::<BTreeSet<_>>();
    let mut order = Vec::new();
    while let Some(id) = ready.pop_first() {
        order.push(id.clone());
        for consumer in outgoing.get(&id).into_iter().flatten() {
            let degree = indegree.get_mut(consumer).expect("selected consumer");
            *degree -= 1;
            if *degree == 0 {
                ready.insert(consumer.clone());
            }
        }
    }
    if order.len() != selected.len() {
        return Err(Error::coded(
            "COMPOSITION_DEPENDENCY_CYCLE",
            Category::Validation,
            "selected component requirements contain a dependency cycle",
        ));
    }
    Ok(order)
}

fn validate_requirement(requirement: &CapabilityRequirement) -> Result<()> {
    portable_capability("requirement capability", &requirement.capability)?;
    if let Some(provider) = &requirement.provider {
        portable_id("requirement provider", provider)?;
    }
    if let Some(range) = &requirement.version {
        if range.trim().is_empty() {
            return Err(Error::coded(
                "COMPOSITION_INVALID_VERSION_CONSTRAINT",
                Category::Configuration,
                format!(
                    "requirement '{}' has an empty version constraint",
                    requirement.capability
                ),
            ));
        }
        // Validate the range independently from any candidate version.
        ost_plugin::satisfies("0", range).map_err(|error| {
            Error::coded(
                "COMPOSITION_INVALID_VERSION_CONSTRAINT",
                Category::Configuration,
                error.to_string(),
            )
        })?;
    }
    Ok(())
}

fn portable_id(field: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(Error::config(format!(
            "{field} '{value}' is not a portable identifier"
        )));
    }
    Ok(())
}

fn portable_capability(field: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':'))
    {
        return Err(Error::config(format!(
            "{field} '{value}' is not a portable capability"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ost_artifact::{
        ArtifactKind, ArtifactSource, ComponentCompatibility, ComponentKind,
        EnvironmentContribution, InstallMapping, TrustLevel, COMPONENT_SCHEMA, RECORD_SCHEMA,
    };

    fn digest(byte: &str) -> String {
        format!("sha256:{}", byte.repeat(64))
    }

    fn record(
        id: &str,
        version: &str,
        target: &str,
        byte: &str,
        provides: &[(&str, bool)],
        requires: &[(&str, Option<&str>)],
    ) -> ArtifactRecord {
        ArtifactRecord {
            schema: RECORD_SCHEMA,
            kind: if id == "openusd" {
                ArtifactKind::Runtime
            } else {
                ArtifactKind::Package
            },
            name: id.into(),
            version: version.into(),
            target: target.into(),
            profile: Some("usd".into()),
            digest: digest(byte),
            archive: format!("{id}.tar.zst"),
            archive_size: 1,
            total_size: 1,
            file_count: 1,
            created_unix: 1,
            producer: Some("ost test".into()),
            imported_by: "ost test".into(),
            source: ArtifactSource::Published,
            trust: TrustLevel::Unsigned,
            validation: "passed".into(),
            licenses: vec!["Apache-2.0".into()],
            sbom: None,
            sbom_digest: None,
            sbom_size: None,
            provenance: None,
            provenance_digest: None,
            provenance_size: None,
            runtime_id: None,
            runtime_digest: None,
            openusd_compatibility: None,
            openusd_verification: None,
            source_identity: None,
            dependency_identities: Vec::new(),
            component: Some(ComponentContract {
                schema: COMPONENT_SCHEMA.into(),
                id: id.into(),
                kind: if id == "openusd" {
                    ComponentKind::Runtime
                } else {
                    ComponentKind::Plugin
                },
                version: version.into(),
                provides: provides
                    .iter()
                    .map(|(capability, singleton)| CapabilityProvision {
                        capability: (*capability).into(),
                        version: version.into(),
                        singleton: *singleton,
                    })
                    .collect(),
                requires: requires
                    .iter()
                    .map(|(capability, range)| CapabilityRequirement {
                        capability: (*capability).into(),
                        version: range.map(str::to_string),
                        provider: None,
                    })
                    .collect(),
                environment: vec![EnvironmentContribution {
                    variable: if id == "openusd" {
                        "PATH".into()
                    } else {
                        "PXR_PLUGINPATH_NAME".into()
                    },
                    operation: EnvironmentOperation::Prepend,
                    values: vec![if id == "openusd" {
                        "bin".into()
                    } else {
                        format!("plugins/{id}")
                    }],
                }],
                install: vec![InstallMapping {
                    source: format!("payload/{id}"),
                    destination: format!("plugins/{id}"),
                }],
                compatibility: ComponentCompatibility {
                    targets: vec![target.into()],
                    abi: Some(
                        match target.split('-').next().unwrap_or_default() {
                            "windows" => "msvc143",
                            "macos" => "libcxx",
                            _ => "libstdcxx",
                        }
                        .into(),
                    ),
                    openusd: (id != "openusd").then(|| ">=26.05,<26.09".into()),
                },
                descriptor: None,
                descriptor_sha256: None,
                cmake: None,
                dependencies: None,
            }),
        }
    }

    fn manifest(target: &str, records: &[ArtifactRecord]) -> RuntimeCompositionManifest {
        RuntimeCompositionManifest {
            schema: COMPOSITION_SCHEMA.into(),
            composition: CompositionHeader {
                name: "geospatial".into(),
                target: target.into(),
            },
            requirements: vec![
                CapabilityRequirement {
                    capability: "usd".into(),
                    version: Some("26.08".into()),
                    provider: None,
                },
                CapabilityRequirement {
                    capability: "usd.fileformat.copc".into(),
                    version: None,
                    provider: None,
                },
            ],
            artifacts: records
                .iter()
                .map(|record| CompositionArtifactRef {
                    artifact: record.digest.clone(),
                    source: None,
                })
                .collect(),
            providers: BTreeMap::new(),
        }
    }

    #[test]
    fn synthetic_graph_has_the_same_order_on_all_hosts() {
        let mut orders = Vec::new();
        for (target, bytes) in [
            ("windows-x86_64-msvc143-py313", ("a", "b", "c")),
            ("linux-x86_64-glibc228-py313", ("d", "e", "f")),
            ("macos-arm64-macos145-py313", ("1", "2", "3")),
        ] {
            let records = vec![
                record("openusd", "26.08", target, bytes.0, &[("usd", true)], &[]),
                record(
                    "http-resolver",
                    "0.4.0",
                    target,
                    bytes.1,
                    &[("usd.resolve.http", true)],
                    &[("usd", Some(">=26.05,<26.09"))],
                ),
                record(
                    "pointcloud",
                    "0.8.0",
                    target,
                    bytes.2,
                    &[("usd.fileformat.copc", true)],
                    &[
                        ("usd", Some(">=26.05,<26.09")),
                        ("usd.resolve.http", Some(">=0.4")),
                    ],
                ),
            ];
            let resolved = resolve_runtime_composition(
                &manifest(target, &records),
                CompositionInput { records },
            )
            .unwrap();
            orders.push(
                resolved
                    .components
                    .iter()
                    .map(|component| component.id.clone())
                    .collect::<Vec<_>>(),
            );
        }
        assert!(orders.windows(2).all(|pair| pair[0] == pair[1]));
        assert_eq!(orders[0], ["openusd", "http-resolver", "pointcloud"]);
    }

    #[test]
    fn initial_geospatial_fixture_is_a_valid_manifest() {
        let fixture = include_str!("../../../fixtures/runtime-composition/geospatial.toml");
        let manifest = RuntimeCompositionManifest::parse(fixture).unwrap();
        assert_eq!(manifest.composition.name, "usd-geospatial-runtime");
        assert_eq!(manifest.requirements.len(), 4);
        assert_eq!(manifest.artifacts.len(), 4);
        assert_eq!(manifest.providers["usd.resolve.http"], "usd-http-resolver");
        let target = manifest.composition.target.as_str();
        let records = vec![
            record("openusd", "26.08", target, "1", &[("usd", true)], &[]),
            record(
                "usd-http-resolver",
                "0.4.0",
                target,
                "2",
                &[("usd.resolve.http", true)],
                &[("usd", Some(">=26.05,<26.09"))],
            ),
            record(
                "usd-pointcloud-plugins",
                "0.8.0",
                target,
                "3",
                &[("usd.fileformat.copc", true)],
                &[
                    ("usd", Some(">=26.05,<26.09")),
                    ("usd.resolve.http", Some(">=0.4,<0.5")),
                ],
            ),
            record(
                "usd-raster-plugins",
                "0.1.0",
                target,
                "4",
                &[("usd.fileformat.geotiff", true)],
                &[("usd", Some(">=26.05,<26.09"))],
            ),
        ];
        let resolved =
            resolve_runtime_composition(&manifest, CompositionInput { records }).unwrap();
        assert_eq!(
            resolved
                .components
                .iter()
                .map(|component| component.id.as_str())
                .collect::<Vec<_>>(),
            [
                "openusd",
                "usd-http-resolver",
                "usd-pointcloud-plugins",
                "usd-raster-plugins"
            ]
        );
    }

    #[test]
    fn singleton_selection_requires_a_provider_pin() {
        let target = "linux-x86_64-glibc228-py313";
        let records = vec![
            record("openusd", "26.08", target, "a", &[("usd", true)], &[]),
            record(
                "http-a",
                "1.0",
                target,
                "b",
                &[("usd.resolve.http", true)],
                &[],
            ),
            record(
                "http-b",
                "1.0",
                target,
                "c",
                &[("usd.resolve.http", true)],
                &[],
            ),
        ];
        let mut declared = manifest(target, &records);
        declared.requirements[1].capability = "usd.resolve.http".into();
        let error = resolve_runtime_composition(
            &declared,
            CompositionInput {
                records: records.clone(),
            },
        )
        .unwrap_err();
        assert_eq!(error.code(), "COMPOSITION_SINGLETON_COLLISION");

        declared
            .providers
            .insert("usd.resolve.http".into(), "http-b".into());
        let resolved =
            resolve_runtime_composition(&declared, CompositionInput { records }).unwrap();
        assert_eq!(
            resolved
                .providers
                .iter()
                .find(|provider| provider.capability == "usd.resolve.http")
                .unwrap()
                .component,
            "http-b"
        );
    }

    #[test]
    fn singleton_conflicts_include_non_singleton_co_providers() {
        let target = "linux-x86_64-glibc228-py313";
        let records = vec![
            record("openusd", "26.08", target, "a", &[("usd", true)], &[]),
            record(
                "feature-a",
                "1.0",
                target,
                "b",
                &[("feature.a", true), ("shared.provider", true)],
                &[],
            ),
            record(
                "feature-b",
                "1.0",
                target,
                "c",
                &[("feature.b", true), ("shared.provider", false)],
                &[],
            ),
        ];
        let mut declared = manifest(target, &records);
        declared.requirements = vec![
            CapabilityRequirement {
                capability: "usd".into(),
                version: None,
                provider: None,
            },
            CapabilityRequirement {
                capability: "feature.a".into(),
                version: None,
                provider: None,
            },
            CapabilityRequirement {
                capability: "feature.b".into(),
                version: None,
                provider: None,
            },
        ];
        let error =
            resolve_runtime_composition(&declared, CompositionInput { records }).unwrap_err();
        assert_eq!(error.code(), "COMPOSITION_SINGLETON_COLLISION");
    }

    #[test]
    fn empty_requirement_versions_are_rejected() {
        let source = format!(
            r#"schema = "{COMPOSITION_SCHEMA}"

[composition]
name = "invalid-version"
target = "linux-x86_64-glibc228-py313"

[[requirements]]
capability = "usd"
version = ""

[[artifacts]]
artifact = "{}"
"#,
            digest("a")
        );
        let error = RuntimeCompositionManifest::parse(&source).unwrap_err();
        assert_eq!(error.code(), "COMPOSITION_INVALID_VERSION_CONSTRAINT");
    }

    #[test]
    fn install_collisions_fail_before_materialization() {
        let target = "linux-x86_64-glibc228-py313";
        let mut runtime = record("openusd", "26.08", target, "a", &[("usd", true)], &[]);
        let mut plugin = record(
            "pointcloud",
            "1.0",
            target,
            "b",
            &[("usd.fileformat.copc", true)],
            &[("usd", Some("26.08"))],
        );
        runtime.component.as_mut().unwrap().install[0].destination = "lib/shared.so".into();
        plugin.component.as_mut().unwrap().install[0].destination = "LIB/shared.so".into();
        let records = vec![runtime, plugin];
        let error =
            resolve_runtime_composition(&manifest(target, &records), CompositionInput { records })
                .unwrap_err();
        assert_eq!(error.code(), "COMPOSITION_INSTALL_PATH_COLLISION");
    }

    #[test]
    fn lexical_install_aliases_collide_before_materialization() {
        let target = "linux-x86_64-glibc228-py313";
        let mut runtime = record("openusd", "26.08", target, "a", &[("usd", true)], &[]);
        let mut plugin = record(
            "pointcloud",
            "1.0",
            target,
            "b",
            &[("usd.fileformat.copc", true)],
            &[("usd", Some("26.08"))],
        );
        runtime.component.as_mut().unwrap().install[0].destination = "lib/shared.so".into();
        plugin.component.as_mut().unwrap().install[0].destination = "lib/./shared.so".into();
        let records = vec![runtime, plugin];
        let error =
            resolve_runtime_composition(&manifest(target, &records), CompositionInput { records })
                .unwrap_err();
        assert_eq!(error.code(), "COMPOSITION_INSTALL_PATH_COLLISION");
    }

    #[test]
    fn missing_version_abi_openusd_and_environment_conflicts_are_coded() {
        let target = "linux-x86_64-glibc228-py313";
        let mut runtime = record("openusd", "26.08", target, "a", &[("usd", true)], &[]);
        let mut plugin = record(
            "pointcloud",
            "1.0",
            target,
            "b",
            &[("usd.fileformat.copc", true)],
            &[("usd", Some("26.08"))],
        );

        let mut missing = manifest(target, std::slice::from_ref(&runtime));
        missing.requirements[1].capability = "usd.fileformat.unknown".into();
        assert_eq!(
            resolve_runtime_composition(
                &missing,
                CompositionInput {
                    records: vec![runtime.clone()]
                }
            )
            .unwrap_err()
            .code(),
            "COMPOSITION_MISSING_PROVIDER"
        );

        let mut wrong_target_runtime = runtime.clone();
        wrong_target_runtime
            .component
            .as_mut()
            .unwrap()
            .compatibility
            .targets = vec!["windows-x86_64-msvc143-py313".into()];
        let records = vec![wrong_target_runtime, plugin.clone()];
        assert_eq!(
            resolve_runtime_composition(&manifest(target, &records), CompositionInput { records })
                .unwrap_err()
                .code(),
            "COMPOSITION_TARGET_CONFLICT"
        );

        let records = vec![runtime.clone(), plugin.clone()];
        let mut version = manifest(target, &records);
        version.requirements[1].version = Some(">=2".into());
        assert_eq!(
            resolve_runtime_composition(&version, CompositionInput { records })
                .unwrap_err()
                .code(),
            "COMPOSITION_VERSION_CONFLICT"
        );

        plugin.component.as_mut().unwrap().compatibility.abi = Some("libcxx".into());
        let records = vec![runtime.clone(), plugin.clone()];
        assert_eq!(
            resolve_runtime_composition(&manifest(target, &records), CompositionInput { records })
                .unwrap_err()
                .code(),
            "COMPOSITION_ABI_CONFLICT"
        );

        plugin.component.as_mut().unwrap().compatibility.abi = Some("libstdcxx".into());
        plugin.component.as_mut().unwrap().compatibility.openusd = Some(">=27".into());
        let records = vec![runtime.clone(), plugin.clone()];
        assert_eq!(
            resolve_runtime_composition(&manifest(target, &records), CompositionInput { records })
                .unwrap_err()
                .code(),
            "COMPOSITION_OPENUSD_CONFLICT"
        );

        plugin.component.as_mut().unwrap().compatibility.openusd = Some("26.08".into());
        runtime.component.as_mut().unwrap().environment = vec![EnvironmentContribution {
            variable: "USD_MODE".into(),
            operation: EnvironmentOperation::Set,
            values: vec!["strict".into()],
        }];
        plugin.component.as_mut().unwrap().environment = vec![EnvironmentContribution {
            variable: "USD_MODE".into(),
            operation: EnvironmentOperation::Set,
            values: vec!["permissive".into()],
        }];
        let records = vec![runtime, plugin];
        assert_eq!(
            resolve_runtime_composition(&manifest(target, &records), CompositionInput { records })
                .unwrap_err()
                .code(),
            "COMPOSITION_ENVIRONMENT_CONFLICT"
        );
    }
}
