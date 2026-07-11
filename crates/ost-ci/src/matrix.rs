// SPDX-License-Identifier: Apache-2.0
//! The CI support matrix (Phase 5): explicit support cells, not a Cartesian
//! product.
//!
//! A **support cell** is a claim the project is willing to stand behind: *this*
//! runtime artifact × *this* plugin artifact × *this* platform/profile,
//! verified up to *this* level, on *this* kind of host. Cells pin both sides by
//! **full registry digest** — never by mutable name — so a green cell means
//! exactly those bytes were verified together, and stays meaningful after
//! either side moves on.
//!
//! The matrix lives beside the project as `openstrata.ci.yaml` and is the
//! single source generators (GitHub Actions today, Jenkins later) read.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use ost_core::{Error, Result};

/// Filename of the support matrix at a project root.
pub const MATRIX_FILE: &str = "openstrata.ci.yaml";

/// Schema version of the matrix document. Extend additively.
pub const MATRIX_SCHEMA: u32 = 1;

/// Verification level `ost plugin test --up-to` accepts (L0..L6).
pub const MAX_LEVEL: u8 = 6;

/// The host OS a cell must run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostOs {
    #[default]
    Linux,
    Windows,
    Macos,
}

impl HostOs {
    pub fn as_str(self) -> &'static str {
        match self {
            HostOs::Linux => "linux",
            HostOs::Windows => "windows",
            HostOs::Macos => "macos",
        }
    }

    /// The GitHub-hosted runner label used when a cell declares no labels of
    /// its own. Real runtime cells usually need self-hosted runners (the local
    /// registry must hold the artifacts); this is the documented fallback.
    pub fn hosted_runner(self) -> &'static str {
        match self {
            HostOs::Linux => "ubuntu-latest",
            HostOs::Windows => "windows-latest",
            HostOs::Macos => "macos-latest",
        }
    }
}

/// Where a cell runs: an OS plus optional runner labels.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostSpec {
    #[serde(default)]
    pub os: HostOs,
    /// Runner labels, e.g. `[self-hosted, linux, x64]`. When empty, the
    /// generator falls back to the GitHub-hosted runner for `os`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

impl HostSpec {
    /// The `runs-on` value for this host: explicit labels win, else hosted.
    pub fn runs_on(&self) -> Vec<String> {
        if self.labels.is_empty() {
            vec![self.os.hosted_runner().to_string()]
        } else {
            self.labels.clone()
        }
    }
}

/// Execution lane of a cell: when its job triggers and what it may do.
/// **Source** lanes (`pull_request` / `main`) build the bundle from the
/// checked-out repo against a pinned runtime SDK; **support** lanes
/// (`scheduled` / `workflow_dispatch`) re-validate pinned runtime×plugin
/// artifact pairs from the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    PullRequest,
    Main,
    /// The pre-lane default: the scheduled support matrix.
    #[default]
    Scheduled,
    WorkflowDispatch,
}

impl Lane {
    pub fn as_str(self) -> &'static str {
        match self {
            Lane::PullRequest => "pull_request",
            Lane::Main => "main",
            Lane::Scheduled => "scheduled",
            Lane::WorkflowDispatch => "workflow_dispatch",
        }
    }

    /// Whether the cell builds from checked-out source (vs pinned artifacts).
    pub fn is_source(self) -> bool {
        matches!(self, Lane::PullRequest | Lane::Main)
    }
}

/// Publication policy of a cell's outputs. The `pull_request` lane must never
/// publish (fork-PR safety); `candidate` marks a cell whose lane may upload
/// candidate output once a publish step exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Publish {
    #[default]
    Never,
    Candidate,
}

/// What kind of infrastructure a runner profile provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerKind {
    /// Disposable GitHub-hosted image: SDK builds, static validation,
    /// packaging. May incur metered billing on private repositories.
    GithubHosted,
    /// Operator-managed runner reached by labels: real runtimes, DCC hosts,
    /// GPUs, private caches. A trust boundary, not just capacity.
    SelfHosted,
}

/// Hosted-runner billing block. Writing `acknowledgement: required` *is* the
/// project's acknowledgement that GitHub-hosted runners may incur billable
/// usage — `ost ci validate` warns while it is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Billing {
    pub acknowledgement: Acknowledgement,
}

/// The only accepted acknowledgement value (the field's presence is the
/// signal; an enum keeps typos from silently acknowledging).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Acknowledgement {
    Required,
}

/// A named capability provider cells reference instead of raw `runs-on`
/// labels, so runner policy lives in one place and a renderer is just a
/// mapping (`image` → `runs-on: <image>`, `labels` → `runs-on: […]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerProfile {
    pub kind: RunnerKind,
    /// GitHub-hosted image, e.g. `ubuntu-24.04` / `windows-2022`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Self-hosted runner labels, e.g. `[self-hosted, linux, x64, usd-26]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Informational capability tags, e.g. `os:windows`, `usd:26`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// Hosted-runner billing acknowledgement (hosted profiles only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing: Option<Billing>,
}

impl RunnerProfile {
    /// The `runs-on` value this profile resolves to.
    pub fn runs_on(&self) -> Vec<String> {
        match self.kind {
            RunnerKind::GithubHosted => vec![self.image.clone().unwrap_or_default()],
            RunnerKind::SelfHosted => self.labels.clone(),
        }
    }

    pub fn is_hosted(&self) -> bool {
        matches!(self.kind, RunnerKind::GithubHosted)
    }

    /// Whether the manifest acknowledges potential hosted-runner billing.
    pub fn billing_acknowledged(&self) -> bool {
        self.billing.is_some()
    }
}

/// Remote transport reference for a cell's runtime artifact
/// (remote-artifact-transport.md, "CI contract extension"). Tags are
/// convenience, digests are the contract: the `uri` must pin the OCI manifest
/// digest, so a hosted runner can fetch exactly the bytes this support line
/// stands behind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRemote {
    /// `oci://<registry>/<repository>@sha256:<oci-manifest-digest>`.
    pub uri: String,
    /// The OCI manifest digest the uri must resolve to. Redundant when the
    /// uri already pins it (both must agree); kept as an explicit field so
    /// the expectation survives copy-paste edits to the uri. Required by the
    /// CI matrix validator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_oci_digest: Option<String>,
}

/// How a GitHub-hosted source-CI runner obtains `ost` itself
/// (remote-artifact-transport.md, "Bootstrap policy"): a version-pinned
/// release asset with checksum verification. Self-hosted runners keep their
/// operator-provisioned `ost`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bootstrap {
    pub ost: OstBootstrap,
}

