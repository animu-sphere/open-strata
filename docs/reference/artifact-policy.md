# Artifact trust policy

`openstrata-artifact-policy.toml` defines the minimum trust accepted during
artifact verification and the identities allowed to publish into protected OCI
namespaces. The parser is fail-closed: unknown fields, duplicate identifiers,
unsupported schema versions, invalid namespace prefixes, and dangling publisher
references are errors.

## Schema

```toml
schema = 1
minimum_trust = "unsigned"

[[protected_namespaces]]
namespace = "ghcr.io/animu-sphere"
minimum_trust = "verified"
allowed_publishers = ["openstrata-release"]

[[allowed_publishers]]
id = "openstrata-release"
trust = "verified"
repository = "animu-sphere/open-strata"
workflow_path = ".github/workflows/release.yml"
git_refs = ["refs/tags/v*"]
actors = ["release-bot"]
events = ["push"]
```

Top-level fields:

| Field | Required | Meaning |
| --- | --- | --- |
| `schema` | yes | Policy schema version. The only accepted value is `1`. |
| `minimum_trust` | no | Minimum for `ost artifact verify --policy`; defaults to `local`. |
| `protected_namespaces` | no | Registry/repository prefixes guarded by publisher rules. |
| `allowed_publishers` | no | Named OIDC identity rules referenced by protected namespaces. |

A protected `namespace` is a lowercase registry/repository prefix without a
scheme, tag, digest, or trailing slash. A numeric registry port is accepted,
for example `registry.internal:5000/vfx`. Matching respects path boundaries and
uses the most-specific configured prefix. Each protected namespace must name at
least one existing publisher whose `trust` meets that namespace's
`minimum_trust`.

Every allowed publisher matches all five identity dimensions: repository,
workflow path, git ref, actor, and event. Repository, workflow, actor, and event
matching is exact. A git-ref rule is exact unless it ends in one `*`, which
matches a prefix; no other wildcard form is accepted.

## Trust levels

Trust levels are ordered from least to most assured:

1. `local`
2. `unsigned`
3. `attested`
4. `verified`
5. `trusted`

An artifact imported directly into the local store is `local`. An artifact
registered through the existing gated publish path is `unsigned` until
attestation and identity evidence raise it in a later trust-chain step. Records
created before the trust field existed deserialize conservatively as `local`.

## Protected publishing

`ost artifact push` automatically searches the current directory and its
parents for `openstrata-artifact-policy.toml`. `--policy <FILE>` selects an
explicit policy, which is recommended for automation that may run outside the
project tree. If no policy is found, destinations retain the existing
unprotected push behavior.

A normalized OpenUSD runtime has a deterministic compatibility selector. When
its OCI destination has no explicit tag, `ost artifact push` uses that selector
as the convenience tag and also records it in the OCI manifest's
`io.openstrata.openusd.selector` annotation. The selector starts with the
platform, OS/architecture, and build variant and ends with a full SHA-256 over
the normalized artifact target (including its measured ABI floor), provider,
exact-version constraint, upstream OpenUSD release, and capability fields.
Provider versions that do not satisfy their declared constraints cannot receive
a selector. It is
therefore safe for exact compatibility comparison while staying within OCI's
128-character tag limit. An explicit destination tag remains authoritative.
In either form the tag is mutable convenience only: pin the returned
`@sha256:...` OCI digest for reproducible consumption.

`ost artifact resolve` reports the selector annotation carried by the resolved
OCI manifest. A digest-pinned `artifact pull` treats that annotation as a claim,
not authority: after fetching the producer manifest it re-derives the normalized
selector and requires exact agreement before importing anything into the local
registry. An annotation on a legacy/non-runtime artifact, an invalid OCI-tag
value, or a selector that disagrees with the producer identity fails closed.
Artifacts published before selector annotations existed remain consumable and
report the `openusd_selector` verification step as `skipped`.

## OpenUSD verification state

Normalized runtime artifacts keep compile, link, loader, physical-device, and
render verification as five independent fields under `openusd_verification`.
Each field is `passed`, `failed`, or `not-run`; absence on an older artifact
means the producer made no versioned claim. A managed source build that exits
successfully records compile and link as `passed`, but leaves loader,
physical-device, and render as `not-run`. In particular, selecting the `vulkan`
variant or compiling HgiVulkan does not establish that the build runner had a
Vulkan device or rendered a frame.

