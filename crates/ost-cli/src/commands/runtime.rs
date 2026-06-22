//! `ost runtime` — pull / list / show runtimes (§14.2).
//!
//! The first backend is local/mock: `pull` materializes the prospective prefix
//! layout under `~/.ost/runtimes/<id>` and writes a digest-bearing
//! `runtime.json`. This makes `ost env` / `ost devshell` report the runtime as
//! pulled and is the seam where a real artifact backend slots in later.

use std::time::{SystemTime, UNIX_EPOCH};

use clap::Subcommand;

use camino::Utf8PathBuf;

use ost_core::paths::Store;
use ost_core::{Error, Result};
use ost_runtime::{
    python_minor, ExtensionRecord, RuntimeManifest, RuntimeSource, Validation, MANIFEST_FILE,
};

use crate::commands::resolve;
use crate::output::{self, Format};

#[derive(Debug, Subcommand)]
pub enum RuntimeCmd {
    /// Materialize a runtime into the local store.
    Pull {
        /// Platform calendar-year id, e.g. `cy2026`.
        platform: String,
        /// Profile to pull, e.g. `usd` or `lookdev`.
        #[arg(long, default_value = "core")]
        profile: String,
        /// Re-pull even if the runtime already exists.
        #[arg(long)]
        force: bool,
        /// Adopt an existing OpenUSD install at this path instead of
        /// materializing a mock layout (`local` source). Falls back to
        /// `OST_USD_ROOT` when unset.
        #[arg(long)]
        from_usd: Option<String>,
    },
    /// List runtimes present in the local store.
    List,
    /// Show the manifest of a pulled runtime.
    Show {
        /// Platform calendar-year id, e.g. `cy2026`.
        platform: String,
        /// Profile, e.g. `usd`.
        #[arg(long, default_value = "core")]
        profile: String,
    },
    /// Validate a pulled runtime and record the outcome in its manifest.
    Validate {
        /// Platform calendar-year id, e.g. `cy2026`.
        platform: String,
        /// Profile, e.g. `usd`.
        #[arg(long, default_value = "core")]
        profile: String,
    },
    /// Explain how a profile resolves to capabilities and extensions.
    Explain {
        /// Platform calendar-year id, e.g. `cy2026`.
        platform: String,
        /// Profile, e.g. `lookdev`.
        #[arg(long, default_value = "core")]
        profile: String,
    },
}

pub fn run(cmd: RuntimeCmd, fmt: Format) -> Result<()> {
    match cmd {
        RuntimeCmd::Pull {
            platform,
            profile,
            force,
            from_usd,
        } => pull(&platform, &profile, force, from_usd, fmt),
        RuntimeCmd::List => list(fmt),
        RuntimeCmd::Show { platform, profile } => show(&platform, &profile, fmt),
        RuntimeCmd::Validate { platform, profile } => validate(&platform, &profile, fmt),
        RuntimeCmd::Explain { platform, profile } => explain(&platform, &profile, fmt),
    }
}

/// Subdirectories the local backend creates inside a runtime prefix.
fn layout_dirs(python_version: &str, has_usd: bool) -> Vec<String> {
    let mut dirs = vec![
        "bin".to_string(),
        "lib".to_string(),
        format!("lib/python{}/site-packages", python_minor(python_version)),
        "include".to_string(),
        "share/cmake".to_string(),
    ];
    if has_usd {
        dirs.push("plugin/usd".to_string());
    }
    dirs
}

