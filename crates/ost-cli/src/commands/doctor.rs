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

    /// The capability you intend to exercise, e.g. `usd-stage-read`. Advice is
    /// scoped to it: without an OpenUSD-dependent capability — in the profile or
    /// named here — `doctor` does not tell you to go and find a real OpenUSD.
    #[arg(long = "capability")]
    capabilities: Vec<String>,
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

/// Severity of a diagnostic issue. Only [`Severity::Error`] fails the run;
/// [`Severity::Warning`] is informational and keeps the exit code at `0`
/// (§14.5: "情報的な warning のみなら exit 0").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    Error,
    Warning,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// One structured diagnostic (§14.5): a stable `id`, a `severity`, a human
/// `summary`, and the `next_action` to take (the command to run, when there is
/// one).
struct Issue {
    id: &'static str,
    severity: Severity,
    summary: String,
    next_action: Option<String>,
    /// Why this matters for what the caller is actually doing — in particular,
    /// what a real runtime would change about the result. A `next_action`
    /// without this reads as an unconditional instruction; on the core profile
    /// that produced the standing advice to go and find an OpenUSD install for a
    /// capability nobody asked for.
    explanation: Option<String>,
}

impl Issue {
    fn error(id: &'static str, summary: impl Into<String>, next_action: Option<String>) -> Issue {
        Issue {
            id,
            severity: Severity::Error,
            summary: summary.into(),
            next_action,
            explanation: None,
        }
    }

    fn warning(id: &'static str, summary: impl Into<String>, next_action: Option<String>) -> Issue {
        Issue {
            id,
            severity: Severity::Warning,
            summary: summary.into(),
            next_action,
            explanation: None,
        }
    }

    fn explaining(mut self, explanation: impl Into<String>) -> Issue {
        self.explanation = Some(explanation.into());
        self
    }
}

/// Whether anything in play actually depends on a real OpenUSD.
///
/// Both the resolved profile's own capabilities and any the caller named count.
/// A `core` profile promises no OpenUSD execution, so a mock runtime is not a
/// deficiency there — it is the profile working as specified.
fn wants_openusd(profile_capabilities: &[String], requested: &[String]) -> bool {
    profile_capabilities
        .iter()
        .chain(requested.iter())
        .any(|capability| capability.starts_with("usd"))
}

/// Whether any issue is an error (warnings do not fail the run).
fn has_errors(issues: &[Issue]) -> bool {
    issues.iter().any(|i| i.severity == Severity::Error)
}

