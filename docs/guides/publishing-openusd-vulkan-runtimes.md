# Publish Vulkan-enabled OpenUSD cy2026 runtimes

> **Current producer:** this guide documents the implemented Vulkan-specific
> bootstrap. v0.22.3 will generalize it into the data-driven multi-variant,
> multi-platform producer defined by the
> [canonical OpenUSD runtime proposal](../design/proposed/canonical-openusd-runtimes.md).
> Until that work ships, the commands and legacy tags below remain the factual
> procedure; they are not the final canonical naming contract.

The producer script builds and exports the four public cy2026 runtime variants
used by animu-sphere:

| OpenUSD | Platform | OCI tag |
| --- | --- | --- |
| 26.05 | Windows x86_64 | `26.05-windows-x86_64` |
| 26.05 | Linux x86_64 | `26.05-linux-x86_64` |
| 26.08 | Windows x86_64 | `26.08-windows-x86_64` |
| 26.08 | Linux x86_64 | `26.08-linux-x86_64` |

The approved Linux build selects the normalized `vulkan` cell with
`--openusd-variant vulkan`; Windows retains the existing explicit `--vulkan`
builder flag until a Windows compatibility cell is declared. Both forward
`--examples` to OpenUSD's `build_usd.py`. On Linux the selected cell supplies
the compatibility-critical Vulkan builder arguments and becomes artifact
selector identity. The 26.08 builds also forward
`--python-install-dir=lib/python` to preserve the 26.05-compatible Python
layout; 26.05 predates that option and already uses the required layout. The
export uses OpenStrata 0.21's slim SDK layout. It retains `share/`, including
OpenUSD 26.08's Exec examples, while excluding the source and build trees.

## Prerequisites

- Windows x86_64 with Visual Studio's C++ workload.
- PowerShell 7, Python 3.13.14, and `ost` 0.21.x or 0.22.x on `PATH`.
- A Windows Vulkan SDK with `VULKAN_SDK` set.
- WSL2 with a working Docker engine for the Linux builds.
- Enough free space for two OpenUSD source builds per operating system.

The Linux build runs in Docker under WSL2. Its Ubuntu 24.04 base deliberately
preserves the target contract of the existing Linux tags; `ost runtime export`
still measures and records the actual glibc floor. OpenStrata normalizes the CY
core's glibc value as a minimum constraint (for example, `>=2.28`) while the
artifact target records the exact measured requirement, so a newer producer is
never mislabeled as `glibc228`. The image pins official
Vulkan-Headers and Vulkan-Utility-Libraries 1.4.350 plus Vulkan Memory
Allocator 3.4.0 because Ubuntu's headers are older than the HgiVulkan API used
by OpenUSD 26.05/26.08; the Vulkan loader, Mesa Vulkan ICD (used for
device/render acceptance when no GPU is passed into the container), and
`shaderc_combined` remain Ubuntu packages. The image also installs Ubuntu's
versioned GCC 14 packages, which resolve to GCC 14.2 and satisfy the CY2026
compiler constraint instead of inheriting Ubuntu 24.04's default GCC 13.
Ubuntu's static `shaderc_combined` archive does not publish its glslang closure
to CMake's imported Vulkan target, so the builder exposes the package's
self-contained shared `libshaderc.so` under the combined lookup name. The
linked runtime records the real `libshaderc.so.1` SONAME and avoids unresolved
glslang symbols at plugin load time.
The Linux producer starts an isolated Xvfb display for `runtime validate` and
keeps it alive through `runtime export`, which deliberately re-runs the current
validation report before packing. This supplies the auxiliary Qt/OpenGL
context required by OpenUSD 26.05's `usdrecord`; the actual normalized render
remains Vulkan through `HGI_ENABLE_VULKAN=1`, backed by the enumerated Mesa
device when no GPU is passed into Docker.
OpenStrata also selects OpenUSD's `--onetbb` path and replaces its older
upstream default archive with the exact oneTBB 2022.1.0 source required by the
CY2026 `2022.x` cell. The fetched archive digest remains part of the captured
dependency identity.

Keep the Windows `-WorkRoot` short. OpenUSD 26.08's Exec examples create deeply
nested MSVC tracking-log paths, so the default is `C:\usd\ovp`.

OpenUSD records an exact Python patch version in `pxrConfig.cmake`. Use
`-Python` to select the same Windows 3.13 installation consumers use; the
Windows producer defaults to Python 3.13.14. The Linux container independently
pins the current deadsnakes Python 3.13.15 package through
`-LinuxPythonVersion`. Both exact versions satisfy the CY2026 `3.13.x` cell and
are recorded in artifact identity. Each producer refuses a different patch
rather than publishing misleading compatibility metadata; move its explicit
pin only when deliberately moving that public runtime and its consumers.

## Build and export

From the OpenStrata repository root:

```powershell
.\support\publish-openusd-vulkan-runtimes.ps1
```

The script isolates the OpenUSD versions and operating systems into separate
`OST_HOME` stores. It validates the normal OpenStrata runtime contract,
including native loader/device observations and an actual `usdrecord` frame
when the build host exposes the required GPU/display, then requires the
installed HgiVulkan header, library, runtime plugin registration, and OpenUSD
examples before export. Missing GPU/display prerequisites remain explicit
`not-run` state rather than blocking a build-only producer. For 26.08 it
additionally requires the OpenExec/ExecIr examples.

Each run writes a new timestamped output directory and a `results.json`
containing the OpenUSD source revision, OpenStrata producer revision, artifact
digest, destination tag, and (after publication) OCI digest.
The producer requires a clean OpenStrata checkout so the recorded revision is
the code that actually ran. During publication, it atomically checkpoints each
successful tag move into `results.json`, allowing a partial multi-tag failure to
be resumed without reconstructing already-published OCI digests.

Select a subset while iterating:

```powershell
.\support\publish-openusd-vulkan-runtimes.ps1 `
  -Version 26.08 -Platform linux -Jobs 16
```

When more than one Windows SDK is installed, pin the one compatible with the
selected MSVC toolset instead of relying on the process environment:

```powershell
.\support\publish-openusd-vulkan-runtimes.ps1 `
  -Platform windows `
  -Python C:\path\to\python-3.13.14\python.exe `
  -VulkanSdk C:\VulkanSDK\1.3.236.0 `
  -VulkanVmaInclude C:\VulkanSDK\1.4.350.0\Include
```

`VulkanVmaInclude` is only needed when the selected SDK predates its bundled
header-only Vulkan Memory Allocator. It supplements compile includes; Vulkan
and shaderc libraries still come entirely from `VulkanSdk`.

## Publish and verify

Set a GHCR identity and token with package write access without writing the
token into the repository:

```powershell
$env:OST_REGISTRY_USER = '<github-user>'
$env:OST_REGISTRY_PASSWORD = '<github-token>'
```

Build, move the four existing tags to the new artifacts, then verify each
published OCI digest through an anonymous pull:

```powershell
.\support\publish-openusd-vulkan-runtimes.ps1 -Publish -VerifyPublished
```

If the build was completed before credentials were available, publish its
recorded outputs without rebuilding:

```powershell
.\support\publish-openusd-vulkan-runtimes.ps1 `
  -PublishFrom C:\usd\openusd-vulkan-publish\output\<run>\results.json `
  -VerifyPublished
```

OCI tag movement changes `expected_oci_digest`, and a changed runtime changes
`runtime_artifact`. Update both pins in consumer `openstrata.ci.yaml` files from
the resulting `results.json`; do not pin the mutable tag.
