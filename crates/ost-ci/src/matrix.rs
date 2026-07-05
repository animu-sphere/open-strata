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

/// One explicit support line: runtime digest × plugin digest × target/profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupportCell {
    /// Unique cell name (kebab-case); becomes the job/report identifier.
    pub name: String,
    /// Full `sha256:<hex>` digest of the runtime artifact to materialize.
    pub runtime_artifact: String,
    /// Full `sha256:<hex>` digest of the plugin artifact to verify.
    pub plugin_artifact: String,
    /// Platform calendar-year id, e.g. `cy2026`.
    pub platform: String,
    /// Profile, e.g. `usd`.
    pub profile: String,
    /// Highest verification level to run (`ost plugin test --up-to`), 0..=6.
    #[serde(default = "default_up_to")]
    pub up_to: u8,
    #[serde(default)]
    pub host: HostSpec,
}

fn default_up_to() -> u8 {
    5
}

/// The support matrix document (`openstrata.ci.yaml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupportMatrix {
    pub schema: u32,
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
        let mut seen: Vec<&str> = Vec::new();
        for cell in &self.cells {
            let name = cell.name.as_str();
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
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

            for (field, value) in [
                ("runtime_artifact", &cell.runtime_artifact),
                ("plugin_artifact", &cell.plugin_artifact),
            ] {
                // CI pins exact bytes: a full digest is required, a prefix is
                // not — a prefix can silently start matching a different
                // artifact as the registry grows.
                if !ost_artifact::is_sha256_ref(value) {
                    return Err(Error::InvalidManifest(format!(
                        "cell '{name}': {field} '{value}' is not a full sha256:<64-hex> digest"
                    )));
                }
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
            for (field, value) in [
                ("runtime_artifact", &cell.runtime_artifact),
                ("plugin_artifact", &cell.plugin_artifact),
            ] {
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
# Generate a GitHub Actions workflow from this file:
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