fn pull(
    platform: &str,
    profile: &str,
    force: bool,
    from_usd: Option<String>,
    fmt: Format,
) -> Result<()> {
    let r = resolve(platform, profile)?;

    if r.pulled && !force {
        return Err(Error::Operation(format!(
            "runtime '{}' already pulled (use --force to re-pull)",
            r.runtime.id()
        )));
    }

    // Resolve the profile's capabilities to concrete extensions. This drives
    // both the prefix layout (USD plugins) and the recorded provenance, so
    // `pull` agrees with `runtime explain`.
    let catalog = ost_extension::load_all()?;
    let resolution = ost_extension::resolve(&catalog, &r.capabilities);
    let has_usd = resolution.extensions.iter().any(|e| e.id == "openusd");
    let extensions: Vec<ExtensionRecord> = resolution
        .extensions
        .iter()
        .map(|e| ExtensionRecord {
            id: e.id.clone(),
            version: e.version.clone(),
            features: e.features.iter().cloned().collect(),
        })
        .collect();

    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Choose the backend source: an adopted USD install (`--from-usd` or
    // `OST_USD_ROOT`) → `local`; otherwise the mock layout.
    let adopt = from_usd.or_else(|| match std::env::var("OST_USD_ROOT") {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    });

    let manifest = match adopt {
        Some(usd_root) => adopt_local(&r, &usd_root, extensions, created)?,
        None => materialize_mock(&r, has_usd, extensions, created)?,
    };

    let manifest_path = r.prefix.join(MANIFEST_FILE);
    let json = manifest
        .to_json()
        .map_err(|e| Error::parse(MANIFEST_FILE, anyhow::Error::new(e)))?;
    std::fs::write(manifest_path.as_std_path(), format!("{json}\n"))
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;

    if fmt.is_json() {
        output::json(&serde_json::json!({
            "pulled": true,
            "runtime": manifest.id,
            "prefix": r.prefix.to_string(),
            "digest": manifest.digest,
            "source": manifest.source.as_str(),
            "external_prefix": manifest.external_prefix,
            "layout": manifest.layout,
            "extensions": manifest.extensions,
        }));
        return Ok(());
    }

    println!(
        "{} runtime {} ({})",
        if manifest.source == RuntimeSource::Local {
            "Adopted"
        } else {
            "Pulled"
        },
        manifest.id,
        manifest.source.as_str()
    );
    println!("  prefix:  {}", r.prefix);
    if let Some(ext) = &manifest.external_prefix {
        println!("  usd:     {ext}");
    }
    println!("  digest:  {}", manifest.digest);
    println!("  layout:  {}", manifest.layout.join(", "));
    if !manifest.extensions.is_empty() {
        let names: Vec<String> = manifest
            .extensions
            .iter()
            .map(|e| format!("{} {}", e.id, e.version))
            .collect();
        println!("  extensions: {}", names.join(", "));
    }
    println!("\nValidate with:");
    println!("  ost runtime validate {} --profile {}", platform, profile);
    Ok(())
}

/// Materialize the mock prefix layout (no real OpenUSD): the original backend.
fn materialize_mock(
    r: &crate::commands::Resolved,
    has_usd: bool,
    extensions: Vec<ExtensionRecord>,
    created: u64,
) -> Result<RuntimeManifest> {
    let layout = layout_dirs(&r.python_version, has_usd);
    for sub in &layout {
        let dir = r.prefix.join(sub);
        std::fs::create_dir_all(dir.as_std_path()).map_err(|e| Error::io(dir.to_string(), e))?;
    }
    Ok(RuntimeManifest::build(
        &r.runtime,
        &r.python_version,
        r.capabilities.clone(),
        layout,
        extensions,
        created,
        RuntimeSource::Mock,
    ))
}

/// Adopt an existing OpenUSD install at `usd_root` in place (`local` source):
/// record a manifest in the store that points at the external prefix, without
/// copying or building. The real artifacts keep USD's own layout.
fn adopt_local(
    r: &crate::commands::Resolved,
    usd_root: &str,
    extensions: Vec<ExtensionRecord>,
    created: u64,
) -> Result<RuntimeManifest> {
    let root = Utf8PathBuf::from(usd_root);
    if !root.as_std_path().is_dir() {
        return Err(Error::Operation(format!(
            "--from-usd path '{root}' is not a directory"
        )));
    }

    // Probe USD's install layout. Require at least one strong marker so we don't
    // silently adopt an unrelated directory.
    let candidates = ["bin", "lib", "lib/python", "plugin/usd", "include"];
    let layout: Vec<String> = candidates
        .iter()
        .filter(|s| root.join(s).as_std_path().is_dir())
        .map(|s| s.to_string())
        .collect();
    let looks_like_usd = ["lib/python", "plugin/usd"]
        .iter()
        .any(|s| root.join(s).as_std_path().is_dir());
    if !looks_like_usd {
        return Err(Error::Operation(format!(
            "'{root}' does not look like an OpenUSD install (no lib/python or plugin/usd)"
        )));
    }

    // The store dir holds only the manifest (a pointer to the external root).
    std::fs::create_dir_all(r.prefix.as_std_path())
        .map_err(|e| Error::io(r.prefix.to_string(), e))?;

    let mut manifest = RuntimeManifest::build(
        &r.runtime,
        &r.python_version,
        r.capabilities.clone(),
        layout,
        extensions,
        created,
        RuntimeSource::Local,
    );
    manifest.external_prefix = Some(root.to_string().replace('\\', "/"));
    Ok(manifest)
}

fn list(fmt: Format) -> Result<()> {
    let store = Store::discover();
    let runtimes_dir = store.runtimes();

    let mut manifests: Vec<RuntimeManifest> = Vec::new();
    if runtimes_dir.as_std_path().is_dir() {
        let entries = std::fs::read_dir(runtimes_dir.as_std_path())
            .map_err(|e| Error::io(runtimes_dir.to_string(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| Error::io(runtimes_dir.to_string(), e))?;
            let manifest_path = entry.path().join(MANIFEST_FILE);
            if !manifest_path.is_file() {
                continue;
            }
            let src = std::fs::read_to_string(&manifest_path)
                .map_err(|e| Error::io(manifest_path.display().to_string(), e))?;
            if let Ok(m) = RuntimeManifest::from_json(&src) {
                manifests.push(m);
            }
        }
    }
    manifests.sort_by(|a, b| a.id.cmp(&b.id));

    if fmt.is_json() {
        let items: Vec<_> = manifests
            .iter()
            .map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "platform": m.platform,
                    "profile": m.profile,
                    "validation": m.validation,
                    "digest": m.digest,
                    "source": m.source.as_str(),
                })
            })
            .collect();
        output::json(&serde_json::json!({ "runtimes": items }));
        return Ok(());
    }

    if manifests.is_empty() {
        println!("No runtimes pulled. Try `ost runtime pull cy2026 --profile usd`.");
        return Ok(());
    }
    println!("{:<48}  {:<9}  {:<8}  DIGEST", "RUNTIME", "VALIDATE", "SOURCE");
    for m in &manifests {
        let validation = format!("{:?}", m.validation).to_lowercase();
        println!(
            "{:<48}  {:<9}  {:<8}  {}",
            m.id,
            validation,
            m.source.as_str(),
            short_digest(&m.digest)
        );
    }
    Ok(())
}

