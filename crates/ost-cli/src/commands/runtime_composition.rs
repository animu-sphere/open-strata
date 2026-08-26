// SPDX-License-Identifier: Apache-2.0
//! Locked composition lifecycle. Reuse the artifact transport, safe extraction,
//! packaging and Formation resolver; no second dependency solver or loader model.

use crate::output::{self, Format};
use camino::{Utf8Path, Utf8PathBuf};
use ost_artifact::{
    ArtifactRecord, ArtifactSource, ArtifactStore, ArtifactTransport, ManifestFile, RemoteReference,
};
use ost_core::{digest, Error, Result};
use ost_formation::{
    canonical_json_digest, composition_error, resolve_runtime_composition, CompositionInput,
    CompositionInventoryEntry, LockedCompositionArtifact, RuntimeCompositionLock,
    RuntimeCompositionManifest,
};
use ost_platform::{ResolvedDependencyIdentity, ResolvedSourceIdentity};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

const LOCK_PATH: &str = "metadata/composition.lock.json";

#[path = "runtime_sdk.rs"]
mod sdk;
pub use sdk::{environment, execute};

fn read_json(path: &Utf8Path) -> Result<Value> {
    serde_json::from_slice(&fs::read(path).map_err(|e| Error::io(path.to_string(), e))?)
        .map_err(|e| Error::parse(path.to_string(), anyhow::Error::new(e)))
}

fn write_json(path: &Utf8Path, value: &impl serde::Serialize) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| Error::io(parent.to_string(), e))?;
    }
    let bytes = serde_json::to_string_pretty(value)
        .map_err(|e| Error::Operation(format!("cannot serialize composition metadata: {e}")))?;
    fs::write(path, format!("{bytes}\n")).map_err(|e| Error::io(path.to_string(), e))
}

fn read_lock(path: &Utf8Path) -> Result<RuntimeCompositionLock> {
    let lock: RuntimeCompositionLock = serde_json::from_value(read_json(path)?)
        .map_err(|e| Error::parse(path.to_string(), anyhow::Error::new(e)))?;
    lock.validate()?;
    Ok(lock)
}

/// Pins the producer manifest as well as the archive and validates all retained
/// sidecars. Re-derive records so an edited index cannot dictate resolution.
fn verified_artifact(
    store: &ArtifactStore,
    digest: &str,
) -> Result<(LockedCompositionArtifact, Value)> {
    let cached = store.resolve(digest)?;
    let manifest = store.producer_manifest(&cached)?;
    let mut record =
        ArtifactRecord::from_producer_manifest(&manifest, ArtifactSource::Imported, 0, "")?;
    if record.digest != digest || cached.digest != digest || !store.verify(digest)?.passed() {
        return Err(composition_error(
            "COMPOSITION_ARTIFACT_VERIFICATION_FAILED",
            format!("artifact {digest} failed integrity verification"),
        ));
    }
    // Check cache metadata against the producer, while retaining sidecar claims
    // that the importer adds after validating their bytes.
    record.sbom = cached.sbom.clone();
    record.sbom_digest = cached.sbom_digest.clone();
    record.sbom_size = cached.sbom_size;
    record.provenance = cached.provenance.clone();
    record.provenance_digest = cached.provenance_digest.clone();
    record.provenance_size = cached.provenance_size;
    let locked = LockedCompositionArtifact::new(record, &manifest)?;
    if LockedCompositionArtifact::new(cached, &manifest)? != locked {
        return Err(composition_error(
            "COMPOSITION_ARTIFACT_VERIFICATION_FAILED",
            format!("artifact {digest} registry record differs from its producer manifest"),
        ));
    }
    verify_evidence(
        store.object_dir(locked.record.digest_hex()).as_path(),
        &locked.record,
        &manifest,
    )?;
    Ok((locked, manifest))
}

fn verify_evidence(root: &Utf8Path, record: &ArtifactRecord, manifest: &Value) -> Result<()> {
    let store = ArtifactStore::at(root.to_owned());
    let (sbom, provenance) = store.evidence(record)?;
    if let Some(evidence) = sbom {
        ost_artifact::verify_evidence_digest(root, &evidence)?;
        ost_artifact::verify_sbom(
            &root.join(evidence.path),
            &record.digest,
            &record.dependency_identities,
        )?;
    }
    if let Some(evidence) = provenance {
        ost_artifact::verify_evidence_digest(root, &evidence)?;
        ost_artifact::verify_provenance(&root.join(evidence.path), manifest, &record.digest, None)?;
    }
    Ok(())
}

