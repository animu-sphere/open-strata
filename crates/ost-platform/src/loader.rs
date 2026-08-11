// SPDX-License-Identifier: Apache-2.0
//! Loading platform manifests.
//!
//! Built-in CY manifests are embedded in the binary so `ost platform list`
//! works on a fresh install with no network or store. User-provided YAML in
//! `~/.ost/platforms/*.yaml` is layered on top and overrides built-ins with
//! the same id (§3.5 resolver philosophy; §17.3 layout).

use std::collections::BTreeMap;

use ost_core::paths::Store;
use ost_core::{Error, Result};

use crate::model::Platform;

/// A built-in manifest: `(id, yaml-source)`.
const BUILTINS: &[(&str, &str)] = &[
    ("cy2025", include_str!("../../../platforms/cy2025.yaml")),
    ("cy2026", include_str!("../../../platforms/cy2026.yaml")),
    ("cy2027", include_str!("../../../platforms/cy2027.yaml")),
];

/// All known platforms, keyed and ordered by id.
pub struct Catalog {
    platforms: BTreeMap<String, Platform>,
}

impl Catalog {
    /// Load built-in manifests, then overlay any user manifests.
    pub fn load() -> Result<Catalog> {
        let user_dir = Store::discover().platforms();
        let platforms = ost_core::catalog::load(BUILTINS, &user_dir, parse)?;
        Ok(Catalog { platforms })
    }

    /// Platforms ordered by id (BTreeMap iteration is sorted).
    pub fn iter(&self) -> impl Iterator<Item = &Platform> {
        self.platforms.values()
    }

    pub fn get(&self, id: &str) -> Result<&Platform> {
        self.platforms
            .get(id)
            .ok_or_else(|| Error::PlatformNotFound(id.to_string()))
    }

    pub fn len(&self) -> usize {
        self.platforms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.platforms.is_empty()
    }
}

fn parse(label: &str, src: &str) -> Result<Platform> {
    let platform: Platform =
        serde_yaml::from_str(src).map_err(|e| Error::parse(format!("platform '{label}'"), e))?;
    validate_openusd(&platform)?;
    Ok(platform)
}

fn validate_openusd(platform: &Platform) -> Result<()> {
    let Some(policy) = &platform.openusd else {
        return Ok(());
    };
    if policy.schema != 1 {
        return Err(Error::InvalidManifest(format!(
            "platform '{}' has unsupported openusd schema {} (expected 1)",
            platform.id, policy.schema
        )));
    }
    let mut cells = std::collections::BTreeSet::new();
    for cell in &policy.cells {
        let key = (cell.os.as_str(), cell.arch.as_str());
        if !cells.insert(key) {
            return Err(Error::InvalidManifest(format!(
                "platform '{}' has duplicate OpenUSD cell {}-{}",
                platform.id, key.0, key.1
            )));
        }
        for reference in [
            &cell.toolchain.version_from,
            &cell.toolchain.cxx_standard_from,
            &cell.toolchain.runtime.version_from,
            &cell.python.version_from,
            &cell.tbb.version_from,
        ] {
            if !platform.core.contains_key(reference) {
                return Err(Error::InvalidManifest(format!(
                    "platform '{}' OpenUSD cell references missing core component '{}'",
                    platform.id, reference
                )));
            }
        }
        let mut variants = std::collections::BTreeSet::new();
        for variant in &cell.variants {
            if !variants.insert(variant.id.as_str()) {
                return Err(Error::InvalidManifest(format!(
                    "platform '{}' OpenUSD cell {}-{} repeats variant '{}'",
                    platform.id,
                    key.0,
                    key.1,
                    variant.id.as_str()
                )));
            }
            if variant.builders.is_empty() {
                return Err(Error::InvalidManifest(format!(
                    "platform '{}' OpenUSD variant '{}' declares no supported builder",
                    platform.id,
                    variant.id.as_str()
                )));
            }
        }
    }
    Ok(())
}

/// Convenience: load the whole catalog.
pub fn load_all() -> Result<Catalog> {
    Catalog::load()
}

/// Convenience: load a single platform by id.
pub fn load_one(id: &str) -> Result<Platform> {
    Catalog::load()?.get(id).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpenUsdBuilder, OpenUsdVariantId};
    use ost_core::host::{Arch, Os};

    #[test]
    fn cy2026_resolves_exact_standard_compatibility() {
        let platform = parse("cy2026", BUILTINS[1].1).unwrap();
        let (resolved, variant) = platform
            .resolve_openusd(Os::Linux, Arch::X86_64, OpenUsdVariantId::Standard)
            .unwrap();
        assert_eq!(resolved.toolchain.family, "gcc");
        assert_eq!(resolved.toolchain.version, "14.2");
        assert_eq!(resolved.toolchain.cxx_standard, "20");
        assert_eq!(resolved.toolchain.runtime.version, "2.28");
        assert_eq!(resolved.python.version, "3.13.x");
        assert_eq!(resolved.tbb.version, "2022.x");
        assert_eq!(resolved.variant, OpenUsdVariantId::Standard);
        assert_eq!(resolved.capabilities, ["usd-core", "imaging", "opengl"]);
        assert!(variant.builders.contains(&OpenUsdBuilder::BuildUsd));
        assert!(variant.builders.contains(&OpenUsdBuilder::Cmake));
    }

    #[test]
    fn missing_core_reference_is_rejected() {
        let invalid = BUILTINS[1]
            .1
            .replace("version_from: tbb", "version_from: missing-tbb");
        let error = parse("bad", &invalid).unwrap_err();
        assert!(error
            .to_string()
            .contains("missing core component 'missing-tbb'"));
    }

    #[test]
    fn undeclared_cartesian_cell_does_not_resolve() {
        let platform = parse("cy2026", BUILTINS[1].1).unwrap();
        assert!(platform
            .resolve_openusd(Os::Windows, Arch::X86_64, OpenUsdVariantId::Standard)
            .is_none());
    }
}
