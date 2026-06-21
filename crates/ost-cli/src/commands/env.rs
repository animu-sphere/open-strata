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

use ost_core::{Host, Result};

use crate::commands::resolve;
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
    let r = resolve(&args.platform, &args.profile)?;
    let shell = super::devshell::pick_shell(args.shell.as_deref(), Host::detect().os)?;

    if fmt.is_json() {
        // An ordered array, not a map: prepend order matters and keys can repeat
        // (on Windows both bin and lib land on PATH).
        let vars: Vec<serde_json::Value> = r
            .env
            .pairs()
            .into_iter()
            .map(|(k, v)| serde_json::json!({ "name": k, "value": v }))
            .collect();
        output::json(&serde_json::json!({
            "runtime": r.runtime.id(),
            "platform": r.runtime.platform,
            "profile": r.runtime.profile,
            "variant": r.runtime.variant.slug(),
            "prefix": r.prefix.to_string(),
            "pulled": r.pulled,
            "shell": format!("{shell:?}").to_lowercase(),
            "env": vars,
        }));
        return Ok(());
    }

    // Header comments are valid in both bash and pwsh, so `eval` stays safe.
    println!("# OpenStrata environment for {}", r.runtime.id());
    println!("# variant: {}", r.runtime.variant.slug());
    if !r.pulled {
        println!("# note: runtime not yet pulled; paths are prospective");
        println!(
            "#       (run `ost runtime pull {}` once available)",
            r.runtime.platform
        );
    }
    print!("{}", r.env.render(shell));
    Ok(())
}
