// SPDX-License-Identifier: Apache-2.0
//! Atomic evidence that an OpenStrata-managed operation completed successfully.
//!
//! Two records live here, and the distinction between them is the point:
//! [`BuildCompletion`] says a target was configured, built and verified, while
//! [`TestCompletion`] says a target's tests were run. `built` and `tested` are
//! different claims, and neither implies the other.

use std::collections::{BTreeMap, BTreeSet};

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::{LockCompiler, LockRuntime, TargetLock};

pub const BUILD_COMPLETION_FILE: &str = ".ost-build-complete.json";
pub const BUILD_COMPLETION_SCHEMA: &str = "openstrata.build-completion/v1";

pub const TEST_COMPLETION_FILE: &str = ".ost-test-complete.json";
pub const TEST_COMPLETION_SCHEMA: &str = "openstrata.test-completion/v1";

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
    pub cache: BTreeMap<String, CMakeCacheEntry>,
}

/// CMake cache entry types accepted by a project-declared build intent.
///
/// Keeping the type in completion evidence prevents values such as `OFF` from
/// being reinterpreted as an ordinary string by a later invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CMakeCacheType {
    Bool,
    String,
    Path,
    Filepath,
}

impl CMakeCacheType {
    pub fn cmake_name(self) -> &'static str {
        match self {
            Self::Bool => "BOOL",
            Self::String => "STRING",
            Self::Path => "PATH",
            Self::Filepath => "FILEPATH",
        }
    }

    pub fn is_path(self) -> bool {
        matches!(self, Self::Path | Self::Filepath)
    }
}

/// Whether a path-valued build input can be reproduced away from this source
/// checkout. This is evidence, not an attempt to make local paths portable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CachePathPortability {
    Portable,
    LocalOverride,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CMakeCacheEntry {
    #[serde(rename = "type")]
    pub kind: CMakeCacheType,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portability: Option<CachePathPortability>,
}

impl CMakeCacheEntry {
    pub fn string(value: impl Into<String>) -> Self {
        Self {
            kind: CMakeCacheType::String,
            value: value.into(),
            portability: None,
        }
    }

    pub fn bool(value: bool) -> Self {
        Self {
            kind: CMakeCacheType::Bool,
            value: if value { "ON" } else { "OFF" }.into(),
            portability: None,
        }
    }

    pub fn cmake_arg(&self, name: &str) -> String {
        format!("-D{name}:{}={}", self.kind.cmake_name(), self.value)
    }
}

/// One package-relevant output published by a completed managed build.
///
/// Paths are project-relative and forward-slashed. The completion fingerprint
/// identifies how the build was configured; these entries bind that identity
/// to the actual bytes a later package operation is about to stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildOutput {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

