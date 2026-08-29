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
    CompositionInventoryEntry, ConsumerComponentIdentity, ConsumerPackageKind,
    ConsumerPackageManifest, ConsumerRuntimeIdentity, LockedCompositionArtifact,
    RuntimeCompositionLock, RuntimeCompositionManifest,
};
use ost_platform::{ResolvedDependencyIdentity, ResolvedSourceIdentity};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

const LOCK_PATH: &str = "metadata/composition.lock.json";

#[path = "runtime_sdk.rs"]
mod sdk;
pub use sdk::{environment, execute};

pub fn consumer_manifest(
    digest: &str,
    kind: ConsumerPackageKind,
    name: String,
    version: String,
    entrypoints: Vec<String>,
    output_path: &Utf8Path,
    fmt: Format,
) -> Result<()> {
    let store = ArtifactStore::discover();
    let verified =
        verified_composed_artifact(&store, digest, Staging::temporary_in(store.root())?)?;
    let lock = &verified.lock;
    require_consumer_sdk(lock)?;
    let manifest = ConsumerPackageManifest::new(
        kind,
        name,
        version,
        consumer_runtime_identity(&verified)?,
        entrypoints,
    )?;
    validate_consumer_entrypoints(
        lock,
        manifest.package.kind,
        &manifest.public_api.entrypoints,
    )?;
    let bytes = serde_json::to_string_pretty(&manifest).map_err(|error| {
        Error::Operation(format!("cannot serialize consumer manifest: {error}"))
    })?;
    if let Some(parent) = output_path
        .parent()
        .filter(|path| !path.as_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent.to_string(), error))?;
    }
    ost_core::fs::write_atomic(output_path.as_std_path(), format!("{bytes}\n").as_bytes())?;
    if fmt.is_json() {
        output::success(&json!({"manifest": manifest, "output": output_path}));
    } else {
        println!(
            "Derived {} consumer manifest for {} at {}",
            manifest.package.kind, manifest.runtime.artifact_digest, output_path
        );
    }
    Ok(())
}

/// Verify the package-private identity boundary before an adapter extracts or
/// activates native code. Acquisition remains separate (`artifact pull`), but
/// every canonical identity and retained evidence digest must still match.
pub fn consumer_verify(manifest_path: &Utf8Path, fmt: Format) -> Result<()> {
    let (manifest, _store, _verified) = verified_consumer(manifest_path)?;

    if fmt.is_json() {
        output::success(&json!({
            "schema": "openstrata.consumer-package-verification/v1alpha1",
            "package": manifest.package,
            "runtime": manifest.runtime,
            "verified": true
        }));
    } else {
        println!(
            "Verified {} consumer package {} {} against {}",
            manifest.package.kind,
            terminal_safe_label(&manifest.package.name),
            terminal_safe_label(&manifest.package.version),
            manifest.runtime.artifact_digest
        );
    }
    Ok(())
}

fn verified_consumer(
    manifest_path: &Utf8Path,
) -> Result<(
    ConsumerPackageManifest,
    ArtifactStore,
    VerifiedComposedArtifact,
)> {
    let manifest: ConsumerPackageManifest = serde_json::from_value(read_json(manifest_path)?)
        .map_err(|error| Error::parse(manifest_path.to_string(), anyhow::Error::new(error)))?;
    manifest.validate()?;

    let store = ArtifactStore::discover();
    let verified = verified_composed_artifact(
        &store,
        &manifest.runtime.artifact_digest,
        Staging::temporary_in(store.root())?,
    )?;
    require_consumer_sdk(&verified.lock)?;
    let observed = consumer_runtime_identity(&verified)?;
    if manifest.runtime != observed {
        return Err(composition_error(
            "CONSUMER_PACKAGE_RUNTIME_MISMATCH",
            format!(
                "consumer package '{}' {} does not match composed runtime {}",
                terminal_safe_label(&manifest.package.name),
                terminal_safe_label(&manifest.package.version),
                manifest.runtime.artifact_digest
            ),
        )
        .with_hint(
            "derive the consumer manifest again from the exact exported artifact and preserve it unchanged in the ecosystem package",
        ));
    }
    validate_consumer_entrypoints(
        &verified.lock,
        manifest.package.kind,
        &manifest.public_api.entrypoints,
    )?;
    Ok((manifest, store, verified))
}

const PYTHON_PRIVATE_LOADER: &str = r#"# Generated by OpenStrata. Package-private implementation detail.
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import uuid