The state is part of schema-7 runtime digest identity. Runtime export copies it
to the producer manifest and retains the same value inside
`provenance.runtime_manifest`; artifact import requires exact agreement and
rejects unsupported state schemas. A local object refuses a second verification
identity for the same archive digest, and an OCI manifest digest binds the
producer-manifest layer carrying the state. `runtime show` and `artifact show`
expose all five fields in human and JSON output. Compatibility selectors remain
selectors for build/ABI compatibility rather than verification claims.

`ost runtime validate` probes the graphics loaders declared by the normalized
cell: OpenGL for `standard`, and OpenGL plus Vulkan for `vulkan`. It opens the
native loader library and requires its canonical API entry point, so the check
does not require `glxinfo`, `vulkaninfo`, or a Vulkan SDK. Each API is reported
independently. A producer-owned build runtime persists the aggregate only to
`openusd_verification.loader`; an artifact-sourced consumer reports its host
observation without replacing the producer's digest-bound state. A headless cell
skips the probe and remains `not-run`; a loader success never changes
`physical_device` or `render`. Exporting a normalized graphics runtime requires
that persisted loader state to be `passed`.

After all required loaders pass, validation separately probes physical devices.
For the approved Linux cells, OpenGL creates a one-pixel GLX pbuffer context and
records the renderer returned by the driver; Vulkan creates a minimal 1.0
instance and calls physical-device enumeration through the native loader. No
`glxinfo`, `vulkaninfo`, Vulkan SDK, or compile step is involved. A Linux host
without `DISPLAY` or `WAYLAND_DISPLAY` skips the OpenGL device observation and
leaves the aggregate producer field `not-run`; an attempted enumeration that
finds no device is `failed`.

When the selected Hgi backend has a passing device observation, validation runs
the runtime's own `usdrecord` on a generated sphere and requires both a clean
exit and a non-empty 64-pixel PNG. Vulkan cells set OpenUSD's documented
`HGI_ENABLE_VULKAN=1` backend selector for this process. That actual-frame probe
alone updates `render`; device state is never inferred from loader success, and
render state is never inferred from device enumeration. As with loader checks,
only a producer-owned build runtime persists these observations. An immutable
artifact consumer reports its current host results without replacing the
producer's digest-bound fields. Missing display or `usdrecord` prerequisites
remain `not-run`, so a non-GPU build host does not manufacture either success or
failure evidence.

A consumer can require an approved compatibility cell during the same pull:

```bash
ost artifact pull oci://registry.example/vfx/openusd@sha256:<digest> \
  --require-openusd cy2026/linux/x86_64/vulkan \
  --require-openusd-version 26.05
```

The cell is resolved from the named platform manifest rather than restated on
the command line. Before local import, `ost` compares the normalized
platform/architecture, compiler and native runtime providers, C++ standard,
exact Python and TBB versions/providers, variant, and capability set. A producer
version must satisfy the consumer cell's constraint; required capabilities must
all be present. `--require-openusd-version` pins the exact upstream OpenUSD
release and requires `--require-openusd`. Failure evidence includes `dimension`,
`requirement`, and `selected_artifact` objects, and the error hint tells the
caller which artifact selection to correct. Without `--require-openusd`, legacy
pull behavior remains unchanged and the `openusd_requirement` verification step
is `skipped`.

The same pull can make trust evidence a pre-import requirement:

```bash
ost artifact pull oci://registry.example/vfx/openusd@sha256:<digest> \
  --require-sbom --require-provenance \
  --minimum-trust verified \
  --policy openstrata-artifact-policy.toml
```

Every fetched SBOM or provenance sidecar is digest- and subject-validated even
when optional. `--require-sbom` and `--require-provenance` additionally reject
absence. The effective trust is derived for this pull only: valid provenance
establishes `attested`; provenance matching an allowed publisher plus a valid
SBOM can establish that publisher's higher trust. The required floor is the
stricter of `--minimum-trust` and the policy's `minimum_trust`. Failed evidence
or trust validation happens before local import and leaves no usable artifact
behind. Successful human and JSON output name the effective and required trust
levels and any matched publisher; the stored record remains conservatively
classified by its transport rather than receiving sticky evidence trust.

For a protected destination, `ost` requests a short-lived GitHub Actions OIDC
token directly from `https://token.actions.githubusercontent.com`, using the
runner-provided request URL and bearer token. It refuses a different request
origin and validates the returned issuer, the fixed
`openstrata-artifact-publish` audience, validity window, and the repository,
workflow path, git ref, actor, and event claims before contacting the registry.
The workflow job therefore needs:

```yaml
permissions:
  contents: read
  id-token: write
  packages: write
```

