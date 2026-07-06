# Remote Artifact Transport and GitHub-hosted Source CI

Direction document for closing the hosted-lane bootstrap gap surfaced by
dogfooding report #10 (2026-07-05, v0.8.0). Targeted at v0.9.0 (P0) with the
publish/trust phases following. The roadmap tracks the ranked backlog; this
document is the design contract.

## Goal

On a GitHub-hosted runner, with no pre-existing local environment or
self-hosted runner state, the following sequence must be reproducible:

```text
PR / push
  -> GitHub-hosted runner
  -> ost bootstrap
  -> pull the runtime SDK artifact from a remote registry by digest
  -> re-verify archive / manifest / file digests
  -> plugin build
  -> plugin test / doctor on the runtime
  -> plugin package
  -> persist validation report / CI evidence / artifact digests
```

The essence of this plan is **not** "build a remote registry." It is closing
the CI contract to the point where a clean GitHub-hosted runner can safely
restore the runtime pinned by a support line, verify a plugin, and return
artifacts with an inspectable evidence trail.

---

## Target outcomes

### Required

- `ost` can be reliably bootstrapped on a GitHub-hosted runner.
- The runtime SDK artifact is fetched by the **full digest the support line
  pins**.
- A fetched artifact receives verification equal to or stronger than local
  import.
- Jobs succeed with an empty cache. Cache is an optimization, never a
  correctness precondition.
- Fork PRs can pull artifacts read-only but have no path to publish,
  privileged runners, or secrets.
- Only protected branches / trusted lanes can publish artifacts.
- Build reports and CI evidence record the source locator, manifest digest,
  archive digest, runtime digest, and transport identity.
- The generated workflow is proven end to end in a public fixture repository.

### Non-goals

- Designing and operating a bespoke general-purpose artifact registry server
  from day one.
- Using GitHub Actions cache as a substitute for an artifact registry.
- Building the runtime on a GitHub-hosted runner on every run.
- Force-porting support lanes that need commercial DCC licenses onto
  GitHub-hosted runners.

---

## Principles

### 1. OCI artifact transport is the first candidate

The first remote transport uses an OCI Distribution Spec-compatible registry.
On the CLI side, adopt a design compatible with the ORAS artifact model.

Rationale:

- OpenStrata artifacts are already content-addressed; a digest-centric design
  composes naturally.
- OCI registries connect to GitHub Container Registry, Harbor, AWS ECR,
  Google Artifact Registry, Azure Container Registry, and others.
- Runtime archives, manifests, validation reports, provenance, and SBOMs can
  travel as one artifact bundle.
- Adding future remote backends never makes artifact identity depend on the
  transport.

### 2. Tags are convenience; digests are the contract

Tags are allowed as human-facing aliases, but CI support lines and the
lockfile treat the digest as the only root of trust.

Forbidden:

- Fetching a runtime in CI by `latest` or any mutable-tag-only reference.
- Skipping the manifest-digest check against the lockfile after a pull.
- Downgrading a digest-verification failure to a warning and continuing.

### 3. The local registry is not retired

The existing local content-addressed registry stays useful for development,
air-gapped environments, self-hosted runner caches, and fixture tests. Remote
transport is an **additional backend**, not a replacement.

### 4. Transport is separated from the artifact core

Archive structure, manifest schema, digest computation, and verification
policy stay in `ost-artifact`. OCI / filesystem / future HTTP transports are
adapters.

---

## Architecture

```text
+-----------------------------+
| ost CLI                     |
| artifact push / pull / show |
+-------------+---------------+
              |
+-------------v---------------+
| Artifact Transport Contract |
| resolve / pull / push       |
| auth context / provenance   |
+-------------+---------------+
              |
   +----------+-----------+
   |                      |
+--v----------------+ +---v--------------------+
| Local Backend     | | OCI Backend             |
| filesystem store  | | GHCR / Harbor / ECR ... |
+-------------------+ +-------------------------+
              |
+-------------v---------------+
| Artifact Verification Core  |
| archive, manifest, files,   |
| type, runtime digest, trust |
+-----------------------------+
```