fn show(platform: &str, profile: &str, fmt: Format) -> Result<()> {
    let r = resolve(platform, profile)?;
    let manifest_path = r.prefix.join(MANIFEST_FILE);
    if !manifest_path.as_std_path().is_file() {
        return Err(Error::Operation(format!(
            "runtime '{}' is not pulled (run `ost runtime pull {} --profile {}`)",
            r.runtime.id(),
            platform,
            profile
        )));
    }
    let src = std::fs::read_to_string(manifest_path.as_std_path())
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;
    let manifest = RuntimeManifest::from_json(&src)
        .map_err(|e| Error::parse(MANIFEST_FILE, anyhow::Error::new(e)))?;

    if fmt.is_json() {
        output::json(&serde_json::to_value(&manifest).expect("manifest serializes"));
        return Ok(());
    }

    println!("Runtime:    {}", manifest.id);
    println!("Platform:   {}", manifest.platform);
    println!("Profile:    {}", manifest.profile);
    println!("Variant:    {}", manifest.variant.slug());
    println!("Python:     {}", manifest.python);
    println!("Digest:     {}", manifest.digest);
    println!("Validation: {:?}", manifest.validation);
    println!("Source:     {}", manifest.source.as_str());
    println!("Prefix:     {}", r.prefix);
    if let Some(ext) = &manifest.external_prefix {
        println!("USD root:   {ext}");
    }
    println!("Capabilities:");
    for cap in &manifest.capabilities {
        println!("  - {cap}");
    }
    if !manifest.extensions.is_empty() {
        println!("Extensions:");
        for ext in &manifest.extensions {
            if ext.features.is_empty() {
                println!("  - {} {}", ext.id, ext.version);
            } else {
                println!(
                    "  - {} {} [{}]",
                    ext.id,
                    ext.version,
                    ext.features.join(", ")
                );
            }
        }
    }
    Ok(())
}