_ROOT = Path(__file__).resolve().parent
_MANIFEST = _ROOT / "consumer-package.json"
_ARTIFACT = _ROOT / "artifact"
_ACTIVATED = False


def _ost(*args: str) -> dict:
    executable = os.environ.get("OST_EXECUTABLE") or shutil.which("ost")
    if executable is None:
        raise RuntimeError("OpenStrata 'ost' is required to activate this runtime package")
    result = subprocess.run(
        [executable, "--json", *args], text=True, capture_output=True, check=False
    )
    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(result.stderr.strip() or "ost returned invalid JSON") from error
    if result.returncode != 0 or not payload.get("ok"):
        detail = payload.get("error", {}).get("message") or result.stderr.strip()
        raise RuntimeError(detail or "OpenStrata runtime activation failed")
    return payload["data"]


def _cache_root() -> Path:
    override = os.environ.get("OST_CONSUMER_CACHE")
    if override:
        return Path(override)
    if sys.platform == "win32":
        base = Path(os.environ.get("LOCALAPPDATA", Path.home() / "AppData" / "Local"))
    elif sys.platform == "darwin":
        base = Path.home() / "Library" / "Caches"
    else:
        base = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache"))
    return base / "openstrata" / "consumers"


def activate() -> Path:
    global _ACTIVATED
    manifest = json.loads(_MANIFEST.read_text(encoding="utf-8"))
    digest = manifest["runtime"]["artifact_digest"]
    runtime_digest = manifest["runtime"]["runtime_digest"]
    prefix = _cache_root() / runtime_digest.removeprefix("sha256:")

    _ost("artifact", "import", str(_ARTIFACT / "manifest.json"))
    _ost("runtime", "consumer-verify", "--manifest", str(_MANIFEST))
    if not prefix.exists():
        prefix.parent.mkdir(parents=True, exist_ok=True)
        temporary = prefix.parent / (".openstrata-" + uuid.uuid4().hex)
        try:
            _ost("runtime", "reconstruct", "--from-artifact", digest, "--output", str(temporary))
            try:
                temporary.replace(prefix)
            except OSError:
                if prefix.exists():
                    shutil.rmtree(temporary, ignore_errors=True)
                else:
                    raise
        except BaseException:
            shutil.rmtree(temporary, ignore_errors=True)
            raise
    environment = _ost("runtime", "env", "--composition", str(prefix))["env"]
    for key, value in environment:
        os.environ[key] = value
    for value in dict(environment).get("PYTHONPATH", "").split(os.pathsep):
        if value and value not in sys.path:
            sys.path.insert(0, value)
    _ACTIVATED = True
    return prefix
"#;

const NPM_PRIVATE_LOADER: &str = r#"// Generated by OpenStrata. Package-private implementation detail.
'use strict';

const child = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const root = __dirname;
const manifestPath = path.join(root, 'consumer-package.json');
const artifactPath = path.join(root, 'artifact');

function ost(...args) {
  const executable = process.env.OST_EXECUTABLE || 'ost';
  const result = child.spawnSync(executable, ['--json', ...args], {encoding: 'utf8'});
  if (result.error) throw result.error;
  let payload;
  try { payload = JSON.parse(result.stdout); }
  catch (_) { throw new Error(result.stderr.trim() || 'ost returned invalid JSON'); }
  if (result.status !== 0 || !payload.ok) {
    throw new Error(payload.error?.message || result.stderr.trim() || 'OpenStrata runtime activation failed');
  }
  return payload.data;
}

function cacheRoot() {
  if (process.env.OST_CONSUMER_CACHE) return process.env.OST_CONSUMER_CACHE;
  if (process.platform === 'win32') return path.join(process.env.LOCALAPPDATA || path.join(os.homedir(), 'AppData', 'Local'), 'OpenStrata', 'consumers');
  if (process.platform === 'darwin') return path.join(os.homedir(), 'Library', 'Caches', 'openstrata', 'consumers');
  return path.join(process.env.XDG_CACHE_HOME || path.join(os.homedir(), '.cache'), 'openstrata', 'consumers');
}

