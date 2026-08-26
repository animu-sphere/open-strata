// SPDX-License-Identifier: Apache-2.0
//! Deterministic SDK projection of a locked component inventory. Original
//! component prefixes remain intact for evidence and relative loader paths.

use std::collections::{BTreeMap, BTreeSet};

use camino::Utf8Path;
use ost_artifact::ManifestFile;
use ost_core::{digest, host::Os, Result};
use ost_runtime::{EnvOp, EnvSet, EnvVar};
use serde::{Deserialize, Serialize};

use crate::{composition_error, EnvironmentContribution, RuntimeCompositionLock};

pub const SDK_ROOTS: &[&str] = &[
    "bin", "lib", "include", "share", "plugins", "python", "node", "metadata",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SdkFile {
    pub component: String,
    pub artifact: String,
    /// Path in the retained component inventory.
    pub source: String,
    pub file: ManifestFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSdkLayout {
    pub schema: String,
    pub roots: Vec<String>,
    pub files: Vec<SdkFile>,
    /// Formation's portable environment contract, separately for each OS.
    pub environment: BTreeMap<String, Vec<EnvironmentContribution>>,
}

fn portable(path: &str) -> Result<String> {
    let path = path.replace('\\', "/");
    if path.starts_with('/') || path.contains(':') || path.contains(['\0', '\n', '\r', ';']) {
        return Err(composition_error(
            "COMPOSITION_SDK_PATH_INVALID",
            format!("unsafe SDK path '{path}'"),
        ));
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                return Err(composition_error(
                    "COMPOSITION_SDK_PATH_INVALID",
                    "SDK paths cannot contain '..'",
                ))
            }
            _ if part.ends_with(['.', ' ']) => {
                return Err(composition_error(
                    "COMPOSITION_SDK_PATH_INVALID",
                    "SDK paths cannot end in dots or spaces",
                ))
            }
            _ => parts.push(part),
        }
    }
    Ok(parts.join("/"))
}

fn mapped(path: &str, source: &str, destination: &str) -> Option<String> {
    if path == source {
        Some(destination.into())
    } else {
        path.strip_prefix(&format!("{source}/"))
            .map(|tail| format!("{destination}/{tail}"))
    }
}

fn sdk_destination(path: &str) -> Result<String> {
    let path = portable(path)?;
    let (top, tail) = path.split_once('/').unwrap_or((&path, ""));
    // Windows Python producers use Lib/, while the SDK root is lib/. Fold
    // only the standard root name so its actual spelling agrees on NTFS and
    // case-sensitive filesystems; preserve all producer-owned names below it.
    if let Some(root) = SDK_ROOTS.iter().find(|root| root.eq_ignore_ascii_case(top)) {
        Ok(if tail.is_empty() {
            root.to_string()
        } else {
            format!("{root}/{tail}")
        })
    } else {
        Ok(path)
    }
}

fn link_destination(source: &str, target: &str) -> Result<String> {
    if target.starts_with(['/', '\\']) || target.contains([':', '\\']) {
        return Err(composition_error(
            "COMPOSITION_SDK_LINK_INVALID",
            "SDK symlinks must be relative",
        ));
    }
    let mut parts = source.split('/').collect::<Vec<_>>();
    parts.pop();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(composition_error(
                        "COMPOSITION_SDK_LINK_INVALID",
                        "symlink escapes component",
                    ));
                }
            }
            _ => parts.push(part),
        }
    }
    Ok(parts.join("/"))
}

fn relative_link(path: &str, target: &str) -> String {
    let mut from = path.split('/').collect::<Vec<_>>();
    from.pop();
    let to = target.split('/').collect::<Vec<_>>();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts = vec![".."; from.len() - common];
    parts.extend_from_slice(&to[common..]);
    if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    }
}

