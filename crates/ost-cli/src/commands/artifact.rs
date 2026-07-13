// SPDX-License-Identifier: Apache-2.0
//! `ost artifact` — local artifact registry operations (Phase 6 MVP).
//!
//! - `import` register a package output (dist dir / manifest.json) by digest.
//! - `list`   show what the registry holds.
//! - `show`   full identity record for one digest.
//! - `verify` recompute the archive digest and re-hash every archived file
//!   against the producer manifest.
//! - `export` copy an artifact (archive + manifest + checksums + record) out
//!   for CI handoff; the exported directory is re-importable.
//! - `resolve` turn a remote reference (a mutable tag) into its immutable
//!   digest — the pin CI and the lockfile require.
//! - `push` publish a stored artifact to a remote OCI registry, emitting the
//!   pull-compatible OCI layout and printing the immutable OCI digest to pin.
//! - `pull` fetch a **digest-pinned** artifact from a remote (`oci://`) or
//!   local (`file://`) source, run the full verification chain, and import it
//!   atomically (remote-artifact-transport.md, Phase 1).
//!
//! Artifacts are addressed by digest (full `sha256:<hex>` or a unique prefix),
//! never by mutable name — the registry is the source of truth CI pins.

use camino::{Utf8Path, Utf8PathBuf};
use clap::Subcommand;

use ost_artifact::{
    verify_evidence_digest, verify_provenance, verify_sbom, ArtifactKind, ArtifactPolicy,
    ArtifactRecord, ArtifactSource, ArtifactStore, ArtifactTransport, EvidenceDigest,
    FileTransport, OciTransport, PublisherIdentity, PullEvidence, PullPolicy, PushOutcome,
    RemoteReference, TrustLevel, VerifyReport,
};
use ost_core::{Error, Result};

use crate::output::{self, Format};

#[derive(Debug, Subcommand)]
pub enum ArtifactCmd {
    /// Import a package output into the local registry, keyed by digest.
    Import {
        /// A dist directory containing manifest.json (+ archive), or the
        /// manifest.json itself.
        path: String,
    },
    /// List artifacts in the local registry.
    List {
        /// Only show artifacts of this kind: runtime | plugin | package.
        #[arg(long)]
        kind: Option<String>,
    },
    /// Show the full identity record for one artifact.
    Show {
        /// Digest reference: sha256:<hex> or a unique hex prefix (>= 6 chars).
        digest: String,
    },
    /// Verify a stored artifact's integrity (archive digest + per-file hashes).
    Verify {
        /// Digest reference: sha256:<hex> or a unique hex prefix (>= 6 chars).
        digest: String,
        /// Enforce minimum trust from an artifact policy TOML file.
        #[arg(long, value_name = "FILE")]
        policy: Option<Utf8PathBuf>,
        /// Fail unless a valid SPDX SBOM is attached to the artifact.
        #[arg(long)]
        require_sbom: bool,
        /// Fail unless valid SLSA/in-toto provenance is attached.
        #[arg(long)]
        require_provenance: bool,
    },
    /// Export an artifact's files into a directory (CI handoff).
    Export {
        /// Digest reference: sha256:<hex> or a unique hex prefix (>= 6 chars).
        digest: String,
        /// Destination directory (created if missing; files must not exist).
        dest: String,
    },
    /// Unpack an artifact's archive into a directory (digest re-verified).
    Extract {
        /// Digest reference: sha256:<hex> or a unique hex prefix (>= 6 chars).
        digest: String,
        /// Destination directory (created if missing). May also be given as
        /// `--into <DEST>`.
        dest: Option<String>,
        /// Named form of the destination directory, e.g.
        /// `ost artifact extract <digest> --into ./out`.
        #[arg(long, conflicts_with = "dest")]
        into: Option<String>,
    },
    /// Resolve a remote reference (tag) to its immutable digest.
    Resolve {
        /// oci://<registry>/<repository>[:tag][@sha256:<digest>]
        reference: String,
        /// Use plain http:// instead of https:// (fixture registries and
        /// air-gapped mirrors only).
        #[arg(long)]
        plain_http: bool,
    },
    /// Push a stored artifact to a remote OCI registry (the producer verb).
    Push {
        /// Digest reference of a stored artifact: sha256:<hex> or a unique hex
        /// prefix (>= 6 chars).
        digest: String,
        /// Destination: oci://<registry>/<repository>[:tag][@sha256:<oci-digest>].
        /// A pinned digest is verified against the computed manifest digest.
        destination: String,
        /// Artifact policy TOML. When omitted, search the current directory
        /// and its parents for openstrata-artifact-policy.toml.
        #[arg(long, value_name = "FILE")]
        policy: Option<Utf8PathBuf>,
        /// Explicitly bypass publisher identity checks for a protected
        /// namespace. The override is recorded in command output.
        #[arg(long)]
        allow_untrusted_publisher: bool,
        /// Use plain http:// instead of https:// (fixture registries and
        /// air-gapped mirrors only).
        #[arg(long)]
        plain_http: bool,
    },
    /// Pull a digest-pinned artifact from a remote source, verify it, and
    /// import it into the local registry.
    Pull {
        /// oci://…@sha256:<oci-manifest-digest> or file://<dist-dir>.
        /// Mutable (tag-only) references are refused: resolve them first.
        reference: String,
        /// Require the pulled OpenStrata artifact digest to equal this pin
        /// (the support line / lockfile contract).
        #[arg(long, value_name = "sha256:<hex>")]
        expect_artifact: Option<String>,
        /// Require the artifact kind: runtime | plugin | package.
        #[arg(long, value_name = "KIND")]
        require_kind: Option<String>,
        /// Require the artifact's target id to match exactly.
        #[arg(long, value_name = "TARGET")]
        require_target: Option<String>,
        /// Use plain http:// instead of https:// (fixture registries and
        /// air-gapped mirrors only).
        #[arg(long)]
        plain_http: bool,
    },
}

