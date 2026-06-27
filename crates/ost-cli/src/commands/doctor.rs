// SPDX-License-Identifier: Apache-2.0
//! `ost doctor` — diagnose host, tools, and (optionally) a runtime (§14.2).
//!
//! General health check: the OpenStrata install, the host descriptor, and host
//! build tools. When a platform is given it also reports the resolved runtime —
//! identity, pulled state, digest, validation, environment, and layout. USD-
//! specific diagnostics (`ost doctor usd`) arrive with the plugin phase.
//!
//! Exit code is deterministic for CI: `0` when healthy, the precondition code
//! (§14.4) when issues are found (a required tool is missing, or a requested
//! runtime is not pulled).

use clap::Args;

use ost_core::paths::Store;
use ost_core::variant::Abi;
use ost_core::{tools, Host, Result};
use ost_runtime::{RuntimeManifest, MANIFEST_FILE};

use crate::commands::resolve;
use crate::output::{self, Format};

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Optional platform to diagnose a specific runtime, e.g. `cy2026`.
    platform: Option<String>,

    /// Profile to diagnose (only with a platform).
    #[arg(long, default_value = "core")]
    profile: String,
}

/// Host tools we look for. `required` ones count as issues when missing.
const TOOLS: &[(&str, &str, bool)] = &[
    ("cmake", "build configure", true),
    ("ninja", "build execution", true),
    ("git", "workspace history", false),
    ("uv", "python dependencies", false),
    ("bash", "devshell", false),
    ("pwsh", "devshell", false),
];

struct ToolStatus {
    name: &'static str,
    role: &'static str,
    required: bool,
    path: Option<String>,
}

pub fn run(args: DoctorArgs, fmt: Format) -> Result<()> {
    let host = Host::detect();
    let abi = Abi::default_for(host.os);
    let store = Store::discover();

    let tools: Vec<ToolStatus> = TOOLS
        .iter()
        .map(|(name, role, required)| ToolStatus {
            name,
            role,
            required: *required,
            path: tools::which(name).map(|p| p.display().to_string()),
        })
        .collect();

    let mut issues: Vec<String> = Vec::new();
    for t in &tools {
        if t.required && t.path.is_none() {
            issues.push(format!("required tool '{}' not found on PATH", t.name));
        }
    }

    // Optional runtime section.
    let runtime_section = match &args.platform {
        Some(platform) => {
            let r = resolve(platform, &args.profile)?;
            if !r.pulled {
                issues.push(format!(
                    "runtime '{}' not pulled (run `ost runtime pull {} --profile {}`)",
                    r.runtime.id(),
                    platform,
                    args.profile
                ));
            }
            Some(build_runtime_report(&r))
        }
        None => None,
    };

    if fmt.is_json() {
        emit_json(
            &host,
            &abi,
            &store,
            &tools,
            runtime_section.as_ref(),
            &issues,
        );
    } else {
        emit_human(
            &host,
            &abi,
            &store,
            &tools,
            runtime_section.as_ref(),
            &issues,
        );
    }

    // Deterministic exit for CI. Issues are missing tools or an unpulled
    // runtime — preconditions (§14.4); the report above is this command's own
    // output, so exit with that category code directly.
    if issues.is_empty() {
        Ok(())
    } else {
        std::process::exit(ost_core::Category::Precondition.exit_code() as i32);
    }
}

/// A computed runtime diagnostic.
struct RuntimeReport {
    id: String,
    variant: String,
    pulled: bool,
    digest: Option<String>,
    validation: Option<String>,
    capabilities: Vec<String>,
    env_keys: Vec<String>,
    layout: Vec<(String, bool)>,
    prefix: String,
}