fn validate(platform: &str, profile: &str, fmt: Format) -> Result<()> {
    let r = resolve(platform, profile)?;
    let manifest_path = r.prefix.join(MANIFEST_FILE);
    if !manifest_path.as_std_path().is_file() {
        return Err(Error::Operation(format!(
            "runtime '{}' is not pulled (run `ost runtime pull {} --profile {}`)",
            r.runtime.id(),
            platform,
            profile
        )));
    }
    let src = std::fs::read_to_string(manifest_path.as_std_path())
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;
    let mut manifest = RuntimeManifest::from_json(&src)
        .map_err(|e| Error::parse(MANIFEST_FILE, anyhow::Error::new(e)))?;

    // Validate against the effective artifact prefix (the external USD root for
    // an adopted runtime; the store prefix otherwise).
    let report = ost_runtime::validate(&r.artifact_prefix, &manifest);
    let passed = report.passed();

    // Record the outcome back into the manifest (digest is unaffected).
    manifest.set_validation(if passed {
        Validation::Passed
    } else {
        Validation::Failed
    });
    let json = manifest
        .to_json()
        .map_err(|e| Error::parse(MANIFEST_FILE, anyhow::Error::new(e)))?;
    std::fs::write(manifest_path.as_std_path(), format!("{json}\n"))
        .map_err(|e| Error::io(manifest_path.to_string(), e))?;

    if fmt.is_json() {
        let checks: Vec<_> = report
            .checks
            .iter()
            .map(|c| {
                serde_json::json!({
                    "name": c.name,
                    "passed": c.passed,
                    "detail": c.detail,
                })
            })
            .collect();
        output::json(&serde_json::json!({
            "runtime": manifest.id,
            "validation": if passed { "passed" } else { "failed" },
            "checks": checks,
        }));
    } else {
        println!("Validating {}", manifest.id);
        for c in &report.checks {
            let mark = if c.passed { "ok  " } else { "FAIL" };
            match &c.detail {
                Some(d) => println!("  [{mark}] {} — {d}", c.name),
                None => println!("  [{mark}] {}", c.name),
            }
        }
        println!(
            "\n{}",
            if passed {
                "Result: passed"
            } else {
                "Result: FAILED"
            }
        );
    }

    // Deterministic exit for CI.
    if passed {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

fn explain(platform: &str, profile: &str, fmt: Format) -> Result<()> {
    let r = resolve(platform, profile)?;
    let catalog = ost_extension::load_all()?;
    let resolution = ost_extension::resolve(&catalog, &r.capabilities);

    if fmt.is_json() {
        let caps: Vec<_> = resolution
            .edges
            .iter()
            .map(|e| {
                serde_json::json!({
                    "capability": e.capability,
                    "provider": e.extension,
                    "feature": e.feature,
                })
            })
            .collect();
        let exts: Vec<_> = resolution
            .extensions
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "version": e.version,
                    "features": e.features,
                    "packages": e.packages,
                    "allowed_range": e.allowed_range,
                    "certified": e.certified.as_ref().map(|c| serde_json::json!({
                        "version": c.version,
                        "features": c.features,
                        "validation": c.validation,
                    })),
                    "uncertified": e.uncertified,
                })
            })
            .collect();
        output::json(&serde_json::json!({
            "runtime": r.runtime.id(),
            "platform": platform,
            "profile": profile,
            "capabilities": caps,
            "extensions": exts,
            "runtime_provided": resolution.runtime_provided,
        }));
        return Ok(());
    }

    println!("Runtime {}", r.runtime.id());
    println!("  platform: {platform}   profile: {profile}");

    println!("\nCapabilities:");
    let width = resolution
        .edges
        .iter()
        .map(|e| e.capability.len())
        .max()
        .unwrap_or(0);
    for edge in &resolution.edges {
        let provider = match (&edge.extension, &edge.feature) {
            (Some(ext), Some(feature)) => format!("{ext} [{feature}]"),
            (Some(ext), None) => ext.clone(),
            (None, _) => "runtime".to_string(),
        };
        println!("  {:<width$}  {provider}", edge.capability);
    }

    if resolution.extensions.is_empty() {
        println!("\nExtensions: (none — base runtime only)");
    } else {
        println!("\nExtensions:");
        for ext in &resolution.extensions {
            println!("  {} {}", ext.id, ext.version);
            if !ext.features.is_empty() {
                let feats: Vec<_> = ext.features.iter().cloned().collect();
                println!("    features:  {}", feats.join(", "));
            }
            if !ext.packages.is_empty() {
                let pkgs: Vec<_> = ext.packages.iter().cloned().collect();
                println!("    packages:  {}", pkgs.join(", "));
            }
            if let Some(c) = &ext.certified {
                let val = c.validation.as_deref().unwrap_or("unvalidated");
                if c.features.is_empty() {
                    println!("    certified: {} ({val})", c.version);
                } else {
                    println!(
                        "    certified: {} [{}] ({val})",
                        c.version,
                        c.features.join(", ")
                    );
                }
            } else if ext.uncertified {
                let feats: Vec<_> = ext.features.iter().cloned().collect();
                println!(
                    "    certified: NONE — no certified build covers [{}] (UNCERTIFIED)",
                    feats.join(", ")
                );
            }
            if let Some(range) = &ext.allowed_range {
                println!("    range:     {range}");
            }
        }
    }
    Ok(())
}

fn short_digest(digest: &str) -> String {
    // `sha256:abcd...` -> `sha256:abcd1234`
    match digest.split_once(':') {
        Some((algo, hex)) => format!("{algo}:{}", &hex[..hex.len().min(12)]),
        None => digest.to_string(),
    }
}
