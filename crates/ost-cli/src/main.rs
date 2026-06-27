// SPDX-License-Identifier: Apache-2.0
//! `ost` — the OpenStrata command-line interface.
//!
//! Phase 0 surface area: `ost platform list|show|diff` and `ost init`. The CLI
//! deliberately stays thin — it parses arguments, calls into the domain crates,
//! and renders results either for humans or as JSON for CI (§13.2).

mod commands;
mod output;
mod progress;
mod project_template;

use clap::{Parser, Subcommand};

use commands::{
    build, configure, devshell, doctor, env, extension, init, lock, package, platform, plugin,
    presets, runtime, uv, validate,
};

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

    /// Pull, list, and inspect runtimes in the local store.
    #[command(subcommand)]
    Runtime(runtime::RuntimeCmd),

    /// Initialise an OpenStrata project in the current directory.
    Init(init::InitArgs),

    /// Print the environment that activates a runtime (for `eval`).
    Env(env::EnvArgs),

    /// Enter an interactive shell with a runtime activated.
    Devshell(devshell::DevshellArgs),

    /// Diagnose host, tools, and (optionally) a runtime.
    Doctor(doctor::DoctorArgs),

    /// Generate CMake toolchain and presets for a target.
    Configure(configure::ConfigureArgs),

    /// Manage OpenStrata's CMake preset includes in CMakePresets.json.
    #[command(subcommand)]
    Presets(presets::PresetsCmd),

    /// Configure and build a target with CMake + Ninja.
    Build(build::BuildArgs),

    /// Install and pack a built target into a tar.zst artifact.
    Package(package::PackageArgs),

    /// Validate a built/packaged target.
    Validate(validate::ValidateArgs),

    /// Inspect and request controlled extensions.
    #[command(subcommand)]
    Extension(extension::ExtensionCmd),

    /// Scaffold, inspect, build, and diagnose OpenUSD plugin bundles.
    #[command(subcommand)]
    Plugin(plugin::PluginCmd),

    /// Generate or verify the project lockfile (strata.lock).
    Lock(lock::LockArgs),

    /// Run `uv` pinned to the project's runtime Python.
    Uv(uv::UvArgs),
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let fmt = output::Format::from_flag(cli.json);

    let result = match cli.command {
        Command::Platform(cmd) => platform::run(cmd, fmt),
        Command::Runtime(cmd) => runtime::run(cmd, fmt),
        Command::Init(args) => init::run(args, fmt),
        Command::Env(args) => env::run(args, fmt),
        Command::Devshell(args) => devshell::run(args),
        Command::Doctor(args) => doctor::run(args, fmt),
        Command::Configure(args) => configure::run(args, fmt),
        Command::Presets(cmd) => presets::run(cmd, fmt),
        Command::Build(args) => build::run(args, fmt),
        Command::Package(args) => package::run(args, fmt),
        Command::Validate(args) => validate::run(args, fmt),
        Command::Extension(cmd) => extension::run(cmd, fmt),
        Command::Plugin(cmd) => plugin::run(cmd, fmt),
        Command::Lock(args) => lock::run(args, fmt),
        Command::Uv(args) => uv::run(args, fmt),
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
