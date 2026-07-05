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

/// One explicit support line: runtime digest × plugin digest × target/profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Publication policy (default `never`).
    #[serde(default)]
    pub publish: Publish,
    #[serde(default)]
    pub host: HostSpec,
}

fn default_up_to() -> u8 {
    5
}

fn is_kebab(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The support matrix document (`openstrata.ci.yaml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupportMatrix {
    pub schema: u32,
    /// Named runner profiles cells reference via `runner:` (deterministic
    /// order for rendering).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub runners: BTreeMap<String, RunnerProfile>,
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

    /// Whether a cell resolves to a GitHub-hosted runner profile.
    pub fn is_hosted(&self, cell: &SupportCell) -> bool {
        cell.runner
            .as_deref()
            .and_then(|name| self.runners.get(name))
            .is_some_and(RunnerProfile::is_hosted)
    }

    /// Cells that build from checked-out source (`pull_request` / `main`).
    pub fn source_cells(&self) -> Vec<&SupportCell> {
        self.cells.iter().filter(|c| c.lane.is_source()).collect()
    }

    /// Cells that re-validate pinned artifacts (`scheduled` / dispatch).
    pub fn support_cells(&self) -> Vec<&SupportCell> {
        self.cells.iter().filter(|c| !c.lane.is_source()).collect()
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
# the bundle from the checked-out repo and omit plugin_artifact:
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
#       bundle: plugins/myPlugin
#       platform: cy2026
#       profile: usd
#       up_to: 4
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
        assert_eq!(
            m.cells[0].host.runs_on(),
            vec!["self-hosted".to_string(), "linux".to_string()]
        );
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

    fn lanes_yaml() -> String {
        format!(
            "\
schema: 1
runners:
  windows-hosted:
    kind: github-hosted
    image: windows-2022
  usd-linux-real:
    kind: self-hosted
    labels: [self-hosted, linux, x64, usd-26]
cells:
  - name: plugin-pr-windows
    lane: pull_request
    runner: windows-hosted
    runtime_artifact: sha256:{a}
    bundle: plugins/toy
    platform: cy2026
    profile: usd
    up_to: 4
  - name: linux-usd-support
    runner: usd-linux-real
    runtime_artifact: sha256:{a}
    plugin_artifact: sha256:{b}
    platform: cy2026
    profile: usd
",
            a = "ab".repeat(32),
            b = "cd".repeat(32)
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
            "    image: windows-2022\n",
            "    image: windows-2022\n    billing:\n      acknowledgement: required\n",
        );
        let m = SupportMatrix::from_yaml(&acked).unwrap();
        assert!(m.hosted_ack_missing().is_empty());

        // A publish-capable cell on an unacknowledged hosted profile is an
        // error (the pull_request lane cannot publish at all, so use main).
        let publishing = lanes_yaml().replace(
            "    lane: pull_request\n",
            "    lane: main\n    publish: candidate\n",
        );
        let m = SupportMatrix::from_yaml(&publishing).unwrap();
        assert_eq!(
            m.hosted_ack_errors(),
            vec!["plugin-pr-windows: runner 'windows-hosted'"]
        );
    }

    #[test]
    fn lane_and_runner_structural_errors_are_rejected() {
        let plugin_line = format!("    plugin_artifact: sha256:{}\n", "cd".repeat(32));
        let cases = [
            (
                lanes_yaml().replace("runner: windows-hosted", "runner: nope"),
                "unknown runner profile",
            ),
            (
                lanes_yaml().replace("    image: windows-2022\n", ""),
                "require an image",
            ),
            (
                lanes_yaml().replace(
                    "    labels: [self-hosted, linux, x64, usd-26]\n",
                    "    image: ubuntu-24.04\n",
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
                    "    lane: pull_request\n",
                    "    lane: pull_request\n    publish: candidate\n",
                ),
                "must never publish",
            ),
            (
                lanes_yaml().replace(
                    "    runner: usd-linux-real\n",
                    "    runner: usd-linux-real\n    host:\n      labels: [self-hosted]\n",
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
}
