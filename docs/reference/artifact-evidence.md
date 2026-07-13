# Artifact evidence bundles

OpenStrata producer commands carry evidence beside the content archive and
`manifest.json`:

| File | Format | OCI media type | Generation |
| --- | --- | --- | --- |
| `sbom.spdx.json` | SPDX 2.3 JSON | `application/spdx+json` | Generated for plugin, project-package, and runtime artifacts. |
| `provenance.intoto.jsonl` | in-toto Statement v1 with SLSA provenance v1 predicate | `application/vnd.in-toto+json` | Generated when complete build metadata is available. |

The archive's `sha256:<digest>` remains the OpenStrata artifact identity.
Evidence files are separate, digest-addressed sidecars recorded in
`record.json`, included in `SHA256SUMS`, preserved by artifact import/export and
pull, and attached as OCI layers by `ost artifact push`.

## Evidence binding

The SPDX package checksum and the in-toto subject both identify the artifact
archive SHA-256. Provenance also binds its source repository and revision to the
producer manifest's `build.source`. When an artifact policy is supplied with a
required provenance check, the attested builder identity must match one of the
policy's `allowed_publishers` rules across repository, workflow path, git ref,
actor, and event.

GitHub Actions packaging records build metadata only when all of
`GITHUB_REPOSITORY`, `GITHUB_SHA`, `GITHUB_WORKFLOW_REF`, `GITHUB_REF`,
`GITHUB_ACTOR`, and `GITHUB_EVENT_NAME` are present and non-empty. A partial
environment does not produce provenance. Other producers may supply the same
`build.source` and `build.builder` manifest shape explicitly; explicit build
metadata always wins and is never replaced by the ambient CI environment.

## Verification

Normal `ost artifact verify` validates any evidence that is present, including
its recorded digest and semantic binding. Older artifacts with no evidence
remain valid. Use release or policy gates to require sidecars:

```text
ost artifact verify sha256:<digest> --require-sbom
ost artifact verify sha256:<digest> --require-provenance --policy openstrata-artifact-policy.toml
```

The JSON report includes `data.evidence.sbom` and
`data.evidence.provenance`, each with required/present/pass state, digest, and a
stable error code. Provenance additionally reports the matched publisher id.

Stable evidence errors include:

| Code | Meaning |
| --- | --- |
| `ARTIFACT_SBOM_REQUIRED` | The caller required an SBOM but none is attached. |
| `ARTIFACT_PROVENANCE_REQUIRED` | The caller required provenance but none is attached. |
| `ARTIFACT_EVIDENCE_DIGEST_MISMATCH` | Stored or transported evidence bytes do not match their record. |
| `ARTIFACT_SBOM_INVALID` | The SBOM is not a valid SPDX 2.3 JSON document. |
| `ARTIFACT_SBOM_SUBJECT_MISMATCH` | The SBOM does not identify the artifact archive digest. |
| `ARTIFACT_PROVENANCE_INVALID` | The provenance JSONL is empty or malformed. |
| `ARTIFACT_PROVENANCE_SUBJECT_MISMATCH` | No statement subject matches the artifact archive digest. |
| `ARTIFACT_PROVENANCE_SOURCE_MISMATCH` | Attested source repository/revision differs from manifest build metadata. |
| `ARTIFACT_PROVENANCE_BUILDER_UNTRUSTED` | Builder identity is missing, invalid, or not allowed by policy. |

