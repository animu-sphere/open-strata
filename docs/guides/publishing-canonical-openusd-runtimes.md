# Publish canonical OpenUSD CY2026 runtimes

The canonical producer expands
[`support/openusd-runtime-matrix.json`](../../support/openusd-runtime-matrix.json)
into 16 immutable leaves: OpenUSD 26.05 and 26.08; Linux x86_64 and Windows
x86_64 `core`/`gl`/`vulkan`; and macOS arm64 `core`/`metal`.

The separate [lookdev matrix](../../support/openusd-lookdev-runtime-matrix.json)
adds OpenUSD 26.08 GL leaves for Linux and Windows x86_64 and a Metal leaf for
macOS arm64. These build usdview,
bundle PySide6 6.8.3 and PyOpenGL 3.1.9 alongside the runtime's `pxr` package,
and validate the viewer imports and `hydra-preview`/`usdview` capabilities.
Install those same Python UI packages on the build host before starting.
On macOS the publisher omits the PySide6 Mimer, ODBC, and PostgreSQL SQL
drivers, whose wheels link to external host libraries not used by usdview.
Viewer-capable profiles select upstream `--usd-imaging --usdview`; they reject
core or explicitly disabled viewer/Python builds. A physical GPU and a usable
display connection are required for the imaging export gate. A software-only
Xvfb/llvmpipe session cannot provide physical-device evidence.

```powershell
pwsh ./support/publish-openusd-runtimes.ps1 -Profile lookdev -PlanOnly
pwsh ./support/publish-openusd-runtimes.ps1 -Profile lookdev -Jobs 16 -Publish -VerifyPublished
```

The lookdev repository is
`oci://ghcr.io/animu-sphere/openstrata-runtime-cy2026-lookdev`, with tags
`26.08-gl-linux-x86_64`, `26.08-gl-windows-x86_64`, and
`26.08-metal-macos-arm64`. The macOS leaf is published with OCI digest
`sha256:0aa6c3b28c3f326b2a439cf1df19b9aa87c8d8d80506b61598b01840cfb6a8b9`
and artifact digest
`sha256:f727e7f75d80a596d94b15a8ae94ee641506c75a641953a353ed815a75be7521`.
It passed viewer imports, physical Metal device and render checks, and an
anonymous digest-pinned pull into a clean store with SBOM and provenance checks.
Source and dependency identity come from the managed build, including CY2026
oneTBB 2022.1.0.
Build/export work directories are separated by profile. On macOS arm64,
`-Profile lookdev` selects the Metal leaf by default; other hosts select GL.

To build and publish only the macOS lookdev leaf from a clean producer checkout:

```powershell
pwsh ./support/publish-openusd-runtimes.ps1 -Profile lookdev -Version 26.08 -Variant metal -PlanOnly
pwsh ./support/publish-openusd-runtimes.ps1 -Profile lookdev -Version 26.08 -Variant metal -Jobs 16 -Publish -VerifyPublished
```

The publisher requires a clean checkout so its recorded source revision matches
the declared producer. A successful digest pull verifies the published leaf.

macOS declares no `gl` lane. OpenStrata observes no physical OpenGL device on
macOS, so such a leaf could never carry the device and render evidence that
`check_exportable` requires of an imaging cell: it would build and validate,
then be refused at export. The matrix rejects it at plan time instead.

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
rejects Metal outside macOS, and both Vulkan and OpenGL on macOS. Imaging
variants always build
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
