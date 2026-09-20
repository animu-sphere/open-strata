// SPDX-License-Identifier: Apache-2.0
//! Materialize a digest-pinned library package from another repository.

use std::fs::File;

use camino::{Utf8Path, Utf8PathBuf};
use ost_artifact::{
    manifest_files, pull, ArtifactKind, ArtifactStore, ComponentKind, FileTransport, OciTransport,
    PullPolicy, RemoteReference,
};
use ost_core::digest;
use ost_core::{Category, Error, Result};
use ost_plugin::{satisfies, Library, LibraryDependency};

#[derive(Debug, Clone)]
pub(crate) struct ExternalLibrary {
    pub id: String,
    pub version: String,
    pub digest: String,
    pub prefix: Utf8PathBuf,
    pub files: Vec<Utf8PathBuf>,
    pub runtime_directories: Vec<Utf8PathBuf>,
    pub cmake_package: String,
    pub cmake_target: String,
    pub required_libraries: Vec<(String, String)>,
}

impl ExternalLibrary {
    pub fn evidence(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "version": self.version,
            "archive_digest": self.digest,
            "cmake_package": self.cmake_package,
            "cmake_target": self.cmake_target,
            "prefix": self.prefix,
            "runtime_directories": self.runtime_directories,
            "provenance": "external-artifact",
        })
    }
}