impl RuntimeSdkLayout {
    pub fn derive(lock: &RuntimeCompositionLock) -> Result<Self> {
        let mut files = BTreeMap::<String, SdkFile>::new();
        // Native runtime producers commonly declare one mapping per file.
        // Index once instead of rescanning a many-thousand-file archive for
        // every mapping; directory expansion visits only its own descendants.
        let inventory = lock
            .inventory
            .iter()
            .map(|entry| (entry.file.path.as_str(), entry))
            .collect::<BTreeMap<_, _>>();
        for mapping in &lock.resolved.install {
            let source = portable(&mapping.source)?;
            let destination = sdk_destination(&mapping.destination)?;
            let top = destination.split('/').next().unwrap_or_default();
            if destination.is_empty()
                || ["components", "metadata"]
                    .iter()
                    .any(|p| top.eq_ignore_ascii_case(p))
            {
                return Err(composition_error(
                    "COMPOSITION_SDK_PATH_INVALID",
                    format!("reserved SDK destination '{destination}'"),
                ));
            }
            let prefix = format!("components/{}/", mapping.component);
            let full_source = format!("{prefix}{source}");
            let descendants = format!("{full_source}/");
            let entries = inventory
                .get(full_source.as_str())
                .copied()
                .into_iter()
                .chain(
                    inventory
                        .range(descendants.as_str()..)
                        .take_while(|(path, _)| path.starts_with(&descendants))
                        .map(|(_, entry)| *entry),
                );
            let mut matched = false;
            for entry in entries {
                let original = entry
                    .file
                    .path
                    .strip_prefix(&prefix)
                    .expect("validated inventory");
                let Some(destination) = mapped(original, &source, &destination) else {
                    continue;
                };
                matched = true;
                let mut file = entry.file.clone();
                file.path = portable(&destination)?;
                let key = file.path.to_ascii_lowercase();
                if files
                    .insert(
                        key,
                        SdkFile {
                            component: entry.component.clone(),
                            artifact: entry.artifact.clone(),
                            source: entry.file.path.clone(),
                            file,
                        },
                    )
                    .is_some()
                {
                    return Err(composition_error(
                        "COMPOSITION_INSTALL_PATH_COLLISION",
                        format!("expanded SDK destination '{destination}' has multiple owners"),
                    ));
                }
            }
            if !matched {
                return Err(composition_error(
                    "COMPOSITION_INSTALL_SOURCE_MISSING",
                    format!(
                        "component '{}' install source '{source}' has no inventory entries",
                        mapping.component
                    ),
                ));
            }
        }
        // A file (including a symlink) cannot also be a destination directory.
        for key in files.keys() {
            let mut parent = key.as_str();
            while let Some((p, _)) = parent.rsplit_once('/') {
                if files.contains_key(p) {
                    return Err(composition_error(
                        "COMPOSITION_INSTALL_PATH_COLLISION",
                        format!("SDK file '{p}' is also a parent directory"),
                    ));
                }
                parent = p;
            }
            if SDK_ROOTS.contains(&key.as_str()) {
                return Err(composition_error(
                    "COMPOSITION_INSTALL_PATH_COLLISION",
                    "SDK roots must be directories",
                ));
            }
        }
        let mut files = files.into_values().collect::<Vec<_>>();
        files.sort_by(|a, b| a.file.path.cmp(&b.file.path));
        let mut directories = BTreeMap::<String, String>::new();
        for entry in &files {
            let mut parent = entry.file.path.as_str();
            while let Some((directory, _)) = parent.rsplit_once('/') {
                if let Some(previous) =
                    directories.insert(directory.to_ascii_lowercase(), directory.into())
                {
                    if previous != directory {
                        return Err(composition_error(
                            "COMPOSITION_INSTALL_PATH_COLLISION",
                            format!("SDK directory casing aliases '{previous}' and '{directory}'"),
                        ));
                    }
                }
                parent = directory;
            }
        }
        let original = files.clone();
        for file in &mut files {
            if let Some(target) = &file.file.link_target {
                let source_prefix = format!("components/{}/", file.component);
                let source = file
                    .source
                    .strip_prefix(&source_prefix)
                    .expect("component source");
                let target = link_destination(source, target)?;
                let full_target = format!("{source_prefix}{target}");
                let destinations = original
                    .iter()
                    .filter_map(|f| {
                        if f.source == full_target {
                            Some(f.file.path.clone())
                        } else {
                            f.source
                                .strip_prefix(&format!("{full_target}/"))
                                .and_then(|tail| {
                                    f.file
                                        .path
                                        .strip_suffix(&format!("/{tail}"))
                                        .map(str::to_owned)
                                })
                        }
                    })
                    .collect::<BTreeSet<_>>();
                if destinations.len() != 1 {
                    return Err(composition_error(
                        "COMPOSITION_SDK_LINK_INVALID",
                        format!(
                            "symlink '{}' has an uninstalled or ambiguous target '{target}'",
                            file.source
                        ),
                    ));
                }
                let target =
                    relative_link(&file.file.path, destinations.first().expect("one target"));
                file.file.sha256 = digest::sha256_hex(target.as_bytes());
                file.file.size = target.len() as u64;
                file.file.link_target = Some(target);
            }
        }
        let mut layout = Self {
            schema: "openstrata.runtime-sdk/v1alpha1".into(),
            roots: SDK_ROOTS.iter().map(|r| r.to_string()).collect(),
            files,
            environment: BTreeMap::new(),
        };
        for os in [Os::Linux, Os::Macos, Os::Windows] {
            let contribution =
                |key: &str, source: &str, paths: Vec<String>| EnvironmentContribution {
                    key: key.into(),
                    operation: "prepend".into(),
                    source: source.into(),
                    paths,
                };
            let mut env = Vec::new();
            let mut set_paths = BTreeMap::<&str, Vec<String>>::new();
            for c in &lock.resolved.environment {
                if !c
                    .variable
                    .bytes()
                    .next()
                    .is_some_and(|b| b.is_ascii_uppercase() || b == b'_')
                {
                    return Err(composition_error(
                        "COMPOSITION_ENVIRONMENT_CONFLICT",
                        "SDK environment names must start with an uppercase letter or underscore",
                    ));
                }
                let mut paths = Vec::new();
                for value in &c.values {
                    let value = portable(value)?;
                    // Keep the producer's relative layout for plugin resource
                    // references and RPATH. The public SDK prefix is added last
                    // (highest prepend priority) below.
                    paths.push(
                        format!("components/{}/{}", c.source, value)
                            .trim_end_matches('/')
                            .into(),
                    );
                }
                if c.operation == "set" {
                    if let Some(previous) = set_paths.insert(&c.variable, paths.clone()) {
                        if previous != paths {
                            return Err(composition_error("COMPOSITION_ENVIRONMENT_CONFLICT", format!("component-relative set values for '{}' resolve to different paths", c.variable)));
                        }
                    }
                }
                env.push(EnvironmentContribution {
                    key: c.variable.clone(),
                    operation: c.operation.clone(),
                    source: c.source.clone(),
                    paths,
                });
            }
            let loader = crate::loader_key(os);
            for (key, path) in [
                ("PATH", "bin"),
                (loader, "lib"),
                ("PYTHONPATH", "python"),
                ("NODE_PATH", "node"),
                ("PXR_PLUGINPATH_NAME", "plugins"),
                ("CMAKE_PREFIX_PATH", ""),
            ] {
                if env.iter().any(|e| e.key == key && e.operation == "set") {
                    return Err(composition_error(
                        "COMPOSITION_ENVIRONMENT_CONFLICT",
                        format!("SDK activation cannot prepend to component set variable '{key}'"),
                    ));
                }
                env.push(contribution(key, "runtime-sdk", vec![path.into()]));
            }
            layout.environment.insert(os.as_str().into(), env);
        }
        Ok(layout)
    }