pub fn run(cmd: ArtifactCmd, fmt: Format) -> Result<()> {
    let store = ArtifactStore::discover();
    match cmd {
        ArtifactCmd::Import { path } => import(&store, &path, fmt),
        ArtifactCmd::List { kind } => list(&store, kind.as_deref(), fmt),
        ArtifactCmd::Show { digest } => show(&store, &digest, fmt),
        ArtifactCmd::Verify {
            digest,
            policy,
            require_sbom,
            require_provenance,
        } => verify(
            &store,
            &digest,
            policy.as_deref(),
            require_sbom,
            require_provenance,
            fmt,
        ),
        ArtifactCmd::Export { digest, dest } => export(&store, &digest, &dest, fmt),
        ArtifactCmd::Extract { digest, dest, into } => {
            let dest = dest.or(into).ok_or_else(|| {
                Error::usage("missing destination: pass a directory or --into <DEST>")
            })?;
            extract(&store, &digest, &dest, fmt)
        }
        ArtifactCmd::Resolve {
            reference,
            plain_http,
        } => resolve_remote(&reference, plain_http, fmt),
        ArtifactCmd::Push {
            digest,
            destination,
            policy,
            allow_untrusted_publisher,
            plain_http,
        } => push_remote(
            &store,
            &digest,
            &destination,
            policy.as_deref(),
            allow_untrusted_publisher,
            plain_http,
            fmt,
        ),
        ArtifactCmd::Pull {
            reference,
            expect_artifact,
            require_kind,
            require_target,
            plain_http,
        } => {
            let policy = PullPolicy {
                expected_artifact_digest: expect_artifact,
                require_kind: require_kind
                    .as_deref()
                    .map(|k| {
                        ArtifactKind::from_tag(k).ok_or_else(|| {
                            Error::usage(format!(
                                "unknown artifact kind '{k}' (expected runtime, plugin, or package)"
                            ))
                        })
                    })
                    .transpose()?,
                require_target,
            };
            pull_remote(&store, &reference, &policy, plain_http, fmt)
        }
    }
}