function activate() {
  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  const digest = manifest.runtime.artifact_digest;
  const prefix = path.join(cacheRoot(), manifest.runtime.runtime_digest.replace(/^sha256:/, ''));
  ost('artifact', 'import', path.join(artifactPath, 'manifest.json'));
  ost('runtime', 'consumer-verify', '--manifest', manifestPath);
  if (!fs.existsSync(prefix)) {
    fs.mkdirSync(path.dirname(prefix), {recursive: true});
    const temporary = path.join(path.dirname(prefix), `.openstrata-${crypto.randomUUID()}`);
    try {
      ost('runtime', 'reconstruct', '--from-artifact', digest, '--output', temporary);
      try { fs.renameSync(temporary, prefix); }
      catch (error) {
        if (fs.existsSync(prefix)) fs.rmSync(temporary, {recursive: true, force: true});
        else throw error;
      }
    } catch (error) {
      fs.rmSync(temporary, {recursive: true, force: true});
      throw error;
    }
  }
  const environment = ost('runtime', 'env', '--composition', prefix).env;
  for (const [key, value] of environment) process.env[key] = value;
  return prefix;
}

module.exports = {activate};
"#;

pub fn consumer_package(
    manifest_path: &Utf8Path,
    adapter: &Utf8Path,
    output_dir: &Utf8Path,
    wheel_tag: Option<&str>,
    fmt: Format,
) -> Result<()> {
    let (manifest, store, verified) = verified_consumer(manifest_path)?;
    if manifest.package.kind == ConsumerPackageKind::NativeSdk {
        return Err(Error::usage(
            "consumer-package assembles python-wheel, npm-javascript, or npm-wasm adapters; native-sdk consumers use the verified composed SDK directly",
        ));
    }
    if !adapter.is_dir() {
        return Err(Error::usage(format!(
            "consumer adapter directory does not exist: {adapter}"
        )));
    }
    fs::create_dir_all(output_dir).map_err(|error| Error::io(output_dir.to_string(), error))?;
    let staging = Staging::temporary_in(store.root())?;
    copy_adapter(adapter, &staging.0, output_dir)?;

    let (filename, private_root) = match manifest.package.kind {
        ConsumerPackageKind::PythonWheel => {
            let distribution = python_distribution(&manifest.package.name)?;
            let version = python_version(&manifest.package.version)?;
            let tag = wheel_tag
                .map(str::to_string)
                .map(Ok)
                .unwrap_or_else(|| derived_wheel_tag(&manifest.runtime.target))?;
            validate_wheel_tag(&tag)?;
            let private = format!(
                "_openstrata_{}_{}",
                distribution.to_ascii_lowercase(),
                &manifest.runtime.artifact_digest[7..19]
            );
            reserve_generated_root(&staging.0.join(&private))?;
            write_generated(
                &staging.0.join(&private).join("__init__.py"),
                PYTHON_PRIVATE_LOADER,
            )?;
            write_generated(
                &staging.0.join(format!("{distribution}.pth")),
                &format!("import {private}; {private}.activate()\n"),
            )?;
            let dist_info = format!("{distribution}-{version}.dist-info");
            reserve_generated_root(&staging.0.join(&dist_info))?;
            write_generated(
                &staging.0.join(&dist_info).join("METADATA"),
                &format!(
                    "Metadata-Version: 2.3\nName: {}\nVersion: {}\n\nDerived from OpenStrata runtime {}.\n",
                    manifest.package.name, manifest.package.version, manifest.runtime.artifact_digest
                ),
            )?;
            write_generated(
                &staging.0.join(&dist_info).join("WHEEL"),
                &format!(
                    "Wheel-Version: 1.0\nGenerator: ost {}\nRoot-Is-Purelib: false\nTag: {tag}\n",
                    env!("CARGO_PKG_VERSION")
                ),
            )?;
            (
                format!("{distribution}-{version}-{tag}.whl"),
                Utf8PathBuf::from(private),
            )
        }
        ConsumerPackageKind::NpmJavascript | ConsumerPackageKind::NpmWasm => {
            if wheel_tag.is_some() {
                return Err(Error::usage(
                    "--wheel-tag is valid only for python-wheel packages",
                ));
            }
            validate_npm_adapter(&staging.0, &manifest)?;
            let npm_name = npm_filename(&manifest.package.name);
            let npm_version = npm_filename(&manifest.package.version);
            if npm_name.is_empty() || npm_version.is_empty() {
                return Err(Error::usage(
                    "npm package name and version must contain portable ASCII letters or digits",
                ));
            }
            let filename = format!("{npm_name}-{npm_version}.tgz");
            reserve_generated_root(&staging.0.join("_openstrata"))?;
            write_generated(
                &staging.0.join("_openstrata/loader.cjs"),
                NPM_PRIVATE_LOADER,
            )?;
            add_npm_openstrata_metadata(&staging.0.join("package.json"))?;
            (filename, Utf8PathBuf::from("_openstrata"))
        }
        ConsumerPackageKind::NativeSdk => unreachable!(),
    };

    let private = staging.0.join(&private_root);
    copy_exact(manifest_path, &private.join("consumer-package.json"))?;
    embed_artifact(&store, &verified.artifact.record, &private.join("artifact"))?;

    let final_path = output_dir.join(&filename);
    if fs::symlink_metadata(&final_path).is_ok() {
        return Err(composition_error(
            "CONSUMER_PACKAGE_OUTPUT_EXISTS",
            format!("consumer package output already exists: {final_path}"),
        ));
    }
    let partial_path = output_dir.join(format!(".{filename}.partial-{}", std::process::id()));
    if fs::symlink_metadata(&partial_path).is_ok() {
        fs::remove_file(&partial_path)
            .map_err(|error| Error::io(partial_path.to_string(), error))?;
    }
    let mut entries = archive_entries(&staging.0)?;
    let result = if manifest.package.kind == ConsumerPackageKind::PythonWheel {
        let distribution = python_distribution(&manifest.package.name)?;
        let version = python_version(&manifest.package.version)?;
        let record_path = format!("{distribution}-{version}.dist-info/RECORD");
        let record = ost_build::wheel_record(&entries, &record_path)
            .map_err(|error| Error::io(staging.0.to_string(), error))?;
        write_generated(&staging.0.join(&record_path), &record)?;
        entries = archive_entries(&staging.0)?;
        ost_build::pack_wheel(&entries, &partial_path)
    } else {
        ost_build::pack_npm_tgz(&entries, &partial_path)
    }
    .map_err(|error| Error::io(partial_path.to_string(), error))?;
    fs::rename(&partial_path, &final_path)
        .map_err(|error| Error::io(final_path.to_string(), error))?;

    if fmt.is_json() {
        output::success(&json!({
            "schema": "openstrata.consumer-package-archive/v1alpha1",
            "package": manifest.package,
            "runtime": manifest.runtime,
            "output": final_path,
            "archive_digest": result.archive_digest,
            "archive_size": result.archive_size,
            "files": result.files,
            "private_loader": manifest.private_loader,
        }));
    } else {
        println!(
            "Assembled {} {} {} at {} ({})",
            manifest.package.kind,
            terminal_safe_label(&manifest.package.name),
            terminal_safe_label(&manifest.package.version),
            final_path,
            result.archive_digest
        );
    }
    Ok(())
}

