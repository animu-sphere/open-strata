//! `ost devshell` — enter an interactive shell with a runtime activated (§14.2).
//!
//! Unlike `ost env` (which only prints), `devshell` launches a child shell with
//! the resolved environment applied to its process environment, then propagates
//! the shell's exit code. The runtime prefix is prospective until
//! `ost runtime pull` lands, so we warn but still launch — useful for inspecting
//! the generated environment.

use clap::Args;

use ost_core::host::Os;
use ost_core::{Error, Host, Result};
use ost_runtime::Shell;

use crate::commands::resolve;

#[derive(Debug, Args)]
pub struct DevshellArgs {
    /// Platform calendar-year id, e.g. `cy2026`.
    platform: String,

    /// Profile to activate, e.g. `usd` or `lookdev`.
    #[arg(long, default_value = "core")]
    profile: String,

    /// Shell to launch. Defaults to the host's conventional shell.
    #[arg(long)]
    shell: Option<String>,
}

/// Resolve a shell from an optional name, falling back to the host default.
pub fn pick_shell(name: Option<&str>, os: Os) -> Result<Shell> {
    match name {
        Some(n) => Shell::from_name(n).ok_or_else(|| {
            Error::InvalidManifest(format!("unknown shell '{n}' (expected bash or pwsh)"))
        }),
        None => Ok(Shell::default_for(os)),
    }
}

pub fn run(args: DevshellArgs) -> Result<()> {
    let r = resolve(&args.platform, &args.profile)?;
    let host = Host::detect();
    let shell = pick_shell(args.shell.as_deref(), host.os)?;

    // Diagnostics go to stderr; stdout belongs to the interactive session.
    eprintln!("Entering OpenStrata devshell: {}", r.runtime.id());
    eprintln!("  variant: {}", r.runtime.variant.slug());
    eprintln!("  prefix:  {}", r.prefix);
    if !r.pulled {
        eprintln!("  warning: runtime not yet pulled; prefix does not exist on disk yet");
    }
    if std::env::var_os("OST_DEVSHELL").is_some() {
        eprintln!("  warning: already inside a devshell; environments will nest");
    }
    eprintln!("  type `exit` to leave.");

    let (program, program_args) = shell.launch_command();
    let mut cmd = std::process::Command::new(program);
    cmd.args(program_args);
    r.env.apply(&mut cmd);
    // Markers so tools (and the user's prompt) can detect the active runtime.
    cmd.env("OST_DEVSHELL", r.runtime.id());
    cmd.env("OST_RUNTIME_PREFIX", r.prefix.to_string());

    let status = cmd.status().map_err(|e| {
        Error::io(
            format!("launch shell '{program}' (is it installed and on PATH?)"),
            e,
        )
    })?;

    // Faithfully propagate the shell's exit code.
    std::process::exit(status.code().unwrap_or(1));
}
