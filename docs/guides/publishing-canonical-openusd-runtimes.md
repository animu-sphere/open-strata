# Publish canonical OpenUSD CY2026 runtimes

The canonical producer expands
[`support/openusd-runtime-matrix.json`](../../support/openusd-runtime-matrix.json)
into 18 immutable leaves: OpenUSD 26.05 and 26.08; Linux x86_64 and Windows
x86_64 `core`/`gl`/`vulkan`; and macOS arm64 `core`/`gl`/`metal`.

Inspect the full cross-platform plan without building:

```bash
python support/plan-openusd-runtimes.py
python support/plan-openusd-runtimes.py --github
```

The planner validates the complete producer contract, including ordered
versions and variants, runner/adapter identity, macOS SDK floors, release gates,
repository and leaf-publication policy. Host publishers consume this exact
expanded plan rather than reconstructing a second matrix in PowerShell. CI runs
the planner's regression suite and exercises the PowerShell `-PlanOnly` bridge.

On a primary producer host, inspect just the applicable local leaves:

```powershell
pwsh ./support/publish-openusd-runtimes.ps1 -PlanOnly
```

Build and export the local primary leaves:

```powershell
pwsh ./support/publish-openusd-runtimes.ps1 -Jobs 16
```

Use `-Version 26.08` or `-Variant metal` for local iteration. The matrix
rejects Metal outside macOS and Vulkan on macOS. Imaging variants always build
the upstream examples through `OpenUsdBuildPlan`; `core` explicitly disables
them. The producer validates the runtime contract and the selected backend,
exports SBOM/provenance, and names each leaf with
`<openusd-version>-<variant>-<os>-<arch>`.

Publication is an explicit protected step:

```powershell
pwsh ./support/publish-openusd-runtimes.ps1 -Publish -VerifyPublished
```

`-VerifyPublished` pulls the returned OCI digest into a clean store. Consumers
pin that digest, not the mutable leaf tag. Multi-platform convenience aliases
remain disabled in the support declaration until deterministic OCI index
transport is separately proven.

macOS uses the SDK and deployment target recorded in the matrix, measures both
back from Mach-O load commands, validates dylib relocation, loads the Metal
framework, observes an `MTLDevice`, and renders through HgiMetal. Linux and
Windows keep their platform-local OpenGL/Vulkan loader and device policies.

The older
[`publish-openusd-vulkan-runtimes.ps1`](../../support/publish-openusd-vulkan-runtimes.ps1)
and its legacy tags remain only for v0.22.0-v0.22.2 artifact maintenance.