/// Exact renderer-report bytes published by a completed managed operation.
///
/// The same session id is stamped into the report. Recording its digest here
/// prevents a copied or stale report from borrowing a newer producer id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererEvidenceBinding {
    /// Build-directory-relative, forward-slashed report path.
    pub path: String,
    pub session: String,
    pub sha256: String,
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
    /// Package-relevant files produced or finalized by this build. Optional so
    /// v0.18.0 project completions and non-package-producing builds remain valid.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<BuildOutput>,
    /// Renderer reports atomically attributed to this managed build.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub renderer_reports: Vec<RendererEvidenceBinding>,
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
            outputs: Vec::new(),
            renderer_reports: Vec::new(),
            completed_unix,
        }
    }

    /// Name the lease-holding invocation this build ran under.
    pub fn with_invocation(mut self, invocation: impl Into<String>) -> Self {
        self.invocation = Some(invocation.into());
        self
    }

    /// Bind the completed build identity to the package-relevant bytes it
    /// finalized. Callers provide a deterministic, path-sorted collection.
    pub fn with_outputs(mut self, outputs: Vec<BuildOutput>) -> Self {
        self.outputs = outputs;
        self
    }

    pub fn with_renderer_reports(mut self, reports: Vec<RendererEvidenceBinding>) -> Self {
        self.renderer_reports = reports;
        self
    }

    /// A digest identifying *this* build, for a later operation to bind itself
    /// to it.
    ///
    /// Everything that decides what was built contributes; `completed_unix` and
    /// `invocation` deliberately do not, so re-running an identical build does
    /// not invalidate test evidence that is still perfectly true of it. What it
    /// does catch is the case that matters: a rebuild against a different
    /// runtime, compiler, generator or configuration, after which an older
    /// `tested` record describes binaries that no longer exist.
    pub fn fingerprint(&self) -> String {
        let (policy, cc, cxx) = self.compiler.fingerprint();
        let material = serde_json::json!({
            "target": self.target,
            "project": self.project,
            "runtime": self.runtime,
            "compiler": [policy, cc, cxx],
            "generator": self.generator,
            "build_dir": self.build_dir,
            "intent": self.intent,
        });
        ost_core::digest::sha256_hex(material.to_string().as_bytes())
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
        let mut output_paths = BTreeSet::new();
        for output in &self.outputs {
            let path_bytes = output.path.as_bytes();
            let has_windows_drive_prefix = path_bytes.len() >= 2
                && path_bytes[0].is_ascii_alphabetic()
                && path_bytes[1] == b':';
            if output.path.is_empty()
                || output.path.starts_with('/')
                || output.path.starts_with('\\')
                || has_windows_drive_prefix
                || output.path.contains('\\')
                || output.path.split('/').any(|segment| segment == "..")
            {
                return Err(format!(
                    "completion output '{}' is not a portable project-relative path",
                    output.path
                ));
            }
            if !output_paths.insert(output.path.as_str()) {
                return Err(format!(
                    "completion records duplicate output '{}'",
                    output.path
                ));
            }
            if !is_sha256(&output.sha256) {
                return Err(format!(
                    "completion output '{}' has an invalid SHA-256 digest",
                    output.path
                ));
            }
        }
        validate_renderer_bindings(&self.renderer_reports)?;
        Ok(())
    }
}

fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

/// Atomic evidence that an OpenStrata-managed test run completed.
///
/// This is a separate claim from [`BuildCompletion`], and separate again from
/// packaging or host-side testing. v0.17.0 had no such record, so a renderer
/// assertion could read PASS from a CTest invocation that inherited none of the
/// build's runtime and later timed out — nothing tied the test run to the build
/// it supposedly exercised, or recorded that it had finished at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestCompletion {
    pub schema: String,
    pub target: String,
    pub project: BuildProjectIdentity,
    pub runtime: LockRuntime,
    pub compiler: LockCompiler,
    pub generator: String,
    /// Project-relative, forward-slashed build directory.
    pub build_dir: String,
    /// The configuration actually tested, propagated from the build rather than
    /// chosen again — testing Debug binaries against a Release build record is
    /// the mismatch this field exists to make visible.
    pub configuration: String,
    /// [`BuildCompletion::fingerprint`] of the build this run exercised. A test
    /// record whose fingerprint no longer matches the build on disk describes
    /// binaries that have since been replaced.
    pub build_fingerprint: String,
    pub totals: TestTotals,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation: Option<String>,
    /// Renderer reports atomically attributed to this managed test run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub renderer_reports: Vec<RendererEvidenceBinding>,
    pub completed_unix: u64,
}

/// What a test run observed. Recorded even when tests failed: the run itself
/// completed, and that is a different fact from every test passing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestTotals {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
}

impl TestCompletion {
    pub fn new(
        build: &BuildCompletion,
        configuration: impl Into<String>,
        totals: TestTotals,
        completed_unix: u64,
    ) -> Self {
        Self {
            schema: TEST_COMPLETION_SCHEMA.into(),
            target: build.target.clone(),
            project: build.project.clone(),
            runtime: build.runtime.clone(),
            compiler: build.compiler.clone(),
            generator: build.generator.clone(),
            build_dir: build.build_dir.clone(),
            configuration: configuration.into(),
            build_fingerprint: build.fingerprint(),
            totals,
            invocation: None,
            renderer_reports: Vec::new(),
            completed_unix,
        }
    }