fn write_generated(path: &Utf8Path, contents: &str) -> Result<()> {
    if fs::symlink_metadata(path).is_ok() {
        return Err(composition_error(
            "CONSUMER_PACKAGE_PATH_CONFLICT",
            format!("adapter occupies generated package path '{path}'"),
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent.to_string(), error))?;
    }
    ost_core::fs::write_atomic(path.as_std_path(), contents.as_bytes())
}

fn reserve_generated_root(path: &Utf8Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok() {
        return Err(composition_error(
            "CONSUMER_PACKAGE_PATH_CONFLICT",
            format!("adapter occupies reserved generated package path '{path}'"),
        ));
    }
    Ok(())
}

fn copy_exact(source: &Utf8Path, destination: &Utf8Path) -> Result<()> {
    if fs::symlink_metadata(destination).is_ok() {
        return Err(composition_error(
            "CONSUMER_PACKAGE_PATH_CONFLICT",
            format!("adapter occupies generated package path '{destination}'"),
        ));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent.to_string(), error))?;
    }
    fs::copy(source, destination).map_err(|error| Error::io(destination.to_string(), error))?;
    Ok(())
}

fn copy_adapter(adapter: &Utf8Path, destination: &Utf8Path, output: &Utf8Path) -> Result<()> {
    let excluded = fs::canonicalize(output).ok();
    fn visit(
        root: &Utf8Path,
        current: &Utf8Path,
        destination: &Utf8Path,
        excluded: Option<&std::path::Path>,
    ) -> Result<()> {
        let mut entries = fs::read_dir(current)
            .map_err(|error| Error::io(current.to_string(), error))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| Error::io(current.to_string(), error))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let source = entry.path();
            if excluded.is_some_and(|path| source == path || source.starts_with(path)) {
                continue;
            }
            let metadata = fs::symlink_metadata(&source)
                .map_err(|error| Error::io(source.display().to_string(), error))?;
            if metadata.file_type().is_symlink() {
                return Err(composition_error(
                    "CONSUMER_PACKAGE_ADAPTER_UNSAFE",
                    format!(
                        "consumer adapter contains a symbolic link: {}",
                        source.display()
                    ),
                ));
            }
            let relative = source.strip_prefix(root.as_std_path()).map_err(|_| {
                Error::Operation("consumer adapter traversal escaped its root".into())
            })?;
            let target = destination.join(relative.to_string_lossy().replace('\\', "/"));
            if metadata.is_dir() {
                visit(
                    root,
                    Utf8Path::from_path(&source)
                        .ok_or_else(|| Error::usage("adapter path must be UTF-8"))?,
                    destination,
                    excluded,
                )?;
            } else if metadata.is_file() {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| Error::io(parent.to_string(), error))?;
                }
                fs::copy(&source, &target).map_err(|error| Error::io(target.to_string(), error))?;
            }
        }
        Ok(())
    }
    visit(adapter, adapter, destination, excluded.as_deref())
}