/// The pinned `ost` release the generated bootstrap step installs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OstBootstrap {
    /// Release version, e.g. `0.9.0` (the release tag is `v<version>`).
    pub version: String,
    /// GitHub repository hosting the `ost` release assets.
    #[serde(default = "default_ost_repository")]
    pub repository: String,
    /// Optional per-target-triple sha256 pins (bare 64-hex) for the release
    /// archives, e.g. `x86_64-unknown-linux-musl: <hex>`. Stronger than the
    /// release's published `.sha256` files: the CI contract pins the exact
    /// bytes instead of trusting the download origin.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sha256: BTreeMap<String, String>,
}

fn default_ost_repository() -> String {
    "animu-sphere/open-strata".to_string()
}

impl RuntimeRemote {
    /// The pinned OCI manifest digest: the uri's `@sha256:…`, repeated by
    /// `expected_oci_digest` in validated CI matrices.
    pub fn pinned_oci_digest(&self) -> Option<String> {
        match ost_artifact::RemoteReference::parse(&self.uri) {
            Ok(ost_artifact::RemoteReference::Oci(r)) => {
                r.digest.or_else(|| self.expected_oci_digest.clone())
            }
            _ => self.expected_oci_digest.clone(),
        }
    }
}

/// One explicit support line: runtime digest × plugin digest × target/profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupportCell {
    /// Unique cell name (kebab-case); becomes the job/report identifier.
    pub name: String,
    /// Execution lane (default `scheduled`, the support matrix).
    #[serde(default)]
    pub lane: Lane,
    /// Named runner profile (a `runners:` key). When set it owns the
    /// `runs-on` mapping and the legacy `host.labels` must stay empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,
    /// Full `sha256:<hex>` digest of the runtime artifact to materialize.
    pub runtime_artifact: String,
    /// Remote transport reference for the runtime artifact. Required for
    /// source cells on GitHub-hosted runners (nothing else can seed their
    /// registry); optional for self-hosted cells, which may keep air-gapped
    /// local import.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_remote: Option<RuntimeRemote>,
    /// Full `sha256:<hex>` digest of the plugin artifact to verify. Required
    /// for support lanes; absent for source lanes (they build the bundle).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_artifact: Option<String>,
    /// Repo-relative bundle path a source-lane cell builds. Default `.`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<String>,
    /// Platform calendar-year id, e.g. `cy2026`.
    pub platform: String,
    /// Profile, e.g. `usd`.
    pub profile: String,
    /// Highest verification level to run (`ost plugin test --up-to`), 0..=6.
    #[serde(default = "default_up_to")]
    pub up_to: u8,
    /// CPython `major.minor` (e.g. `3.13`) the runtime's schema tooling
    /// (`usdGenSchema`) needs, declared when the runtime artifact does **not**
    /// bundle a runnable interpreter under `bin/`. On a GitHub-hosted source
    /// cell the generator renders a first-class `setup-python` prerequisite for
    /// exactly this ABI before `ost plugin build`, so schema-generate never
    /// depends on an accidental host interpreter (v0.12.0 macOS dogfood). Left
    /// unset when the runtime ships its own interpreter or the profile needs no
    /// schema tooling. Self-hosted runners keep their operator-provisioned
    /// Python, so the step is gated on `matrix.hosted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_python: Option<String>,
    /// Publication policy (default `never`).
    #[serde(default)]
    pub publish: Publish,
    #[serde(default)]
    pub host: HostSpec,
}

fn default_up_to() -> u8 {
    5
}

/// A repo-specific extra step spliced into the **source-CI** job after the
/// verification pyramid, before packaging. This is how a project keeps its own
/// smoke coverage (e.g. a standalone corpus CTest run) in the generated
/// workflow: regenerating the workflow no longer silently drops a hand-added
/// step, because the step is declared here and re-rendered every time
/// (report ask #5). Support-lane jobs re-verify pinned artifacts and never
/// build from source, so checks do not apply to them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCheck {
    /// Step name, rendered verbatim as the `- name:` of a workflow step.
    pub name: String,
    /// Bash run script, rendered as a literal block scalar (`run: |`). Executes
    /// with the built plugin present, after `ost plugin test`.
    pub run: String,
}

fn is_kebab(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A bare `<major>.<minor>` CPython version (e.g. `3.13`) — the only form
/// `setup-python`'s `python-version` and the runtime Python-ABI contract accept
/// here (no patch, no range, no wildcard, so the rendered pin is exact).
fn is_major_minor(v: &str) -> bool {
    match v.split_once('.') {
        Some((maj, min)) => {
            !maj.is_empty()
                && !min.is_empty()
                && maj.bytes().all(|b| b.is_ascii_digit())
                && min.bytes().all(|b| b.is_ascii_digit())
        }
        None => false,
    }
}

/// The support matrix document (`openstrata.ci.yaml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupportMatrix {
    pub schema: u32,
    /// The pinned `ost` bootstrap for GitHub-hosted source CI. Required as
    /// soon as any source cell resolves to a hosted runner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap: Option<Bootstrap>,
    /// Named runner profiles cells reference via `runner:` (deterministic
    /// order for rendering).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub runners: BTreeMap<String, RunnerProfile>,
    /// Repo-specific extra steps rendered into the source-CI job(s) after the
    /// verification pyramid. Empty by default; see [`SourceCheck`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_checks: Vec<SourceCheck>,
    pub cells: Vec<SupportCell>,
}

impl SupportMatrix {
    /// Parse and validate a matrix document.
    pub fn from_yaml(src: &str) -> Result<SupportMatrix> {
        let matrix: SupportMatrix = serde_yaml::from_str(src)
            .map_err(|e| Error::InvalidManifest(format!("support matrix does not parse: {e}")))?;
        matrix.validate()?;
        Ok(matrix)
    }

