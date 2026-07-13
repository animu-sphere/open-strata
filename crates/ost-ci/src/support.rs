// SPDX-License-Identifier: Apache-2.0
//! Validation of CI cells against the public feature/platform declaration.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use ost_core::{Error, Result};

use crate::SupportMatrix;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupportDeclaration {
    levels: BTreeMap<String, Level>,
    platforms: Vec<Platform>,
    features: Vec<Feature>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Level {
    description: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Platform {
    id: String,
    label: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Feature {
    id: String,
    label: String,
    #[serde(default, rename = "note")]
    _note: Option<String>,
    support: BTreeMap<String, String>,
}

impl SupportDeclaration {
    /// Parse the same declaration used to generate the public support table.
    pub fn from_toml(src: &str) -> Result<Self> {
        let declaration: Self = toml::from_str(src).map_err(|error| {
            Error::config(format!("support declaration does not parse: {error}"))
        })?;
        declaration.validate()?;
        Ok(declaration)
    }

    fn validate(&self) -> Result<()> {
        if !self.levels.contains_key("unsupported") {
            return Err(Error::config(
                "support declaration must define levels.unsupported",
            ));
        }
        for (name, level) in &self.levels {
            if level.description.trim().is_empty() {
                return Err(Error::config(format!(
                    "support level '{name}' has an empty description"
                )));
            }
        }
        let mut platforms = BTreeSet::new();
        for platform in &self.platforms {
            if platform.id.is_empty()
                || platform.label.trim().is_empty()
                || !platforms.insert(platform.id.as_str())
            {
                return Err(Error::config(format!(
                    "invalid or duplicate support platform '{}'",
                    platform.id
                )));
            }
        }
        let mut features = BTreeSet::new();
        for feature in &self.features {
            if feature.id.is_empty()
                || feature.label.trim().is_empty()
                || !features.insert(feature.id.as_str())
            {
                return Err(Error::config(format!(
                    "invalid or duplicate support feature '{}'",
                    feature.id
                )));
            }
            for platform in &platforms {
                let Some(level) = feature.support.get(*platform) else {
                    return Err(Error::config(format!(
                        "support feature '{}' omits platform '{platform}'",
                        feature.id
                    )));
                };
                if !self.levels.contains_key(level) {
                    return Err(Error::config(format!(
                        "support feature '{}' uses unknown level '{level}' for '{platform}'",
                        feature.id
                    )));
                }
            }
            if feature
                .support
                .keys()
                .any(|platform| !platforms.contains(platform.as_str()))
            {
                return Err(Error::config(format!(
                    "support feature '{}' names an unknown platform",
                    feature.id
                )));
            }
        }
        Ok(())
    }

    /// Return deterministic diagnostics for hosted cells that omit a public
    /// platform mapping or claim a feature marked unsupported. Self-hosted
    /// cells are checked when they opt into a claim, but are not required to
    /// carry one.
    pub fn matrix_issues(&self, matrix: &SupportMatrix) -> Vec<String> {
        let features: BTreeMap<&str, &Feature> = self
            .features
            .iter()
            .map(|feature| (feature.id.as_str(), feature))
            .collect();
        let platforms: BTreeSet<&str> = self
            .platforms
            .iter()
            .map(|platform| platform.id.as_str())
            .collect();
        let mut issues = Vec::new();

        for cell in &matrix.cells {
            let hosted = matrix.is_hosted(cell);
            let Some(claim) = &cell.support else {
                if hosted {
                    issues.push(format!(
                        "{}: hosted cell omits support.platform and support.features",
                        cell.name
                    ));
                }
                continue;
            };
            if !platforms.contains(claim.platform.as_str()) {
                issues.push(format!(
                    "{}: unknown support platform '{}'",
                    cell.name, claim.platform
                ));
                continue;
            }

            let infrastructure = if hosted {
                "github_hosted_ci"
            } else {
                "self_hosted_ci"
            };
            let claimed: BTreeSet<&str> = std::iter::once(infrastructure)
                .chain(claim.features.iter().map(String::as_str))
                .collect();
            for feature_id in claimed {
                let Some(feature) = features.get(feature_id) else {
                    issues.push(format!(
                        "{}: unknown support feature '{feature_id}'",
                        cell.name
                    ));
                    continue;
                };
                match feature.support.get(&claim.platform) {
                    Some(level) if level == "unsupported" => issues.push(format!(
                        "{}: feature '{feature_id}' is unsupported on '{}'",
                        cell.name, claim.platform
                    )),
                    Some(_) => {}
                    None => issues.push(format!(
                        "{}: feature '{feature_id}' omits platform '{}'",
                        cell.name, claim.platform
                    )),
                }
            }
        }
        issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_claims_fail_closed() {
        let declaration = SupportDeclaration::from_toml(
            "[levels.stable]\ndescription='yes'\n[levels.unsupported]\ndescription='no'\n\
             [[platforms]]\nid='linux_x86_64'\nlabel='Linux'\n\
             [[features]]\nid='github_hosted_ci'\nlabel='Hosted'\n\
             support={linux_x86_64='stable'}\n\
             [[features]]\nid='plugin_build'\nlabel='Build'\n\
             support={linux_x86_64='unsupported'}\n",
        )
        .unwrap();
        let digest = "ab".repeat(32);
        let matrix = SupportMatrix::from_yaml(&format!(
            "schema: 1
cells:
  - name: hosted
    runtime_artifact: sha256:{digest}
    plugin_artifact: sha256:{digest}
    platform: cy2026
    profile: usd
    support:
      platform: linux_x86_64
      features: [plugin_build]
"
        ))
        .unwrap();
        assert_eq!(
            declaration.matrix_issues(&matrix),
            vec!["hosted: feature 'plugin_build' is unsupported on 'linux_x86_64'"]
        );
    }
}
