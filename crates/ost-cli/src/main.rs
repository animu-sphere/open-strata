//! `ost` — the OpenStrata command-line interface.
//!
//! Phase 0 surface area: `ost platform list|show|diff` and `ost init`. The CLI
//! deliberately stays thin — it parses arguments, calls into the domain crates,
//! and renders results either for humans or as JSON for CI (§13.2).

mod commands;
mod output;

use clap::{Parser, Subcommand};

use commands::{env, init, platform};

/// OpenStrata: VFX Reference Platform aware runtime, build and extension manager.
#[derive(Debug, Parser)]
#[command(name = "ost", version, about, long_about = None)]
struct Cli {
    /// Emit machine-readable JSON instead of human-formatted output.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect VFX Reference Platform calendar-year definitions.
    #[command(subcommand)]
    Platform(platform::PlatformCmd),

    /// Initialise an OpenStrata project in the current directory.
    Init(init::InitArgs),

    /// Print the environment that activates a runtime (for `eval`).
    Env(env::EnvArgs),
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let fmt = output::Format::from_flag(cli.json);

    let result = match cli.command {
        Command::Platform(cmd) => platform::run(cmd, fmt),
        Command::Init(args) => init::run(args, fmt),
        Command::Env(args) => env::run(args, fmt),
    };

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            output::error(&err, fmt);
            // Deterministic non-zero exit for CI (§13.2).
            std::process::ExitCode::FAILURE
        }
    }
}
