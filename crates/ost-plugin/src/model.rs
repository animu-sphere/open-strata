// SPDX-License-Identifier: Apache-2.0
//! The plugin bundle manifest data model (`openstrata.plugin.yaml`).
//!
//! A plugin is a *self-describing bundle*, not a bare shared library (harness
//! §8). The manifest declares what the plugin is, the OpenUSD runtime range it
//! tolerates, the capabilities it provides (in the same vocabulary the runtime
//! resolver speaks), what it requires, where its `plugInfo.json` lives, and the
//! fixtures its test levels consume.

use indexmap::IndexMap;
use ost_core::host::Os;
use serde::{Deserialize, Serialize};

/// Filename of the plugin bundle manifest at a bundle root.
pub const PLUGIN_MANIFEST: &str = "openstrata.plugin.yaml";

/// Schema identifier required by manifests that opt into bundle composition.
pub const PLUGIN_SCHEMA: &str = "openstrata.plugin/v1alpha1";

/// Version header for additive plugin-manifest extensions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifestHeader {
    pub schema: String,
}

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

/// The C++ ABI a plugin was built against.
///
/// A *source* bundle's correct ABI is per-target (`msvc143` on Windows,
/// `libstdcxx` on Linux, `libcxx` on macOS), so a single scalar cannot describe a
/// cross-platform bundle. This is therefore one of:
/// - a **scalar** tag (`cxx_abi: msvc143`) — the same ABI for every target, or
///   the literal [`CxxAbi::INHERIT`] sentinel;
/// - a **per-OS map** (`cxx_abi: { windows: msvc143, linux: libstdcxx }`) — the
///   tag resolved against the target OS;
/// - the **`inherit`** sentinel — the source defers its ABI to whatever runtime
///   it is verified against (the common case for a source bundle; `ost plugin
///   package` then freezes the one resolved ABI into the artifact).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CxxAbi {
    /// A single tag for all targets, or the `inherit` sentinel.
    Scalar(String),
    /// Per-OS tags keyed by `windows`/`linux`/`macos`.
    PerOs(IndexMap<String, String>),
}

impl CxxAbi {
    /// The sentinel that defers the plugin's ABI to the runtime's.
    pub const INHERIT: &'static str = "inherit";

    /// Whether the plugin defers its ABI to the runtime (`cxx_abi: inherit`).
    pub fn is_inherit(&self) -> bool {
        matches!(self, CxxAbi::Scalar(s) if s == Self::INHERIT)
    }

    /// The concrete ABI tag declared for `os`, or `None` when the plugin defers to
    /// the runtime (`inherit`) or the per-OS map has no entry for that target.
    pub fn tag_for(&self, os: Option<Os>) -> Option<&str> {
        match self {
            CxxAbi::Scalar(s) if s == Self::INHERIT => None,
            CxxAbi::Scalar(s) => Some(s.as_str()),
            CxxAbi::PerOs(map) => os.and_then(|os| map.get(os_key(os)).map(String::as_str)),
        }
    }
}

/// The per-OS map key for a target OS.
fn os_key(os: Os) -> &'static str {
    match os {
        Os::Linux => "linux",
        Os::Macos => "macos",
        Os::Windows => "windows",
    }
}

/// The runtime the plugin targets: an OpenUSD version range plus an ABI tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeReq {
    /// OpenUSD version *range* the plugin tolerates, e.g. `>=24.11,<25.0`.
    pub openusd: String,
    /// C++ ABI the plugin was built against — a scalar tag, a per-OS map, or the
    /// `inherit` sentinel. See [`CxxAbi`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cxx_abi: Option<CxxAbi>,
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
    /// Extra bundle-relative directories containing runtime shared libraries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_libs: Vec<String>,
    /// Independently discoverable plugin bundles required by this bundle.
    /// Workspace validation resolves these by plugin identity before any build.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bundles: Vec<BundleDependency>,
}

/// A versioned dependency on another plugin bundle in the same composition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleDependency {
    pub id: String,
    pub version: String,
    /// Authored-data contract required from a schema bundle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<u64>,
}

/// Where the bundle's USD `plugInfo.json` lives, relative to the bundle root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsdSection {
    pub plug_info: String,
}