/// The transport for a parsed reference: OCI for `oci://`, filesystem for
/// `file://`. Both implement the same contract, so callers never branch again.
fn transport_for(reference: &RemoteReference, plain_http: bool) -> Box<dyn ArtifactTransport> {
    match reference {
        RemoteReference::Oci(_) => Box::new(OciTransport::new(plain_http)),
        RemoteReference::File(_) => Box::new(FileTransport::new()),
    }
}

fn resolve_remote(reference: &str, plain_http: bool, fmt: Format) -> Result<()> {
    let parsed = RemoteReference::parse(reference)?;
    let transport = transport_for(&parsed, plain_http);
    let resolved = transport.resolve(&parsed)?;
    if fmt.is_json() {
        output::success(&serde_json::json!({
            "reference": reference,
            "resolved": {
                "locator": resolved.locator,
                "registry": resolved.registry,
                "repository": resolved.repository,
                "oci_digest": resolved.oci_digest,
                "auth_mode": resolved.auth_mode,
            },
        }));
        return Ok(());
    }
    println!("Resolved {reference}");
    println!("  locator:   {}", resolved.locator);
    if let Some(dg) = &resolved.oci_digest {
        println!("  digest:    {dg}");
    }
    println!("  registry:  {}", resolved.registry);
    println!("  auth mode: {}", resolved.auth_mode);
    println!();
    println!("Pin this in CI / the lockfile and pull it with:");
    println!("  ost artifact pull {}", resolved.locator);
    Ok(())
}

fn push_remote(
    store: &ArtifactStore,
    digest: &str,
    destination: &str,
    policy_path: Option<&Utf8Path>,
    allow_untrusted_publisher: bool,
    plain_http: bool,
    fmt: Format,
) -> Result<()> {
    let parsed = RemoteReference::parse(destination)?;
    let oci = match &parsed {
        RemoteReference::Oci(oci) => oci,
        RemoteReference::File(_) => {
            return Err(Error::usage(
                "push targets an OCI registry (oci://…); to write a local dist directory \
                 use `ost artifact export <digest> <dir>`",
            ));
        }
    };
    let policy = authorize_push(oci, policy_path, allow_untrusted_publisher)?;
    let transport = transport_for(&parsed, plain_http);
    let outcome = ost_artifact::push(transport.as_ref(), store, digest, &parsed)?;
    if fmt.is_json() {
        output::success(&push_outcome_json(&outcome, policy.as_ref()));
        return Ok(());
    }
    let verb = if outcome.already_present {
        "Already present"
    } else {
        "Pushed"
    };
    println!("{verb} on {}", outcome.registry);
    println!("  locator:      {}", outcome.locator);
    println!("  oci digest:   {}", outcome.oci_digest);
    println!("  artifact:     {}", outcome.artifact_digest);
    println!("  auth mode:    {}", outcome.auth_mode);
    if let Some(policy) = &policy {
        println!("  policy:       {}", policy.path);
        match &policy.namespace {
            Some(namespace) if policy.overridden => {
                println!("  publisher:    OVERRIDDEN ({namespace})");
            }
            Some(namespace) => {
                println!(
                    "  publisher:    {} ({}, {namespace})",
                    policy.publisher.as_deref().unwrap_or("unknown"),
                    policy.trust.unwrap_or(TrustLevel::Local),
                );
            }
            None => println!("  publisher:    not required (unprotected destination)"),
        }
    }
    println!();
    println!("Pin this in a support line's runtime_remote:");
    println!(
        "  uri:                 oci://{}/{}",
        outcome.registry, outcome.repository
    );
    println!("  expected_oci_digest: {}", outcome.oci_digest);
    Ok(())
}

#[derive(Debug)]
struct PushPolicyEvidence {
    path: Utf8PathBuf,
    namespace: Option<String>,
    publisher: Option<String>,
    trust: Option<TrustLevel>,
    overridden: bool,
}