/// Map a runtime backend `source` to its user-facing *kind* (§14.5):
/// `mock` / `adopted` (`local`) / `built` (`build`) / `downloaded` (`artifact`).
fn runtime_kind(source: &str) -> &'static str {
    match source {
        "mock" => "mock",
        "local" => "adopted",
        "build" => "built",
        "artifact" => "downloaded",
        _ => "unknown",
    }
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

    let mut issues: Vec<Issue> = Vec::new();
    for t in &tools {
        if t.required && t.path.is_none() {
            issues.push(Issue::error(
                "tool.missing",
                format!("required tool '{}' not found on PATH", t.name),
                Some(format!("install {} and ensure it is on PATH", t.name)),
            ));
        }
    }

    // Optional runtime section.
    let runtime_section = match &args.platform {
        Some(platform) => {
            let r = resolve(platform, &args.profile)?;
            let report = build_runtime_report(&r);
            if !r.pulled {
                issues.push(Issue::error(
                    "runtime.not_pulled",
                    format!("runtime '{}' not pulled", r.runtime.id()),
                    Some(format!(
                        "ost runtime pull {platform} --profile {}",
                        args.profile
                    )),
                ));
            } else if report.kind.as_deref() == Some("mock") {
                // A mock runtime is healthy but can only drive static validation;
                // execution-type checks (`ost plugin test` L2+) need a real one.
                // Informational — does not fail the run (§14.5).
                //
                // Whether that is worth acting on depends entirely on what is
                // being exercised. Telling someone on the `core` profile to go
                // and find an OpenUSD install advises them to fix a limitation
                // their profile never claimed not to have.
                let openusd_matters = wants_openusd(&r.capabilities, &args.capabilities);
                let issue = Issue::warning(
                    "MOCK_RUNTIME_ACTIVE",
                    format!(
                        "runtime '{}' is a mock — static validation only, no real OpenUSD execution",
                        r.runtime.id()
                    ),
                    openusd_matters.then(|| {
                        format!(
                            "ost runtime pull {platform} --profile {} --from-usd <path>",
                            args.profile
                        )
                    }),
                );
                issues.push(if openusd_matters {
                    issue.explaining(
                        "a real runtime would let OpenUSD-dependent checks actually execute: \
                         plugin discovery, stage open and the L2+ levels currently report SKIP \
                         because there is no OpenUSD to run them against",
                    )
                } else {
                    issue.explaining(format!(
                        "nothing selected depends on OpenUSD — the '{}' profile exercises no \
                         usd* capability — so a real runtime would not change any result here; \
                         pass --capability <name> if you intend to exercise one",
                        args.profile
                    ))
                });
            }
            // A recorded OpenUSD version that disagrees with the install's pxr.h is
            // stale — it skews the L1 range check. Warning, not error (§14.5).
            if let Some((recorded, real)) = &report.openusd_drift {
                issues.push(Issue::warning(
                    "OPENUSD_VERSION_STALE",
                    format!(
                        "manifest records OpenUSD {recorded}, but the install's pxr.h is {real} \
                         — the version gate may not reflect the real runtime"
                    ),
                    Some(format!(
                        "ost runtime pull {platform} --profile {} --from-usd <usd-root> --force",
                        args.profile
                    )),
                ));
            }
            Some(report)
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

    // Deterministic exit for CI. Error issues are missing tools or an unpulled
    // runtime — preconditions (§14.4); the report above is this command's own
    // output, so exit with that category code directly. Warning-only runs
    // (e.g. an active mock runtime) stay at exit 0 (§14.5).
    if has_errors(&issues) {
        std::process::exit(ost_core::Category::Precondition.exit_code() as i32);
    } else {
        Ok(())
    }
}

/// A computed runtime diagnostic.
struct RuntimeReport {
    id: String,
    variant: String,
    pulled: bool,
    /// User-facing runtime kind (mock/adopted/built/downloaded), or `None` until
    /// the manifest is read (e.g. an unpulled runtime).
    kind: Option<String>,
    /// Whether this runtime can execute real OpenUSD (anything but `mock`), so
    /// callers know whether `ost plugin test` L2+ can run against it.
    executes_real: Option<bool>,
    digest: Option<String>,
    validation: Option<String>,
    /// `(recorded, real)` OpenUSD versions when the manifest disagrees with the
    /// install's `pxr.h` — a stale recorded version that would skew the L1 gate.
    openusd_drift: Option<(String, String)>,
    capabilities: Vec<String>,
    env_keys: Vec<String>,
    layout: Vec<(String, bool)>,
    prefix: String,
}

impl RuntimeReport {
    /// One-word execution capability for display: what the `kind` can drive.
    fn execution(&self) -> &'static str {
        match self.executes_real {
            Some(true) => "real OpenUSD execution",
            Some(false) => "static validation only",
            None => "unknown",
        }
    }
}

