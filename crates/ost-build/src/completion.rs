// SPDX-License-Identifier: Apache-2.0
//! Atomic evidence that an OpenStrata-managed build completed successfully.

use std::collections::BTreeMap;

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::{LockCompiler, LockRuntime, TargetLock};

pub const BUILD_COMPLETION_FILE: &str = ".ost-build-complete.json";
pub const BUILD_COMPLETION_SCHEMA: &str = "openstrata.build-completion/v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildProjectIdentity {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildIntent {
    pub name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cache: BTreeMap<String, String>,
}

impl Default for BuildIntent {
    fn default() -> Self {
        Self {
            name: "default".into(),
            cache: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildCompletion {
    pub schema: String,
    pub target: String,
    pub project: BuildProjectIdentity,
    pub runtime: LockRuntime,
    pub compiler: LockCompiler,
    pub generator: String,
    /// Project-relative, forward-slashed build directory.
    pub build_dir: String,
    pub intent: BuildIntent,
    /// The invocation that held the target lease while this build ran, so a
    /// completion can be traced to the run that produced it — and to the entries
    /// that run wrote in the build log.
    ///
    /// Defaulted: records written before v0.18.0 held no lease and name no
    /// invocation, which is exactly what their absence should say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation: Option<String>,
    pub completed_unix: u64,
}

impl BuildCompletion {
    pub fn from_lock(
        lock: &TargetLock,
        project: BuildProjectIdentity,
        build_dir: impl Into<String>,
        intent: BuildIntent,
        completed_unix: u64,
    ) -> Self {
        Self {
            schema: BUILD_COMPLETION_SCHEMA.into(),
            target: lock.target.clone(),
            project,
            runtime: lock.runtime.clone(),
            compiler: lock.compiler.clone(),
            generator: lock.generator.clone(),
            build_dir: build_dir.into().replace('\\', "/"),
            intent,
            invocation: None,
            completed_unix,
        }
    }

    /// Name the lease-holding invocation this build ran under.
    pub fn with_invocation(mut self, invocation: impl Into<String>) -> Self {
        self.invocation = Some(invocation.into());
        self
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Fail closed when a record is stale, copied, or belongs to another
    /// configured target/project/build directory.
    pub fn validate_against(
        &self,
        lock: &TargetLock,
        project_name: &str,
        project_version: &str,
        build_dir: &Utf8Path,
    ) -> Result<(), String> {
        if self.schema != BUILD_COMPLETION_SCHEMA {
            return Err(format!("unsupported completion schema '{}'", self.schema));
        }
        if self.target != lock.target {
            return Err(format!(
                "completion target '{}' != configured target '{}'",
                self.target, lock.target
            ));
        }
        if self.project.name != project_name || self.project.version != project_version {
            return Err(format!(
                "completion project '{} {}' != current project '{} {}'",
                self.project.name, self.project.version, project_name, project_version
            ));
        }
        if self.runtime != lock.runtime {
            return Err("completion runtime does not match target.lock.json".into());
        }
        if self.compiler.fingerprint() != lock.compiler.fingerprint() {
            return Err("completion compiler does not match target.lock.json".into());
        }
        if self.generator != lock.generator {
            return Err(format!(
                "completion generator '{}' != configured generator '{}'",
                self.generator, lock.generator
            ));
        }
        let expected = build_dir.as_str().replace('\\', "/");
        if self.build_dir != expected {
            return Err(format!(
                "completion build directory '{}' != expected '{}'",
                self.build_dir, expected
            ));
        }
        if self.intent.name.trim().is_empty() {
            return Err("completion build intent is empty".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ost_core::host::{Arch, Os};
    use ost_core::variant::Abi;
    use ost_core::Variant;

    fn lock() -> TargetLock {
        TargetLock {
            lock_version: 1,
            target: "cy2026-linux-x86_64-py313-usd".into(),
            platform: "cy2026".into(),
            profile: "usd".into(),
            variant: Variant {
                os: Os::Linux,
                arch: Arch::X86_64,
                abi: Abi::Glibc {
                    version: "2.28".into(),
                },
                python: "313".into(),
            },
            runtime: LockRuntime {
                id: "runtime".into(),
                digest: "sha256:abc".into(),
            },
            python: "3.13.x".into(),
            cxx_standard: "20".into(),
            generator: "Ninja".into(),
            compiler: LockCompiler::default(),
            toolchain: ".strata/targets/x/toolchain.cmake".into(),
            created_unix: 1,
        }
    }

    #[test]
    fn completion_binds_target_project_and_directory() {
        let lock = lock();
        let completion = BuildCompletion::from_lock(
            &lock,
            BuildProjectIdentity {
                name: "demo".into(),
                version: "1.2.3".into(),
            },
            "build/cy2026-linux-x86_64-py313-usd",
            BuildIntent::default(),
            2,
        );
        assert!(completion
            .validate_against(
                &lock,
                "demo",
                "1.2.3",
                Utf8Path::new("build/cy2026-linux-x86_64-py313-usd")
            )
            .is_ok());
        assert!(completion
            .validate_against(
                &lock,
                "other",
                "1.2.3",
                Utf8Path::new("build/cy2026-linux-x86_64-py313-usd")
            )
            .is_err());
    }
}