fn authorize_push(
    destination: &ost_artifact::OciReference,
    explicit_path: Option<&Utf8Path>,
    allow_untrusted_publisher: bool,
) -> Result<Option<PushPolicyEvidence>> {
    let loaded = if let Some(path) = explicit_path {
        Some((path.to_owned(), ArtifactPolicy::load(path)?))
    } else {
        let cwd = std::env::current_dir().map_err(|source| Error::io(".", source))?;
        let cwd = Utf8PathBuf::from_path_buf(cwd).map_err(|path| {
            Error::config(format!(
                "current directory '{}' is not valid UTF-8",
                path.display()
            ))
        })?;
        ArtifactPolicy::discover(&cwd)?
    };
    let Some((path, policy)) = loaded else {
        return Ok(None);
    };

    let destination = format!(
        "{}/{}",
        destination.registry.to_ascii_lowercase(),
        destination.repository
    );
    let Some(protected) = policy.protected_namespace(&destination) else {
        return Ok(Some(PushPolicyEvidence {
            path,
            namespace: None,
            publisher: None,
            trust: None,
            overridden: false,
        }));
    };
    let namespace = protected.namespace.clone();
    if allow_untrusted_publisher {
        return Ok(Some(PushPolicyEvidence {
            path,
            namespace: Some(namespace),
            publisher: None,
            trust: None,
            overridden: true,
        }));
    }

    let identity = PublisherIdentity::from_github_actions_oidc()?;
    let authorization = policy
        .authorize_publisher(&destination, &identity)?
        .expect("the destination was already matched to a protected namespace");
    Ok(Some(PushPolicyEvidence {
        path,
        namespace: Some(authorization.namespace),
        publisher: Some(authorization.publisher),
        trust: Some(authorization.trust),
        overridden: false,
    }))
}

/// Push outcome as JSON, carrying every digest a caller might pin.
fn push_outcome_json(
    outcome: &PushOutcome,
    policy: Option<&PushPolicyEvidence>,
) -> serde_json::Value {
    serde_json::json!({
        "status": if outcome.already_present { "already-present" } else { "pushed" },
        "oci_digest": outcome.oci_digest,
        "artifact_digest": outcome.artifact_digest,
        "locator": outcome.locator,
        "registry": outcome.registry,
        "repository": outcome.repository,
        "already_present": outcome.already_present,
        "auth_mode": outcome.auth_mode,
        "policy": policy.map(|policy| serde_json::json!({
            "path": policy.path,
            "protected_namespace": policy.namespace,
            "publisher": policy.publisher,
            "trust": policy.trust,
            "overridden": policy.overridden,
        })),
    })
}

fn pull_remote(
    store: &ArtifactStore,
    reference: &str,
    policy: &PullPolicy,
    plain_http: bool,
    fmt: Format,
) -> Result<()> {
    let parsed = RemoteReference::parse(reference)?;
    let transport = transport_for(&parsed, plain_http);
    let evidence = ost_artifact::pull(transport.as_ref(), &parsed, store, policy)?;
    if fmt.is_json() {
        output::success(&pull_evidence_json(store, &evidence));
        return Ok(());
    }
    let r = &evidence.record;
    println!(
        "Pulled {} ({} {} {}) from {}",
        r.short_digest(),
        r.kind.as_str(),
        r.name,
        r.version,
        evidence.remote.registry
    );
    println!("  locator:    {}", evidence.remote.locator);
    if let Some(dg) = &evidence.remote.oci_digest {
        println!("  oci digest: {dg}");
    }
    println!("  artifact:   {}", r.digest);
    for (step, status) in &evidence.verification {
        println!("  {status:<7} {step}");
    }
    println!(
        "  import:     {} ({})",
        evidence.import_status, evidence.import_path
    );
    Ok(())
}

/// Pull evidence as JSON (transport plan, "Minimum JSON output").
fn pull_evidence_json(store: &ArtifactStore, evidence: &PullEvidence) -> serde_json::Value {
    let mut verification = serde_json::Map::new();
    for (step, status) in &evidence.verification {
        verification.insert((*step).to_string(), serde_json::json!(status));
    }
    serde_json::json!({
        "status": "ok",
        "artifact_digest": evidence.record.digest,
        "reference": evidence.reference,
        "remote": {
            "locator": evidence.remote.locator,
            "resolved_oci_digest": evidence.remote.oci_digest,
            "registry": evidence.remote.registry,
            "repository": evidence.remote.repository,
            "auth_mode": evidence.remote.auth_mode,
        },
        "verification": verification,
        "local_import": {
            "status": evidence.import_status,
            "path": evidence.import_path.to_string(),
            "store": store.root().to_string(),
        },
        "artifact": record_json(&evidence.record),
    })
}