    /// Structural validation. Digest *presence in a registry* is a separate,
    /// explicitly requested check (`ost ci validate --resolve`) — the matrix
    /// file itself must stay valid on a machine that has not fetched anything.
    pub fn validate(&self) -> Result<()> {
        if self.schema != MATRIX_SCHEMA {
            return Err(Error::InvalidManifest(format!(
                "unsupported support-matrix schema {} (this build reads {MATRIX_SCHEMA})",
                self.schema
            )));
        }
        if self.cells.is_empty() {
            return Err(Error::InvalidManifest(
                "the support matrix declares no cells".to_string(),
            ));
        }
        if let Some(bootstrap) = &self.bootstrap {
            validate_bootstrap(bootstrap)?;
        }
        for (i, check) in self.source_checks.iter().enumerate() {
            validate_source_check(i, check)?;
        }
        for (name, profile) in &self.runners {
            if !is_kebab(name) {
                return Err(Error::InvalidManifest(format!(
                    "runner profile name '{name}' is not kebab-case ([a-z0-9-])"
                )));
            }
            match profile.kind {
                RunnerKind::GithubHosted => {
                    if profile.image.as_deref().unwrap_or("").is_empty() {
                        return Err(Error::InvalidManifest(format!(
                            "runner '{name}': github-hosted profiles require an image \
                             (e.g. ubuntu-24.04, windows-2022)"
                        )));
                    }
                    if !profile.labels.is_empty() {
                        return Err(Error::InvalidManifest(format!(
                            "runner '{name}': github-hosted profiles use a fixed image, not labels"
                        )));
                    }
                }
                RunnerKind::SelfHosted => {
                    if profile.labels.is_empty() {
                        return Err(Error::InvalidManifest(format!(
                            "runner '{name}': self-hosted profiles require runs-on labels"
                        )));
                    }
                    if profile.image.is_some() {
                        return Err(Error::InvalidManifest(format!(
                            "runner '{name}': self-hosted profiles use labels, not an image"
                        )));
                    }
                }
            }
        }
        let mut seen: Vec<&str> = Vec::new();
        for cell in &self.cells {
            let name = cell.name.as_str();
            if !is_kebab(name) {
                return Err(Error::InvalidManifest(format!(
                    "cell name '{name}' is not kebab-case ([a-z0-9-])"
                )));
            }
            if seen.contains(&name) {
                return Err(Error::InvalidManifest(format!(
                    "duplicate cell name '{name}'"
                )));
            }
            seen.push(name);

            if let Some(runner) = &cell.runner {
                if !self.runners.contains_key(runner) {
                    return Err(Error::InvalidManifest(format!(
                        "cell '{name}': unknown runner profile '{runner}' \
                         (declare it under runners:)"
                    )));
                }
                if !cell.host.labels.is_empty() {
                    return Err(Error::InvalidManifest(format!(
                        "cell '{name}': declares both a runner profile and host.labels — \
                         the profile owns the runs-on mapping"
                    )));
                }
            }

            // CI pins exact bytes: a full digest is required, a prefix is
            // not — a prefix can silently start matching a different
            // artifact as the registry grows.
            if !ost_artifact::is_sha256_ref(&cell.runtime_artifact) {
                return Err(Error::InvalidManifest(format!(
                    "cell '{name}': runtime_artifact '{}' is not a full sha256:<64-hex> digest",
                    cell.runtime_artifact
                )));
            }
            match &cell.plugin_artifact {
                Some(value) if !ost_artifact::is_sha256_ref(value) => {
                    return Err(Error::InvalidManifest(format!(
                        "cell '{name}': plugin_artifact '{value}' is not a full \
                         sha256:<64-hex> digest"
                    )));
                }
                None if !cell.lane.is_source() => {
                    return Err(Error::InvalidManifest(format!(
                        "cell '{name}': plugin_artifact is required for the '{}' support \
                         lane (source lanes build the bundle instead)",
                        cell.lane.as_str()
                    )));
                }
                _ => {}
            }
            if let Some(bundle) = &cell.bundle {
                let escapes = bundle.is_empty()
                    || bundle.starts_with('/')
                    || bundle.starts_with('\\')
                    || bundle.contains(':')
                    || bundle.split(['/', '\\']).any(|c| c == "..");
                if escapes {
                    return Err(Error::InvalidManifest(format!(
                        "cell '{name}': bundle '{bundle}' must be a repo-relative path \
                         without '..'"
                    )));
                }
            }
            if cell.publish != Publish::Never && cell.lane == Lane::PullRequest {
                return Err(Error::InvalidManifest(format!(
                    "cell '{name}': the pull_request lane must never publish (fork-PR safety)"
                )));
            }
            if cell.platform.is_empty() || cell.profile.is_empty() {
                return Err(Error::InvalidManifest(format!(
                    "cell '{name}': platform and profile must be non-empty"
                )));
            }
            if cell.up_to > MAX_LEVEL {
                return Err(Error::InvalidManifest(format!(
                    "cell '{name}': up_to {} exceeds the highest verification level {MAX_LEVEL}",
                    cell.up_to
                )));
            }
            if let Some(py) = &cell.host_python {
                if !is_major_minor(py) {
                    return Err(Error::InvalidManifest(format!(
                        "cell '{name}': host_python '{py}' must be a CPython major.minor \
                         version like '3.13'"
                    )));
                }
                if !cell.lane.is_source() {
                    return Err(Error::InvalidManifest(format!(
                        "cell '{name}': host_python applies only to source lanes \
                         (a support lane re-verifies a pinned plugin and never builds)"
                    )));
                }
            }

            if let Some(remote) = &cell.runtime_remote {
                validate_runtime_remote(name, remote)?;
            }
            // A hosted source cell has no operator to seed its registry or
            // put `ost` on PATH: the remote reference and the bootstrap pin
            // are what make the generated workflow able to run at all
            // (remote-artifact-transport.md, Phase 2). Self-hosted source
            // cells may keep air-gapped local import.
            if cell.lane.is_source() && self.is_hosted(cell) {
                if cell.runtime_remote.is_none() {
                    return Err(Error::InvalidManifest(format!(
                        "cell '{name}': source cells on GitHub-hosted runners require a \
                         runtime_remote reference (uri: oci://…@sha256:<digest>) — \
                         nothing else can seed the runner's registry"
                    )));
                }
                if self.bootstrap.is_none() {
                    return Err(Error::InvalidManifest(format!(
                        "cell '{name}': source cells on GitHub-hosted runners require the \
                         matrix-level bootstrap block (bootstrap.ost.version) so the \
                         generated workflow can install a pinned `ost`"
                    )));
                }
            }
        }
        Ok(())
    }

    /// The `runs-on` value for a cell: its runner profile wins, else the
    /// legacy `host` spec (labels, or the hosted fallback for its OS).
    pub fn runs_on(&self, cell: &SupportCell) -> Vec<String> {
        cell.runner
            .as_deref()
            .and_then(|name| self.runners.get(name))
            .map(RunnerProfile::runs_on)
            .unwrap_or_else(|| cell.host.runs_on())
    }

