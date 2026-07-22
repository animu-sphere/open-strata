// SPDX-License-Identifier: Apache-2.0
//! Digest-pinned, cross-repository Formation composition.
//!
//! This crate owns the portable declaration, resolved model, and lock contract.
//! Materialization and process launch stay at the CLI boundary; the model never
//! persists machine-local paths.

use std::collections::{BTreeMap, BTreeSet};

use camino::{Utf8Path, Utf8PathBuf};
use ost_artifact::{ArtifactKind, ArtifactRecord};
use ost_core::{digest, Category, Error, Result};
use ost_plugin::{diagnose, Bundle, RuntimeContext, Status};
use ost_runtime::{EnvOp, EnvSet, EnvVar, RuntimeManifest};
use serde::{Deserialize, Serialize};

pub const FORMATION_SCHEMA: &str = "openstrata.formation/v1alpha1";
pub const RESOLVED_SCHEMA: &str = "openstrata.formation-resolved/v1alpha1";
pub const LOCK_SCHEMA: &str = "openstrata.formation-lock/v1alpha1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormationManifest {
    pub schema: String,
    pub formation: FormationHeader,
    pub runtime: RuntimeRef,
    #[serde(default)]
    pub components: Vec<ComponentRef>,
    pub command: CommandSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormationHeader {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRef {
    pub artifact: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComponentKind {
    Plugin,
    Renderer,
}

impl ComponentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ComponentKind::Plugin => "plugin",
            ComponentKind::Renderer => "renderer",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentRef {
    pub id: String,
    pub kind: ComponentKind,
    pub artifact: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

impl FormationManifest {
    pub fn parse(source: &str) -> Result<Self> {
        let value: Self = toml::from_str(source)
            .map_err(|error| Error::parse("formation.toml", anyhow::Error::new(error)))?;
        value.validate()?;
        Ok(value)
    }

    pub fn load(path: &Utf8Path) -> Result<Self> {
        let source = std::fs::read_to_string(path.as_std_path())
            .map_err(|error| Error::io(path.to_string(), error))?;
        Self::parse(&source)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != FORMATION_SCHEMA {
            return Err(Error::config(format!(
                "unsupported Formation schema '{}' (expected '{FORMATION_SCHEMA}')",
                self.schema
            )));
        }
        validate_id("formation.name", &self.formation.name)?;
        validate_full_digest("runtime.artifact", &self.runtime.artifact)?;
        if self.command.program.trim().is_empty() {
            return Err(Error::config("command.program must not be empty"));
        }
        let mut ids = BTreeSet::new();
        for component in &self.components {
            validate_id("components.id", &component.id)?;
            validate_full_digest(
                &format!("component '{}'.artifact", component.id),
                &component.artifact,
            )?;
            if !ids.insert(component.id.clone()) {
                return Err(Error::coded(
                    "FORMATION_COMPONENT_CONFLICT",
                    Category::Configuration,
                    format!("component id '{}' is declared more than once", component.id),
                ));
            }
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String> {
        let bytes = serde_json::to_vec(self)
            .map_err(|error| Error::Operation(format!("cannot serialize Formation: {error}")))?;
        Ok(digest::sha256_hex(&bytes))
    }
}

fn validate_id(field: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(Error::config(format!(
            "{field} must use only ASCII letters, digits, '.', '-' or '_'"
        )));
    }
    Ok(())
}

pub fn validate_full_digest(field: &str, value: &str) -> Result<()> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(Error::config(format!(
            "{field} must be a full sha256:<64-hex> artifact identity"
        )));
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(Error::config(format!(
            "{field} must be a full sha256:<64-hex> artifact identity"
        )));
    }
    Ok(())
}

/// An extracted component and any plugin bundles it contributes. Product
/// artifacts contribute one bundle per product member.
#[derive(Debug, Clone)]
pub struct ComponentInput {
    pub declared: ComponentRef,
    pub record: ArtifactRecord,
    pub root: Utf8PathBuf,
    pub bundles: Vec<Bundle>,
    pub activation: ActivationInput,
}

#[derive(Debug, Clone, Default)]
pub struct ActivationInput {
    pub plugin_paths: Vec<Utf8PathBuf>,
    pub library_paths: Vec<Utf8PathBuf>,
    pub python_paths: Vec<Utf8PathBuf>,
    pub bin_paths: Vec<Utf8PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ResolutionInput {
    pub runtime_record: ArtifactRecord,
    pub runtime_manifest: RuntimeManifest,
    pub runtime_root: Utf8PathBuf,
    pub components: Vec<ComponentInput>,
}

#[derive(Debug, Clone)]
pub struct MaterializedFormation {
    pub resolved: ResolvedFormation,
    pub env: EnvSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedFormation {
    pub schema: String,
    pub name: String,
    pub manifest_digest: String,
    pub target: String,
    pub runtime: ResolvedArtifact,
    pub components: Vec<ResolvedComponent>,
    pub environment: Vec<EnvironmentContribution>,
    pub command: CommandSpec,
    pub conflicts: Vec<FormationConflict>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedArtifact {
    pub kind: String,
    pub name: String,
    pub version: String,
    pub digest: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_digest: Option<String>,
}

impl From<&ArtifactRecord> for ResolvedArtifact {
    fn from(record: &ArtifactRecord) -> Self {
        Self {
            kind: record.kind.as_str().into(),
            name: record.name.clone(),
            version: record.version.clone(),
            digest: record.digest.clone(),
            target: record.target.clone(),
            profile: record.profile.clone(),
            runtime_id: record.runtime_id.clone(),
            runtime_digest: record.runtime_digest.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedComponent {
    pub id: String,
    pub declared_kind: ComponentKind,
    #[serde(flatten)]
    pub artifact: ResolvedArtifact,
    pub bundles: Vec<ResolvedBundle>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedBundle {
    pub name: String,
    pub version: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentContribution {
    pub key: String,
    pub operation: String,
    pub source: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormationConflict {
    pub key: String,
    pub sources: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormationLock {
    pub schema: String,
    pub resolution: ResolvedFormation,
}

impl FormationLock {
    pub fn from_resolved(resolved: &ResolvedFormation) -> Self {
        Self {
            schema: LOCK_SCHEMA.into(),
            resolution: resolved.clone(),
        }
    }

    pub fn digest(&self) -> Result<String> {
        let bytes = serde_json::to_vec(self).map_err(|error| {
            Error::Operation(format!("cannot serialize Formation lock: {error}"))
        })?;
        Ok(digest::sha256_hex(&bytes))
    }
}

pub fn resolve(
    declared: &FormationManifest,
    input: ResolutionInput,
) -> Result<MaterializedFormation> {
    declared.validate()?;
    validate_full_digest("resolved runtime digest", &input.runtime_record.digest)?;
    if input.runtime_record.digest != declared.runtime.artifact {
        return Err(Error::validation(format!(
            "runtime resolved to {}, but Formation pins {}",
            input.runtime_record.digest, declared.runtime.artifact
        )));
    }
    if input.runtime_record.kind != ArtifactKind::Runtime {
        return Err(kind_mismatch(
            "runtime",
            ArtifactKind::Runtime,
            &input.runtime_record,
        ));
    }
    if input.runtime_record.validation != "passed" {
        return Err(Error::coded(
            "FORMATION_RUNTIME_UNVALIDATED",
            Category::Validation,
            format!(
                "runtime artifact {} records validation '{}' rather than passed",
                input.runtime_record.short_digest(),
                input.runtime_record.validation
            ),
        ));
    }
    if input.runtime_manifest.validation != ost_runtime::Validation::Passed {
        return Err(Error::coded(
            "FORMATION_RUNTIME_UNVALIDATED",
            Category::Validation,
            format!(
                "runtime '{}' is {} rather than passed",
                input.runtime_manifest.id,
                input.runtime_manifest.validation.as_str()
            ),
        )
        .with_hint("use a runtime artifact exported from a passed runtime validation"));
    }
    if input.runtime_manifest.compute_digest() != input.runtime_manifest.digest {
        return Err(Error::coded(
            "FORMATION_RUNTIME_IDENTITY_MISMATCH",
            Category::Validation,
            format!(
                "runtime '{}' manifest digest does not match its canonical fields",
                input.runtime_manifest.id
            ),
        ));
    }
    if input.runtime_record.target != input.runtime_manifest.variant.slug() {
        return Err(Error::coded(
            "FORMATION_RUNTIME_IDENTITY_MISMATCH",
            Category::Validation,
            format!(
                "runtime artifact target '{}' != embedded runtime variant '{}'",
                input.runtime_record.target,
                input.runtime_manifest.variant.slug()
            ),
        ));
    }
    if let Some(profile) = &input.runtime_record.profile {
        if profile != &input.runtime_manifest.profile {
            return Err(Error::coded(
                "FORMATION_RUNTIME_IDENTITY_MISMATCH",
                Category::Validation,
                format!(
                    "runtime artifact profile '{profile}' != embedded runtime profile '{}'",
                    input.runtime_manifest.profile
                ),
            ));
        }
    }
    if input.components.len() != declared.components.len() {
        return Err(Error::validation(format!(
            "Formation declares {} component(s), but resolution supplied {}",
            declared.components.len(),
            input.components.len()
        )));
    }

    let host = ost_core::Host::detect();
    let runtime_env = EnvSet::for_usd_install(&input.runtime_root, host.os);
    let mut env_vars = runtime_env.vars.clone();
    let mut environment = portable_runtime_environment(host.os);
    let runtime_context = runtime_context(&input.runtime_manifest);
    let runtime_id = &input.runtime_manifest.id;
    let runtime_digest = &input.runtime_manifest.digest;
    let mut resolved_components = Vec::new();
    let mut bundle_versions: BTreeMap<String, (String, String)> = BTreeMap::new();

    for (expected, component) in declared.components.iter().zip(input.components.iter()) {
        if &component.declared != expected {
            return Err(Error::validation(format!(
                "component resolution order/identity mismatch at '{}'",
                expected.id
            )));
        }
        if component.record.digest != expected.artifact {
            return Err(Error::validation(format!(
                "component '{}' resolved to {}, but Formation pins {}",
                expected.id, component.record.digest, expected.artifact
            )));
        }
        validate_component_kind(expected, &component.record)?;
        if component.record.validation != "passed" {
            return Err(Error::coded(
                "FORMATION_COMPONENT_UNVALIDATED",
                Category::Validation,
                format!(
                    "component '{}' artifact {} records validation '{}' rather than passed",
                    expected.id,
                    component.record.short_digest(),
                    component.record.validation
                ),
            ));
        }
        validate_target(expected, &component.record, &input.runtime_manifest)?;
        if let Some(id) = &component.record.runtime_id {
            if id != runtime_id {
                return Err(compatibility_error(
                    &expected.id,
                    format!("runtime id '{id}' != selected '{runtime_id}'"),
                ));
            }
        }
        if let Some(digest) = &component.record.runtime_digest {
            if digest != runtime_digest {
                return Err(compatibility_error(
                    &expected.id,
                    format!("runtime digest '{digest}' != selected '{runtime_digest}'"),
                ));
            }
        }
        if let Some(profile) = &component.record.profile {
            if profile != &input.runtime_manifest.profile {
                return Err(compatibility_error(
                    &expected.id,
                    format!(
                        "profile '{profile}' != selected '{}'",
                        input.runtime_manifest.profile
                    ),
                ));
            }
        }

        let mut resolved_bundles = Vec::new();
        for bundle in &component.bundles {
            validate_bundle_compatibility(
                &expected.id,
                bundle,
                &runtime_context,
                &input.runtime_manifest,
            )?;
            let identity = &bundle.manifest.plugin;
            if let Some((version, owner)) = bundle_versions.get(&identity.name) {
                return Err(Error::coded(
                    "FORMATION_COMPONENT_CONFLICT",
                    Category::Validation,
                    format!(
                        "bundle '{}' is contributed by both '{}' (version {}) and '{}' (version {}); two discovery roots for one plugin identity are ambiguous",
                        identity.name, owner, version, expected.id, identity.version
                    ),
                )
                .with_hint("remove the duplicate component or use one aggregate product artifact"));
            } else {
                bundle_versions.insert(
                    identity.name.clone(),
                    (identity.version.clone(), expected.id.clone()),
                );
            }
            resolved_bundles.push(ResolvedBundle {
                name: identity.name.clone(),
                version: identity.version.clone(),
                kind: identity.kind.as_str().into(),
            });
        }

        append_activation(
            expected,
            &component.root,
            &component.activation,
            host.os,
            &mut env_vars,
            &mut environment,
        )?;
        resolved_components.push(ResolvedComponent {
            id: expected.id.clone(),
            declared_kind: expected.kind,
            artifact: ResolvedArtifact::from(&component.record),
            bundles: resolved_bundles,
        });
    }

    let resolved = ResolvedFormation {
        schema: RESOLVED_SCHEMA.into(),
        name: declared.formation.name.clone(),
        manifest_digest: declared.digest()?,
        target: format!(
            "{}-{}-{}",
            input.runtime_manifest.platform,
            input.runtime_manifest.variant.slug(),
            input.runtime_manifest.profile
        ),
        runtime: ResolvedArtifact::from(&input.runtime_record),
        components: resolved_components,
        environment,
        command: declared.command.clone(),
        conflicts: Vec::new(),
    };
    Ok(MaterializedFormation {
        resolved,
        env: EnvSet {
            sep: runtime_env.sep,
            vars: env_vars,
        },
    })
}

fn validate_component_kind(component: &ComponentRef, record: &ArtifactRecord) -> Result<()> {
    let accepted = match component.kind {
        ComponentKind::Plugin => {
            matches!(record.kind, ArtifactKind::Plugin | ArtifactKind::Product)
        }
        ComponentKind::Renderer => matches!(
            record.kind,
            ArtifactKind::Plugin | ArtifactKind::Product | ArtifactKind::Package
        ),
    };
    if !accepted {
        return Err(Error::coded(
            "FORMATION_ARTIFACT_KIND_MISMATCH",
            Category::Validation,
            format!(
                "component '{}' is declared {}, but {} is a {} artifact",
                component.id,
                component.kind.as_str(),
                record.short_digest(),
                record.kind.as_str()
            ),
        ));
    }
    Ok(())
}

fn kind_mismatch(subject: &str, expected: ArtifactKind, record: &ArtifactRecord) -> Error {
    Error::coded(
        "FORMATION_ARTIFACT_KIND_MISMATCH",
        Category::Validation,
        format!(
            "{subject} needs a {} artifact, but {} is {}",
            expected.as_str(),
            record.short_digest(),
            record.kind.as_str()
        ),
    )
}

fn validate_target(
    component: &ComponentRef,
    record: &ArtifactRecord,
    runtime: &RuntimeManifest,
) -> Result<()> {
    let variant = runtime.variant.slug();
    let platform_variant = format!("{}-{variant}", runtime.platform);
    if record.target != variant && !record.target.starts_with(&platform_variant) {
        return Err(compatibility_error(
            &component.id,
            format!(
                "target '{}' does not match runtime platform '{}' and variant '{}'",
                record.target, runtime.platform, variant
            ),
        ));
    }
    Ok(())
}

fn compatibility_error(component: &str, reason: String) -> Error {
    Error::coded(
        "FORMATION_INCOMPATIBLE_COMPONENT",
        Category::Validation,
        format!("component '{component}' is incompatible: {reason}"),
    )
    .with_hint("select an artifact built and validated against the pinned runtime")
}

fn runtime_context(runtime: &RuntimeManifest) -> RuntimeContext {
    let mut context = RuntimeContext {
        target_os: Some(runtime.variant.os),
        pulled: true,
        source: Some("artifact".into()),
        real: true,
        reproducible: true,
        cxx_abi: Some(match runtime.variant.os {
            ost_core::host::Os::Linux => "libstdcxx".into(),
            ost_core::host::Os::Macos => "libcxx".into(),
            ost_core::host::Os::Windows => match &runtime.variant.abi {
                ost_core::variant::Abi::Msvc { toolset } => format!("msvc{toolset}"),
                _ => "msvc".into(),
            },
        }),
        python_abi: Some(runtime.variant.python_abi()),
        ..RuntimeContext::default()
    };
    for extension in &runtime.extensions {
        context
            .components
            .insert(extension.id.clone(), extension.version.clone());
        if extension.id == "openusd" {
            context.openusd_version = Some(extension.version.clone());
        }
    }
    context
}

fn validate_bundle_compatibility(
    component_id: &str,
    bundle: &Bundle,
    context: &RuntimeContext,
    runtime: &RuntimeManifest,
) -> Result<()> {
    for capability in &bundle.manifest.requires.capabilities {
        if !runtime.capabilities.iter().any(|have| have == capability) {
            return Err(compatibility_error(
                component_id,
                format!("requires runtime capability '{capability}'"),
            ));
        }
    }
    let report = diagnose(bundle, context, 1);
    if let Some(failure) = report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.level == 1 && diagnostic.status == Status::Fail)
    {
        return Err(compatibility_error(
            component_id,
            format!("{}: {}", failure.id, failure.observed),
        ));
    }
    Ok(())
}

fn portable_runtime_environment(os: ost_core::host::Os) -> Vec<EnvironmentContribution> {
    let loader = loader_key(os);
    vec![
        contribution("PATH", "runtime", vec!["runtime/bin".into()]),
        contribution(loader, "runtime", vec!["runtime/lib".into()]),
        contribution("PYTHONPATH", "runtime", vec!["runtime/lib/python".into()]),
        contribution("CMAKE_PREFIX_PATH", "runtime", vec!["runtime".into()]),
        contribution(
            "PXR_PLUGINPATH_NAME",
            "runtime",
            vec!["runtime/plugin/usd".into()],
        ),
    ]
}

fn append_activation(
    component: &ComponentRef,
    root: &Utf8Path,
    activation: &ActivationInput,
    os: ost_core::host::Os,
    vars: &mut Vec<EnvVar>,
    portable: &mut Vec<EnvironmentContribution>,
) -> Result<()> {
    let groups = [
        ("PXR_PLUGINPATH_NAME", &activation.plugin_paths),
        (loader_key(os), &activation.library_paths),
        ("PYTHONPATH", &activation.python_paths),
        ("PATH", &activation.bin_paths),
    ];
    for (key, paths) in groups {
        if paths.is_empty() {
            continue;
        }
        let mut portable_paths = Vec::new();
        let mut seen = BTreeSet::new();
        for path in paths.iter().rev() {
            let path_text = path.to_string().replace('\\', "/");
            vars.push(EnvVar {
                key: key.into(),
                op: EnvOp::Prepend(path_text),
            });
        }
        for path in paths {
            let relative = path.strip_prefix(root).map_err(|_| {
                Error::coded(
                    "FORMATION_ACTIVATION_ESCAPE",
                    Category::Validation,
                    format!(
                        "component '{}' activation path '{}' escapes its artifact root",
                        component.id, path
                    ),
                )
            })?;
            let rendered = format!(
                "components/{}/{}",
                component.id,
                relative.to_string().replace('\\', "/")
            )
            .trim_end_matches('/')
            .to_string();
            if seen.insert(rendered.clone()) {
                portable_paths.push(rendered);
            }
        }
        portable.push(contribution(key, &component.id, portable_paths));
    }
    Ok(())
}

fn contribution(key: &str, source: &str, paths: Vec<String>) -> EnvironmentContribution {
    EnvironmentContribution {
        key: key.into(),
        operation: "prepend".into(),
        source: source.into(),
        paths,
    }
}

fn loader_key(os: ost_core::host::Os) -> &'static str {
    match os {
        ost_core::host::Os::Linux => "LD_LIBRARY_PATH",
        ost_core::host::Os::Macos => "DYLD_LIBRARY_PATH",
        ost_core::host::Os::Windows => "PATH",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ost_artifact::{ArtifactSource, TrustLevel};
    use ost_core::{host::Os, variant::Abi, Host, Variant};
    use ost_runtime::{RuntimeSource, Validation};

    fn manifest() -> String {
        format!(
            r#"schema = "openstrata.formation/v1alpha1"

[formation]
name = "vrm-merlin"

[runtime]
artifact = "sha256:{runtime}"

[[components]]
id = "usdVrmFileFormat"
kind = "plugin"
artifact = "sha256:{plugin}"

[command]
program = "usdview"
args = ["avatar.vrm"]
"#,
            runtime = "a1".repeat(32),
            plugin = "b2".repeat(32)
        )
    }

    #[test]
    fn parses_versioned_digest_pinned_manifest() {
        let parsed = FormationManifest::parse(&manifest()).unwrap();
        assert_eq!(parsed.formation.name, "vrm-merlin");
        assert_eq!(parsed.components[0].kind, ComponentKind::Plugin);
        assert!(parsed.digest().unwrap().starts_with("sha256:"));
    }

    #[test]
    fn rejects_digest_prefixes() {
        let source = manifest().replace(&"a1".repeat(32), "a1a1a1");
        let error = FormationManifest::parse(&source).unwrap_err();
        assert!(error.to_string().contains("full sha256"));
    }

    #[test]
    fn rejects_unknown_fields() {
        let source = manifest().replace(
            "name = \"vrm-merlin\"",
            "name = \"vrm-merlin\"\nsource = \"mutable\"",
        );
        assert!(FormationManifest::parse(&source).is_err());
    }

    #[test]
    fn rejects_duplicate_component_ids() {
        let component = format!(
            "\n[[components]]\nid = \"usdVrmFileFormat\"\nkind = \"renderer\"\nartifact = \"sha256:{}\"\n",
            "c3".repeat(32)
        );
        let source = manifest().replace("\n[command]", &format!("{component}\n[command]"));
        let error = FormationManifest::parse(&source).unwrap_err();
        assert_eq!(error.code(), "FORMATION_COMPONENT_CONFLICT");
    }

    #[test]
    fn empty_resolution_and_lock_are_portable_and_deterministic() {
        let runtime_digest = format!("sha256:{}", "a1".repeat(32));
        let declared = FormationManifest::parse(
            &manifest()
                .lines()
                .filter(|line| {
                    !matches!(
                        *line,
                        "[[components]]" | "id = \"usdVrmFileFormat\"" | "kind = \"plugin\""
                    ) && !line.starts_with("artifact = \"sha256:b2")
                })
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let variant = Variant::new(
            &Host {
                os: Os::Windows,
                arch: ost_core::host::Arch::X86_64,
            },
            Abi::Msvc {
                toolset: "143".into(),
            },
            "313",
        );
        let mut runtime_manifest = RuntimeManifest {
            schema: RuntimeManifest::SCHEMA_VERSION,
            id: "openstrata-cy2026-usd-windows-x86_64-py313".into(),
            platform: "cy2026".into(),
            profile: "usd".into(),
            variant,
            python: "3.13.x".into(),
            capabilities: vec!["usd-stage-read".into()],
            layout: vec!["bin".into(), "lib".into()],
            extensions: Vec::new(),
            digest: format!("sha256:{}", "d4".repeat(32)),
            validation: Validation::Passed,
            created_unix: 1,
            source: RuntimeSource::Artifact,
            external_prefix: None,
            runtime_deps: Vec::new(),
            artifact_digest: Some(runtime_digest.clone()),
        };
        runtime_manifest.digest = runtime_manifest.compute_digest();
        let record = ArtifactRecord {
            schema: 1,
            kind: ArtifactKind::Runtime,
            name: "openstrata-runtime".into(),
            version: "2026".into(),
            target: "windows-x86_64-msvc143-py313".into(),
            profile: Some("usd".into()),
            digest: runtime_digest,
            archive: "runtime.tar.zst".into(),
            archive_size: 1,
            total_size: 1,
            file_count: 1,
            created_unix: 1,
            producer: Some("ost test".into()),
            imported_by: "ost test".into(),
            source: ArtifactSource::Published,
            trust: TrustLevel::Local,
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
        };
        let resolved = resolve(
            &declared,
            ResolutionInput {
                runtime_record: record,
                runtime_manifest,
                runtime_root: Utf8PathBuf::from("C:/private/runtime"),
                components: Vec::new(),
            },
        )
        .unwrap();
        let lock = FormationLock::from_resolved(&resolved.resolved);
        let json = serde_json::to_string(&lock).unwrap();
        assert!(!json.contains("C:/private"));
        assert_eq!(lock.digest().unwrap(), lock.digest().unwrap());
        assert!(json.contains("runtime/bin"));
    }
}