### Transport contract

Conceptually, the Rust interface owns these responsibilities:

```rust
trait ArtifactTransport {
    fn resolve(&self, reference: &ArtifactReference) -> Result<ResolvedArtifact>;
    fn pull(&self, resolved: &ResolvedArtifact, destination: &Path) -> Result<PullReceipt>;
    fn push(&self, artifact: &VerifiedArtifact, destination: &ArtifactReference) -> Result<PushReceipt>;
}
```

Implementation requirements:

- `resolve` may turn a tag into a digest, but CI policy can reject mutable
  references outright.
- `pull` is never successful on transport success alone. The result must pass
  through the verification core.
- `push` cannot execute unless the trusted-lane and credential policies pass.
- `PullReceipt` and `PushReceipt` record the remote locator, resolved digest,
  registry identity, timestamp, and credential mode.

---

## Artifact bundle specification

An OCI bundle represents a runtime or a plugin package. The artifact type
must always be identifiable from the manifest.

### Recommended layout

```text
OCI manifest
  - config: OpenStrata artifact descriptor (JSON)
  - layer: canonical artifact archive
  - layer: artifact manifest JSON
  - layer: validation report JSON (optional / required by artifact type)
  - layer: provenance JSON (optional initially; required in release lane later)
  - layer: SBOM JSON (future required for release lane)
```

### Required metadata

- `artifact_type`: `runtime-sdk` / `plugin-package` / future types
- `artifact_digest`: OpenStrata canonical artifact digest
- `archive_digest`: downloaded archive digest
- `manifest_digest`: canonical manifest digest
- `runtime_digest`: the artifact itself for runtime artifacts; the build/test
  runtime for plugin packages
- `platform`: OS / arch / ABI / Python ABI
- `validation_status`
- `created_at`
- `producer`: `ost` version and build identity

### Verification order on pull

1. Resolve the remote reference.
2. Obtain the resolved OCI digest.
3. Download descriptor / layers.
4. Verify the archive digest.
5. Verify the manifest schema and manifest digest.
6. Run pre-extraction safety checks on the archive.
7. Verify post-extraction file digests against the manifest file list.
8. Match the artifact type against the support line's requirements.
9. Verify runtime digest / target / ABI / platform.
10. Evaluate the trust policy.
11. Import atomically into the local registry.

If any step fails, the artifact is never left in a usable state.

---

## CLI design

Remote backends are handled explicitly, consistent with existing commands.

### Reference form

```text
oci://ghcr.io/owner/openstrata-runtime@sha256:<oci-manifest-digest>
oci://registry.example.com/vfx/openusd-runtime:usd-24.08-linux-x86_64
```

CI and the lockfile require digest-bearing references. Tag references are
limited to interactive resolve and publish-time aliases.

### Proposed commands

```bash
# Resolve a remote reference and print the immutable digest
ost artifact resolve oci://ghcr.io/owner/openstrata-runtime:usd-24.08-linux-x86_64 --json

# Fetch from remote, verify, and import into the local registry
ost artifact pull oci://ghcr.io/owner/openstrata-runtime@sha256:<digest> --json

# Publish a local artifact to a remote registry
ost artifact push sha256:<openstrata-artifact-digest> \
  oci://ghcr.io/owner/openstrata-runtime:usd-24.08-linux-x86_64 \
  --json

# Inspect provenance including remote origin
ost artifact show sha256:<digest> --provenance --json
```

### Minimum JSON output

`ost artifact pull --json` returns at least:

