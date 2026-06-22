pub mod build;
pub mod configure;
pub mod devshell;
pub mod doctor;
pub mod env;
pub mod extension;
pub mod init;
pub mod lock;
pub mod package;
pub mod platform;
pub mod plugin;
pub mod runtime;
pub mod uv;
pub mod validate;

use camino::Utf8PathBuf;

use ost_core::paths::Store;
use ost_core::{Error, Host, Result};
use ost_platform::Catalog;
use ost_runtime::{
    python_minor, EnvSet, ProfileCatalog, Runtime, RuntimeManifest, MANIFEST_FILE,
};

/// Everything needed to activate a runtime, shared by `env`, `devshell`, `runtime`.
pub struct Resolved {
    pub runtime: Runtime,
    /// Store location of the runtime: where its `runtime.json` manifest lives.
    pub prefix: Utf8PathBuf,
    /// Where the runtime's real artifacts (bin/lib/python) live. Equals `prefix`
    /// for mock/build runtimes; for an adopted `local` runtime it is the external
    /// USD install root recorded in the manifest.
    pub artifact_prefix: Utf8PathBuf,
    pub env: EnvSet,
    /// Platform Python version, e.g. `3.13.x`.
    pub python_version: String,
    /// C++ standard from the platform, e.g. `20`.
    pub cxx_standard: String,
    /// Capabilities provided by the selected profile.
    pub capabilities: Vec<String>,
    /// Whether the runtime has been pulled (its manifest exists on disk).
    pub pulled: bool,
}

/// Resolve a platform + profile selection into a runtime and its environment.
///
/// This does not pull artifacts; the prefix is the prospective store location.
pub fn resolve(platform_id: &str, profile_id: &str) -> Result<Resolved> {
    let platforms = Catalog::load()?;
    let platform = platforms.get(platform_id)?;
    let python_version = platform.component("python").ok_or_else(|| {
        Error::InvalidManifest(format!(
            "platform '{}' does not define a 'python' version",
            platform.id
        ))
    })?;

    let profiles = ProfileCatalog::load()?;
    let profile = profiles.get(profile_id)?;

    let host = Host::detect();
    let runtime = Runtime::resolve(&platform.id, &profile.id, &host, python_version);
    let store = Store::discover();
    let prefix = runtime.prefix(&store);

    let capabilities = profile.capabilities().to_vec();
    let usd_plugins = capabilities.iter().any(|c| c.starts_with("usd"));
    let pulled = prefix.join(MANIFEST_FILE).as_std_path().is_file();

    // An adopted (`local`) runtime keeps its manifest in the store but its real
    // artifacts live at an external root with USD's own layout. Read the manifest
    // to learn the source, then build the env against the effective prefix.
    let manifest = if pulled {
        std::fs::read_to_string(prefix.join(MANIFEST_FILE).as_std_path())
            .ok()
            .and_then(|s| RuntimeManifest::from_json(&s).ok())
    } else {
        None
    };

    // Real OpenUSD runtimes (adopted `local`, built `build`, fetched `artifact`)
    // carry USD's own install layout; the mock backend uses the OpenStrata layout.
    let (artifact_prefix, env) = match &manifest {
        Some(m) if m.source.is_real() => {
            let ep = Utf8PathBuf::from(m.effective_prefix(&prefix));
            let mut env = EnvSet::for_usd_install(&ep, host.os);
            // A build linked against external deps needs their lib dirs at runtime.
            for dep in &m.runtime_deps {
                env.add_dep_libs(camino::Utf8Path::new(dep), host.os);
            }
            (ep, env)
        }
        _ => {
            let env =
                EnvSet::for_runtime(&prefix, host.os, &python_minor(python_version), usd_plugins);
            (prefix.clone(), env)
        }
    };

    let cxx_standard = platform
        .component("cxx_standard")
        .unwrap_or("17")
        .to_string();

    Ok(Resolved {
        runtime,
        prefix,
        artifact_prefix,
        env,
        python_version: python_version.to_string(),
        cxx_standard,
        capabilities,
        pulled,
    })
}