fn fetch_inputs(
    manifest: &RuntimeCompositionManifest,
    store: &ArtifactStore,
    verify_sources: bool,
) -> Result<()> {
    manifest.validate()?;
    let cached = store
        .list()?
        .into_iter()
        .map(|r| r.digest)
        .collect::<BTreeSet<_>>();
    for artifact in &manifest.artifacts {
        let is_cached = cached.contains(&artifact.artifact);
        // Existing locks pin the cached metadata independently of acquisition
        // locations, so locked operations remain usable offline.
        if is_cached && (!verify_sources || artifact.source.is_none()) {
            continue;
        }
        let Some(source) = &artifact.source else {
            return Err(composition_error("COMPOSITION_SOURCE_REQUIRED",
                format!("artifact {} is not cached; add an immutable source or import the pinned artifact", artifact.artifact)));
        };
        let reference = RemoteReference::parse(source)?;
        let transport: Box<dyn ArtifactTransport> = match reference {
            RemoteReference::File(_) => Box::new(ost_artifact::FileTransport::new()),
            RemoteReference::Oci(_) => Box::new(ost_artifact::OciTransport::new(false)),
        };
        if is_cached {
            let (cached, _) = verified_artifact(store, &artifact.artifact)?;
            let staging = Staging::for_destination(&store.root().join("composition-source-check"))?;
            let resolved = transport.resolve(&reference)?;
            let snapshot = transport.fetch_metadata(&reference, &resolved, &staging.0)?;
            let producer = read_json(&snapshot.dist.join("manifest.json"))?;
            let evidence = |name: &str| -> Result<Option<ost_artifact::EvidenceDigest>> {
                let path = snapshot.dist.join(name);
                if path
                    .try_exists()
                    .map_err(|e| Error::io(path.to_string(), e))?
                {
                    Ok(Some(ost_artifact::EvidenceDigest::from_file(&path, name)?))
                } else {
                    Ok(None)
                }
            };
            if canonical_json_digest(&producer)? != cached.manifest_digest
                || (
                    evidence(ost_artifact::SBOM_FILE)?,
                    evidence(ost_artifact::PROVENANCE_FILE)?,
                ) != store.evidence(&cached.record)?
            {
                return Err(composition_error(
                    "COMPOSITION_SOURCE_MISMATCH",
                    format!("source metadata for {} differs from the verified cache; use a source matching the cached metadata or a separate OST_HOME", artifact.artifact),
                ));
            }
            continue;
        }
        ost_artifact::pull(
            transport.as_ref(),
            &reference,
            store,
            &ost_artifact::PullPolicy {
                expected_artifact_digest: Some(artifact.artifact.clone()),
                ..Default::default()
            },
        )?;
    }
    Ok(())
}

fn build_lock(
    manifest: RuntimeCompositionManifest,
    store: &ArtifactStore,
    verify_sources: bool,
    sdk: bool,
) -> Result<RuntimeCompositionLock> {
    fetch_inputs(&manifest, store, verify_sources)?;
    let inputs = manifest
        .artifacts
        .iter()
        .map(|a| verified_artifact(store, &a.artifact))
        .collect::<Result<Vec<_>>>()?;
    let resolved = resolve_runtime_composition(
        &manifest,
        CompositionInput {
            records: inputs.iter().map(|(a, _)| a.record.clone()).collect(),
        },
    )?;
    let mut inventory = Vec::new();
    for component in &resolved.components {
        let (_, producer) = inputs
            .iter()
            .find(|(a, _)| a.record.digest == component.digest)
            .ok_or_else(|| {
                composition_error("COMPOSITION_LOCK_INVALID", "missing selected artifact")
            })?;
        for mut file in ost_artifact::manifest_files(producer)? {
            file.path = format!("components/{}/{}", component.id, file.path);
            inventory.push(CompositionInventoryEntry {
                component: component.id.clone(),
                artifact: component.digest.clone(),
                file,
            });
        }
    }
    let lock = RuntimeCompositionLock::new(
        manifest,
        inputs.into_iter().map(|(a, _)| a).collect(),
        inventory,
    )?;
    if sdk {
        lock.with_sdk()
    } else {
        Ok(lock)
    }
}

pub fn resolve_manifest(
    manifest: RuntimeCompositionManifest,
) -> Result<ost_formation::ResolvedRuntimeComposition> {
    Ok(build_lock(manifest, &ArtifactStore::discover(), true, true)?.resolved)
}