```json
{
  "status": "ok",
  "artifact_digest": "sha256:...",
  "remote": {
    "locator": "oci://ghcr.io/owner/openstrata-runtime@sha256:...",
    "resolved_oci_digest": "sha256:...",
    "registry": "ghcr.io",
    "auth_mode": "github-oidc-or-token"
  },
  "verification": {
    "archive_digest": "passed",
    "manifest_digest": "passed",
    "file_digests": "passed",
    "support_line_match": "passed",
    "trust_policy": "passed"
  },
  "local_import": {
    "status": "imported",
    "path": "..."
  },
  "warnings": []
}
```

### Error classification

Define stable error codes so CI can classify actionable failures:

- `ARTIFACT_REFERENCE_MUTABLE`
- `ARTIFACT_REMOTE_NOT_FOUND`
- `ARTIFACT_AUTH_DENIED`
- `ARTIFACT_TRANSPORT_FAILED`
- `ARTIFACT_OCI_DIGEST_MISMATCH`
- `ARTIFACT_ARCHIVE_DIGEST_MISMATCH`
- `ARTIFACT_MANIFEST_INVALID`
- `ARTIFACT_FILE_DIGEST_MISMATCH`
- `ARTIFACT_PLATFORM_MISMATCH`
- `ARTIFACT_SUPPORT_LINE_MISMATCH`
- `ARTIFACT_TRUST_POLICY_DENIED`
- `ARTIFACT_PUBLISH_POLICY_DENIED`

---

## GitHub-hosted source CI design

### Source CI responsibilities

GitHub-hosted runners are limited to the public, portable source lane:

- bootstrapping `ost`
- pulling the runtime artifact pinned by the support line
- plugin build
- plugin test / doctor
- package
- uploading validation reports and CI evidence

Never placed on GitHub-hosted runners:

- private DCC installs
- private license servers
- publish steps that need protected secrets
- proprietary runtime builds
- privileged support lanes

### Workflow shape

```yaml
jobs:
  plugin-source-ci:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      packages: read
      id-token: write # only if OIDC is adopted
    steps:
      - checkout
      - install ost
      - restore cache # optional only
      - ost artifact pull <digest-pinned-runtime-reference>
      - ost plugin build
      - ost plugin doctor
      - ost plugin test
      - ost plugin package
      - upload reports and package as GitHub workflow artifacts
```

### Bootstrap policy

`ost` is installed from a version-pinned release asset or a verified package
manager artifact.

Requirements:

- The version is pinned by the workflow / CI contract.
- The checksum or provenance is verified.
- `ost --version --json` is saved into the evidence.
- A bootstrap failure is never conflated with an artifact/runtime failure.

### Runtime pull policy

- Source CI does not build runtimes.
- The `openstrata.ci.yaml` support line carries the runtime's remote reference
  and the expected OpenStrata artifact digest.
- `ost artifact pull` emits both the resolved remote digest and the local
  artifact digest into CI evidence.
- GitHub Actions cache may be used to reuse a validated local registry
  directory, but a cache miss falls back to the remote pull.

### Cache policy

The cache key includes at least:

```text
ost-version
os
arch
support-line-id
runtime-artifact-digest
```

Mutable tags, branch names, and workflow run IDs are never used as cache-key
identity.

---

## CI contract extension

Add remote transport to the support-line runtime specification in
`openstrata.ci.yaml`:

```yaml
support_lines:
  - id: usd-24.08-linux-x86_64
    target: linux-x86_64
    runtime:
      artifact_digest: sha256:<openstrata-artifact-digest>
      remote:
        uri: oci://ghcr.io/owner/openstrata-runtime@sha256:<oci-manifest-digest>
        expected_oci_digest: sha256:<oci-manifest-digest>
      trust: verified
    runner_profile: github-hosted-linux
    lane: source-ci
```

> **Shipped shape (v0.9.0):** the existing `openstrata.ci.yaml` schema pins
> the runtime as a flat `runtime_artifact` digest, so the remote reference
> landed additively as a sibling `runtime_remote: {uri, expected_oci_digest}`
> block per cell — not a nested `runtime:` object — plus a matrix-level
> `bootstrap.ost: {version, repository, sha256}` pin for the hosted `ost`
> install. Semantics are as specified here.