fn build_runtime_report(r: &crate::commands::Resolved) -> RuntimeReport {
    let mut digest = None;
    let mut validation = None;
    let mut layout = Vec::new();

    let manifest_path = r.prefix.join(MANIFEST_FILE);
    if let Ok(src) = std::fs::read_to_string(manifest_path.as_std_path()) {
        if let Ok(m) = RuntimeManifest::from_json(&src) {
            digest = Some(m.digest.clone());
            validation = Some(format!("{:?}", m.validation).to_lowercase());
            // Layout dirs live under the effective artifact prefix (the external
            // USD root for an adopted runtime).
            for sub in &m.layout {
                let exists = r.artifact_prefix.join(sub).as_std_path().is_dir();
                layout.push((sub.clone(), exists));
            }
        }
    }

    RuntimeReport {
        id: r.runtime.id(),
        variant: r.runtime.variant.slug(),
        pulled: r.pulled,
        digest,
        validation,
        capabilities: r.capabilities.clone(),
        env_keys: r.env.pairs().into_iter().map(|(k, _)| k).collect(),
        layout,
        prefix: r.prefix.to_string(),
    }
}

fn emit_human(
    host: &Host,
    abi: &Abi,
    store: &Store,
    tools: &[ToolStatus],
    runtime: Option<&RuntimeReport>,
    issues: &[String],
) {
    println!("OpenStrata");
    println!("  version: {}", env!("CARGO_PKG_VERSION"));
    println!("  store:   {}", store.root);

    println!("\nHost");
    println!("  os:      {}", host.os.as_str());
    println!("  arch:    {}", host.arch.as_str());
    println!("  abi:     {}", abi.describe());
    println!(
        "  target:  {}",
        if host.is_primary() {
            "primary (first-class)"
        } else {
            "secondary (modeled; may be unavailable)"
        }
    );

    println!("\nTools");
    let width = tools.iter().map(|t| t.name.len()).max().unwrap_or(0);
    for t in tools {
        let mark = if t.path.is_some() { "ok " } else { "MISS" };
        let detail = t.path.clone().unwrap_or_else(|| {
            if t.required {
                format!("not found — needed for {}", t.role)
            } else {
                format!("not found ({})", t.role)
            }
        });
        println!("  [{mark}] {:<width$}  {detail}", t.name);
    }

    if let Some(rt) = runtime {
        println!("\nRuntime");
        println!("  id:         {}", rt.id);
        println!("  variant:    {}", rt.variant);
        println!("  pulled:     {}", rt.pulled);
        if let Some(d) = &rt.digest {
            println!("  digest:     {d}");
        }
        if let Some(v) = &rt.validation {
            println!("  validation: {v}");
        }
        println!("  prefix:     {}", rt.prefix);
        if !rt.capabilities.is_empty() {
            println!("  capabilities: {}", rt.capabilities.join(", "));
        }
        println!("  env vars:   {}", rt.env_keys.join(", "));
        if !rt.layout.is_empty() {
            println!("  layout:");
            for (dir, exists) in &rt.layout {
                let mark = if *exists { "ok " } else { "MISS" };
                println!("    [{mark}] {dir}");
            }
        }
    }

    println!();
    if issues.is_empty() {
        println!("No issues found.");
    } else {
        println!("Issues ({}):", issues.len());
        for issue in issues {
            println!("  - {issue}");
        }
    }
}

fn emit_json(
    host: &Host,
    abi: &Abi,
    store: &Store,
    tools: &[ToolStatus],
    runtime: Option<&RuntimeReport>,
    issues: &[String],
) {
    let tool_items: Vec<_> = tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "role": t.role,
                "required": t.required,
                "found": t.path.is_some(),
                "path": t.path,
            })
        })
        .collect();

    let runtime_json = runtime.map(|rt| {
        let layout: Vec<_> = rt
            .layout
            .iter()
            .map(|(dir, exists)| serde_json::json!({ "dir": dir, "exists": exists }))
            .collect();
        serde_json::json!({
            "id": rt.id,
            "variant": rt.variant,
            "pulled": rt.pulled,
            "digest": rt.digest,
            "validation": rt.validation,
            "prefix": rt.prefix,
            "capabilities": rt.capabilities,
            "env_keys": rt.env_keys,
            "layout": layout,
        })
    });

    output::report(
        issues.is_empty(),
        &serde_json::json!({
            "openstrata": {
                "version": env!("CARGO_PKG_VERSION"),
                "store": store.root.to_string(),
            },
            "host": {
                "os": host.os.as_str(),
                "arch": host.arch.as_str(),
                "abi": abi.describe(),
                "primary": host.is_primary(),
            },
            "tools": tool_items,
            "runtime": runtime_json,
            "issues": issues,
        }),
    );
}