pub fn compose(
    path: &Utf8Path,
    lock_path: Option<&Utf8Path>,
    locked: bool,
    output_path: Option<&Utf8Path>,
    fmt: Format,
) -> Result<()> {
    if !locked {
        if let (Some(lock), Some(dest)) = (lock_path, output_path) {
            let lock = prospective_path(lock)?;
            let dest = prospective_path(dest)?;
            if lock.starts_with(&dest) || dest.starts_with(&lock) {
                return Err(composition_error(
                    "COMPOSITION_OUTPUT_OVERLAP",
                    "lock file and composed output must use separate paths",
                ));
            }
        }
    }
    let source = fs::read_to_string(path).map_err(|e| Error::io(path.to_string(), e))?;
    let manifest = RuntimeCompositionManifest::parse(&source)?;
    // Parse the existing lock before fetching or creating any output.
    let previous = if locked {
        Some(read_lock(
            lock_path.ok_or_else(|| Error::usage("--locked requires --lock"))?,
        )?)
    } else {
        None
    };
    if let Some(previous) = &previous {
        if previous.manifest.canonical() != manifest.canonical() {
            return Err(composition_error(
                "COMPOSITION_LOCK_MISMATCH",
                "manifest differs from the locked composition",
            ));
        }
    }
    let store = ArtifactStore::discover();
    let lock = build_lock(
        manifest,
        &store,
        !locked,
        previous.as_ref().is_none_or(|l| l.sdk.is_some()),
    )?;
    if let Some(previous) = &previous {
        require_same_lock(previous, &lock)?;
    }
    if let Some(dest) = output_path {
        materialize(&lock, &store, dest)?;
    }
    if !locked {
        if let Some(path) = lock_path {
            write_json(path, &lock)?;
        }
    }
    result(&lock, output_path, fmt)
}

fn require_same_lock(
    expected: &RuntimeCompositionLock,
    actual: &RuntimeCompositionLock,
) -> Result<()> {
    let mut expected = expected.clone();
    let mut actual = actual.clone();
    expected.manifest = expected.manifest.canonical();
    actual.manifest = actual.manifest.canonical();
    if expected != actual {
        return Err(composition_error(
            "COMPOSITION_LOCK_MISMATCH",
            "immutable artifacts, producer metadata or resolved inventory differ from the lock",
        ));
    }
    Ok(())
}

/// Resolve existing path components (including directory symlinks) without
/// creating the missing tail. Process `..` after symlinks, as the filesystem does.
fn prospective_path(path: &Utf8Path) -> Result<std::path::PathBuf> {
    use std::path::Component;
    let absolute = std::path::absolute(path).map_err(|e| Error::io(path.to_string(), e))?;
    #[cfg(windows)]
    let win32_path = !matches!(absolute.components().next(), Some(Component::Prefix(p)) if p.kind().is_verbatim());
    let mut resolved = std::path::PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            _ => {
                // Canonical ancestors use verbatim Windows paths, but missing
                // suffixes must retain the original caller's Win32 semantics.
                #[cfg(windows)]
                if win32_path && matches!(component, Component::Normal(_)) {
                    let name = component
                        .as_os_str()
                        .to_str()
                        .ok_or_else(|| Error::usage("non-UTF8 output path"))?;
                    resolved.push(name.trim_end_matches([' ', '.']));
                } else {
                    resolved.push(component);
                }
                #[cfg(not(windows))]
                resolved.push(component);
                match fs::symlink_metadata(&resolved) {
                    Ok(_) => {
                        resolved = fs::canonicalize(&resolved)
                            .map_err(|e| Error::io(path.to_string(), e))?;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(Error::io(path.to_string(), e)),
                }
            }
        }
    }
    // Nonexistent suffixes cannot be canonicalized to their on-disk spelling.
    #[cfg(windows)]
    let resolved = std::path::PathBuf::from(
        resolved
            .to_str()
            .ok_or_else(|| Error::usage("non-UTF8 output path"))?
            .to_lowercase(),
    );
    Ok(resolved)
}

