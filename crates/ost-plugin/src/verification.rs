// SPDX-License-Identifier: Apache-2.0
//! Packaged verification-content contract.
//!
//! Source bundles may carry an adjacent `<fixture>.golden.usda` oracle for a
//! declared round-trip fixture. Packaging writes the pairs that actually exist
//! to [`PLUGIN_VERIFICATION`], including both content digests, so an extracted
//! package can distinguish "this source never declared an oracle" from "the
//! package dropped or changed the oracle it declared".

use std::fs::File;

use camino::Utf8Path;
use ost_core::{digest, Error, Result};
use serde::{Deserialize, Serialize};

use crate::bundle::{check_safe_relative, Bundle};

pub const PLUGIN_VERIFICATION: &str = "openstrata.verification.json";
pub const PLUGIN_VERIFICATION_SCHEMA: &str = "openstrata.plugin-verification/v1alpha1";
const ORACLE_CONVENTION: &str = "<fixture>.golden.usda";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginVerification {
    pub schema: String,
    pub oracle_convention: String,
    #[serde(default)]
    pub roundtrip: Vec<RoundtripVerification>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundtripVerification {
    pub fixture: String,
    pub fixture_sha256: String,
    pub oracle: String,
    pub oracle_sha256: String,
}

/// The deterministic adjacent oracle name for a round-trip fixture.
pub fn adjacent_golden(fixture: &str) -> String {
    format!("{fixture}.golden.usda")
}

impl PluginVerification {
    /// Discover and hash the adjacent oracles present in `bundle`.
    ///
    /// A round-trip fixture without an adjacent oracle is intentionally absent
    /// from `roundtrip`: L5 remains optional for source bundles that have never
    /// made a golden claim. Once an entry is emitted, both files become required
    /// verification content.
    pub fn from_bundle(bundle: &Bundle) -> Result<Self> {
        let mut roundtrip = Vec::new();
        for fixture in &bundle.manifest.tests.roundtrip {
            if roundtrip
                .iter()
                .any(|entry: &RoundtripVerification| entry.fixture == *fixture)
            {
                continue;
            }
            let oracle = adjacent_golden(fixture);
            let oracle_path = bundle.path(&oracle);
            if !oracle_path.as_std_path().is_file() {
                continue;
            }
            roundtrip.push(RoundtripVerification {
                fixture: fixture.clone(),
                fixture_sha256: hash_required(&bundle.path(fixture))?,
                oracle,
                oracle_sha256: hash_required(&oracle_path)?,
            });
        }
        let contract = Self {
            schema: PLUGIN_VERIFICATION_SCHEMA.into(),
            oracle_convention: ORACLE_CONVENTION.into(),
            roundtrip,
        };
        contract.validate()?;
        Ok(contract)
    }

    /// Load and structurally validate a packaged contract when one is present.
    pub fn load(root: &Utf8Path) -> Result<Option<Self>> {
        let path = root.join(PLUGIN_VERIFICATION);
        match std::fs::symlink_metadata(path.as_std_path()) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::validation(format!(
                    "{PLUGIN_VERIFICATION} must be a regular in-package file, not a symlink"
                )));
            }
            Ok(metadata) if !metadata.is_file() => {
                return Err(Error::validation(format!(
                    "{PLUGIN_VERIFICATION} must be a regular file"
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::io(path.to_string(), error)),
        }
        let source = std::fs::read_to_string(path.as_std_path())
            .map_err(|error| Error::io(path.to_string(), error))?;
        let contract: Self = serde_json::from_str(&source)
            .map_err(|error| Error::parse(PLUGIN_VERIFICATION, anyhow::Error::new(error)))?;
        contract.validate()?;
        Ok(Some(contract))
    }

    pub fn oracle_for(&self, fixture: &str) -> Option<&RoundtripVerification> {
        self.roundtrip.iter().find(|entry| entry.fixture == fixture)
    }

    fn validate(&self) -> Result<()> {
        if self.schema != PLUGIN_VERIFICATION_SCHEMA {
            return Err(Error::config(format!(
                "{PLUGIN_VERIFICATION}: schema '{}' is unsupported (expected '{PLUGIN_VERIFICATION_SCHEMA}')",
                self.schema
            )));
        }
        if self.oracle_convention != ORACLE_CONVENTION {
            return Err(Error::config(format!(
                "{PLUGIN_VERIFICATION}: oracle_convention '{}' is unsupported (expected '{ORACLE_CONVENTION}')",
                self.oracle_convention
            )));
        }
        let mut seen = Vec::new();
        for entry in &self.roundtrip {
            check_safe_relative("verification fixture", &entry.fixture)?;
            check_safe_relative("verification oracle", &entry.oracle)?;
            if entry.oracle != adjacent_golden(&entry.fixture) {
                return Err(Error::config(format!(
                    "{PLUGIN_VERIFICATION}: oracle '{}' does not follow the adjacent convention for fixture '{}'",
                    entry.oracle, entry.fixture
                )));
            }
            if seen.contains(&entry.fixture.as_str()) {
                return Err(Error::config(format!(
                    "{PLUGIN_VERIFICATION}: duplicate roundtrip fixture '{}'",
                    entry.fixture
                )));
            }
            seen.push(entry.fixture.as_str());
            for (field, value) in [
                ("fixture_sha256", entry.fixture_sha256.as_str()),
                ("oracle_sha256", entry.oracle_sha256.as_str()),
            ] {
                if !valid_sha256(value) {
                    return Err(Error::config(format!(
                        "{PLUGIN_VERIFICATION}: {field} for '{}' is not a sha256:<64 lowercase hex> digest",
                        entry.fixture
                    )));
                }
            }
        }
        Ok(())
    }
}

impl RoundtripVerification {
    /// Verify that a declared packaged fixture/oracle pair is present and still
    /// matches the content identity recorded when the archive was produced.
    pub fn verify(&self, root: &Utf8Path) -> Result<()> {
        verify_file(
            root,
            "roundtrip fixture",
            &self.fixture,
            &self.fixture_sha256,
        )?;
        verify_file(root, "roundtrip oracle", &self.oracle, &self.oracle_sha256)
    }
}

fn verify_file(root: &Utf8Path, kind: &str, relative: &str, expected: &str) -> Result<()> {
    let path = root.join(relative);
    if !path.as_std_path().is_file() {
        return Err(Error::validation(format!(
            "packaged {kind} '{relative}' is missing (declared by {PLUGIN_VERIFICATION})"
        )));
    }
    let observed = hash_required(&path)?;
    if observed != expected {
        return Err(Error::validation(format!(
            "packaged {kind} '{relative}' digest mismatch: expected {expected}, observed {observed}"
        )));
    }
    Ok(())
}

fn hash_required(path: &Utf8Path) -> Result<String> {
    let mut file =
        File::open(path.as_std_path()).map_err(|error| Error::io(path.to_string(), error))?;
    digest::sha256_hex_reader(&mut file)
        .map(|(sha256, _)| sha256)
        .map_err(|error| Error::io(path.to_string(), error))
}

fn valid_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PluginManifest;

    fn bundle_with_oracle() -> (tempdir_like::Dir, Bundle) {
        let dir = tempdir_like::Dir::new("verification-contract");
        std::fs::create_dir_all(dir.path.join("tests/fixtures").as_std_path()).unwrap();
        std::fs::write(
            dir.path.join("tests/fixtures/basic.toy").as_std_path(),
            b"toy",
        )
        .unwrap();
        std::fs::write(
            dir.path
                .join("tests/fixtures/basic.toy.golden.usda")
                .as_std_path(),
            b"#usda 1.0\n",
        )
        .unwrap();
        let manifest = PluginManifest::parse(
            r#"
plugin: { name: toy, version: 0.1.0, kind: usd-fileformat }
runtime: { openusd: ">=25.05,<26.0" }
provides: ["usd-fileformat:toy"]
usd: { plug_info: plugin/resources/toy/plugInfo.json }
tests: { smoke: [tests/fixtures/basic.toy], roundtrip: [tests/fixtures/basic.toy] }
"#,
        )
        .unwrap();
        let root = dir.path.clone();
        (dir, Bundle { root, manifest })
    }

    #[test]
    fn discovers_and_verifies_adjacent_oracle_content() {
        let (_dir, bundle) = bundle_with_oracle();
        let contract = PluginVerification::from_bundle(&bundle).unwrap();
        assert_eq!(contract.roundtrip.len(), 1);
        let entry = &contract.roundtrip[0];
        assert_eq!(entry.fixture, "tests/fixtures/basic.toy");
        assert_eq!(entry.oracle, "tests/fixtures/basic.toy.golden.usda");
        assert!(entry.fixture_sha256.starts_with("sha256:"));
        assert!(entry.oracle_sha256.starts_with("sha256:"));
        entry.verify(&bundle.root).unwrap();
    }

    #[test]
    fn declared_oracle_missing_or_changed_fails_closed() {
        let (_dir, bundle) = bundle_with_oracle();
        let contract = PluginVerification::from_bundle(&bundle).unwrap();
        let entry = &contract.roundtrip[0];
        std::fs::write(bundle.path(&entry.oracle).as_std_path(), b"changed").unwrap();
        assert!(entry.verify(&bundle.root).is_err());
        std::fs::remove_file(bundle.path(&entry.oracle).as_std_path()).unwrap();
        assert!(entry.verify(&bundle.root).is_err());
    }

    #[test]
    fn absent_oracle_makes_no_packaged_claim() {
        let (_dir, bundle) = bundle_with_oracle();
        std::fs::remove_file(
            bundle
                .path("tests/fixtures/basic.toy.golden.usda")
                .as_std_path(),
        )
        .unwrap();
        let contract = PluginVerification::from_bundle(&bundle).unwrap();
        assert!(contract.roundtrip.is_empty());
    }

    /// Minimal scoped temp directory helper (no external dev-deps).
    mod tempdir_like {
        use camino::Utf8PathBuf;

        pub struct Dir {
            pub path: Utf8PathBuf,
        }

        impl Dir {
            pub fn new(tag: &str) -> Dir {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let mut path = Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
                path.push(format!(
                    "ost-verification-{tag}-{}-{nanos}",
                    std::process::id()
                ));
                std::fs::create_dir_all(path.as_std_path()).unwrap();
                Dir { path }
            }
        }

        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(self.path.as_std_path());
            }
        }
    }
}
