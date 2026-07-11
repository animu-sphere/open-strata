# OpenStrata × Kubernetes — execution backend (direction)

> Status: directional plan. OpenStrata owns the **runtime contract, artifacts,
> and validation**; Kubernetes is a **backend** that runs those contracts on a
> cluster — in parallel, isolated, and at scale.

Kubernetes is never required. `local` execution stays first-class; Kubernetes is
one of several pluggable **execution backends**:

```text
local · kubernetes · jenkins · (future) slurm / cloud batch / render farm
```

## Responsibility split

| OpenStrata (what & under which contract) | Kubernetes (where it runs) |
| --- | --- |
| CY / target / OS / Python-ABI resolution | Pod lifecycle |
| profile / capability / extension resolution | CPU / memory / GPU scheduling |
| OpenUSD feature + MaterialX resolution | parallel `Job` execution, node selection |
| CMake toolchain / preset generation | retry / timeout / autoscaling |
| plugin compatibility + verification | volume attachment, network isolation |
| runtime / extension / source digests | — |
| lockfile / provenance / validation report | — |
| artifact package / publish | — |

Principle: **Kubernetes decides *where*; OpenStrata decides *what*, and under
*which compatibility contract*.**

## Target use cases

CY×profile C++ build matrices; OpenUSD/MaterialX runtime validation; USD
file-format plugin validation farms; large USD asset convert/QC/lint; AI/GPU
runtime validation and batch inference; ephemeral task apps; and Jenkins as the
orchestration front-end delegating execution to the cluster.

## Non-goals (initially)

No CRD/Operator, no cluster setup/operation, no GPU-driver/GPU-Operator
management, no auto-authored RBAC/NetworkPolicy, no bespoke distributed cache or
sync service, no long-running Deployments. The initial goal is narrow: `ost`
resolves a target and can **generate → submit → monitor → collect** a Kubernetes
`Job`.

## Command surface

```text
ost submit build     --target <cy> --profile <p> --backend kubernetes [--dry-run --output yaml]
ost submit plugin-test --plugin <name> --target <cy> --profile <p> --backend kubernetes
ost submit ai-validate --profile <ai-profile> --backend kubernetes
ost submit matrix    --task <kind> --targets a,b --profiles x,y --backend kubernetes --max-parallel N [--fail-fast]
ost jobs list | show <id> | logs <id> [--follow] | wait <id> | artifacts <id> [--download <kind>] | cancel <id>
ost doctor kubernetes [--profile <ai-profile>]
```

The user-facing id is an OpenStrata **logical job id** (`ostj_…`); the Kubernetes
`Job` name (e.g. `ost-build-cy2026-usd-a1b2c3d4`) is an internal mapping.
`--output json` is a deterministic, machine-readable contract for CI.

## Execution-backend abstraction

A new `ost-execution` crate keeps the domain model separate from any YAML:

```text
ResolvedTask -> KubernetesJobRequest -> Kubernetes Job YAML
```

```rust
pub trait ExecutionBackend {
    fn submit(&self, request: SubmitRequest) -> Result<JobHandle>;
    fn status(&self, job: &JobHandle) -> Result<JobStatus>;
    fn logs(&self, job: &JobHandle, follow: bool) -> Result<LogStream>;
    fn cancel(&self, job: &JobHandle) -> Result<()>;
    fn collect_artifacts(&self, job: &JobHandle) -> Result<ArtifactCollection>;
}
```