fn import(store: &ArtifactStore, path: &str, fmt: Format) -> Result<()> {
    let out = store.import(Utf8PathBuf::from(path).as_path(), ArtifactSource::Imported)?;
    if fmt.is_json() {
        output::success(&serde_json::json!({
            "imported": true,
            "already_present": out.already_present,
            "artifact": record_json(&out.record),
        }));
        return Ok(());
    }
    if out.already_present {
        println!(
            "Already in the registry as {} ({} {} {})",
            out.record.short_digest(),
            out.record.kind.as_str(),
            out.record.name,
            out.record.version
        );
    } else {
        println!(
            "Imported {} {} {} for {}",
            out.record.kind.as_str(),
            out.record.name,
            out.record.version,
            out.record.target
        );
        println!("  digest: {}", out.record.digest);
    }
    Ok(())
}

fn list(store: &ArtifactStore, kind: Option<&str>, fmt: Format) -> Result<()> {
    let kind = kind
        .map(|k| {
            ArtifactKind::from_tag(k).ok_or_else(|| {
                Error::usage(format!(
                    "unknown artifact kind '{k}' (expected runtime, plugin, or package)"
                ))
            })
        })
        .transpose()?;
    let records: Vec<ArtifactRecord> = store
        .list()?
        .into_iter()
        .filter(|r| kind.is_none_or(|k| r.kind == k))
        .collect();

    if fmt.is_json() {
        output::success(&serde_json::json!({
            "artifacts": records.iter().map(record_json).collect::<Vec<_>>(),
        }));
        return Ok(());
    }
    if records.is_empty() {
        println!("No artifacts in the local registry ({})", store.root());
        println!("  import one with `ost artifact import <dist-dir>` or `ost plugin publish`");
        return Ok(());
    }
    println!("Artifacts in {} :", store.root());
    for r in &records {
        println!(
            "  {}  {:<7} {:<20} {:<10} {}  [{}]",
            r.short_digest(),
            r.kind.as_str(),
            r.name,
            r.version,
            r.target,
            r.validation
        );
    }
    Ok(())
}

fn show(store: &ArtifactStore, digest: &str, fmt: Format) -> Result<()> {
    let r = store.resolve(digest)?;
    if fmt.is_json() {
        output::success(&serde_json::json!({ "artifact": record_json(&r) }));
        return Ok(());
    }
    println!("{} {} {}", r.kind.as_str(), r.name, r.version);
    println!("  digest:      {}", r.digest);
    println!("  target:      {}", r.target);
    if let Some(profile) = &r.profile {
        println!("  profile:     {profile}");
    }
    println!("  archive:     {} ({} bytes)", r.archive, r.archive_size);
    println!(
        "  contents:    {} file(s), {} bytes uncompressed",
        r.file_count, r.total_size
    );
    println!("  source:      {}", r.source.as_str());
    println!("  trust:       {}", r.trust);
    println!("  validation:  {}", r.validation);
    if r.licenses.is_empty() {
        println!("  licenses:    (none recorded)");
    } else {
        println!("  licenses:    {}", r.licenses.join(", "));
    }
    if let (Some(id), Some(dg)) = (&r.runtime_id, &r.runtime_digest) {
        println!("  runtime:     {id} ({dg})");
    }
    if let (Some(path), Some(digest)) = (&r.sbom, &r.sbom_digest) {
        println!("  SBOM:        {path} ({digest})");
    }
    if let (Some(path), Some(digest)) = (&r.provenance, &r.provenance_digest) {
        println!("  provenance:  {path} ({digest})");
    }
    println!("  producer:    {}", r.producer);
    println!("  store:       {}", store.object_dir(r.digest_hex()));
    Ok(())
}