### Policy

- `source-ci` requires `runtime.remote.uri` and a digest pin.
- `release` / `support` lanes may require `trust: trusted`.
- `self-hosted` lanes may allow air-gapped local import, but the evidence
  records the source.
- `publish` actions require a trusted lane, a protected ref, and explicit
  permission.

---

## Authentication and permission boundaries

### Fork PRs

Allowed:

- anonymous pull from a public registry
- pull with a read-only package token
- source build / test / package
- report upload as GitHub workflow artifacts

Forbidden:

- remote artifact push
- writes to a release registry
- selecting self-hosted runners
- reading protected secrets
- exposure of publish credentials in the environment

### Protected branch / trusted lane

Allowed:

- signed / trusted runtime artifact pull
- plugin package publish
- provenance / SBOM attach
- registry write token or OIDC federation

### Credential model

In order of preference:

1. Short-lived credentials via OIDC federation
2. GitHub Packages read/write via `GITHUB_TOKEN`
3. Scoped repository / organization secret tokens

Avoid parking long-lived PATs in workflows.

---

## Trust policy

Transport integrity and artifact trust are separate concerns.

### Integrity

The downloaded bytes match the expected digest / manifest / file digests.

### Trust

Who produced and published the artifact, and under what policy.

Initial levels:

- `local`: local import or unsigned artifact. Development lanes only.
- `verified`: passed digest / manifest / file verification. Allowed in
  source CI.
- `trusted`: satisfies trusted-publisher / provenance / policy requirements.
  Required for release/support lanes.

Future:

- registry publisher allowlist
- Sigstore / cosign verification
- SLSA provenance verification
- SBOM presence / license policy
- runtime source revision allowlist

---

## Implementation phases

### Phase 1: transport abstraction and read-only OCI pull

Goal: a GitHub-hosted runner can pull a digest-pinned runtime.

- Add the `ArtifactTransport` contract.
- Adapterize the filesystem backend, preserving existing behavior.
- Implement the OCI backend starting from read-only pull.
- Add `ost artifact resolve` and `ost artifact pull`.
- Connect archive / manifest / file verification and atomic local import.
- Add the digest-pin enforcement policy.
- Build integration tests against a fixture or mock OCI registry.

Done when:

- A runtime pull succeeds in a clean-Linux-runner-equivalent environment with
  no cache.
- Corrupt archives, manifest mismatches, wrong platforms, and mutable-only
  references all fail.
- Pull results return JSON evidence.

### Phase 2: GitHub Actions bootstrap and source-CI closure

Goal: the generated workflow passes plugin tests on a clean hosted runner.

- Add a pinned `ost` release-asset bootstrap step to the renderer.
- Add checksum / provenance verification.
- Add the runtime pull step to the generated workflow.
- Add cache restore/save as an optional optimization.
- Connect plugin build / doctor / test / package / report upload end to end.
- Stand up a public fixture repository; verify PR, push, and cache-miss runs.

Done when:

- Read-only source CI succeeds on a public fork PR.
- Cache miss and cache hit both use the identical runtime digest.
- Reports record ost version, runtime digest, OCI digest, plugin digest, and
  workflow identity.

### Phase 3: OCI push and publish policy

Goal: trusted lanes can publish validated artifacts to a remote registry.

- Add `ost artifact push`.
- Connect plugin package publish to the OCI transport.
- Implement the protected-branch / trusted-lane / explicit-permission policy.
- Use OIDC or scoped GitHub tokens.
- Perform a remote pull verification after push.
- Record the immutable release tag / digest in the evidence.

Done when:

- Publish is a policy failure on fork PRs.
- Only the protected main/release lane can publish.
- A published artifact can be pulled and re-verified on a clean runner.