    /// Whether a cell resolves to GitHub-hosted infrastructure.
    pub fn is_hosted(&self, cell: &SupportCell) -> bool {
        match cell.runner.as_deref() {
            Some(name) => self.runners.get(name).is_some_and(RunnerProfile::is_hosted),
            None => cell.host.labels.is_empty(),
        }
    }

    /// Cells that build from checked-out source (`pull_request` / `main`).
    pub fn source_cells(&self) -> Vec<&SupportCell> {
        self.cells.iter().filter(|c| c.lane.is_source()).collect()
    }

    /// Cells that re-validate pinned artifacts (`scheduled` / dispatch).
    pub fn support_cells(&self) -> Vec<&SupportCell> {
        self.cells.iter().filter(|c| !c.lane.is_source()).collect()
    }

    /// Whether any source cell declares a `host_python` ABI — the signal for
    /// the source-CI generator to render the hosted `setup-python`
    /// prerequisite. (The step itself is per-cell gated on `matrix.host_python`,
    /// so a cell that ships its own interpreter renders it as a skip.)
    pub fn needs_host_python(&self) -> bool {
        self.source_cells().iter().any(|c| c.host_python.is_some())
    }

    /// Hosted runner profiles referenced by at least one cell without a
    /// `billing.acknowledgement` block — `ost ci validate` warns on these.
    pub fn hosted_ack_missing(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for cell in &self.cells {
            let Some(name) = cell.runner.as_deref() else {
                continue;
            };
            let Some(profile) = self.runners.get(name) else {
                continue;
            };
            if profile.is_hosted()
                && !profile.billing_acknowledged()
                && !names.iter().any(|n| n == name)
            {
                names.push(name.to_string());
            }
        }
        names
    }

    /// `"<cell>: runner '<name>'"` for every publish-capable cell on a hosted
    /// profile whose billing is unacknowledged — an error, not a warning:
    /// publish-capable CI must not run on silently metered infrastructure.
    pub fn hosted_ack_errors(&self) -> Vec<String> {
        let mut hits = Vec::new();
        for cell in &self.cells {
            if cell.publish == Publish::Never {
                continue;
            }
            let Some(name) = cell.runner.as_deref() else {
                continue;
            };
            let Some(profile) = self.runners.get(name) else {
                continue;
            };
            if profile.is_hosted() && !profile.billing_acknowledged() {
                hits.push(format!("{}: runner '{name}'", cell.name));
            }
        }
        hits
    }
}

/// Validate one cell's remote runtime reference: a digest-pinned `oci://`
/// uri whose pin is repeated in, and agrees with, `expected_oci_digest`.
/// The values are later spliced into rendered workflow YAML/bash, so the
/// reference parser's charset rules double as injection hardening.
fn validate_runtime_remote(cell: &str, remote: &RuntimeRemote) -> Result<()> {
    let parsed = ost_artifact::RemoteReference::parse(&remote.uri)
        .map_err(|e| Error::InvalidManifest(format!("cell '{cell}': runtime_remote.uri: {e}")))?;
    let uri_digest = match &parsed {
        ost_artifact::RemoteReference::Oci(r) => r.digest.clone(),
        ost_artifact::RemoteReference::File(_) => {
            return Err(Error::InvalidManifest(format!(
                "cell '{cell}': runtime_remote.uri must be an oci:// reference \
                 (file:// sources are the air-gapped local-import path, not a remote pin)"
            )));
        }
    };
    // Tags are convenience, digests are the contract: the uri itself must be
    // pinned — the generated `ost artifact pull` refuses mutable references
    // (transport plan, digest-pin policy).
    let Some(uri_digest) = uri_digest else {
        return Err(Error::InvalidManifest(format!(
            "cell '{cell}': runtime_remote.uri '{}' pins no digest — resolve the tag \
             (`ost artifact resolve`) and pin the @sha256:<digest> form",
            remote.uri
        )));
    };
    let Some(expected) = &remote.expected_oci_digest else {
        return Err(Error::InvalidManifest(format!(
            "cell '{cell}': runtime_remote.expected_oci_digest is required and must repeat \
             the uri's pinned OCI digest"
        )));
    };
    if !ost_artifact::is_sha256_ref(expected) {
        return Err(Error::InvalidManifest(format!(
            "cell '{cell}': runtime_remote.expected_oci_digest '{expected}' is not a \
             full sha256:<64-hex> digest"
        )));
    }
    if &uri_digest != expected {
        return Err(Error::InvalidManifest(format!(
            "cell '{cell}': runtime_remote.uri pins {uri_digest} but \
             expected_oci_digest is {expected} — the two must agree"
        )));
    }
    Ok(())
}

/// Validate the bootstrap block. Version/repository/pins are spliced into
/// rendered workflow bash, so their charsets are deliberately narrow.
fn validate_bootstrap(bootstrap: &Bootstrap) -> Result<()> {
    let ost = &bootstrap.ost;
    let version_ok = !ost.version.is_empty()
        && ost
            .version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        && !ost.version.starts_with('v');
    if !version_ok {
        return Err(Error::InvalidManifest(format!(
            "bootstrap.ost.version '{}' must be a bare release version like 0.9.0 \
             (no leading 'v', characters [A-Za-z0-9._-])",
            ost.version
        )));
    }
    let repo_ok = ost.repository.split('/').count() == 2
        && ost.repository.split('/').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        });
    if !repo_ok {
        return Err(Error::InvalidManifest(format!(
            "bootstrap.ost.repository '{}' must be an <owner>/<name> GitHub repository",
            ost.repository
        )));
    }
    for (triple, hex) in &ost.sha256 {
        let triple_ok = !triple.is_empty()
            && triple
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b));
        if !triple_ok {
            return Err(Error::InvalidManifest(format!(
                "bootstrap.ost.sha256 key '{triple}' is not a target triple"
            )));
        }
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::InvalidManifest(format!(
                "bootstrap.ost.sha256['{triple}'] must be a bare 64-hex sha256, got '{hex}'"
            )));
        }
    }
    Ok(())
}