Implementations: `LocalBackend`, `KubernetesBackend` (later `JenkinsBackend`,
`SlurmBackend`). **Transport (MVP): drive `kubectl` as a subprocess** (reuses the
user's kubeconfig/auth, light to implement, YAML is inspectable). A native `kube`
crate client is added only after the CLI contract stabilizes.

## Runtime / artifact strategy

Three layers, runtime not baked into the image by default:

```text
1. bootstrap image   ost + git + CMake/Ninja + uv + CA certs
2. runtime artifact  CY native libs, Python ABI, OpenUSD/MaterialX/…, extensions
3. task source       source + openstrata.toml + strata.lock + fixtures
```

```text
bootstrap image -> ost runtime pull (pinned digest) -> ost build/validate -> upload reports/artifacts
```

Pre-baked `openstrata/runtime:cy2026-usd-linux-x86_64` images are allowed later
for hot targets. **Every Job references digest-pinned runtime/extensions; `latest`
is rejected.** This is exactly the manifest/digest discipline OpenStrata already
applies to runtimes, plugins, and packages today, projected onto cluster jobs.

Source input modes: `git` (immutable commit SHA in CI), `local-archive`
(deterministic, `.gitignore`/`.ostignore`-aware, digested, uploaded), `artifact`
(by digest).

## Job model

`batch/v1 Job` for bounded build/validation/conversion/inference tasks. Initial
task kinds: `build`, `validate`, `plugin-test`, `ai-validate`, `asset-qc`,
`asset-convert`, `app-run`. Every Job carries stable labels
(`app.kubernetes.io/{name,managed-by}`, `openstrata.io/{job-id,task,target,profile,runtime-digest,project}`).
MVP manifests default to safe + minimal (non-root where possible,
`privileged: false`, no `hostPath`, explicit CPU/memory limits, `emptyDir`
workspace, minimal service account). Secrets only via Kubernetes/CI references —
never printed in YAML/stdout or written to provenance.

## GPU / AI

GPU **drivers are a host capability** (OpenStrata never installs them); the AI
runtime (CUDA toolkit, cuDNN, PyTorch, ONNX Runtime, TensorRT) is OpenStrata's.
An `ai-*` profile declares `requires_host.gpu` (vendor, min driver, CUDA driver
API, architectures, VRAM); the generated Job maps that to `nvidia.com/gpu`
requests + nodeSelector/affinity/tolerations. GPU validation includes accelerator
inspection, driver/CUDA-API report, `torch.cuda.is_available()`, a small CUDA op,
and ORT/TensorRT smoke tests when requested. This builds on Phase 8's host GPU
detection and AI profiles.

## Configuration & precedence

`~/.ost/config.toml` `[execution]` / `[execution.kubernetes]` (default backend,
namespace, context, service account, bootstrap image, artifact store, cache PVC,
TTL); project `openstrata.toml` may override execution/resources. Precedence:
**CLI flags > project config > user config > built-in defaults.**

## Provenance

Each Job emits job metadata, the resolved target manifest, runtime/extension/
source digests, build logs, validation JSON, JUnit XML, an artifact manifest, and
a provenance record linking source ↔ runtime ↔ extensions ↔ job result. Artifacts
are uploaded to the registry by the task (`kubectl cp` is debug-only). Eviction/
preemption is reported distinctly from a real build failure.

## Phased delivery

1. **Manifest export** — `ost submit … --backend kubernetes --dry-run --output
   yaml`: target/profile resolution + deterministic Job YAML with digest
   references; no cluster interaction.
2. **kubectl submission** — `submit` + `jobs show|logs|wait|cancel`; logical job
   metadata store.
3. **Artifact collection** — in-task upload, report download, provenance JSON,
   output digests (`jobs artifacts`).
4. **Matrix execution** — client-side scheduler with `--max-parallel`, status
   aggregation, `--fail-fast [--cancel-running-on-failure]`, result table/JSON.
5. **GPU tasks** — GPU resource requests, host-requirement → routing, GPU smoke
   tests (ties into Phase 8).
6. **Jenkins bridge** — `ost submit --output json` + `jobs wait` +
   `jobs artifacts --download reports` + a Jenkinsfile template (ties into
   Phase 5).
7. **Native Kubernetes client** — optionally replace `kubectl` subprocess calls
   with the `kube` crate, once the CLI contract is stable.
8. **CRD / Operator** — only if Job-based workflows hit a concrete limit that
   CLI + API integration cannot solve.

## First vertical slice (acceptance)

Against a dev cluster: `ost submit plugin-test --plugin toy --target cy2026
--profile usd --backend kubernetes` (with `--dry-run --output yaml` first), then
`ost jobs logs/wait/artifacts`. Success: the Job uses an immutable bootstrap
image digest and a pinned runtime digest; the source has a content digest; the
plugin builds against the designated CY/OpenUSD runtime; `plugInfo.json` is
discovered; `Sdf.FileFormat.FindByExtension("toy")` succeeds; the `.toy` fixture
opens as `SdfLayer`/`UsdStage`; JUnit + JSON reports upload; provenance links
source/runtime/extensions/result; and OpenStrata reports a logical job state
independent of raw Pod state.

## Decisions

1. **CLI + Job first, not an Operator (decided).** Build a robust CLI contract
   and a predictable `batch/v1 Job` backend before any CRD/Operator.
2. **`kubectl` transport for the MVP (decided).** Native `kube`-crate client is a
   later, optional swap behind the same `ExecutionBackend` trait.
3. **Local stays first-class (decided).** Kubernetes is an opt-in remote backend;
   nothing in the core workflow requires a cluster.
4. **Digest-pinned everything (decided).** Jobs reference runtime/extension/source
   by digest; `latest` is rejected — consistent with OpenStrata's existing
   manifest/provenance discipline.