### Phase 4: trust and release hardening

Goal: support/release lanes only use verified, trustable artifacts.

- Add trust levels to the manifest / CI contract.
- Add publisher identity / provenance attach.
- Introduce SBOM attach and the release policy.
- Introduce a trusted runtime allowlist for support lines.

Done when:

- Runtimes that do not meet the `trusted` policy are rejected in
  release/support lanes.
- Provenance / SBOM are traceable from both package evidence and the registry
  artifact.

---

## Test strategy

### Unit tests

- reference parsing
- OCI digest pin policy
- descriptor-to-manifest mapping
- archive / manifest / file verification
- trust policy evaluation
- error code stability

### Integration tests

- local registry <-> OCI registry round trip
- digest-pinned pull
- mutable tag resolve then explicit digest pull
- corrupt layer
- manifest substitution
- wrong-platform artifact
- interrupted download / retry
- cache miss and cache hit behavior
- package publish and remote re-pull

### Security tests

- a fork-PR workflow cannot obtain publish credentials
- a fork-PR generated workflow never references a self-hosted label
- mutable remote references are rejected in source CI
- artifact traversal / symlink / special-file checks remain enforced after a
  remote pull
- OIDC audience / registry scope mismatch is rejected

### Public E2E fixture

Maintain at least one public fixture repository:

- tiny OpenUSD plugin
- pinned public runtime artifact
- generated GitHub workflow committed to the repository
- PR source CI
- push source CI
- explicit cache-disable run
- report artifact inspection

The fixture is a product-level contract that validates OpenStrata
implementation changes.

---

## Acceptance criteria

"GitHub-hosted source CI is fully closed" when all of the following hold:

1. `ost` bootstraps version-pinned on a fresh GitHub-hosted runner.
2. The runtime SDK is fetched from the remote registry by immutable digest.
3. The runtime artifact passes archive, manifest, file-list, platform, and
   support-line verification.
4. Plugin build/test/package succeeds on a cache miss.
5. The runtime digest in the evidence is identical on a cache hit.
6. Fork PRs run read-only source CI but cannot reach publish, secrets, or
   privileged runners.
7. The trusted-branch publish lane publishes a remote artifact that a clean
   runner can pull and re-verify.
8. CI reports include the source revision, ost version, workflow identity,
   runtime OpenStrata digest, remote OCI digest, plugin artifact digest, and
   validation outcome.
9. The fixture E2E test for generated workflows stays green in continuous CI.

---

## Implementation priority

### P0

1. OCI read-only pull
2. digest pin / verification policy
3. generated GitHub-hosted source CI including the `ost` bootstrap
4. public E2E fixture

### P1

1. OCI push
2. protected publish policy
3. OIDC credential integration
4. cache optimization

### P2

1. provenance / SBOM attach
2. trust allowlist
3. registry mirroring / air-gapped sync
4. multi-registry failover

---

## Decision notes

- The initial implementation does not make the remote registry the sole
  source of truth. The local registry remains as a verified cache /
  air-gapped import / developer workflow.
- GitHub-hosted source CI stays focused on public, portable OpenUSD plugin
  verification. Jobs needing DCC licenses or proprietary runtimes are
  isolated to self-hosted support lanes.
- GitHub Actions cache is a bandwidth/time optimization, never the basis of
  artifact identity or trust.
- Mutable tags may stay resolvable for developer experience, but are fixed to
  an immutable digest before CI execution.
- Adding transport backends never changes the meaning of the OpenStrata
  artifact digest, manifest schema, or validation evidence.

---

## End value

When this plan completes, OpenStrata stops being "a tool that generates
workflows."

A clean GitHub-hosted runner restores the runtime pinned by the support
contract, builds / verifies / packages an OpenUSD plugin, and returns an
evidence trail that can be inspected after the fact. Plugin developers get
reproducible verification per PR without depending on self-hosted runners or
hand-prepared runtimes.