/// Validate one source-CI check. Both fields are spliced into generated
/// workflow YAML — the `name` into a quoted `- name:` scalar and the `run` into
/// a literal block scalar — so the charset rules double as injection hardening
/// and preserve source CI's fork-PR safety invariant (no secrets, no structural
/// breakout).
fn validate_source_check(index: usize, check: &SourceCheck) -> Result<()> {
    let at = format!("source_checks[{index}]");
    // Keep names to a readable, printable step-title subset. They render quoted
    // in generated YAML, but a narrow charset avoids surprising control or
    // expression syntax in a field humans scan in the Actions UI.
    if check.name.trim().is_empty() {
        return Err(Error::InvalidManifest(format!(
            "{at}.name must not be empty"
        )));
    }
    let name_ok = check
        .name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || " -_()./,+".contains(c));
    if !name_ok {
        return Err(Error::InvalidManifest(format!(
            "{at}.name '{}' must be a plain step title (characters [A-Za-z0-9 -_()./,+]) — \
             no ':', '#', quotes, or newlines",
            check.name
        )));
    }
    // The run script becomes a literal block scalar; every line is re-indented
    // under `run:` at render time, so a script cannot escape its step. Still
    // reject control characters other than tab/newline (a stray CR corrupts the
    // block) and forbid the GitHub Actions `secrets` context so a check cannot
    // smuggle a credential reference into a workflow the fork-PR contract
    // guarantees uses none.
    if check.run.trim().is_empty() {
        return Err(Error::InvalidManifest(format!(
            "{at}.run must not be empty"
        )));
    }
    if check
        .run
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(Error::InvalidManifest(format!(
            "{at}.run contains a control character (only newline and tab are allowed) — \
             use LF line endings"
        )));
    }
    if references_github_secrets_context(&check.run)
        || references_github_secrets_context(&check.name)
    {
        return Err(Error::InvalidManifest(format!(
            "{at} references the GitHub Actions 'secrets' context — source CI never uses secrets (fork-PR safety); \
             a smoke check must run without them"
        )));
    }
    Ok(())
}

fn references_github_secrets_context(value: &str) -> bool {
    let mut tail = value;
    while let Some(start) = tail.find("${{") {
        let after_start = &tail[start + 3..];
        if let Some(end) = after_start.find("}}") {
            if contains_standalone_ascii_word(&after_start[..end], "secrets") {
                return true;
            }
            tail = &after_start[end + 2..];
        } else {
            return contains_standalone_ascii_word(after_start, "secrets");
        }
    }
    false
}