fn build_runtime_report(r: &crate::commands::Resolved) -> RuntimeReport {
    let mut kind = None;
    let mut executes_real = None;
    let mut digest = None;
    let mut validation = None;
    let mut openusd_drift = None;
    let mut layout = Vec::new();

    let manifest_path = r.prefix.join(MANIFEST_FILE);
    if let Ok(src) = std::fs::read_to_string(manifest_path.as_std_path()) {
        if let Ok(m) = RuntimeManifest::from_json(&src) {
            kind = Some(runtime_kind(m.source.as_str()).to_string());
            executes_real = Some(m.source.is_real());
            digest = Some(m.digest.clone());
            validation = Some(format!("{:?}", m.validation).to_lowercase());
            openusd_drift = crate::commands::runtime::openusd_version_drift(&m, &r.artifact_prefix);
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
        kind,
        executes_real,
        openusd_drift,
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
    issues: &[Issue],
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
        if let Some(kind) = &rt.kind {
            println!("  kind:       {kind} ({})", rt.execution());
        }
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
        let errors = issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .count();
        let warnings = issues.len() - errors;
        println!("Issues ({errors} error(s), {warnings} warning(s)):");
        for issue in issues {
            println!(
                "  [{}] {}: {}",
                issue.severity.as_str(),
                issue.id,
                issue.summary
            );
            if let Some(explanation) = &issue.explanation {
                println!("        why: {explanation}");
            }
            if let Some(action) = &issue.next_action {
                println!("        ↳ {action}");
            }
        }
    }
}

fn emit_json(
    host: &Host,
    abi: &Abi,
    store: &Store,
    tools: &[ToolStatus],
    runtime: Option<&RuntimeReport>,
    issues: &[Issue],
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
            "kind": rt.kind,
            "executes_real": rt.executes_real,
            "execution": rt.execution(),
            "digest": rt.digest,
            "validation": rt.validation,
            "prefix": rt.prefix,
            "capabilities": rt.capabilities,
            "env_keys": rt.env_keys,
            "layout": layout,
        })
    });

    let issue_items: Vec<_> = issues
        .iter()
        .map(|i| {
            serde_json::json!({
                "id": i.id,
                "severity": i.severity.as_str(),
                "summary": i.summary,
                "next_action": i.next_action,
                // Why the issue matters here, and what a real runtime would
                // change — a null `next_action` with an explanation means
                // "nothing to do for what you selected", which is a different
                // statement from having no advice to give.
                "explanation": i.explanation,
            })
        })
        .collect();

    output::report(
        !has_errors(issues),
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
            "issues": issue_items,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_kind_maps_source_to_user_facing_label() {
        assert_eq!(runtime_kind("mock"), "mock");
        assert_eq!(runtime_kind("local"), "adopted");
        assert_eq!(runtime_kind("build"), "built");
        assert_eq!(runtime_kind("artifact"), "downloaded");
        assert_eq!(runtime_kind("nonsense"), "unknown");
    }

    #[test]
    fn only_errors_fail_the_run() {
        // A warning-only run (e.g. an active mock runtime) stays healthy (exit 0).
        let warnings = vec![Issue::warning("MOCK_RUNTIME_ACTIVE", "mock", None)];
        assert!(!has_errors(&warnings));

        // Any error makes the run fail.
        let mixed = vec![
            Issue::warning("MOCK_RUNTIME_ACTIVE", "mock", None),
            Issue::error(
                "tool.missing",
                "cmake missing",
                Some("install cmake".into()),
            ),
        ];
        assert!(has_errors(&mixed));

        assert!(!has_errors(&[]));
    }

    /// Advice must follow what is actually being exercised. The core profile
    /// promises no OpenUSD execution, so a mock runtime there is the profile
    /// working as specified — not a deficiency to send someone hunting an
    /// install for.
    #[test]
    fn openusd_advice_follows_the_profile_and_the_requested_capability() {
        let core = vec!["build-cxx".to_string()];
        let usd = vec!["usd-stage-read".to_string()];

        assert!(!wants_openusd(&core, &[]), "core alone needs no OpenUSD");
        assert!(wants_openusd(&usd, &[]), "a usd* profile does");
        // An explicit request counts even when the profile does not provide it:
        // the caller has said what they intend to do.
        assert!(
            wants_openusd(&core, &["usd-stage-read".to_string()]),
            "a requested usd* capability makes it relevant"
        );
        // …and a non-OpenUSD request does not.
        assert!(!wants_openusd(&core, &["build-cxx".to_string()]));
    }

    /// A warning with no action must still say why there is nothing to do —
    /// silence there reads as missing advice rather than a considered verdict.
    #[test]
    fn an_issue_can_explain_itself_without_prescribing_an_action() {
        let issue = Issue::warning("MOCK_RUNTIME_ACTIVE", "mock runtime", None)
            .explaining("nothing selected depends on OpenUSD");
        assert!(issue.next_action.is_none());
        assert_eq!(
            issue.explanation.as_deref(),
            Some("nothing selected depends on OpenUSD")
        );
        // A warning never fails the run (§14.5).
        assert!(!has_errors(&[issue]));
    }
}