fn verify(
    store: &ArtifactStore,
    digest: &str,
    policy_path: Option<&camino::Utf8Path>,
    require_sbom: bool,
    require_provenance: bool,
    fmt: Format,
) -> Result<()> {
    let report = store.verify(digest)?;
    let record = store.resolve(digest)?;
    let policy = policy_path.map(ArtifactPolicy::load).transpose()?;
    let policy_error = policy
        .as_ref()
        .and_then(|value| value.verify_trust(record.trust).err());
    let policy_passed = policy_error.is_none();
    let object_dir = store.object_dir(record.digest_hex());
    let manifest = store.producer_manifest(&record)?;
    let (sbom, provenance) = store.evidence(&record)?;
    let sbom = verify_sbom_check(&object_dir, &record.digest, sbom, require_sbom);
    let provenance = verify_provenance_check(
        &object_dir,
        &manifest,
        &record.digest,
        provenance,
        require_provenance,
        policy.as_ref(),
    );
    let passed = report.passed() && policy_passed && sbom.passed() && provenance.passed();
    if fmt.is_json() {
        output::report(
            passed,
            &serde_json::json!({
                "digest": report.digest,
                "passed": passed,
                "archive_digest_ok": report.archive_digest_ok,
                "files_matched": report.files_matched,
                "files_mismatched": report.files_mismatched,
                "files_missing": report.files_missing,
                "files_extra": report.files_extra,
                "trust": record.trust,
                "policy": policy.as_ref().map(|value| serde_json::json!({
                    "path": policy_path.map(ToString::to_string),
                    "minimum_trust": value.minimum_trust,
                    "passed": policy_passed,
                    "error_code": policy_error.as_ref().map(|e| e.code()),
                    "message": policy_error.as_ref().map(ToString::to_string),
                })),
                "evidence": {
                    "sbom": sbom.json(),
                    "provenance": provenance.json(),
                },
            }),
        );
    } else {
        render_verify(
            &report,
            policy.as_ref(),
            record.trust,
            policy_error.as_ref(),
            &sbom,
            &provenance,
        );
    }
    // The report above is this command's single document (§14.3); a failed
    // verification exits with the validation category code directly.
    if !passed {
        std::process::exit(ost_core::Category::Validation.exit_code() as i32);
    }
    Ok(())
}

struct EvidenceCheck {
    required: bool,
    descriptor: Option<EvidenceDigest>,
    matched_publisher: Option<String>,
    error: Option<Error>,
}

impl EvidenceCheck {
    fn passed(&self) -> bool {
        self.error.is_none()
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "required": self.required,
            "present": self.descriptor.is_some(),
            "path": self.descriptor.as_ref().map(|value| value.path.as_str()),
            "digest": self.descriptor.as_ref().map(|value| value.digest.as_str()),
            "size": self.descriptor.as_ref().map(|value| value.size),
            "matched_publisher": self.matched_publisher,
            "passed": self.passed(),
            "error_code": self.error.as_ref().map(|error| error.code()),
            "message": self.error.as_ref().map(ToString::to_string),
        })
    }
}

fn verify_sbom_check(
    object_dir: &Utf8Path,
    artifact_digest: &str,
    descriptor: Option<EvidenceDigest>,
    required: bool,
) -> EvidenceCheck {
    let error = match descriptor.as_ref() {
        Some(evidence) => verify_evidence_digest(object_dir, evidence)
            .and_then(|()| verify_sbom(&object_dir.join(&evidence.path), artifact_digest))
            .err(),
        None if required => Some(Error::coded(
            "ARTIFACT_SBOM_REQUIRED",
            ost_core::Category::Validation,
            "artifact has no attached SPDX SBOM",
        )),
        None => None,
    };
    EvidenceCheck {
        required,
        descriptor,
        matched_publisher: None,
        error,
    }
}