fn contains_standalone_ascii_word(value: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let value = value.as_bytes();
    let needle = needle.as_bytes();
    if value.len() < needle.len() {
        return false;
    }
    value.windows(needle.len()).enumerate().any(|(i, window)| {
        window
            .iter()
            .zip(needle.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
            && i.checked_sub(1)
                .is_none_or(|before| !is_expression_ident_byte(value[before]))
            && value
                .get(i + needle.len())
                .is_none_or(|after| !is_expression_ident_byte(*after))
    })
}

fn is_expression_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Whether a digest is the scaffold's all-zero placeholder. Structurally a
/// valid `sha256:<64-hex>` reference, but never the digest of real bytes worth
/// standing behind — cells still carrying it are not usable support claims.
pub fn is_placeholder_digest(digest: &str) -> bool {
    digest
        .strip_prefix("sha256:")
        .is_some_and(|hex| !hex.is_empty() && hex.bytes().all(|b| b == b'0'))
}

impl SupportMatrix {
    /// `"<cell>: <field>"` for every digest still carrying the scaffold's
    /// all-zero placeholder, in document order.
    pub fn placeholder_digests(&self) -> Vec<String> {
        let mut hits = Vec::new();
        for cell in &self.cells {
            let mut digests = vec![("runtime_artifact", &cell.runtime_artifact)];
            if let Some(plugin) = &cell.plugin_artifact {
                digests.push(("plugin_artifact", plugin));
            }
            for (field, value) in digests {
                if is_placeholder_digest(value) {
                    hits.push(format!("{}: {field}", cell.name));
                }
            }
        }
        hits
    }
}

/// The commented starter matrix `ost ci init` writes.
pub fn starter_matrix() -> String {
    format!(
        "\
# OpenStrata CI support matrix.
#
# Each cell is an explicit support line: a runtime artifact x a plugin
# artifact x a platform/profile, verified up to a level (ost plugin test
# --up-to N) on a host. Both sides are pinned by FULL registry digest --
# produce them with `ost runtime export` and `ost plugin publish`, then
# paste the digests here.
#
# Cells may declare a lane (pull_request | main | scheduled |
# workflow_dispatch; default scheduled) and reference a named runner
# profile instead of raw labels. Source lanes (pull_request/main) build
# the bundle from the checked-out repo and omit plugin_artifact.
#
# Source cells on GitHub-hosted runners additionally need (1) a
# `runtime_remote` reference so the runner can pull the pinned runtime
# SDK from an OCI registry, and (2) the matrix-level `bootstrap` block
# so the generated workflow installs a pinned, checksum-verified `ost`:
#
#   bootstrap:
#     ost:
#       version: \"0.9.0\"
#       # repository: animu-sphere/open-strata   # release-asset origin
#       # sha256:                                # optional exact-byte pins
#       #   x86_64-unknown-linux-musl: <64-hex of the release archive>
#
#   runners:
#     windows-hosted:
#       kind: github-hosted
#       image: windows-2022
#       billing:
#         acknowledgement: required   # hosted runners may be metered
#     usd-linux-real:
#       kind: self-hosted
#       labels: [self-hosted, linux, x64, usd-26]
#
#   cells:
#     - name: plugin-pr-windows
#       lane: pull_request
#       runner: windows-hosted
#       runtime_artifact: sha256:<runtime SDK digest>
#       runtime_remote:
#         uri: oci://ghcr.io/<owner>/<runtime-repo>@sha256:<oci-digest>
#         expected_oci_digest: sha256:<oci-digest>
#       bundle: plugins/myPlugin
#       platform: cy2026
#       profile: usd
#       up_to: 4
#       # host_python: \"3.13\"   # see below
#
# If the pinned runtime does NOT bundle a runnable interpreter under bin/
# but its profile still needs schema tooling (usdGenSchema), declare the
# CPython major.minor the tooling expects with `host_python` on the source
# cell. On a hosted runner the generator installs exactly that Python
# (pinned setup-python) before `ost plugin build`, so schema-generate never
# depends on an accidental host interpreter. Omit it when the runtime ships
# its own interpreter; self-hosted runners keep their operator-provisioned
# Python regardless.
#
# Self-hosted cells may omit runtime_remote and keep air-gapped local
# import (`ost artifact import` on the runner); CI evidence records the
# runtime's source either way.
#
# Keep repo-specific smoke coverage in the generated source-CI workflow
# with `source_checks` -- each renders as an extra step after the
# verification pyramid (with the built plugin present), so regenerating
# the workflow never silently drops it. Source lanes only; no secrets.
#
#   source_checks:
#     - name: Run corpus CTest smoke
#       run: |
#         set -euo pipefail
#         ctest --test-dir build/corpus --output-on-failure
#
# Generate GitHub Actions workflows from this file:
#   ost ci generate github
schema: {MATRIX_SCHEMA}
cells:
  - name: example-linux-cy2026-usd
    runtime_artifact: sha256:{zeros}
    plugin_artifact: sha256:{zeros}
    platform: cy2026
    profile: usd
    up_to: 5
    host:
      os: linux
      labels: [self-hosted, linux, x64]
",
        zeros = "0".repeat(64)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_yaml() -> String {
        format!(
            "\
schema: 1
cells:
  - name: linux-usd-toy
    runtime_artifact: sha256:{a}
    plugin_artifact: sha256:{b}
    platform: cy2026
    profile: usd
    up_to: 4
    host:
      os: linux
      labels: [self-hosted, linux]
  - name: windows-usd-toy
    runtime_artifact: sha256:{a}
    plugin_artifact: sha256:{b}
    platform: cy2026
    profile: usd
    host:
      os: windows
",
            a = "ab".repeat(32),
            b = "cd".repeat(32)
        )
    }

    #[test]
    fn valid_matrix_parses_with_defaults() {
        let m = SupportMatrix::from_yaml(&valid_yaml()).unwrap();
        assert_eq!(m.cells.len(), 2);
        // up_to defaults to 5; labels default to the hosted runner.
        assert_eq!(m.cells[1].up_to, 5);
        assert_eq!(m.cells[1].host.runs_on(), vec!["windows-latest"]);
        assert!(m.is_hosted(&m.cells[1]));
        assert_eq!(
            m.cells[0].host.runs_on(),
            vec!["self-hosted".to_string(), "linux".to_string()]
        );
        assert!(!m.is_hosted(&m.cells[0]));
    }

    #[test]
    fn unknown_keys_are_rejected_at_every_mapping_level() {
        let base = valid_yaml();
        let lanes = lanes_yaml();
        let remote = remote_yaml();
        let cases = [
            (format!("{base}publsih: true\n"), "publsih"),
            (
                base.replace("    up_to: 4\n", "    up_to: 4\n    publsih: candidate\n"),
                "publsih",
            ),
            (
                base.replace(
                    "      labels: [self-hosted, linux]\n",
                    "      labels: [self-hosted, linux]\n      labls: [trusted]\n",
                ),
                "labls",
            ),
            (
                lanes.replace(
                    "        image: windows-2022\n",
                    "        image: windows-2022\n        imgae: windows-2025\n",
                ),
                "imgae",
            ),
            (
                remote.replace(
                    "      acknowledgement: required\n",
                    "      acknowledgement: required\n      acknowlegement: required\n",
                ),
                "acknowlegement",
            ),
            (
                remote.replace(
                    "bootstrap:\n",
                    "bootstrap:\n  unsigned: true\n",
                ),
                "unsigned",
            ),
            (
                remote.replace(
                    "    version: \"0.9.0\"\n",
                    "    version: \"0.9.0\"\n    checksums: {}\n",
                ),
                "checksums",
            ),
            (
                remote.replace(
                    "      expected_oci_digest:",
                    "      expected_oci_digset: ignored\n      expected_oci_digest:",
                ),
                "expected_oci_digset",
            ),
            (
                format!(
                    "{base}source_checks:\n  - name: smoke\n    run: echo ok\n    rn: echo skipped\n"
                ),
                "rn",
            ),
        ];
        for (yaml, key) in cases {
            let err = SupportMatrix::from_yaml(&yaml).expect_err(key);
            assert!(
                err.to_string().contains(key),
                "expected unknown key '{key}' in: {err}"
            );
        }
    }

    #[test]
    fn starter_matrix_is_a_valid_document() {
        // The scaffold must parse; its placeholder digests are structurally
        // valid so only the *values* need editing.
        let m = SupportMatrix::from_yaml(&starter_matrix()).unwrap();
        assert_eq!(m.cells.len(), 1);
        // ... but both placeholders are flagged so validate/generate can warn
        // or refuse instead of quietly treating the scaffold as usable.
        assert_eq!(
            m.placeholder_digests(),
            vec![
                "example-linux-cy2026-usd: runtime_artifact",
                "example-linux-cy2026-usd: plugin_artifact"
            ]
        );
    }

    #[test]
    fn placeholder_detection_matches_only_all_zero_digests() {
        assert!(is_placeholder_digest(&format!("sha256:{}", "0".repeat(64))));
        assert!(!is_placeholder_digest(&format!(
            "sha256:{}",
            "ab".repeat(32)
        )));
        // A real digest that merely starts with zeros is not a placeholder.
        assert!(!is_placeholder_digest(&format!(
            "sha256:0{}",
            "ab".repeat(31)
        )));
        assert!(!is_placeholder_digest("sha256:"));
        assert!(!is_placeholder_digest("0000"));
        let m = SupportMatrix::from_yaml(&valid_yaml()).unwrap();
        assert!(m.placeholder_digests().is_empty());
    }

    #[test]
    fn source_checks_parse_default_empty_and_accept_a_smoke_step() {
        // Absent by default.
        let m = SupportMatrix::from_yaml(&valid_yaml()).unwrap();
        assert!(m.source_checks.is_empty());

        // A repo can declare a post-build smoke step (report ask #5).
        let with = format!(
            "{}source_checks:\n  - name: Run corpus CTest smoke\n    run: |\n      ctest --test-dir build/corpus --output-on-failure\n",
            valid_yaml()
        );
        let m = SupportMatrix::from_yaml(&with).unwrap();
        assert_eq!(m.source_checks.len(), 1);
        assert_eq!(m.source_checks[0].name, "Run corpus CTest smoke");
        assert!(m.source_checks[0].run.contains("ctest --test-dir"));
    }

    #[test]
    fn source_checks_reject_injection_and_secrets() {
        let base = valid_yaml();
        // A name that could break the `- name:` YAML line.
        let bad_name = format!("{base}source_checks:\n  - name: \"oops: run\"\n    run: echo hi\n");
        let err = SupportMatrix::from_yaml(&bad_name).unwrap_err().to_string();
        assert!(err.contains("name"), "got: {err}");

        // A run referencing secrets violates fork-PR safety.
        let secret = format!(
            "{base}source_checks:\n  - name: leak\n    run: echo ${{{{ secrets.TOKEN }}}}\n"
        );
        let err = SupportMatrix::from_yaml(&secret).unwrap_err().to_string();
        assert!(err.contains("secrets"), "got: {err}");

        // Bracket syntax is the same GitHub Actions secrets context and must
        // not sneak through a string check for only `secrets.`.
        let bracket_secret = format!(
            "{base}source_checks:\n  - name: leak\n    run: echo ${{{{ secrets['TOKEN'] }}}}\n"
        );
        let err = SupportMatrix::from_yaml(&bracket_secret)
            .unwrap_err()
            .to_string();
        assert!(err.contains("secrets"), "got: {err}");

        // Empty run is rejected.
        let empty = format!("{base}source_checks:\n  - name: nop\n    run: \"  \"\n");
        assert!(SupportMatrix::from_yaml(&empty).is_err());
    }

    fn lanes_yaml() -> String {
        format!(
            "\
schema: 1
bootstrap:
    ost:
        version: \"0.9.0\"
runners:
    windows-hosted:
        kind: github-hosted
        image: windows-2022
    usd-linux-real:
        kind: self-hosted
        labels: [self-hosted, linux, x64, usd-26]
cells:
    -
        name: plugin-pr-windows
        lane: pull_request
        runner: windows-hosted
        runtime_artifact: sha256:{a}
        runtime_remote:
            uri: oci://ghcr.io/owner/openstrata-runtime@sha256:{o}
            expected_oci_digest: sha256:{o}
        bundle: plugins/toy
        platform: cy2026
        profile: usd
        up_to: 4
    -
        name: linux-usd-support
        runner: usd-linux-real
        runtime_artifact: sha256:{a}
        plugin_artifact: sha256:{b}
        platform: cy2026
        profile: usd
",
            a = "ab".repeat(32),
            b = "cd".repeat(32),
            o = "ee".repeat(32),
        )
    }

    #[test]
    fn runner_profiles_and_lanes_resolve() {
        let m = SupportMatrix::from_yaml(&lanes_yaml()).unwrap();
        let pr = &m.cells[0];
        let support = &m.cells[1];
        assert_eq!(pr.lane, Lane::PullRequest);
        assert!(pr.lane.is_source());
        assert!(pr.plugin_artifact.is_none());
        assert_eq!(pr.bundle.as_deref(), Some("plugins/toy"));
        assert_eq!(support.lane, Lane::Scheduled);
        assert_eq!(m.runs_on(pr), vec!["windows-2022"]);
        assert!(m.is_hosted(pr));
        assert_eq!(
            m.runs_on(support),
            vec!["self-hosted", "linux", "x64", "usd-26"]
        );
        assert!(!m.is_hosted(support));
        assert_eq!(m.source_cells().len(), 1);
        assert_eq!(m.support_cells().len(), 1);
        // The hosted profile has no billing acknowledgement yet: warning
        // material, but not an error while nothing is publish-capable.
        assert_eq!(m.hosted_ack_missing(), vec!["windows-hosted"]);
        assert!(m.hosted_ack_errors().is_empty());
    }

    #[test]
    fn billing_acknowledgement_clears_the_warning_and_gates_publish() {
        let acked = lanes_yaml().replace(
            "        image: windows-2022\n",
            "        image: windows-2022\n        billing:\n            acknowledgement: required\n",
        );
        let m = SupportMatrix::from_yaml(&acked).unwrap();
        assert!(m.hosted_ack_missing().is_empty());

        // A publish-capable cell on an unacknowledged hosted profile is an
        // error (the pull_request lane cannot publish at all, so use main).
        let publishing = lanes_yaml().replace(
            "        lane: pull_request\n",
            "        lane: main\n        publish: candidate\n",
        );
        let m = SupportMatrix::from_yaml(&publishing).unwrap();
        assert_eq!(
            m.hosted_ack_errors(),
            vec!["plugin-pr-windows: runner 'windows-hosted'"]
        );
    }

    #[test]
    fn lane_and_runner_structural_errors_are_rejected() {
        let plugin_line = format!("        plugin_artifact: sha256:{}\n", "cd".repeat(32));
        let cases = [
            (
                lanes_yaml().replace("runner: windows-hosted", "runner: nope"),
                "unknown runner profile",
            ),
            (
                lanes_yaml().replace("        image: windows-2022\n", ""),
                "require an image",
            ),
            (
                lanes_yaml().replace(
                    "        labels: [self-hosted, linux, x64, usd-26]\n",
                    "        image: ubuntu-24.04\n",
                ),
                "self-hosted profiles require runs-on labels",
            ),
            (
                lanes_yaml().replace(&plugin_line, ""),
                "plugin_artifact is required",
            ),
            (
                lanes_yaml().replace("bundle: plugins/toy", "bundle: ../escape"),
                "repo-relative",
            ),
            (
                lanes_yaml().replace(
                    "        lane: pull_request\n",
                    "        lane: pull_request\n        publish: candidate\n",
                ),
                "must never publish",
            ),
            (
                lanes_yaml().replace(
                    "        runner: usd-linux-real\n",
                    "        runner: usd-linux-real\n        host:\n            labels: [self-hosted]\n",
                ),
                "both a runner profile and host.labels",
            ),
        ];
        for (yaml, needle) in cases {
            let err = SupportMatrix::from_yaml(&yaml).expect_err(needle);
            assert!(
                err.to_string().contains(needle),
                "expected '{needle}' in: {err}"
            );
        }
    }

    /// A hosted source cell with the full v0.9.0 contract: remote runtime
    /// reference + matrix-level bootstrap pin.
    fn remote_yaml() -> String {
        format!(
            "\
schema: 1
bootstrap:
  ost:
    version: \"0.9.0\"
runners:
  linux-hosted:
    kind: github-hosted
    image: ubuntu-24.04
    billing:
      acknowledgement: required
cells:
  - name: plugin-pr-linux
    lane: pull_request
    runner: linux-hosted
    runtime_artifact: sha256:{a}
    runtime_remote:
      uri: oci://ghcr.io/owner/openstrata-runtime@sha256:{o}
      expected_oci_digest: sha256:{o}
    bundle: plugins/toy
    platform: cy2026
    profile: usd
    up_to: 4
",
            a = "ab".repeat(32),
            o = "ee".repeat(32),
        )
    }

    #[test]
    fn remote_contract_parses_and_resolves_the_pin() {
        let m = SupportMatrix::from_yaml(&remote_yaml()).unwrap();
        let cell = &m.cells[0];
        let remote = cell.runtime_remote.as_ref().unwrap();
        assert_eq!(
            remote.pinned_oci_digest().as_deref(),
            Some(format!("sha256:{}", "ee".repeat(32)).as_str())
        );
        let bootstrap = m.bootstrap.as_ref().unwrap();
        assert_eq!(bootstrap.ost.version, "0.9.0");
        assert_eq!(bootstrap.ost.repository, "animu-sphere/open-strata");
    }

    #[test]
    fn hosted_source_cells_require_remote_and_bootstrap() {
        // Dropping the remote block fails: nothing can seed a hosted runner.
        let no_remote = remote_yaml()
            .lines()
            .filter(|l| {
                !l.contains("runtime_remote")
                    && !l.contains("uri:")
                    && !l.contains("expected_oci_digest")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let err = SupportMatrix::from_yaml(&no_remote).expect_err("remote required");
        assert!(err.to_string().contains("runtime_remote"), "got: {err}");

        // Dropping the bootstrap block fails: no pinned ost install.
        let no_bootstrap =
            remote_yaml().replace("bootstrap:\n  ost:\n    version: \"0.9.0\"\n", "");
        let err = SupportMatrix::from_yaml(&no_bootstrap).expect_err("bootstrap required");
        assert!(err.to_string().contains("bootstrap"), "got: {err}");

        // A self-hosted source cell needs neither (air-gapped local import).
        let self_hosted = no_bootstrap
            .replace(
                "  linux-hosted:\n    kind: github-hosted\n    image: ubuntu-24.04\n    billing:\n      acknowledgement: required\n",
                "  linux-real:\n    kind: self-hosted\n    labels: [self-hosted, linux, x64]\n",
            )
            .replace("runner: linux-hosted", "runner: linux-real")
            .lines()
            .filter(|l| {
                !l.contains("runtime_remote")
                    && !l.contains("uri:")
                    && !l.contains("expected_oci_digest")
            })
            .collect::<Vec<_>>()
            .join("\n");
        SupportMatrix::from_yaml(&self_hosted).expect("self-hosted source cells stay valid");
    }

    #[test]
    fn remote_and_bootstrap_structural_errors_are_rejected() {
        let o = "ee".repeat(32);
        let cases = [
            (
                // Mutable-only remote reference: tags are not a contract,
                // even when an expected digest sits beside them.
                remote_yaml().replace(
                    &format!("oci://ghcr.io/owner/openstrata-runtime@sha256:{o}"),
                    "oci://ghcr.io/owner/openstrata-runtime:latest",
                ),
                "pins no digest",
            ),
            (
                remote_yaml().replace(
                    &format!("expected_oci_digest: sha256:{o}"),
                    &format!("expected_oci_digest: sha256:{}", "ff".repeat(32)),
                ),
                "must agree",
            ),
            (
                remote_yaml()
                    .lines()
                    .filter(|l| !l.contains("expected_oci_digest"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                "expected_oci_digest is required",
            ),
            (
                remote_yaml().replace(
                    "uri: oci://ghcr.io/owner/openstrata-runtime",
                    "uri: file:///seeded/dist",
                ),
                "must be an oci:// reference",
            ),
            (
                remote_yaml().replace("version: \"0.9.0\"", "version: \"v0.9.0\""),
                "bare release version",
            ),
            (
                remote_yaml().replace("version: \"0.9.0\"", "version: \"0.9.0; rm -rf /\""),
                "bare release version",
            ),
            (
                remote_yaml().replace(
                    "    version: \"0.9.0\"\n",
                    "    version: \"0.9.0\"\n    repository: not-a-repo\n",
                ),
                "<owner>/<name>",
            ),
            (
                remote_yaml().replace(
                    "    version: \"0.9.0\"\n",
                    "    version: \"0.9.0\"\n    sha256:\n      x86_64-unknown-linux-musl: nothex\n",
                ),
                "64-hex",
            ),
        ];
        for (yaml, needle) in cases {
            let err = SupportMatrix::from_yaml(&yaml).expect_err(needle);
            assert!(
                err.to_string().contains(needle),
                "expected '{needle}' in: {err}"
            );
        }
    }

    #[test]
    fn structural_errors_are_rejected() {
        let cases = [
            ("schema: 1\ncells: []\n", "no cells"),
            (
                &valid_yaml().replace("windows-usd-toy", "linux-usd-toy"),
                "duplicate cell name",
            ),
            (
                &valid_yaml().replace("linux-usd-toy", "Linux_USD"),
                "not kebab-case",
            ),
            (&valid_yaml().replace("up_to: 4", "up_to: 7"), "exceeds"),
            (
                &valid_yaml().replace(&format!("sha256:{}", "ab".repeat(32)), "sha256:ab12"),
                "full sha256",
            ),
        ];
        for (yaml, needle) in cases {
            let err = SupportMatrix::from_yaml(yaml).expect_err(needle);
            assert!(
                err.to_string().contains(needle),
                "expected '{needle}' in: {err}"
            );
        }
    }

    #[test]
    fn host_python_is_validated() {
        assert!(is_major_minor("3.13"));
        assert!(is_major_minor("3.10"));
        assert!(!is_major_minor("3"));
        assert!(!is_major_minor("3.13.2"));
        assert!(!is_major_minor("3.x"));
        assert!(!is_major_minor("py313"));
        assert!(!is_major_minor(""));

        // A well-formed ABI on a source cell parses and is surfaced.
        let src = valid_yaml().replace(
            "  - name: linux-usd-toy\n    runtime_artifact",
            "  - name: linux-usd-toy\n    lane: pull_request\n    host_python: \"3.13\"\n    runtime_artifact",
        );
        let m = SupportMatrix::from_yaml(&src).unwrap();
        assert_eq!(m.cells[0].host_python.as_deref(), Some("3.13"));
        assert!(m.needs_host_python());

        // A malformed ABI is rejected.
        let bad = src.replace("host_python: \"3.13\"", "host_python: \"3\"");
        let err = SupportMatrix::from_yaml(&bad).expect_err("bad host_python");
        assert!(err.to_string().contains("major.minor"), "{err}");

        // host_python on a support (non-source) lane is rejected: it never builds.
        let support = valid_yaml().replace(
            "  - name: linux-usd-toy\n    runtime_artifact",
            "  - name: linux-usd-toy\n    host_python: \"3.13\"\n    runtime_artifact",
        );
        let err = SupportMatrix::from_yaml(&support).expect_err("host_python on support lane");
        assert!(err.to_string().contains("source lanes"), "{err}");
    }
}
