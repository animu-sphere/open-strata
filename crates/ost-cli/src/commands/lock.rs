// SPDX-License-Identifier: Apache-2.0
//! `ost lock` — generate or verify the project lockfile `strata.lock` (§9.4).
//!
//! The lock pins the resolved runtime (id/variant/digest), the Python ABI and
//! companion `uv.lock` hash, the resolved extensions, and the validation status.
//! It is fully deterministic (no timestamps), so `--check` can gate CI on the
//! lock being up to date. `ost configure` refreshes it automatically.

use camino::Utf8Path;
use clap::Args;

use ost_core::{digest, Error, Result};
use ost_manifest::{Lock, LockExtension, LockPython, LockRuntime, Validation};
use ost_runtime::{RuntimeManifest, MANIFEST_FILE};

use crate::commands::configure::{build_target, resolve_selection};
use crate::output::{self, Format};

/// Project lockfile name, written at the project root.
pub const LOCK_FILE: &str = "strata.lock";

#[derive(Debug, Args)]
pub struct LockArgs {
    /// Platform target, e.g. `cy2026`. Defaults to the project's platform.
    #[arg(long)]
    target: Option<String>,

    /// Profile to lock. Defaults to the project's profile.
    #[arg(long)]
    profile: Option<String>,

    /// Verify the on-disk lock is up to date instead of writing it (exit 1 if not).
    #[arg(long)]
    check: bool,
}

pub fn run(args: LockArgs, fmt: Format) -> Result<()> {
    let (root, platform, profile) = resolve_selection(args.target, args.profile)?;
    let lock = build_lock(&root, &platform, &profile)?;
    let path = root.join(LOCK_FILE);

    if args.check {
        let on_disk = std::fs::read_to_string(path.as_std_path()).ok();
        let expected = render(&lock)?;
        let up_to_date = on_disk.as_deref() == Some(expected.as_str());

        if fmt.is_json() {
            output::report(
                up_to_date,
                &serde_json::json!({ "lock": LOCK_FILE, "up_to_date": up_to_date }),
            );
        } else if up_to_date {
            println!("{LOCK_FILE} is up to date.");
        } else {
            println!("{LOCK_FILE} is out of date or missing — run `ost lock`.");
        }
        // A stale lock is a validation mismatch (§14.4); the report above is this
        // command's own output, so exit with the category code directly.
        if !up_to_date {
            std::process::exit(ost_core::Category::Validation.exit_code() as i32);
        }
        return Ok(());
    }

    write_lock(&root, &lock)?;

    if fmt.is_json() {
        output::success(&serde_json::json!({
            "lock": LOCK_FILE,
            "runtime": lock.runtime.id,
            "digest": lock.runtime.digest,
            "validation": lock.validation,
            "extensions": lock.extensions.iter().map(|e| &e.id).collect::<Vec<_>>(),
        }));
    } else {
        println!("Wrote {}", path);
        println!("  runtime:    {}", lock.runtime.id);
        let digest = if lock.runtime.digest.is_empty() {
            "(runtime not pulled)"
        } else {
            &lock.runtime.digest
        };
        println!("  digest:     {digest}");
        println!("  python abi: {}", lock.python.abi);
        if let Some(h) = &lock.python.uv_lock_hash {
            println!("  uv.lock:    {h}");
        }
        if !lock.extensions.is_empty() {
            let names: Vec<String> = lock
                .extensions
                .iter()
                .map(|e| format!("{} {}", e.id, e.version))
                .collect();
            println!("  extensions: {}", names.join(", "));
        }
        println!("  validation: {:?}", lock.validation);
    }
    Ok(())
}

/// Build the lock for a resolved platform+profile. Shared with `ost configure`.
pub(crate) fn build_lock(root: &Utf8Path, platform: &str, profile: &str) -> Result<Lock> {
    let (_target, r) = build_target(platform, profile)?;

    let (digest_str, validation) = read_runtime_state(&r.prefix);

    // Hash the companion uv.lock if the project has one.
    let uv_lock_hash = std::fs::read(root.join("uv.lock").as_std_path())
        .ok()
        .map(|bytes| digest::sha256_hex(&bytes));

    let catalog = ost_extension::load_all()?;
    let resolution = ost_extension::resolve(&catalog, &r.capabilities);
    let extensions: Vec<LockExtension> = resolution
        .extensions
        .iter()
        .map(|e| LockExtension {
            id: e.id.clone(),
            version: e.version.clone(),
            features: e.features.iter().cloned().collect(),
        })
        .collect();

    Ok(Lock {
        lock_version: 1,
        runtime: LockRuntime {
            id: r.runtime.id(),
            platform: platform.to_string(),
            profile: profile.to_string(),
            variant: r.runtime.variant.clone(),
            digest: digest_str,
        },
        python: LockPython {
            version: r.python_version.clone(),
            abi: r.runtime.variant.python_abi(),
            manager: "uv".to_string(),
            uv_lock_hash,
        },
        extensions,
        validation,
    })
}

/// Write the lock to `<root>/strata.lock`.
pub(crate) fn write_lock(root: &Utf8Path, lock: &Lock) -> Result<()> {
    let path = root.join(LOCK_FILE);
    std::fs::write(path.as_std_path(), render(lock)?).map_err(|e| Error::io(path.to_string(), e))
}

fn render(lock: &Lock) -> Result<String> {
    let json = lock
        .to_json()
        .map_err(|e| Error::parse(LOCK_FILE, anyhow::Error::new(e)))?;
    Ok(format!("{json}\n"))
}

/// Read the runtime's digest and validation status from its manifest, if pulled.
fn read_runtime_state(prefix: &Utf8Path) -> (String, Validation) {
    let manifest = std::fs::read_to_string(prefix.join(MANIFEST_FILE).as_std_path())
        .ok()
        .and_then(|s| RuntimeManifest::from_json(&s).ok());
    match manifest {
        Some(m) => (m.digest, map_validation(m.validation)),
        None => (String::new(), Validation::Pending),
    }
}

fn map_validation(v: ost_runtime::Validation) -> Validation {
    match v {
        ost_runtime::Validation::Passed => Validation::Passed,
        ost_runtime::Validation::Failed => Validation::Failed,
        ost_runtime::Validation::Pending => Validation::Pending,
    }
}