fn verify_provenance_check(
    object_dir: &Utf8Path,
    manifest: &serde_json::Value,
    artifact_digest: &str,
    descriptor: Option<EvidenceDigest>,
    required: bool,
    policy: Option<&ArtifactPolicy>,
) -> EvidenceCheck {
    let result = match descriptor.as_ref() {
        Some(evidence) => verify_evidence_digest(object_dir, evidence).and_then(|()| {
            verify_provenance(
                &object_dir.join(&evidence.path),
                manifest,
                artifact_digest,
                policy.filter(|_| required),
            )
        }),
        None if required => Err(Error::coded(
            "ARTIFACT_PROVENANCE_REQUIRED",
            ost_core::Category::Validation,
            "artifact has no attached SLSA/in-toto provenance",
        )),
        None => Ok(None),
    };
    let (matched_publisher, error) = match result {
        Ok(publisher) => (publisher, None),
        Err(error) => (None, Some(error)),
    };
    EvidenceCheck {
        required,
        descriptor,
        matched_publisher,
        error,
    }
}

fn render_verify(
    report: &VerifyReport,
    policy: Option<&ArtifactPolicy>,
    trust: ost_artifact::TrustLevel,
    policy_error: Option<&Error>,
    sbom: &EvidenceCheck,
    provenance: &EvidenceCheck,
) {
    let passed = report.passed() && policy_error.is_none() && sbom.passed() && provenance.passed();
    println!("Verify {}", report.digest);
    println!(
        "  archive digest: {}",
        if report.archive_digest_ok {
            "OK"
        } else {
            "MISMATCH"
        }
    );
    if report.archive_digest_ok {
        println!("  files matched:  {}", report.files_matched);
        for f in &report.files_mismatched {
            println!("  MISMATCH: {f}");
        }
        for f in &report.files_missing {
            println!("  MISSING:  {f}");
        }
        for f in &report.files_extra {
            println!("  EXTRA:    {f}");
        }
    }
    if let Some(policy) = policy {
        println!("  trust:          {trust}");
        println!("  policy minimum: {}", policy.minimum_trust);
        println!(
            "  policy result:  {}",
            if policy_error.is_none() { "OK" } else { "FAIL" }
        );
        if let Some(error) = policy_error {
            println!("  {}: {error}", error.code());
        }
    }
    render_evidence("SBOM", sbom);
    render_evidence("provenance", provenance);
    println!("  result: {}", if passed { "PASS" } else { "FAIL" });
}

fn render_evidence(label: &str, check: &EvidenceCheck) {
    let status = if check.descriptor.is_none() && !check.required {
        "not present"
    } else if check.passed() {
        "OK"
    } else {
        "FAIL"
    };
    println!("  {label}: {status}");
    if let Some(error) = &check.error {
        println!("  {}: {error}", error.code());
    }
    if let Some(publisher) = &check.matched_publisher {
        println!("  provenance publisher: {publisher}");
    }
}

fn export(store: &ArtifactStore, digest: &str, dest: &str, fmt: Format) -> Result<()> {
    let dest = Utf8PathBuf::from(dest);
    let (record, written) = store.export(digest, &dest)?;
    if fmt.is_json() {
        output::success(&serde_json::json!({
            "exported": true,
            "digest": record.digest,
            "dest": dest.to_string(),
            "files": written.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
        }));
        return Ok(());
    }
    println!(
        "Exported {} ({} {} {}) to {dest}",
        record.short_digest(),
        record.kind.as_str(),
        record.name,
        record.version
    );
    for p in &written {
        println!("  {p}");
    }
    Ok(())
}

fn extract(store: &ArtifactStore, digest: &str, dest: &str, fmt: Format) -> Result<()> {
    let dest = Utf8PathBuf::from(dest);
    let record = store.extract(digest, &dest)?;
    if fmt.is_json() {
        output::success(&serde_json::json!({
            "extracted": true,
            "digest": record.digest,
            "dest": dest.to_string(),
            "files": record.file_count,
        }));
        return Ok(());
    }
    println!(
        "Extracted {} ({} {} {}) to {dest} ({} file(s))",
        record.short_digest(),
        record.kind.as_str(),
        record.name,
        record.version,
        record.file_count
    );
    Ok(())
}

/// The record as JSON for envelopes (serde derives the stable field order).
fn record_json(r: &ArtifactRecord) -> serde_json::Value {
    serde_json::to_value(r).unwrap_or_else(|_| serde_json::json!({}))
}
