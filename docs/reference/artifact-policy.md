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
scheme, tag, digest, or trailing slash. Matching respects path boundaries and
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

## Verification

```bash
ost artifact verify sha256:... \
  --policy openstrata-artifact-policy.toml
```

The command first runs the existing archive and per-file integrity checks, then
compares the record's trust with `minimum_trust`. Human output includes the
actual and required levels. JSON output adds `data.trust` and a `data.policy`
object containing `path`, `minimum_trust`, `passed`, and any policy error code.
A trust failure exits with validation status `5`.

## Stable errors

| Code | Category | Meaning |
| --- | --- | --- |
| `ARTIFACT_POLICY_READ_FAILED` | I/O | The policy file could not be read. |
| `ARTIFACT_POLICY_PARSE_FAILED` | configuration | TOML or its strict field shape is invalid. |
| `ARTIFACT_POLICY_SCHEMA_UNSUPPORTED` | configuration | `schema` is not supported. |
| `ARTIFACT_POLICY_INVALID` | configuration | Cross-field or semantic validation failed. |
| `ARTIFACT_POLICY_TRUST_INSUFFICIENT` | validation | Artifact trust is below `minimum_trust`. |
| `ARTIFACT_POLICY_PUBLISHER_UNTRUSTED` | validation | No allowed publisher matched every identity claim. |