    pub fn with_invocation(mut self, invocation: impl Into<String>) -> Self {
        self.invocation = Some(invocation.into());
        self
    }

    pub fn with_renderer_reports(mut self, reports: Vec<RendererEvidenceBinding>) -> Self {
        self.renderer_reports = reports;
        self
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Fail closed when the record is stale, copied, or describes a build other
    /// than the one currently on disk.
    pub fn validate_against(&self, build: &BuildCompletion) -> Result<(), String> {
        if self.schema != TEST_COMPLETION_SCHEMA {
            return Err(format!(
                "unsupported test completion schema '{}'",
                self.schema
            ));
        }
        if self.target != build.target {
            return Err(format!(
                "test completion target '{}' != built target '{}'",
                self.target, build.target
            ));
        }
        if self.build_fingerprint != build.fingerprint() {
            return Err(
                "test completion describes an earlier build of this target — re-run `ost test`"
                    .into(),
            );
        }
        if self.totals.failed > 0 {
            return Err(format!(
                "{} of {} tests failed",
                self.totals.failed, self.totals.total
            ));
        }
        // A record that ran nothing asserts nothing. `ost test` refuses to write
        // one, so reaching here means the record was hand-made or truncated —
        // either way it cannot stand behind a `tested` claim.
        if self.totals.total == 0 {
            return Err("test completion records no tests".into());
        }
        validate_renderer_bindings(&self.renderer_reports)?;
        Ok(())
    }
}

fn validate_renderer_bindings(bindings: &[RendererEvidenceBinding]) -> Result<(), String> {
    let mut paths = BTreeSet::new();
    for binding in bindings {
        let path_bytes = binding.path.as_bytes();
        let has_windows_drive_prefix =
            path_bytes.len() >= 2 && path_bytes[0].is_ascii_alphabetic() && path_bytes[1] == b':';
        if binding.path.is_empty()
            || binding.path.starts_with('/')
            || binding.path.starts_with('\\')
            || has_windows_drive_prefix
            || binding.path.contains('\\')
            || binding.path.split('/').any(|segment| segment == "..")
        {
            return Err(format!(
                "renderer evidence path '{}' is not build-directory-relative",
                binding.path
            ));
        }
        if !paths.insert(binding.path.as_str()) {
            return Err(format!(
                "renderer evidence repeats report '{}'",
                binding.path
            ));
        }
        if binding.session.trim().is_empty() {
            return Err(format!(
                "renderer evidence '{}' has an empty producer session",
                binding.path
            ));
        }
        if !is_sha256(&binding.sha256) {
            return Err(format!(
                "renderer evidence '{}' has an invalid SHA-256 digest",
                binding.path
            ));
        }
    }
    Ok(())
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

    fn build_completion() -> BuildCompletion {
        BuildCompletion::from_lock(
            &lock(),
            BuildProjectIdentity {
                name: "demo".into(),
                version: "1.2.3".into(),
            },
            "build/cy2026-linux-x86_64-py313-usd",
            BuildIntent::default(),
            2,
        )
    }

    /// Re-running the same build must not invalidate test evidence that is still
    /// true of it, so the fingerprint ignores when the build happened and who
    /// ran it.
    #[test]
    fn fingerprint_ignores_time_and_invocation() {
        let mut later = build_completion();
        later.completed_unix = 9999;
        let owned = build_completion().with_invocation("abc");
        assert_eq!(build_completion().fingerprint(), later.fingerprint());
        assert_eq!(build_completion().fingerprint(), owned.fingerprint());
    }

    /// …but a rebuild against a different runtime produces binaries an older
    /// test record cannot speak for.
    #[test]
    fn fingerprint_changes_when_the_build_identity_changes() {
        let mut other = build_completion();
        other.runtime.digest = "sha256:different".into();
        assert_ne!(build_completion().fingerprint(), other.fingerprint());

        let mut regenerated = build_completion();
        regenerated.generator = "Visual Studio 17 2022".into();
        assert_ne!(build_completion().fingerprint(), regenerated.fingerprint());
    }

    #[test]
    fn renderer_evidence_bindings_are_portable_and_digest_pinned() {
        let mut completion =
            build_completion().with_renderer_reports(vec![RendererEvidenceBinding {
                path: "renderer-report.json".into(),
                session: "ost-build-session".into(),
                sha256: format!("sha256:{}", "a".repeat(64)),
            }]);
        assert!(completion
            .validate_against(
                &lock(),
                "demo",
                "1.2.3",
                Utf8Path::new("build/cy2026-linux-x86_64-py313-usd"),
            )
            .is_ok());

        completion.renderer_reports[0].path = "../copied-report.json".into();
        assert!(completion
            .validate_against(
                &lock(),
                "demo",
                "1.2.3",
                Utf8Path::new("build/cy2026-linux-x86_64-py313-usd"),
            )
            .unwrap_err()
            .contains("build-directory-relative"));
    }

    #[test]
    fn test_completion_binds_to_the_build_it_exercised() {
        let build = build_completion();
        let tested = TestCompletion::new(
            &build,
            "Release",
            TestTotals {
                total: 3,
                passed: 3,
                failed: 0,
            },
            10,
        );
        assert!(tested.validate_against(&build).is_ok());

        // A rebuild against another runtime strands the test record.
        let mut rebuilt = build.clone();
        rebuilt.runtime.digest = "sha256:new".into();
        let error = tested
            .validate_against(&rebuilt)
            .expect_err("stale test evidence is refused");
        assert!(error.contains("earlier build"), "{error}");
    }

    /// A completed run with failing tests is still a completed run — but it
    /// cannot claim the target is tested.
    #[test]
    fn failing_tests_are_recorded_but_do_not_pass_validation() {
        let build = build_completion();
        let tested = TestCompletion::new(
            &build,
            "Release",
            TestTotals {
                total: 4,
                passed: 3,
                failed: 1,
            },
            10,
        );
        let error = tested.validate_against(&build).expect_err("failures fail");
        assert!(error.contains("1 of 4 tests failed"), "{error}");
    }

    /// A suite that ran nothing is not evidence that anything works.
    #[test]
    fn a_record_with_no_tests_cannot_claim_tested() {
        let build = build_completion();
        let empty = TestCompletion::new(&build, "Release", TestTotals::default(), 10);
        let error = empty
            .validate_against(&build)
            .expect_err("a zeroed record asserts nothing");
        assert!(error.contains("records no tests"), "{error}");
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

    #[test]
    fn package_outputs_roundtrip_without_changing_the_configuration_fingerprint() {
        let base = build_completion();
        let with_outputs = base.clone().with_outputs(vec![BuildOutput {
            path: "lib/libToy.so".into(),
            sha256: format!("sha256:{}", "ab".repeat(32)),
            size: 3,
        }]);
        assert_eq!(base.fingerprint(), with_outputs.fingerprint());
        let json = with_outputs.to_json().unwrap();
        let decoded: BuildCompletion = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.outputs, with_outputs.outputs);
        assert!(decoded
            .validate_against(
                &lock(),
                "demo",
                "1.2.3",
                Utf8Path::new("build/cy2026-linux-x86_64-py313-usd")
            )
            .is_ok());

        let mut absolute = decoded;
        absolute.outputs[0].path = "C:/plugin/lib/Toy.dll".into();
        assert!(absolute
            .validate_against(
                &lock(),
                "demo",
                "1.2.3",
                Utf8Path::new("build/cy2026-linux-x86_64-py313-usd")
            )
            .is_err());
    }
}
