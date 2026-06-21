pub mod devshell;
pub mod env;
pub mod init;
pub mod platform;

use camino::Utf8PathBuf;

use ost_core::paths::Store;
use ost_core::{Error, Host, Result};
use ost_platform::Catalog;
use ost_runtime::{python_minor, EnvSet, ProfileCatalog, Runtime};

/// Everything needed to activate a runtime, shared by `env` and `devshell`.
pub struct Resolved {
    pub runtime: Runtime,
    pub prefix: Utf8PathBuf,
    pub env: EnvSet,
    /// Whether the runtime prefix actually exists on disk yet.
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

    let usd_plugins = profile.capabilities().iter().any(|c| c.starts_with("usd"));
    let env = EnvSet::for_runtime(&prefix, host.os, &python_minor(python_version), usd_plugins);
    let pulled = prefix.as_std_path().is_dir();

    Ok(Resolved {
        runtime,
        prefix,
        env,
        pulled,
    })
}
