//! `ost env` — print the environment that activates a runtime (§14.2).
//!
//! Output is pure, evaluable shell so it composes:
//!
//! ```bash
//! eval "$(ost env cy2026 --profile usd)"
//! ```
//!
//! The first vertical slice does not pull artifacts; the printed paths are the
//! prospective layout under `~/.ost/runtimes/<id>`. A header comment makes that
//! explicit without breaking `eval`.

use clap::Args;

use ost_core::paths::Store;
use ost_core::{Error, Host, Result};
use ost_platform::Catalog;
use ost_runtime::{python_minor, EnvSet, ProfileCatalog, Runtime, Shell};

use crate::output::{self, Format};

#[derive(Debug, Args)]
pub struct EnvArgs {
    /// Platform calendar-year id, e.g. `cy2026`.
    platform: String,

    /// Profile to activate, e.g. `usd` or `lookdev`.
    #[arg(long, default_value = "core")]
    profile: String,

    /// Target shell. Defaults to the host's conventional shell.
    #[arg(long)]
    shell: Option<String>,
}

pub fn run(args: EnvArgs, fmt: Format) -> Result<()> {
    let platforms = Catalog::load()?;
    let platform = platforms.get(&args.platform)?;
    let python_version = platform.component("python").ok_or_else(|| {
        Error::InvalidManifest(format!(
            "platform '{}' does not define a 'python' version",
            platform.id
        ))
    })?;

    let profiles = ProfileCatalog::load()?;
    let profile = profiles.get(&args.profile)?;

    let host = Host::detect();
    let shell = match &args.shell {
        Some(name) => Shell::from_name(name)
            .ok_or_else(|| Error::InvalidManifest(format!("unknown shell '{name}'")))?,
        None => Shell::default_for(host.os),
    };

    let runtime = Runtime::resolve(&platform.id, &profile.id, &host, python_version);
    let store = Store::discover();
    let prefix = runtime.prefix(&store);

    let usd_plugins = profile
        .capabilities()
        .iter()
        .any(|c| c.starts_with("usd"));
    let env = EnvSet::for_runtime(&prefix, host.os, &python_minor(python_version), usd_plugins);

    let pulled = prefix.as_std_path().is_dir();

    if fmt.is_json() {
        // An ordered array, not a map: prepend order matters and keys can repeat
        // (on Windows both bin and lib land on PATH).
        let vars: Vec<serde_json::Value> = env
            .pairs()
            .into_iter()
            .map(|(k, v)| serde_json::json!({ "name": k, "value": v }))
            .collect();
        output::json(&serde_json::json!({
            "runtime": runtime.id(),
            "platform": runtime.platform,
            "profile": runtime.profile,
            "variant": runtime.variant.slug(),
            "prefix": prefix.to_string(),
            "pulled": pulled,
            "shell": format!("{shell:?}").to_lowercase(),
            "env": vars,
        }));
        return Ok(());
    }

    // Header comments are valid in both bash and pwsh, so `eval` stays safe.
    println!("# OpenStrata environment for {}", runtime.id());
    println!("# variant: {}", runtime.variant.slug());
    if !pulled {
        println!("# note: runtime not yet pulled; paths are prospective");
        println!("#       (run `ost runtime pull {}` once available)", runtime.platform);
    }
    print!("{}", env.render(shell));
    Ok(())
}
