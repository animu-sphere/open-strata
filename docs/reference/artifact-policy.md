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