fn embed_artifact(
    store: &ArtifactStore,
    record: &ArtifactRecord,
    destination: &Utf8Path,
) -> Result<()> {
    fs::create_dir_all(destination).map_err(|error| Error::io(destination.to_string(), error))?;
    let object = store.object_dir(record.digest_hex());
    let mut names = vec![record.archive.as_str(), "manifest.json", "SHA256SUMS"];
    names.extend(record.sbom.as_deref());
    names.extend(record.provenance.as_deref());
    names.sort_unstable();
    names.dedup();
    for name in names {
        let source = object.join(name);
        let target = destination.join(name);
        if !source.is_file() {
            return Err(composition_error(
                "CONSUMER_PACKAGE_ARTIFACT_INCOMPLETE",
                format!("canonical artifact object is missing '{name}'"),
            ));
        }
        fs::copy(&source, &target).map_err(|error| Error::io(target.to_string(), error))?;
    }
    Ok(())
}

fn archive_entries(root: &Utf8Path) -> Result<Vec<ost_build::ConsumerArchiveEntry>> {
    fn visit(
        root: &Utf8Path,
        current: &Utf8Path,
        output: &mut Vec<ost_build::ConsumerArchiveEntry>,
    ) -> Result<()> {
        let mut entries = fs::read_dir(current)
            .map_err(|error| Error::io(current.to_string(), error))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| Error::io(current.to_string(), error))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|path| {
                Error::usage(format!("package path must be UTF-8: {}", path.display()))
            })?;
            let metadata =
                fs::symlink_metadata(&path).map_err(|error| Error::io(path.to_string(), error))?;
            if metadata.is_dir() {
                visit(root, &path, output)?;
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| Error::Operation("package traversal escaped its root".into()))?;
                output.push(
                    ost_build::ConsumerArchiveEntry::new(
                        path.clone(),
                        relative.as_str().replace('\\', "/"),
                    )
                    .map_err(|error| Error::io(path.to_string(), error))?,
                );
            } else {
                return Err(composition_error(
                    "CONSUMER_PACKAGE_ADAPTER_UNSAFE",
                    format!("consumer package contains a non-regular path: {path}"),
                ));
            }
        }
        Ok(())
    }
    let mut entries = Vec::new();
    visit(root, root, &mut entries)?;
    Ok(entries)
}

fn python_distribution(value: &str) -> Result<String> {
    if !value.is_ascii()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        || !value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        || !value
            .bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(Error::usage("python wheel name must use ASCII letters, digits, '.', '_' or '-' and start/end with a letter or digit"));
    }
    let mut normalized = String::new();
    let mut separator = false;
    for byte in value.bytes() {
        if b"-_.".contains(&byte) {
            if !separator {
                normalized.push('_');
                separator = true;
            }
        } else {
            normalized.push((byte as char).to_ascii_lowercase());
            separator = false;
        }
    }
    Ok(normalized)
}

fn python_version(value: &str) -> Result<String> {
    if value.is_empty()
        || !value.is_ascii()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".!+_".contains(&byte))
        || !value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(Error::usage(
            "python wheel version is not a portable normalized version",
        ));
    }
    Ok(value.to_ascii_lowercase())
}

fn validate_wheel_tag(tag: &str) -> Result<()> {
    let parts = tag.split('-').collect::<Vec<_>>();
    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || part.split('.').any(|segment| segment.is_empty()))
        || !tag
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err(Error::usage(
            "wheel tag must be '<python>-<abi>-<platform>' using letters, digits, '.', '_' and '-'",
        ));
    }
    Ok(())
}

