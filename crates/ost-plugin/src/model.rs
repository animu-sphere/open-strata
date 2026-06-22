//! The plugin bundle manifest data model (`openstrata.plugin.yaml`).
//!
//! A plugin is a *self-describing bundle*, not a bare shared library (harness
//! §8). The manifest declares what the plugin is, the OpenUSD runtime range it
//! tolerates, the capabilities it provides (in the same vocabulary the runtime
//! resolver speaks), what it requires, where its `plugInfo.json` lives, and the
//! fixtures its test levels consume.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Filename of the plugin bundle manifest at a bundle root.
pub const PLUGIN_MANIFEST: &str = "openstrata.plugin.yaml";

/// The kind of OpenUSD plugin a bundle ships. The MVP covers three kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginKind {
    UsdFileformat,
    UsdAssetResolver,
    UsdSchema,
}

impl PluginKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PluginKind::UsdFileformat => "usd-fileformat",
            PluginKind::UsdAssetResolver => "usd-asset-resolver",
            PluginKind::UsdSchema => "usd-schema",
        }
    }

    pub fn from_tag(s: &str) -> Option<PluginKind> {
        match s {
            "usd-fileformat" => Some(PluginKind::UsdFileformat),
            "usd-asset-resolver" => Some(PluginKind::UsdAssetResolver),
            "usd-schema" => Some(PluginKind::UsdSchema),
            _ => None,
        }
    }

    /// Every kind, for help text and validation messages.
    pub const ALL: [PluginKind; 3] = [
        PluginKind::UsdFileformat,
        PluginKind::UsdAssetResolver,
        PluginKind::UsdSchema,
    ];
}

/// Identity of the plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginIdentity {
    pub name: String,
    pub version: String,
    pub kind: PluginKind,
}

/// The runtime the plugin targets: an OpenUSD version range plus an ABI tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeReq {
    /// OpenUSD version *range* the plugin tolerates, e.g. `>=24.11,<25.0`.
    pub openusd: String,
    /// C++ ABI tag the plugin was built against, e.g. `libcxx` or `libstdcxx`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cxx_abi: Option<String>,
    /// Python ABI tag, e.g. `cp311`. Optional; relevant to schema/python plugins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python_abi: Option<String>,
}

/// What the plugin requires from its environment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requires {
    /// Capabilities the runtime must provide, e.g. `usd-stage-read`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// component -> version range, e.g. `materialx: ">=1.39,<1.40"`.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub components: IndexMap<String, String>,
}

/// Where the bundle's USD `plugInfo.json` lives, relative to the bundle root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsdSection {
    pub plug_info: String,
}

/// Fixtures consumed by each verification level. All optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tests {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub smoke: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roundtrip: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub negative: Vec<String>,
}

/// The full `openstrata.plugin.yaml` document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    pub plugin: PluginIdentity,
    pub runtime: RuntimeReq,
    /// Capabilities this plugin provides, e.g. `usd-fileformat:lumagraph`.
    #[serde(default)]
    pub provides: Vec<String>,
    #[serde(default)]
    pub requires: Requires,
    pub usd: UsdSection,
    #[serde(default)]
    pub tests: Tests,
}

impl PluginManifest {
    /// Parse a manifest from YAML source.
    pub fn parse(src: &str) -> Result<PluginManifest, serde_yaml::Error> {
        serde_yaml::from_str(src)
    }

    /// The plugin's name (identity shorthand).
    pub fn name(&self) -> &str {
        &self.plugin.name
    }

    pub fn kind(&self) -> PluginKind {
        self.plugin.kind
    }

    /// All fixtures referenced across every test level, deduplicated in order.
    pub fn all_fixtures(&self) -> Vec<&str> {
        let mut seen = Vec::new();
        for f in self
            .tests
            .smoke
            .iter()
            .chain(&self.tests.roundtrip)
            .chain(&self.tests.negative)
        {
            if !seen.contains(&f.as_str()) {
                seen.push(f.as_str());
            }
        }
        seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
plugin:
  name: usdluma
  version: 0.1.0
  kind: usd-fileformat
runtime:
  openusd: ">=24.11,<25.0"
  cxx_abi: libcxx
provides:
  - usd-fileformat:lumagraph
requires:
  capabilities: [usd-stage-read]
  components: { materialx: ">=1.39,<1.40" }
usd:
  plug_info: plugin/resources/usdluma/plugInfo.json
tests:
  smoke: [tests/fixtures/basic.lumagraph]
  roundtrip: [tests/fixtures/basic.lumagraph]
  negative: [tests/fixtures/invalid.lumagraph]
"#;

    #[test]
    fn parses_the_documented_shape() {
        let m = PluginManifest::parse(SAMPLE).expect("manifest parses");
        assert_eq!(m.name(), "usdluma");
        assert_eq!(m.kind(), PluginKind::UsdFileformat);
        assert_eq!(m.runtime.openusd, ">=24.11,<25.0");
        assert_eq!(m.runtime.cxx_abi.as_deref(), Some("libcxx"));
        assert_eq!(m.provides, vec!["usd-fileformat:lumagraph"]);
        assert_eq!(m.requires.capabilities, vec!["usd-stage-read"]);
        assert_eq!(
            m.requires.components.get("materialx").map(String::as_str),
            Some(">=1.39,<1.40")
        );
    }

    #[test]
    fn fixtures_are_deduplicated_in_order() {
        let m = PluginManifest::parse(SAMPLE).expect("manifest parses");
        // basic.lumagraph appears in both smoke and roundtrip; invalid is distinct.
        assert_eq!(
            m.all_fixtures(),
            vec![
                "tests/fixtures/basic.lumagraph",
                "tests/fixtures/invalid.lumagraph"
            ]
        );
    }

    #[test]
    fn kind_round_trips_through_str() {
        for k in PluginKind::ALL {
            assert_eq!(PluginKind::from_tag(k.as_str()), Some(k));
        }
        assert_eq!(PluginKind::from_tag("bogus"), None);
    }
}