/// Own only a freshly created sibling directory, and publish by rename after
/// validation. No force mode and no deletion of an existing user destination.
struct Staging(Utf8PathBuf);
impl Staging {
    fn for_destination(dest: &Utf8Path) -> Result<Self> {
        if fs::symlink_metadata(dest).is_ok() {
            return Err(composition_error(
                "COMPOSITION_OUTPUT_EXISTS",
                format!("destination '{dest}' already exists"),
            ));
        }
        let parent = dest
            .parent()
            .filter(|p| !p.as_str().is_empty())
            .unwrap_or(Utf8Path::new("."));
        fs::create_dir_all(parent).map_err(|e| Error::io(parent.to_string(), e))?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let scratch = parent.join(format!(".ost-composition-{}-{nanos}", std::process::id()));
        fs::create_dir(&scratch).map_err(|e| Error::io(scratch.to_string(), e))?;
        Ok(Self(scratch))
    }
    fn publish(self, dest: &Utf8Path) -> Result<()> {
        if fs::symlink_metadata(dest).is_ok() {
            return Err(composition_error(
                "COMPOSITION_OUTPUT_EXISTS",
                format!("destination '{dest}' already exists"),
            ));
        }
        fs::rename(&self.0, dest).map_err(|e| Error::io(dest.to_string(), e))
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn component_metadata(root: &Utf8Path, id: &str) -> Utf8PathBuf {
    root.join("metadata/components").join(id)
}

fn materialize(
    lock: &RuntimeCompositionLock,
    store: &ArtifactStore,
    dest: &Utf8Path,
) -> Result<()> {
    lock.validate()?;
    let staging = Staging::for_destination(dest)?;
    for component in &lock.resolved.components {
        store.extract(
            &component.digest,
            &staging.0.join("components").join(&component.id),
        )?;
        let (artifact, producer) = verified_artifact(store, &component.digest)?;
        let metadata = component_metadata(&staging.0, &component.id);
        write_json(&metadata.join("manifest.json"), &producer)?;
        let object = store.object_dir(artifact.record.digest_hex());
        for name in [
            artifact.record.sbom.as_deref(),
            artifact.record.provenance.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            fs::copy(object.join(name), metadata.join(name))
                .map_err(|e| Error::io(metadata.to_string(), e))?;
        }
    }
    materialize_sdk(&staging.0, lock)?;
    write_json(&staging.0.join(LOCK_PATH), lock)?;
    write_json(
        &staging.0.join("metadata/attribution.json"),
        &attribution(lock),
    )?;
    write_json(
        &staging.0.join("metadata/validation.json"),
        &validation_report(lock),
    )?;
    verify_prefix(&staging.0)?;
    staging.publish(dest)
}

fn sdk_roots(root: &Utf8Path, lock: &RuntimeCompositionLock) -> Result<()> {
    if let Some(sdk) = &lock.sdk {
        for directory in &sdk.roots {
            let path = root.join(directory);
            if let Ok(metadata) = fs::symlink_metadata(&path) {
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Err(composition_error(
                        "COMPOSITION_SDK_INVALID",
                        "SDK roots must be regular directories",
                    ));
                }
            }
            fs::create_dir_all(&path).map_err(|e| Error::io(path.to_string(), e))?;
        }
    }
    Ok(())
}

fn restore_sdk_roots(root: &Utf8Path) -> Result<()> {
    // Archives store files, not empty directories. Check paths before restoring
    // only the fixed, lock-validated roots; never repair payload or metadata.
    ost_build::stage_files(root).map_err(|e| Error::io(root.to_string(), e))?;
    regular_metadata(root, &root.join(LOCK_PATH))?;
    sdk_roots(root, &read_lock(&root.join(LOCK_PATH))?)
}

fn materialize_sdk(root: &Utf8Path, lock: &RuntimeCompositionLock) -> Result<()> {
    let Some(sdk) = &lock.sdk else { return Ok(()) };
    sdk_roots(root, lock)?;
    // Copy regular files first so Windows symlink kind can be determined from
    // the verified original. Never hardlink mutable views of the same bytes.
    for entry in sdk.files.iter().filter(|e| e.file.link_target.is_none()) {
        let dest = root.join(&entry.file.path);
        fs::create_dir_all(dest.parent().expect("SDK parent"))
            .map_err(|e| Error::io(dest.to_string(), e))?;
        fs::copy(root.join(&entry.source), &dest).map_err(|e| Error::io(dest.to_string(), e))?;
    }
    for entry in sdk.files.iter().filter(|e| e.file.link_target.is_some()) {
        let dest = root.join(&entry.file.path);
        fs::create_dir_all(dest.parent().expect("SDK parent"))
            .map_err(|e| Error::io(dest.to_string(), e))?;
        let target = entry.file.link_target.as_ref().expect("symlink");
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, &dest).map_err(|e| Error::io(dest.to_string(), e))?;
        #[cfg(windows)]
        {
            let original = root.join(&entry.source);
            let result = if original.is_dir() {
                std::os::windows::fs::symlink_dir(target, &dest)
            } else {
                std::os::windows::fs::symlink_file(target, &dest)
            };
            result.map_err(|e| Error::io(dest.to_string(), e))?;
        }
    }
    write_json(&root.join("metadata/sdk.json"), sdk)
}