If the identity is missing or does not match, the push fails before any
registry request. `--allow-untrusted-publisher` is the explicit break-glass
override for a protected destination. Human and JSON success output record the
policy path, protected namespace, matched publisher/trust, or that the override
was used.

## Verification

```bash
ost artifact verify sha256:... \
  --policy openstrata-artifact-policy.toml
```

Generated CI can also pass an explicit lane/target floor:

```bash
ost artifact verify sha256:... --minimum-trust verified \
  --require-sbom --require-provenance \
  --policy openstrata-artifact-policy.toml
```

When both controls are present, verification enforces the stricter of
`--minimum-trust` and the policy file's `minimum_trust`. The policy file still
provides the allowed-publisher identities used by required provenance checks.

The command first runs the existing archive and per-file integrity checks, then
compares the artifact's effective trust with `minimum_trust`. Effective trust is
the stronger of the stored record trust and independently revalidated evidence:
valid subject-bound provenance establishes `attested`; when required provenance
matches an allowed publisher and a valid SBOM is also present, that publisher's
declared trust applies. This derivation is non-sticky — importing an exported
artifact still records `local`, so a copied `record.json` cannot grant trust.
Provenance content is digest-bound and policy-matched but not yet
cryptographically signed (SEC-005): treat evidence-derived trust as an assertion
about a handoff you already control — such as artifacts inside one workflow
run — not as protection against an attacker who can author the sidecar files.
Human output includes the effective and required levels. JSON output keeps
`data.trust` as the effective value and adds `record_trust` plus
`evidence_trust`, alongside the `data.policy` result. A trust failure exits with
validation status `5`.

With `--require-provenance`, the same policy's `allowed_publishers` also gates
the builder identity embedded in the SLSA/in-toto sidecar. This is distinct
from the live OIDC identity used at push time: verification proves the recorded
build, while protected publishing authorizes the current registry mutation.
See [artifact-evidence.md](artifact-evidence.md).

## Stable errors

| Code | Category | Meaning |
| --- | --- | --- |
| `ARTIFACT_POLICY_READ_FAILED` | I/O | The policy file could not be read. |
| `ARTIFACT_POLICY_PARSE_FAILED` | configuration | TOML or its strict field shape is invalid. |
| `ARTIFACT_POLICY_SCHEMA_UNSUPPORTED` | configuration | `schema` is not supported. |
| `ARTIFACT_POLICY_INVALID` | configuration | Cross-field or semantic validation failed. |
| `ARTIFACT_POLICY_IDENTITY_UNAVAILABLE` | precondition | GitHub Actions cannot provide an OIDC identity, usually because `id-token: write` is missing. |
| `ARTIFACT_POLICY_IDENTITY_INVALID` | validation | The OIDC endpoint or returned issuer, audience, validity window, or claims are invalid. |
| `ARTIFACT_POLICY_TRUST_INSUFFICIENT` | validation | Artifact trust is below `minimum_trust`. |
| `ARTIFACT_POLICY_PUBLISHER_UNTRUSTED` | validation | No allowed publisher matched every identity claim. |
| `ARTIFACT_OPENUSD_SELECTOR_MISMATCH` | validation | The resolved OCI selector annotation cannot be re-derived exactly from the fetched producer manifest. |
| `ARTIFACT_OPENUSD_IDENTITY_MISSING` | validation | A consumer cell was required but the selected artifact has no normalized OpenUSD identity. |
| `ARTIFACT_OPENUSD_VERSION_MISMATCH` | validation | The selected artifact's upstream OpenUSD release differs from the exact consumer requirement. |
| `ARTIFACT_OPENUSD_PLATFORM_MISMATCH` | validation | The normalized platform, OS, architecture, or compatibility schema differs from the required cell. |
| `ARTIFACT_OPENUSD_TOOLCHAIN_MISMATCH` | validation | Compiler, C++ standard, or native runtime identity does not satisfy the required cell. |
| `ARTIFACT_OPENUSD_PYTHON_MISMATCH` | validation | Python family, provider, or exact version does not satisfy the required cell. |
| `ARTIFACT_OPENUSD_TBB_MISMATCH` | validation | TBB family, provider, or exact version does not satisfy the required cell. |
| `ARTIFACT_OPENUSD_GRAPHICS_MISMATCH` | validation | The selected variant or capability set does not satisfy the required cell. |
| `ARTIFACT_SBOM_REQUIRED` | validation | Pull or verify required a subject-bound SPDX SBOM, but none was present. |
| `ARTIFACT_PROVENANCE_REQUIRED` | validation | Pull or verify required subject-bound SLSA/in-toto provenance, but none was present. |