fn derived_wheel_tag(target: &str) -> Result<String> {
    let lower = target.to_ascii_lowercase();
    let python = lower
        .split('-')
        .find_map(|part| {
            part.strip_prefix("py")
                .filter(|digits| digits.len() >= 2 && digits.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(|digits| format!("cp{digits}-cp{digits}"))
        .unwrap_or_else(|| "py3-none".into());
    let platform = if lower.contains("windows-x86_64") {
        "win_amd64"
    } else if lower.contains("windows-aarch64") || lower.contains("windows-arm64") {
        "win_arm64"
    } else if lower.contains("linux-x86_64") {
        "linux_x86_64"
    } else if lower.contains("linux-aarch64") || lower.contains("linux-arm64") {
        "linux_aarch64"
    } else if lower.contains("macos-aarch64") || lower.contains("macos-arm64") {
        "macosx_11_0_arm64"
    } else if lower.contains("macos-x86_64") {
        "macosx_10_15_x86_64"
    } else {
        return Err(Error::usage(format!(
            "cannot derive a wheel platform tag from runtime target '{target}'; pass --wheel-tag"
        )));
    };
    Ok(format!("{python}-{platform}"))
}

fn npm_filename(value: &str) -> String {
    value
        .trim_start_matches('@')
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn validate_npm_adapter(root: &Utf8Path, manifest: &ConsumerPackageManifest) -> Result<()> {
    let path = root.join("package.json");
    let package = read_json(&path)?;
    if package["name"].as_str() != Some(&manifest.package.name)
        || package["version"].as_str() != Some(&manifest.package.version)
    {
        return Err(composition_error(
            "CONSUMER_PACKAGE_ADAPTER_MISMATCH",
            "npm adapter package.json name/version must exactly match the consumer manifest",
        ));
    }
    let exports = package.get("exports").ok_or_else(|| {
        composition_error(
            "CONSUMER_PACKAGE_ADAPTER_MISMATCH",
            "npm adapter package.json must declare exports for every public entrypoint",
        )
    })?;
    for entrypoint in &manifest.public_api.entrypoints {
        let present = if entrypoint == "." {
            exports.is_string()
                || exports.is_array()
                || exports.as_object().is_some_and(|values| {
                    values.contains_key(".") || values.keys().all(|key| !key.starts_with('.'))
                })
        } else {
            exports
                .as_object()
                .is_some_and(|values| values.contains_key(entrypoint))
        };
        if !present {
            return Err(composition_error(
                "CONSUMER_PACKAGE_ADAPTER_MISMATCH",
                format!("npm adapter package.json does not export '{entrypoint}'"),
            ));
        }
    }
    Ok(())
}

fn add_npm_openstrata_metadata(path: &Utf8Path) -> Result<()> {
    let mut package = read_json(path)?;
    let object = package.as_object_mut().ok_or_else(|| {
        composition_error(
            "CONSUMER_PACKAGE_ADAPTER_MISMATCH",
            "package.json must be an object",
        )
    })?;
    if object.contains_key("openstrata") {
        return Err(composition_error(
            "CONSUMER_PACKAGE_PATH_CONFLICT",
            "npm adapter package.json already defines reserved 'openstrata' metadata",
        ));
    }
    object.insert(
        "openstrata".into(),
        json!({
            "consumerManifest": "./_openstrata/consumer-package.json",
            "privateLoader": "./_openstrata/loader.cjs"
        }),
    );
    write_json(path, &package)
}

fn consumer_runtime_identity(
    verified: &VerifiedComposedArtifact,
) -> Result<ConsumerRuntimeIdentity> {
    let lock = &verified.lock;
    let mut components = lock
        .resolved
        .components
        .iter()
        .map(|component| {
            let source = lock
                .artifacts
                .iter()
                .find(|artifact| artifact.record.digest == component.digest)
                .ok_or_else(|| {
                    composition_error(
                        "COMPOSITION_LOCK_INVALID",
                        format!("component '{}' has no locked artifact", component.id),
                    )
                })?;
            Ok(ConsumerComponentIdentity {
                id: component.id.clone(),
                version: component.version.clone(),
                digest: component.digest.clone(),
                sbom_digest: source.record.sbom_digest.clone(),
                provenance_digest: source.record.provenance_digest.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    components.sort();
    let evidence = |kind: &str, digest: &Option<String>| {
        digest.clone().ok_or_else(|| {
            composition_error(
                "CONSUMER_PACKAGE_EVIDENCE_REQUIRED",
                format!("composed runtime has no verified {kind} evidence"),
            )
        })
    };
    Ok(ConsumerRuntimeIdentity {
        artifact_digest: verified.artifact.record.digest.clone(),
        runtime_digest: lock.runtime_digest.clone(),
        sbom_digest: evidence("SBOM", &verified.artifact.record.sbom_digest)?,
        provenance_digest: evidence("provenance", &verified.artifact.record.provenance_digest)?,
        target: lock.resolved.target.clone(),
        components,
    })
}

fn require_consumer_sdk(lock: &RuntimeCompositionLock) -> Result<()> {
    if lock.sdk.is_none() {
        return Err(composition_error(
            "COMPOSITION_SDK_REQUIRED",
            "consumer packages require a composed runtime with an SDK; compose and export it again",
        ));
    }
    Ok(())
}

/// Package routing metadata comes from the caller's manifest. Keep it intact
/// in JSON, but never pass terminal control characters through human output or
/// the stderr mirror used by JSON failures.
fn terminal_safe_label(value: &str) -> String {
    let mut safe = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() {
            safe.extend(character.escape_default());
        } else {
            safe.push(character);
        }
    }
    safe
}

/// Native SDK entrypoints name CMake config packages already carried by the
/// canonical runtime. Verify that claim from the locked SDK inventory without
/// executing target package code on the producer host. Python and JavaScript
/// entrypoints are adapter-owned public APIs, so only their portable syntax is
/// validated by `ConsumerPackageManifest`.
fn validate_consumer_entrypoints(
    lock: &RuntimeCompositionLock,
    kind: ConsumerPackageKind,
    entrypoints: &[String],
) -> Result<()> {
    if kind != ConsumerPackageKind::NativeSdk {
        return Ok(());
    }
    let sdk = lock.sdk.as_ref().ok_or_else(|| {
        composition_error(
            "COMPOSITION_SDK_REQUIRED",
            "consumer packages require a composed runtime with an SDK; compose and export it again",
        )
    })?;
    for entrypoint in entrypoints {
        let canonical = format!("{entrypoint}Config.cmake");
        let lowercase = format!("{}-config.cmake", entrypoint.to_ascii_lowercase());
        let installed = sdk
            .files
            .iter()
            .any(|entry| cmake_config_path_matches(&entry.file.path, entrypoint));
        if !installed {
            return Err(composition_error(
                "CONSUMER_PACKAGE_ENTRYPOINT_MISSING",
                format!(
                    "native SDK entrypoint '{entrypoint}' has no installed '{canonical}' or \
                     '{lowercase}' in a CMake config-mode search location in the verified runtime \
                     SDK; install that CMake config package before composing, or select an \
                     existing --entrypoint"
                ),
            ));
        }
    }
    Ok(())
}

/// Mirror the CMake config-mode installation-prefix layouts without executing
/// package code. Package-name directory matches are case-insensitive and may
/// carry a version suffix, as specified by `find_package`; fixed layout names
/// retain their portable spelling.
fn cmake_config_path_matches(path: &str, package: &str) -> bool {
    let path = Utf8Path::new(path);
    let Some(file_name) = path.file_name() else {
        return false;
    };
    let canonical = format!("{package}Config.cmake");
    let lowercase = format!("{}-config.cmake", package.to_ascii_lowercase());
    if file_name != canonical && file_name != lowercase {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    let directories = parent
        .components()
        .filter_map(|component| match component {
            camino::Utf8Component::Normal(value) => Some(value),
            _ => None,
        })
        .collect::<Vec<_>>();
    cmake_config_directory_matches(&directories, package)
}

fn cmake_config_directory_matches(directories: &[&str], package: &str) -> bool {
    let package_directory = |value: &str| {
        value
            .to_ascii_lowercase()
            .starts_with(&package.to_ascii_lowercase())
    };
    let cmake_directory = |value: &str| matches!(value, "cmake" | "CMake");
    let library_root = |value: &str| value == "share" || value.starts_with("lib");
    let unix_tail = |tail: &[&str]| match tail {
        [name] => package_directory(name),
        [first, second] => {
            (cmake_directory(first) && package_directory(second))
                || (package_directory(first) && cmake_directory(second))
        }
        _ => false,
    };
    let after_library_root = |value: &[&str]| {
        value
            .first()
            .is_some_and(|root| library_root(root) && unix_tail(&value[1..]))
            || matches!(value, ["lib", _, tail @ ..] if unix_tail(tail))
    };

    let windows_layout = match directories {
        [] => true,
        [only] => cmake_directory(only) || package_directory(only),
        [first, second] => {
            (cmake_directory(first) && package_directory(second))
                || (package_directory(first) && cmake_directory(second))
        }
        [name, cmake, nested] => {
            package_directory(name) && cmake_directory(cmake) && package_directory(nested)
        }
        _ => false,
    };
    let apple_layout = match directories {
        [framework, "Resources"] => framework
            .strip_suffix(".framework")
            .is_some_and(|name| name.eq_ignore_ascii_case(package)),
        [framework, "Resources", "CMake"] => framework
            .strip_suffix(".framework")
            .is_some_and(|name| name.eq_ignore_ascii_case(package)),
        [framework, "Versions", _, "Resources"] => framework
            .strip_suffix(".framework")
            .is_some_and(|name| name.eq_ignore_ascii_case(package)),
        [framework, "Versions", _, "Resources", "CMake"] => framework
            .strip_suffix(".framework")
            .is_some_and(|name| name.eq_ignore_ascii_case(package)),
        [app, "Contents", "Resources"] => app
            .strip_suffix(".app")
            .is_some_and(|name| name.eq_ignore_ascii_case(package)),
        [app, "Contents", "Resources", "CMake"] => app
            .strip_suffix(".app")
            .is_some_and(|name| name.eq_ignore_ascii_case(package)),
        _ => false,
    };
    let unix_layout = after_library_root(directories)
        || directories
            .first()
            .is_some_and(|name| package_directory(name) && after_library_root(&directories[1..]));

    windows_layout || apple_layout || unix_layout
}

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
    fn temporary_in(parent: &Utf8Path) -> Result<Self> {
        fs::create_dir_all(parent).map_err(|e| Error::io(parent.to_string(), e))?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let scratch = parent.join(format!(".ost-composition-{}-{nanos}", std::process::id()));
        fs::create_dir(&scratch).map_err(|e| Error::io(scratch.to_string(), e))?;
        Ok(Self(scratch))
    }

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
        Self::temporary_in(parent)
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

struct VerifiedComposedArtifact {
    artifact: LockedCompositionArtifact,
    lock: RuntimeCompositionLock,
    staging: Staging,
}

fn verified_composed_artifact(
    store: &ArtifactStore,
    digest: &str,
    staging: Staging,
) -> Result<VerifiedComposedArtifact> {
    ost_formation::validate_full_digest("composed artifact", digest)?;
    let (artifact, producer) = verified_artifact(store, digest)?;
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
    Ok(VerifiedComposedArtifact {
        artifact,
        lock,
        staging,
    })
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
        let staging = Staging::for_destination(dest)?;
        let verified = verified_composed_artifact(&store, digest, staging)?;
        let lock = verified.lock;
        let staging = verified.staging;
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

#[cfg(test)]
mod consumer_entrypoint_tests {
    use super::{cmake_config_path_matches, terminal_safe_label};

    #[test]
    fn package_labels_cannot_emit_terminal_controls() {
        let rendered = terminal_safe_label("safe\u{1b}[2J\u{7}label");
        assert!(!rendered.chars().any(char::is_control));
        assert!(rendered.contains("safe"));
        assert!(rendered.contains("label"));
    }

    #[test]
    fn cmake_config_entrypoints_require_a_searchable_prefix_layout() {
        for path in [
            "TinyConfig.cmake",
            "cmake/TinyConfig.cmake",
            "Tiny-1.0/tiny-config.cmake",
            "Tiny/CMake/tiny-config.cmake",
            "Tiny/CMake/Tiny-1.0/TinyConfig.cmake",
            "lib/cmake/Tiny/TinyConfig.cmake",
            "lib/x86_64-linux-gnu/cmake/Tiny/TinyConfig.cmake",
            "share/Tiny-1.0/tiny-config.cmake",
            "Tiny/lib64/cmake/Tiny/TinyConfig.cmake",
            "Tiny.framework/Resources/TinyConfig.cmake",
            "Tiny.app/Contents/Resources/CMake/tiny-config.cmake",
        ] {
            assert!(
                cmake_config_path_matches(path, "Tiny"),
                "expected searchable CMake config path: {path}"
            );
        }
        for path in [
            "share/docs/TinyConfig.cmake",
            "docs/Tiny/TinyConfig.cmake",
            "lib/cmake/Other/TinyConfig.cmake",
            "share/Tiny/OtherConfig.cmake",
        ] {
            assert!(
                !cmake_config_path_matches(path, "Tiny"),
                "unexpected CMake config match: {path}"
            );
        }
    }
}