fn attribution(lock: &RuntimeCompositionLock) -> Value {
    json!({"schema": "openstrata.composition-attribution/v1alpha1",
        "runtime_digest": lock.runtime_digest,
        "components": lock.resolved.components.iter().map(|c| {
            let a = lock.artifacts.iter().find(|a| a.record.digest == c.digest).expect("validated lock");
            json!({"id": c.id, "version": c.version, "digest": c.digest,
                "licenses": a.record.licenses, "source": a.record.source_identity,
                "sbom_digest": a.record.sbom_digest, "provenance_digest": a.record.provenance_digest})
        }).collect::<Vec<_>>()})
}

fn composition_dependencies(lock: &RuntimeCompositionLock) -> Vec<ResolvedDependencyIdentity> {
    let mut dependencies = lock
        .artifacts
        .iter()
        .filter(|a| {
            lock.resolved
                .components
                .iter()
                .any(|c| c.digest == a.record.digest)
        })
        .map(|a| ResolvedDependencyIdentity {
            name: a.record.name.clone(),
            version: a.record.version.clone(),
            archive_digest: Some(a.record.digest.clone()),
            source: ResolvedSourceIdentity {
                repository: "urn:openstrata:artifact".into(),
                revision: a.record.digest.clone(),
            },
        })
        .collect::<Vec<_>>();
    dependencies.sort_unstable();
    dependencies
}

fn validation_report(lock: &RuntimeCompositionLock) -> Value {
    let mut report = json!({"schema": "openstrata.composition-validation/v1alpha1",
    "runtime_digest": lock.runtime_digest,
    "scope": "locked-graph-and-materialized-inventory",
    "components": lock.resolved.components.iter().map(|c| {
        let a = lock.artifacts.iter().find(|a| a.record.digest == c.digest).expect("validated lock");
        json!({"id": c.id, "digest": c.digest, "producer_validation": a.record.validation,
            "openusd_verification": a.record.openusd_verification})
    }).collect::<Vec<_>>(),
    "checks": [
        {"name": "provider-and-compatibility-lock", "status": "passed"},
        {"name": "materialized-inventory", "status": "passed"},
        {"name": "component-evidence", "status": "passed"},
        {"name": "runtime-execution", "status": "not-run", "detail": "SDK activation and loader/plugin/render probes are not part of this validation scope"}
    ]});
    if lock.sdk.is_some() {
        report["checks"][3]["detail"] =
            "Native loader/plugin/resolver/render execution is not part of structural validation"
                .into();
        report["checks"]
            .as_array_mut()
            .expect("checks array")
            .push(json!({"name": "sdk-layout-and-activation", "status": "passed"}));
    }
    report
}

fn observed_file(
    path: &Utf8Path,
    relative: String,
    expected_executable: bool,
) -> Result<ManifestFile> {
    let metadata = fs::symlink_metadata(path).map_err(|e| Error::io(path.to_string(), e))?;
    let (sha256, size, link_target) = if metadata.file_type().is_symlink() {
        let target = fs::read_link(path).map_err(|e| Error::io(path.to_string(), e))?;
        let target = target
            .to_str()
            .ok_or_else(|| composition_error("COMPOSITION_INVENTORY_INVALID", "non-UTF8 symlink"))?
            .to_owned();
        (
            digest::sha256_hex(target.as_bytes()),
            target.len() as u64,
            Some(target),
        )
    } else {
        let mut file = fs::File::open(path).map_err(|e| Error::io(path.to_string(), e))?;
        let (digest, size) =
            digest::sha256_hex_reader(&mut file).map_err(|e| Error::io(path.to_string(), e))?;
        (digest, size, None)
    };
    // NTFS does not retain the Unix executable bit; archive verification has
    // already checked it, and exports restore it from the locked file contract.
    #[cfg(not(unix))]
    let executable = expected_executable;
    #[cfg(unix)]
    let executable = {
        use std::os::unix::fs::PermissionsExt;
        let _ = expected_executable;
        link_target.is_none() && metadata.permissions().mode() & 0o111 != 0
    };
    Ok(ManifestFile {
        path: relative,
        sha256,
        size,
        link_target,
        executable,
    })
}