    /// Resolve against an empty environment using Formation's ordered portable
    /// contributions. No ambient paths enter activation or runtime identity.
    pub fn activate(&self, root: &Utf8Path, os: Os) -> Result<EnvSet> {
        let separator = if os == Os::Windows { ';' } else { ':' };
        if root.as_str().contains([';', '\0', '\n', '\r'])
            || (separator == ':' && root.as_str().contains(':'))
        {
            return Err(composition_error(
                "COMPOSITION_SDK_PATH_INVALID",
                "SDK prefix contains an environment/CMake path-list separator or control character",
            ));
        }
        let mut values = BTreeMap::<String, String>::new();
        let contributions = self.environment.get(os.as_str()).ok_or_else(|| {
            composition_error("COMPOSITION_SDK_INVALID", "missing target environment")
        })?;
        for c in contributions {
            let paths = c
                .paths
                .iter()
                .map(|p| {
                    root.join(p)
                        .as_str()
                        .trim_end_matches(['/', '\\'])
                        .replace('\\', "/")
                })
                .collect::<Vec<_>>()
                .join(&separator.to_string());
            let previous = values.get(&c.key).filter(|v| !v.is_empty());
            let value = match (c.operation.as_str(), previous) {
                ("set", _) | ("prepend" | "append", None) => paths,
                ("prepend", Some(old)) => format!("{paths}{separator}{old}"),
                ("append", Some(old)) => format!("{old}{separator}{paths}"),
                _ => {
                    return Err(composition_error(
                        "COMPOSITION_SDK_INVALID",
                        "unknown environment operation",
                    ))
                }
            };
            values.insert(c.key.clone(), value);
        }
        Ok(EnvSet {
            sep: separator,
            vars: values
                .into_iter()
                .map(|(key, value)| EnvVar {
                    key,
                    op: EnvOp::Set(value),
                })
                .collect(),
        })
    }
}
