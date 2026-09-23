// SPDX-License-Identifier: Apache-2.0
//! Verified, digest-pinned bundle and tool packages from another repository.

use std::fs::File;

use camino::{Utf8Path, Utf8PathBuf};
use ost_artifact::{
    manifest_files, pull, ArtifactKind, ArtifactStore, ComponentKind, FileTransport, OciTransport,
    PullPolicy, RemoteReference,
};
use ost_core::{digest, Category, Error, Result};
use ost_plugin::{satisfies, LibraryArtifactPin};

#[derive(Clone, Copy)]
pub(crate) enum MemberKind {
    Bundle,
    Tool,
}

impl MemberKind {
    fn label(self) -> &'static str {
        match self {
            Self::Bundle => "bundle",
            Self::Tool => "tool",
        }
    }

    fn artifact(self) -> ArtifactKind {
        match self {
            Self::Bundle => ArtifactKind::Plugin,
            Self::Tool => ArtifactKind::Package,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ExternalMember {
    pub prefix: Utf8PathBuf,
    pub digest: String,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn materialize(
    id: &str,
    version: &str,
    artifact: &LibraryArtifactPin,
    kind: MemberKind,
    target: &str,
    runtime_id: &str,
    runtime_digest: &str,
    project_root: &Utf8Path,
) -> Result<ExternalMember> {
    materialize_from_store(
        &ArtifactStore::discover(),
        id,
        version,
        artifact,
        kind,
        target,
        runtime_id,
        runtime_digest,
        project_root,
    )
}

#[allow(clippy::too_many_arguments)]
fn materialize_from_store(
    store: &ArtifactStore,
    id: &str,
    version: &str,
    artifact: &LibraryArtifactPin,
    kind: MemberKind,
    target: &str,
    runtime_id: &str,
    runtime_digest: &str,
    project_root: &Utf8Path,
) -> Result<ExternalMember> {
    let label = kind.label();
    if Utf8Path::new(id).components().count() != 1 || id == "." || id == ".." {
        return Err(Error::validation(format!(
            "invalid external {label} id '{id}'"
        )));
    }
    let pin = artifact.for_target(target).ok_or_else(|| {
        Error::coded(
            "WORKSPACE_MEMBER_ARTIFACT_TARGET_MISSING",
            Category::Validation,
            format!("{label} '{id}' has no artifact for target '{target}'"),
        )
    })?;
    if !ost_artifact::is_sha256_ref(&pin.digest) {
        return Err(Error::validation(format!(
            "{label} '{id}' has an invalid artifact digest '{}'",
            pin.digest
        )));
    }
    let source = pin
        .source
        .as_deref()
        .map(RemoteReference::parse)
        .transpose()?;
    if source.as_ref().is_some_and(|source| !source.is_pinned()) {
        return Err(Error::coded(
            "WORKSPACE_MEMBER_ARTIFACT_SOURCE_MUTABLE",
            Category::Validation,
            format!("{label} '{id}' artifact source must pin a digest"),
        ));
    }
    let record = match store.resolve(&pin.digest) {
        Ok(record) => record,
        Err(error) if error.code() == "ARTIFACT_NOT_FOUND" => {
            let source = source.as_ref().ok_or_else(|| {
                Error::coded(
                    "WORKSPACE_MEMBER_ARTIFACT_MISSING",
                    Category::Precondition,
                    format!(
                        "{label} '{id}' artifact {} is absent from the local store",
                        pin.digest
                    ),
                )
                .with_hint("pull the exact artifact digest or declare artifact.source")
            })?;
            let policy = PullPolicy {
                expected_artifact_digest: Some(pin.digest.clone()),
                require_kind: Some(kind.artifact()),
                require_target: Some(target.to_string()),
                ..PullPolicy::default()
            };
            match source {
                RemoteReference::Oci(_) => {
                    pull(&OciTransport::new(false), source, store, &policy)?.record
                }
                RemoteReference::File(_) => {
                    pull(&FileTransport::new(), source, store, &policy)?.record
                }
            }
        }
        Err(error) => return Err(error),
    };
    if !store.verify(&pin.digest)?.passed() {
        return Err(Error::coded(
            "WORKSPACE_MEMBER_ARTIFACT_INVALID",
            Category::Validation,
            format!("{label} '{id}' artifact {} failed verification", pin.digest),
        ));
    }
    if record.digest != pin.digest
        || record.kind != kind.artifact()
        || record.target != target
        || record.name != id
        || !matches!(satisfies(&record.version, version), Ok(true))
    {
        return Err(Error::coded(
            "WORKSPACE_MEMBER_ARTIFACT_IDENTITY_MISMATCH",
            Category::Validation,
            format!(
                "artifact {} is not a compatible '{id}' {label} at {version} for target '{target}'",
                pin.digest
            ),
        ));
    }
    let component = record.component.as_ref().ok_or_else(|| {
        Error::validation(format!("{label} '{id}' artifact has no component contract"))
    })?;
    let component_kind_matches = match kind {
        MemberKind::Bundle => matches!(
            component.kind,
            ComponentKind::Plugin | ComponentKind::HostAddon
        ),
        MemberKind::Tool => component.kind == ComponentKind::Tool,
    };
    if !component_kind_matches || component.id != id || component.version != record.version {
        return Err(Error::coded(
            "WORKSPACE_MEMBER_ARTIFACT_IDENTITY_MISMATCH",
            Category::Validation,
            format!("{label} '{id}' artifact has an incompatible component contract"),
        ));
    }
    if record.runtime_id.as_deref() != Some(runtime_id)
        || record.runtime_digest.as_deref() != Some(runtime_digest)
    {
        return Err(Error::coded(
            "WORKSPACE_MEMBER_ARTIFACT_RUNTIME_MISMATCH",
            Category::Validation,
            format!(
                "{label} '{id}' artifact {} requires runtime '{}@{}', selected '{}@{}'",
                pin.digest,
                record.runtime_id.as_deref().unwrap_or("<none>"),
                record.runtime_digest.as_deref().unwrap_or("<none>"),
                runtime_id,
                runtime_digest
            ),
        ));
    }
    let manifest = store.producer_manifest(&record)?;
    let files = manifest_files(&manifest)?;
    let prefix = project_root
        .join(match kind {
            MemberKind::Bundle => ".strata/external-bundles",
            MemberKind::Tool => ".strata/external-tools",
        })
        .join(target)
        .join(id)
        .join(record.digest_hex());
    if !prefix.as_std_path().exists() {
        std::fs::create_dir_all(prefix.parent().unwrap().as_std_path())
            .map_err(|error| Error::io(prefix.to_string(), error))?;
        store.extract(&pin.digest, &prefix)?;
    }
    for entry in files {
        let relative = Utf8Path::new(&entry.path);
        if !relative.is_relative()
            || relative
                .as_std_path()
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(Error::validation(format!(
                "{label} '{id}' artifact contains unsafe file path '{}'",
                entry.path
            )));
        }
        let path = prefix.join(relative);
        let (actual, size) = if let Some(link_target) = &entry.link_target {
            let metadata = std::fs::symlink_metadata(path.as_std_path())
                .map_err(|error| Error::io(path.to_string(), error))?;
            if !metadata.file_type().is_symlink() {
                return Err(Error::validation(format!(
                    "{label} '{id}' expected symlink '{}'",
                    entry.path
                )));
            }
            let observed = std::fs::read_link(path.as_std_path())
                .map_err(|error| Error::io(path.to_string(), error))?
                .to_string_lossy()
                .replace('\\', "/");
            if &observed != link_target {
                return Err(Error::validation(format!(
                    "{label} '{id}' symlink '{}' changed target",
                    entry.path
                )));
            }
            (
                digest::sha256_hex(link_target.as_bytes()),
                link_target.len() as u64,
            )
        } else {
            let mut file = File::open(path.as_std_path())
                .map_err(|error| Error::io(path.to_string(), error))?;
            digest::sha256_hex_reader(&mut file)
                .map_err(|error| Error::io(path.to_string(), error))?
        };
        if actual != entry.sha256 || size != entry.size {
            return Err(Error::coded(
                "WORKSPACE_MEMBER_ARTIFACT_INVALID",
                Category::Validation,
                format!(
                    "{label} '{id}' materialized file '{}' differs from its manifest",
                    entry.path
                ),
            ));
        }
    }
    Ok(ExternalMember {
        prefix,
        digest: record.digest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ost_artifact::ArtifactSource;

    fn fixture(root: &Utf8Path, store: &ArtifactStore, kind: MemberKind) -> LibraryArtifactPin {
        let (id, tag, component_kind, descriptor) = match kind {
            MemberKind::Bundle => (
                "execMotion",
                "openstrata.plugin-bundle",
                "plugin",
                "openstrata.plugin.yaml",
            ),
            MemberKind::Tool => (
                "motion_convert",
                "openstrata.tool",
                "tool",
                "openstrata.tool.yaml",
            ),
        };
        let stage = root.join(format!("stage-{id}"));
        let dist = root.join(format!("dist-{id}"));
        std::fs::create_dir_all(stage.as_std_path()).unwrap();
        match kind {
            MemberKind::Bundle => {
                std::fs::create_dir_all(stage.join("plugin").as_std_path()).unwrap();
                std::fs::write(stage.join("plugin/plugInfo.json").as_std_path(), "{}").unwrap();
                std::fs::write(
                    stage.join(descriptor).as_std_path(),
                    "manifest: { schema: openstrata.plugin/v1alpha1 }\nplugin: { name: execMotion, version: 0.5.0, kind: usd-exec }\nruntime: { openusd: '==26.08' }\nusd: { plug_info: plugin/plugInfo.json }\n",
                )
                .unwrap();
            }
            MemberKind::Tool => {
                std::fs::create_dir_all(stage.join("bin").as_std_path()).unwrap();
                std::fs::write(stage.join("bin/motion_convert.exe").as_std_path(), b"test")
                    .unwrap();
                std::fs::write(
                    stage.join(descriptor).as_std_path(),
                    "schema: openstrata.tool/v1alpha1\ntool: { id: motion_convert, version: 0.5.0 }\nexecutables: [motion_convert]\n",
                )
                .unwrap();
            }
        }
        let archive_name = format!("{id}-0.5.0-test.tar.zst");
        let files = ost_build::stage_files(&stage).unwrap();
        let packed = ost_build::pack_dir(&stage, &dist.join(&archive_name), &files).unwrap();
        let identity = match kind {
            MemberKind::Bundle => {
                serde_json::json!({"plugin": {"name": id, "version": "0.5.0", "kind": "usd-exec"}})
            }
            MemberKind::Tool => serde_json::json!({"tool": {"id": id, "version": "0.5.0"}}),
        };
        let mut manifest = serde_json::json!({
            "schema": 1,
            "kind": tag,
            "target": "cy2026-test",
            "archive": archive_name,
            "archive_digest": packed.archive_digest,
            "archive_size": packed.archive_size,
            "total_size": packed.total_size,
            "component": {
                "schema": ost_artifact::COMPONENT_SCHEMA,
                "id": id,
                "kind": component_kind,
                "version": "0.5.0",
                "provides": [{"capability": format!("{component_kind}:{id}"), "version": "0.5.0"}],
            },
            "provenance": {"runtime": {"id": "runtime-test", "digest": format!("sha256:{}", "ab".repeat(32))}},
            "files": packed.files.iter().map(|file| file.manifest_json()).collect::<Vec<_>>(),
        });
        manifest
            .as_object_mut()
            .unwrap()
            .extend(identity.as_object().unwrap().clone());
        std::fs::write(
            dist.join("manifest.json").as_std_path(),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let imported = store.import(&dist, ArtifactSource::Imported).unwrap();
        LibraryArtifactPin {
            digest: imported.record.digest,
            source: None,
            targets: Default::default(),
        }
    }

    #[test]
    fn published_bundle_and_tool_are_verified_and_materialized() {
        let root = Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
            "ost-external-member-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        )))
        .unwrap();
        std::fs::create_dir_all(root.as_std_path()).unwrap();
        let store = ArtifactStore::at(root.join("store"));
        let runtime_digest = format!("sha256:{}", "ab".repeat(32));
        for (kind, id) in [
            (MemberKind::Bundle, "execMotion"),
            (MemberKind::Tool, "motion_convert"),
        ] {
            let pin = fixture(&root, &store, kind);
            let member = materialize_from_store(
                &store,
                id,
                ">=0.5,<0.6",
                &pin,
                kind,
                "cy2026-test",
                "runtime-test",
                &runtime_digest,
                &root,
            )
            .unwrap();
            assert_eq!(member.digest, pin.digest);
            assert!(member
                .prefix
                .join(match kind {
                    MemberKind::Bundle => "openstrata.plugin.yaml",
                    MemberKind::Tool => "openstrata.tool.yaml",
                })
                .as_std_path()
                .is_file());
            let error = materialize_from_store(
                &store,
                id,
                ">=0.5,<0.6",
                &pin,
                kind,
                "cy2026-test",
                "other-runtime",
                &runtime_digest,
                &root,
            )
            .unwrap_err();
            assert_eq!(error.code(), "WORKSPACE_MEMBER_ARTIFACT_RUNTIME_MISMATCH");
        }
        std::fs::remove_dir_all(root.as_std_path()).ok();
    }
}