fn verify_prefix(root: &Utf8Path) -> Result<RuntimeCompositionLock> {
    // Validate every symlink before opening metadata or following payload paths.
    let paths = ost_build::stage_files(root).map_err(|e| Error::io(root.to_string(), e))?;
    regular_metadata(root, &root.join(LOCK_PATH))?;
    let lock = read_lock(&root.join(LOCK_PATH))?;
    let mut expected = lock
        .inventory
        .iter()
        .map(|e| (e.file.path.clone(), e.file.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut metadata_paths = BTreeSet::from([
        LOCK_PATH.to_string(),
        "metadata/attribution.json".into(),
        "metadata/validation.json".into(),
    ]);
    if let Some(sdk) = &lock.sdk {
        for directory in &sdk.roots {
            let path = root.join(directory);
            let metadata =
                fs::symlink_metadata(&path).map_err(|e| Error::io(path.to_string(), e))?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(composition_error(
                    "COMPOSITION_SDK_INVALID",
                    "SDK roots must be regular directories",
                ));
            }
        }
        regular_metadata(root, &root.join("metadata/sdk.json"))?;
        if read_json(&root.join("metadata/sdk.json"))? != json!(sdk) {
            return Err(composition_error(
                "COMPOSITION_EVIDENCE_MISMATCH",
                "SDK ownership or activation differs from lock",
            ));
        }
        metadata_paths.insert("metadata/sdk.json".into());
        expected.extend(
            sdk.files
                .iter()
                .map(|e| (e.file.path.clone(), e.file.clone())),
        );
    }
    for component in &lock.resolved.components {
        let artifact = lock
            .artifacts
            .iter()
            .find(|a| a.record.digest == component.digest)
            .ok_or_else(|| {
                composition_error("COMPOSITION_LOCK_INVALID", "missing component record")
            })?;
        let metadata = component_metadata(root, &component.id);
        regular_metadata(root, &metadata.join("manifest.json"))?;
        let producer = read_json(&metadata.join("manifest.json"))?;
        if canonical_json_digest(&producer)? != artifact.manifest_digest {
            return Err(composition_error(
                "COMPOSITION_LOCK_MISMATCH",
                format!("producer metadata changed for {}", component.id),
            ));
        }
        let mut record =
            ArtifactRecord::from_producer_manifest(&producer, ArtifactSource::Imported, 0, "")?;
        record.sbom = artifact.record.sbom.clone();
        record.sbom_digest = artifact.record.sbom_digest.clone();
        record.sbom_size = artifact.record.sbom_size;
        record.provenance = artifact.record.provenance.clone();
        record.provenance_digest = artifact.record.provenance_digest.clone();
        record.provenance_size = artifact.record.provenance_size;
        if LockedCompositionArtifact::new(record, &producer)? != *artifact {
            return Err(composition_error(
                "COMPOSITION_LOCK_MISMATCH",
                "locked record differs from retained producer metadata",
            ));
        }
        // Inventory cannot omit, add or relabel payload entries relative to the
        // pinned producer manifest, even if someone recomputes a lock digest.
        let producer_files = ost_artifact::manifest_files(&producer)?
            .into_iter()
            .map(|mut f| {
                f.path = format!("components/{}/{}", component.id, f.path);
                f
            })
            .collect::<Vec<_>>();
        let actual = lock
            .inventory
            .iter()
            .filter(|e| e.component == component.id)
            .map(|e| e.file.clone())
            .collect::<Vec<_>>();
        let mut producer_files = producer_files;
        producer_files.sort_by(|a, b| a.path.cmp(&b.path));
        if actual != producer_files {
            return Err(composition_error(
                "COMPOSITION_INVENTORY_MISMATCH",
                "lock inventory differs from component manifest",
            ));
        }
        for name in [
            Some("manifest.json"),
            artifact.record.sbom.as_deref(),
            artifact.record.provenance.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            regular_metadata(root, &metadata.join(name))?;
            metadata_paths.insert(format!("metadata/components/{}/{name}", component.id));
        }
        verify_evidence(&metadata, &artifact.record, &producer)?;
    }
    for (name, value) in [
        ("attribution", attribution(&lock)),
        ("validation", validation_report(&lock)),
    ] {
        regular_metadata(root, &root.join(format!("metadata/{name}.json")))?;
        if read_json(&root.join(format!("metadata/{name}.json")))? != value {
            return Err(composition_error(
                "COMPOSITION_EVIDENCE_MISMATCH",
                format!("composition {name} evidence differs from the lock"),
            ));
        }
    }
    for path in paths {
        let relative = path
            .strip_prefix(root)
            .map_err(|e| Error::Operation(e.to_string()))?
            .as_str()
            .replace('\\', "/");
        if metadata_paths.remove(&relative) {
            if fs::symlink_metadata(&path)
                .map_err(|e| Error::io(path.to_string(), e))?
                .file_type()
                .is_symlink()
            {
                return Err(composition_error(
                    "COMPOSITION_INVENTORY_INVALID",
                    "composition metadata must be regular files",
                ));
            }
            continue;
        }
        let expected_file = expected.remove(&relative).ok_or_else(|| {
            composition_error(
                "COMPOSITION_INVENTORY_MISMATCH",
                format!("unexpected file '{relative}'"),
            )
        })?;
        if observed_file(&path, relative.clone(), expected_file.executable)? != expected_file {
            return Err(composition_error(
                "COMPOSITION_INVENTORY_MISMATCH",
                format!("file '{relative}' differs from the locked inventory"),
            ));
        }
    }
    if !expected.is_empty() || !metadata_paths.is_empty() {
        return Err(composition_error(
            "COMPOSITION_INVENTORY_MISMATCH",
            "locked files or evidence are missing",
        ));
    }
    Ok(lock)
}

fn regular_metadata(root: &Utf8Path, path: &Utf8Path) -> Result<()> {
    let relative = path
        .strip_prefix(root)
        .map_err(|e| Error::Operation(e.to_string()))?;
    let mut current = root.to_owned();
    let parts = relative.components().collect::<Vec<_>>();
    for (index, part) in parts.iter().enumerate() {
        if !matches!(part, camino::Utf8Component::Normal(_)) {
            return Err(composition_error(
                "COMPOSITION_INVENTORY_INVALID",
                "unsafe metadata path",
            ));
        }
        current.push(part.as_str());
        let metadata =
            fs::symlink_metadata(&current).map_err(|e| Error::io(current.to_string(), e))?;
        let expected_type = if index + 1 == parts.len() {
            metadata.is_file()
        } else {
            metadata.is_dir()
        };
        if metadata.file_type().is_symlink() || !expected_type {
            return Err(composition_error(
                "COMPOSITION_INVENTORY_INVALID",
                "composition metadata must use regular files and directories",
            ));
        }
    }
    Ok(())
}

pub fn reconstruct(
    lock_path: Option<&Utf8Path>,
    artifact: Option<&str>,
    dest: &Utf8Path,
    fmt: Format,
) -> Result<()> {
    let store = ArtifactStore::discover();
    let lock = if let Some(digest) = artifact {
        ost_formation::validate_full_digest("composed artifact", digest)?;
        let (artifact, producer) = verified_artifact(&store, digest)?;
        if artifact.record.kind != ost_artifact::ArtifactKind::ComposedRuntime {
            return Err(composition_error(
                "COMPOSITION_ARTIFACT_REQUIRED",
                "artifact is not a composed runtime",
            ));
        }
        let expected = producer
            .pointer("/composition/runtime_digest")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                composition_error(
                    "COMPOSITION_ARTIFACT_REQUIRED",
                    "artifact is not an exported composition",
                )
            })?;
        let staging = Staging::for_destination(dest)?;
        store.extract(digest, &staging.0)?;
        restore_sdk_roots(&staging.0)?;
        let lock = verify_prefix(&staging.0)?;
        if lock.runtime_digest != expected
            || artifact.record.name != lock.resolved.name
            || artifact.record.target != lock.resolved.target
            || artifact.record.dependency_identities != composition_dependencies(&lock)
            || producer["composition"]["validation"] != validation_report(&lock)
            || producer["composition"]["attribution"] != attribution(&lock)
        {
            return Err(composition_error(
                "COMPOSITION_LOCK_MISMATCH",
                "artifact and embedded runtime identities differ",
            ));
        }
        staging.publish(dest)?;
        lock
    } else {
        let lock = read_lock(
            lock_path
                .ok_or_else(|| Error::usage("reconstruct requires a lock or --from-artifact"))?,
        )?;
        let actual = build_lock(lock.manifest.clone(), &store, false, lock.sdk.is_some())?;
        require_same_lock(&lock, &actual)?;
        materialize(&lock, &store, dest)?;
        lock
    };
    result(&lock, Some(dest), fmt)
}