pub(crate) fn validate_closure(external: &[ExternalLibrary], local: &[Library]) -> Result<()> {
    for provider in external {
        for (required_id, required_version) in &provider.required_libraries {
            let actual = external
                .iter()
                .find(|library| library.id == *required_id)
                .map(|library| library.version.as_str())
                .or_else(|| {
                    local
                        .iter()
                        .find(|library| library.id() == required_id)
                        .map(Library::version)
                });
            if !actual
                .is_some_and(|version| matches!(satisfies(version, required_version), Ok(true)))
            {
                return Err(Error::coded(
                    "WORKSPACE_LIBRARY_ARTIFACT_CLOSURE_INCOMPLETE",
                    Category::Validation,
                    format!("library '{}' artifact requires '{}@{}' outside the selected pinned closure", provider.id, required_id, required_version),
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn materialize(
    dependency: &LibraryDependency,
    target: &str,
    runtime_id: &str,
    runtime_digest: &str,
    project_root: &Utf8Path,
) -> Result<ExternalLibrary> {
    let selection = dependency.artifact.as_ref().ok_or_else(|| {
        Error::config(format!(
            "library '{}' has no external artifact pin",
            dependency.id
        ))
    })?;
    let pin = selection.for_target(target).ok_or_else(|| {
        Error::coded(
            "WORKSPACE_LIBRARY_ARTIFACT_TARGET_MISSING",
            Category::Validation,
            format!(
                "library '{}' has no artifact for target '{target}'",
                dependency.id
            ),
        )
    })?;
    if !ost_artifact::is_sha256_ref(&pin.digest) {
        return Err(Error::config(format!(
            "library '{}' has an invalid artifact digest '{}'",
            dependency.id, pin.digest
        )));
    }
    let source_reference = pin
        .source
        .as_deref()
        .map(RemoteReference::parse)
        .transpose()?;
    if source_reference
        .as_ref()
        .is_some_and(|reference| !reference.is_pinned())
    {
        return Err(Error::coded(
            "WORKSPACE_LIBRARY_ARTIFACT_SOURCE_MUTABLE",
            Category::Validation,
            format!(
                "library '{}' artifact source must pin a digest",
                dependency.id
            ),
        ));
    }
    let store = ArtifactStore::discover();
    let record = match store.resolve(&pin.digest) {
        Ok(record) => record,
        Err(error) if error.code() == "ARTIFACT_NOT_FOUND" => {
            let reference = source_reference.as_ref().ok_or_else(|| {
                Error::coded(
                    "WORKSPACE_LIBRARY_ARTIFACT_MISSING",
                    Category::Precondition,
                    format!(
                        "library '{}' artifact {} is absent from the local store",
                        dependency.id, pin.digest
                    ),
                )
                .with_hint("pull the exact artifact digest or declare artifact.source")
            })?;
            let policy = PullPolicy {
                expected_artifact_digest: Some(pin.digest.clone()),
                require_kind: Some(ArtifactKind::Package),
                require_target: Some(target.to_string()),
                ..PullPolicy::default()
            };
            match reference {
                RemoteReference::Oci(_) => {
                    pull(&OciTransport::new(false), reference, &store, &policy)?.record
                }
                RemoteReference::File(_) => {
                    pull(&FileTransport::new(), reference, &store, &policy)?.record
                }
            }
        }
        Err(error) => return Err(error),
    };
    let report = store.verify(&pin.digest)?;
    if !report.passed() {
        return Err(Error::coded(
            "WORKSPACE_LIBRARY_ARTIFACT_INVALID",
            Category::Validation,
            format!(
                "library '{}' artifact {} failed archive/file verification",
                dependency.id, pin.digest
            ),
        ));
    }
    if record.digest != pin.digest
        || record.kind != ArtifactKind::Package
        || record.target != target
    {
        return Err(Error::coded(
            "WORKSPACE_LIBRARY_ARTIFACT_IDENTITY_MISMATCH",
            Category::Validation,
            format!("library '{}' artifact does not match its digest, package kind or target '{target}'", dependency.id),
        ));
    }
    let component = record.component.as_ref().ok_or_else(|| {
        Error::validation(format!(
            "library '{}' artifact has no component contract",
            dependency.id
        ))
    })?;
    if component.kind != ComponentKind::Library
        || component.id != dependency.id
        || record.name != dependency.id
        || component.version != record.version
        || !matches!(satisfies(&record.version, &dependency.version), Ok(true))
    {
        return Err(Error::coded(
            "WORKSPACE_LIBRARY_ARTIFACT_IDENTITY_MISMATCH",
            Category::Validation,
            format!(
                "artifact {} is not a compatible '{}' library at {}",
                pin.digest, dependency.id, dependency.version
            ),
        ));
    }
    if record.runtime_id.as_deref() != Some(runtime_id)
        || record.runtime_digest.as_deref() != Some(runtime_digest)
    {
        return Err(Error::coded(
            "WORKSPACE_LIBRARY_ARTIFACT_RUNTIME_MISMATCH",
            Category::Validation,
            format!(
                "library '{}' artifact {} was built for a different runtime",
                dependency.id, pin.digest
            ),
        ));
    }
    let required_libraries = component
        .requires
        .iter()
        .map(|requirement| {
            let id = requirement
                .capability
                .strip_prefix("library:")
                .ok_or_else(|| {
                    Error::validation(format!(
                        "library '{}' artifact requires unsupported capability '{}'",
                        dependency.id, requirement.capability
                    ))
                })?;
            let version = requirement.version.as_deref().ok_or_else(|| {
                Error::validation(format!(
                    "library '{}' artifact requires '{id}' without a version",
                    dependency.id
                ))
            })?;
            Ok((id.to_string(), version.to_string()))
        })
        .collect::<Result<Vec<_>>>()?;
    let cmake = component.cmake.as_ref().ok_or_else(|| {
        Error::validation(format!(
            "library '{}' artifact has no CMake package contract",
            dependency.id
        ))
    })?;
    let cmake_package = cmake["package"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            Error::validation(format!(
                "library '{}' artifact has no CMake package name",
                dependency.id
            ))
        })?
        .to_string();
    let cmake_target = cmake["target"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            Error::validation(format!(
                "library '{}' artifact has no exported CMake target",
                dependency.id
            ))
        })?
        .to_string();
    let manifest = store.producer_manifest(&record)?;
    let files = manifest_files(&manifest)?;
    let prefix = project_root
        .join(".strata/external-libraries")
        .join(target)
        .join(&dependency.id)
        .join(record.digest_hex());
    if !prefix.as_std_path().exists() {
        if let Some(parent) = prefix.parent() {
            std::fs::create_dir_all(parent.as_std_path())
                .map_err(|error| Error::io(parent.to_string(), error))?;
        }
        store.extract(&pin.digest, &prefix)?;
    }
    let mut installed = Vec::new();
    for entry in files {
        let relative = Utf8Path::new(&entry.path);
        if !relative.is_relative()
            || relative
                .as_std_path()
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(Error::validation(format!(
                "library '{}' artifact contains unsafe file path '{}'",
                dependency.id, entry.path
            )));
        }
        let path = prefix.join(relative);
        let (actual, size) = if let Some(link_target) = &entry.link_target {
            let metadata = std::fs::symlink_metadata(path.as_std_path())
                .map_err(|error| Error::io(path.to_string(), error))?;
            if !metadata.file_type().is_symlink() {
                return Err(Error::validation(format!(
                    "library '{}' expected symlink '{}'",
                    dependency.id, entry.path
                )));
            }
            let observed = std::fs::read_link(path.as_std_path())
                .map_err(|error| Error::io(path.to_string(), error))?;
            let observed = observed.to_string_lossy().replace('\\', "/");
            if &observed != link_target {
                return Err(Error::validation(format!(
                    "library '{}' symlink '{}' changed target",
                    dependency.id, entry.path
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
                "WORKSPACE_LIBRARY_ARTIFACT_INVALID",
                Category::Validation,
                format!(
                    "library '{}' materialized file '{}' differs from its manifest",
                    dependency.id, entry.path
                ),
            ));
        }
        installed.push(relative.to_path_buf());
    }
    let runtime_directories = ["bin", "lib"]
        .into_iter()
        .map(|path| prefix.join(path))
        .filter(|directory| {
            installed
                .iter()
                .any(|file| prefix.join(file).starts_with(directory))
        })
        .collect();
    Ok(ExternalLibrary {
        id: dependency.id.clone(),
        version: record.version,
        digest: record.digest,
        prefix,
        files: installed,
        runtime_directories,
        cmake_package,
        cmake_target,
        required_libraries,
    })
}
