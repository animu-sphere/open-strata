// SPDX-License-Identifier: Apache-2.0
//! `ost` — the OpenStrata command-line interface.
//!
//! Phase 0 surface area: `ost platform list|show|diff` and `ost init`. The CLI
//! deliberately stays thin — it parses arguments, calls into the domain crates,
//! and renders results either for humans or as JSON for CI (§13.2).

mod commands;
mod notify;
mod output;
mod progress;
mod project_template;

use clap::{Parser, Subcommand};

use commands::{
    artifact, build, ci, configure, devshell, doctor, env, extension, external, formation, init,
    internal, lock, package, platform, plugin, presets, renderer, runtime, test, uv, validate,
};

/// OpenStrata: VFX Reference Platform aware runtime, build and extension manager.
#[derive(Debug, Parser)]
#[command(name = "ost", version, about, long_about = None)]
struct Cli {
    /// Emit machine-readable JSON instead of human-formatted output.
    #[arg(long, global = true)]
    json: bool,

    /// Replace local filesystem and managed-environment values in JSON with
    /// stable placeholders suitable for attaching to a public report.
    #[arg(long, global = true, requires = "json")]
    redact_paths: bool,

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

    /// Run a built target's tests under the runtime that built it.
    Test(test::TestArgs),

    /// Install and pack a built target into a tar.zst artifact.
    Package(package::PackageArgs),

    /// Validate a built/packaged target.
    Validate(validate::ValidateArgs),

    /// Import and inspect provenance for a build OpenStrata did not perform.
    #[command(subcommand)]
    External(external::ExternalCmd),

    /// Inspect and request controlled extensions.
    #[command(subcommand)]
    Extension(extension::ExtensionCmd),

    /// Scaffold, inspect, build, and diagnose OpenUSD plugin bundles.
    #[command(subcommand)]
    Plugin(plugin::PluginCmd),

    /// Inspect renderer projects in host applications.
    #[command(subcommand)]
    Renderer(renderer::RendererCmd),

    /// Import, inspect, verify, export, and pull artifacts (local registry +
    /// remote OCI transport).
    #[command(subcommand)]
    Artifact(artifact::ArtifactCmd),

    /// Resolve, inspect, lock, and run digest-pinned component Formations.
    #[command(subcommand)]
    Formation(formation::FormationCmd),

    /// Manage the CI support matrix and generate CI configuration.
    #[command(subcommand)]
    Ci(ci::CiCmd),

    /// Generate or verify the project lockfile (strata.lock).
    Lock(lock::LockArgs),

    /// Run `uv` pinned to the project's runtime Python.
    Uv(uv::UvArgs),

    /// Internal developer tasks (hidden; e.g. regenerating reference docs).
    #[command(subcommand, hide = true)]
    Internal(internal::InternalCmd),
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let fmt = output::Format::from_flag(cli.json);
    output::set_redact_paths(cli.redact_paths);

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
        Command::Test(args) => test::run(args, fmt),
        Command::Package(args) => package::run(args, fmt),
        Command::Validate(args) => validate::run(args, fmt),
        Command::External(cmd) => external::run(cmd, fmt),
        Command::Extension(cmd) => extension::run(cmd, fmt),
        Command::Plugin(cmd) => plugin::run(cmd, fmt),
        Command::Renderer(cmd) => renderer::run(cmd, fmt),
        Command::Artifact(cmd) => artifact::run(cmd, fmt),
        Command::Formation(cmd) => formation::run(cmd, fmt),
        Command::Ci(cmd) => ci::run(cmd, fmt),
        Command::Lock(args) => lock::run(args, fmt),
        Command::Uv(args) => uv::run(args, fmt),
        Command::Internal(cmd) => internal::run(cmd, fmt),
    };

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            output::error(&err, fmt);
            // Deterministic, category-based exit for CI and agents (§13.2/§14.4).
            std::process::ExitCode::from(err.exit_code())
        }
    }
}