pub fn validate(root: &Utf8Path, sdk: bool, packages: &[String], fmt: Format) -> Result<()> {
    let lock = verify_prefix(root)?;
    if sdk {
        return self::sdk::validate(root, &lock, packages, fmt);
    }
    if fmt.is_json() {
        output::success(&validation_report(&lock));
    } else {
        println!(
            "Composition {} verified: {} (graph and inventory; execution not run)",
            lock.resolved.name, lock.runtime_digest
        );
    }
    Ok(())
}

fn result(lock: &RuntimeCompositionLock, path: Option<&Utf8Path>, fmt: Format) -> Result<()> {
    if fmt.is_json() {
        output::success(
            &json!({"schema": "openstrata.runtime-composition-result/v1alpha1",
            "runtime_digest": lock.runtime_digest, "resolved": lock.resolved, "prefix": path,
            "files": lock.inventory.len()}),
        );
    } else {
        println!(
            "Locked composition {}: {}",
            lock.resolved.name, lock.runtime_digest
        );
        if let Some(path) = path {
            println!("  prefix: {path}");
        }
        println!(
            "  {} components, {} files",
            lock.resolved.components.len(),
            lock.inventory.len()
        );
    }
    Ok(())
}

pub fn export(root: &Utf8Path, dist: Option<&str>, level: i32, fmt: Format) -> Result<()> {
    if !(1..=22).contains(&level) {
        return Err(Error::usage("compression level must be between 1 and 22"));
    }
    let lock = verify_prefix(root)?;
    let store = ArtifactStore::discover();
    let dest = dist.map(Utf8PathBuf::from).unwrap_or_else(|| {
        store
            .root()
            .join(format!("composition-export-{}", &lock.runtime_digest[7..]))
    });
    if prospective_path(&dest)?.starts_with(prospective_path(root)?) {
        return Err(composition_error(
            "COMPOSITION_OUTPUT_OVERLAP",
            "export destination must be outside the composed prefix",
        ));
    }
    let staging = Staging::for_destination(&dest)?;
    let name = format!("{}.tar.zst", lock.resolved.name);
    let files = ost_build::stage_files(root).map_err(|e| Error::io(root.to_string(), e))?;
    let packed = ost_build::pack_dir_with(
        root,
        &staging.0.join(&name),
        &files,
        ost_build::PackOptions {
            level,
            identity_digest: Some(lock.runtime_digest.clone()),
            executable_paths: lock
                .inventory
                .iter()
                .filter(|e| e.file.executable)
                .map(|e| e.file.path.clone())
                .chain(
                    lock.sdk
                        .iter()
                        .flat_map(|sdk| sdk.files.iter())
                        .filter(|e| e.file.executable)
                        .map(|e| e.file.path.clone()),
                )
                .collect(),
            ..Default::default()
        },
        &mut |_| {},
    )
    .map_err(|e| Error::io(root.to_string(), e))?;
    // Recheck before publishing in case the source tree changed during packing.
    let check = staging.0.join("check");
    ost_artifact::extract_archive(&staging.0.join(&name), &packed.archive_digest, &check)?;
    restore_sdk_roots(&check)?;
    require_same_lock(&lock, &verify_prefix(&check)?)?;
    fs::remove_dir_all(&check).map_err(|e| Error::io(check.to_string(), e))?;
    let selected = lock
        .artifacts
        .iter()
        .filter(|a| {
            lock.resolved
                .components
                .iter()
                .any(|c| c.digest == a.record.digest)
        })
        .collect::<Vec<_>>();
    let licenses = selected
        .iter()
        .flat_map(|a| a.record.licenses.iter().cloned())
        .collect::<BTreeSet<_>>();
    let mut producer = json!({
        "schema": 1, "kind": ost_artifact::COMPOSED_RUNTIME_KIND, "name": lock.resolved.name,
        "version": "1", "target": lock.resolved.target, "producer": format!("ost {}", env!("CARGO_PKG_VERSION")),
        "archive": name, "archive_digest": packed.archive_digest, "archive_size": packed.archive_size,
        "total_size": packed.total_size, "created_unix": 0,
        "files": packed.files.iter().map(|f| f.manifest_json()).collect::<Vec<_>>(),
        "licenses": licenses, "layout_profile": "composition",
        "composition": {"schema": "openstrata.composed-runtime/v1alpha1", "runtime_digest": lock.runtime_digest,
            "lock": LOCK_PATH, "validation": validation_report(&lock), "attribution": attribution(&lock)},
        // Do not turn structural composition success into runtime probe success.
        "provenance": {"validation": "pending"},
        "build": {
            "source": {"repository": "urn:openstrata:runtime-composition", "revision": lock.resolved.manifest_digest},
            "builder": {"id": "https://openstrata.dev/runtime/compose/v1", "identity": {"kind": "local-composition"}},
            "dependencies": composition_dependencies(&lock)
        }
    });
    let evidence = ost_artifact::generate_evidence(&staging.0, &mut producer)?;
    write_json(&staging.0.join("manifest.json"), &producer)?;
    let mut checksums = format!("{}  {name}\n", &packed.archive_digest[7..]);
    for evidence in evidence {
        checksums.push_str(&format!("{}  {}\n", &evidence.digest[7..], evidence.path));
    }
    fs::write(staging.0.join("SHA256SUMS"), checksums)
        .map_err(|e| Error::io(staging.0.to_string(), e))?;
    let imported = store.import(&staging.0, ArtifactSource::Imported)?;
    if dist.is_some() {
        staging.publish(&dest)?;
    }
    if fmt.is_json() {
        output::success(&json!({"digest": imported.record.digest,
        "runtime_digest": lock.runtime_digest, "target": lock.resolved.target, "dist": dist}));
    } else {
        println!(
            "Exported composition {} as {}",
            lock.runtime_digest, imported.record.digest
        );
    }
    Ok(())
}