/// Schema-specific bundle settings; only meaningful when `plugin.kind` is
/// `usd-schema`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaSection {
    /// A *codeless* schema ships only resources — an enriched `plugInfo.json`
    /// with a `Types` block (and a `generatedSchema.usda`) — with no generated
    /// C++ and therefore no shared library. The L0 library checks
    /// (`plugin.shared_library`, `bundle.plug_info.library_path`) do not apply to
    /// it; doctor validates the schema `Types` block instead.
    #[serde(default)]
    pub codeless: bool,
    /// Bundle-relative path of the `schema.usda` source the schema build step
    /// (usdGenSchema) regenerates from, e.g. `schema/schema.usda`. Defaults to
    /// the conventional `schema.usda` at the bundle root when absent. Validated
    /// as bundle-relative at load (SEC-002). Meaningful for both `usd-schema`
    /// bundles and non-schema bundles that co-host a schema via
    /// `provides: usd-schema:<Type>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Version of the authored type/property/token surface exported by a public
    /// schema bundle. Independent from the plugin implementation version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<u64>,
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
    /// Optional for legacy standalone manifests. Required when
    /// `requires.bundles` opts into the versioned composition extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<PluginManifestHeader>,
    pub plugin: PluginIdentity,
    /// SPDX license expression for the plugin's own code, e.g. `Apache-2.0`.
    /// Surfaced by `ost plugin inspect` and recorded in `ost plugin package`'s
    /// artifact manifest, so a packaged bundle never ships without its license.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    pub runtime: RuntimeReq,
    /// Capabilities this plugin provides, e.g. `usd-fileformat:lumagraph`.
    #[serde(default)]
    pub provides: Vec<String>,
    #[serde(default)]
    pub requires: Requires,
    pub usd: UsdSection,
    /// Schema-specific settings; only meaningful when `plugin.kind` is
    /// `usd-schema`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<SchemaSection>,
    /// Bundle-relative paths to third-party notice/license files (e.g. for
    /// vendored dependencies). Validated as bundle-relative at load and copied
    /// into the package by `ost plugin package`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<String>,
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

    /// The schema type identifiers this bundle *declares* via `provides`
    /// (`usd-schema:<TypeName>`), e.g. `VrmHumanoidAPI`. This is the explicit,
    /// declaration-only signal — unlike inferring types from the `plugInfo.json`,
    /// it never mistakes a file-format plugin's own `Info.Types` for a schema — so
    /// it is the gate for whether a non-schema bundle *co-hosts* a schema.
    pub fn schema_provides(&self) -> Vec<&str> {
        self.provides
            .iter()
            .filter_map(|p| p.strip_prefix("usd-schema:"))
            .collect()
    }

    /// Whether this bundle is a *codeless* schema: `kind: usd-schema` with
    /// `schema.codeless: true`. Such a bundle has no shared library — its entire
    /// contribution is the `plugInfo.json` `Types` block — so the L0 library
    /// checks must be skipped and the schema `Types` validated instead.
    pub fn is_codeless_schema(&self) -> bool {
        self.kind() == PluginKind::UsdSchema
            && self.schema.as_ref().map(|s| s.codeless).unwrap_or(false)
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
license: Apache-2.0
runtime:
  openusd: ">=24.11,<25.0"
  cxx_abi: libcxx
provides:
  - usd-fileformat:lumagraph
requires:
  capabilities: [usd-stage-read]
  components: { materialx: ">=1.39,<1.40" }
  runtime_libs: [third_party/lib]
usd:
  plug_info: plugin/resources/usdluma/plugInfo.json
notices: [third_party/NOTICES.md]
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
        assert_eq!(m.runtime.cxx_abi, Some(CxxAbi::Scalar("libcxx".into())));
        assert_eq!(
            m.runtime.cxx_abi.as_ref().unwrap().tag_for(None),
            Some("libcxx")
        );
        assert_eq!(m.provides, vec!["usd-fileformat:lumagraph"]);
        assert_eq!(m.requires.capabilities, vec!["usd-stage-read"]);
        assert_eq!(
            m.requires.components.get("materialx").map(String::as_str),
            Some(">=1.39,<1.40")
        );
        assert_eq!(m.requires.runtime_libs, vec!["third_party/lib"]);
        assert!(m.requires.bundles.is_empty());
        assert_eq!(m.license.as_deref(), Some("Apache-2.0"));
        assert_eq!(m.notices, vec!["third_party/NOTICES.md"]);
    }

    #[test]
    fn license_and_notices_are_optional() {
        // A manifest without license/notices still parses; the fields default.
        let minimal = "plugin:\n  name: x\n  version: 0.1.0\n  kind: usd-fileformat\n\
                       runtime:\n  openusd: \">=24.11,<25.0\"\nusd:\n  plug_info: p/plugInfo.json\n";
        let m = PluginManifest::parse(minimal).expect("manifest parses");
        assert_eq!(m.license, None);
        assert!(m.notices.is_empty());
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
    fn cxx_abi_parses_scalar_per_os_map_and_inherit() {
        // Scalar: same tag for every target; resolves regardless of OS.
        let scalar: CxxAbi = serde_yaml::from_str("msvc143").unwrap();
        assert_eq!(scalar.tag_for(Some(Os::Windows)), Some("msvc143"));
        assert_eq!(scalar.tag_for(None), Some("msvc143"));
        assert!(!scalar.is_inherit());

        // Per-OS map: resolved against the target OS; None when not listed.
        let per_os: CxxAbi =
            serde_yaml::from_str("{ windows: msvc143, linux: libstdcxx }").unwrap();
        assert_eq!(per_os.tag_for(Some(Os::Windows)), Some("msvc143"));
        assert_eq!(per_os.tag_for(Some(Os::Linux)), Some("libstdcxx"));
        assert_eq!(per_os.tag_for(Some(Os::Macos)), None);
        assert_eq!(per_os.tag_for(None), None);

        // Inherit sentinel: defers to the runtime, so no concrete tag to compare.
        let inherit: CxxAbi = serde_yaml::from_str("inherit").unwrap();
        assert!(inherit.is_inherit());
        assert_eq!(inherit.tag_for(Some(Os::Windows)), None);
    }

    #[test]
    fn codeless_schema_is_detected_from_kind_and_flag() {
        let codeless = "plugin:\n  name: vrmSchema\n  version: 0.1.0\n  kind: usd-schema\n\
                        runtime:\n  openusd: \">=24.11,<27.0\"\nschema:\n  codeless: true\n\
                        usd:\n  plug_info: plugin/resources/vrmSchema/plugInfo.json\n";
        let m = PluginManifest::parse(codeless).expect("manifest parses");
        assert_eq!(m.kind(), PluginKind::UsdSchema);
        assert!(m.is_codeless_schema());

        // A schema without the flag is a *compiled* schema — the library checks apply.
        let compiled = "plugin:\n  name: vrmSchema\n  version: 0.1.0\n  kind: usd-schema\n\
                        runtime:\n  openusd: \">=24.11,<27.0\"\n\
                        usd:\n  plug_info: plugin/resources/vrmSchema/plugInfo.json\n";
        assert!(!PluginManifest::parse(compiled)
            .unwrap()
            .is_codeless_schema());

        // The flag only means codeless on a schema, never on a file-format plugin.
        let fmt = "plugin:\n  name: toy\n  version: 0.1.0\n  kind: usd-fileformat\n\
                   runtime:\n  openusd: \">=24.11,<27.0\"\nschema:\n  codeless: true\n\
                   usd:\n  plug_info: p/plugInfo.json\n";
        assert!(!PluginManifest::parse(fmt).unwrap().is_codeless_schema());
    }

    #[test]
    fn kind_round_trips_through_str() {
        for k in PluginKind::ALL {
            assert_eq!(PluginKind::from_tag(k.as_str()), Some(k));
        }
        assert_eq!(PluginKind::from_tag("bogus"), None);
    }

    #[test]
    fn parses_versioned_bundle_dependencies_and_schema_contract() {
        let source = r#"
manifest:
  schema: openstrata.plugin/v1alpha1
plugin: { name: consumer, version: 1.2.0, kind: usd-fileformat }
runtime: { openusd: ">=25.05,<27.0" }
requires:
  bundles:
    - { id: publicSchema, version: ">=2.0,<3.0", contract: 4 }
usd: { plug_info: resources/plugInfo.json }
schema: { contract: 4 }
"#;
        let manifest = PluginManifest::parse(source).expect("composition manifest parses");
        assert_eq!(manifest.manifest.unwrap().schema, PLUGIN_SCHEMA);
        assert_eq!(manifest.requires.bundles[0].id, "publicSchema");
        assert_eq!(manifest.requires.bundles[0].contract, Some(4));
        assert_eq!(manifest.schema.unwrap().contract, Some(4));
    }

    #[test]
    fn dependency_entries_reject_unknown_keys() {
        let source = r#"
plugin: { name: consumer, version: 1.0.0, kind: usd-fileformat }
runtime: { openusd: ">=25.05,<27.0" }
requires:
  bundles:
    - { id: schema, version: ">=1.0,<2.0", typo: true }
usd: { plug_info: resources/plugInfo.json }
"#;
        assert!(PluginManifest::parse(source).is_err());
    }
}
