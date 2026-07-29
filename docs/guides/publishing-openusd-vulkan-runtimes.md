# Publish Vulkan-enabled OpenUSD cy2026 runtimes

The producer script builds and exports the four public cy2026 runtime variants
used by animu-sphere:

| OpenUSD | Platform | OCI tag |
| --- | --- | --- |
| 26.05 | Windows x86_64 | `26.05-windows-x86_64` |
| 26.05 | Linux x86_64 | `26.05-linux-x86_64` |
| 26.08 | Windows x86_64 | `26.08-windows-x86_64` |
| 26.08 | Linux x86_64 | `26.08-linux-x86_64` |

Every build explicitly forwards `--vulkan` and `--examples` to OpenUSD's
`build_usd.py`. The 26.08 builds also forward
`--python-install-dir=lib/python` to preserve the 26.05-compatible Python
layout; 26.05 predates that option and already uses the required layout. The
export uses OpenStrata 0.21's slim SDK layout. It retains `share/`, including
OpenUSD 26.08's Exec examples, while excluding the source and build trees.

## Prerequisites

- Windows x86_64 with Visual Studio's C++ workload.
- Python 3.13 and `ost` 0.21.x on `PATH`.
- A Windows Vulkan SDK with `VULKAN_SDK` set.
- WSL2 with a working Docker engine for the Linux builds.
- Enough free space for two OpenUSD source builds per operating system.

The Linux build runs in Docker under WSL2. Its Ubuntu 24.04 base deliberately
preserves the target contract of the existing Linux tags; `ost runtime export`
still measures and records the actual glibc floor. The image pins official
Vulkan-Headers and Vulkan-Utility-Libraries 1.4.350 plus Vulkan Memory
Allocator 3.4.0 because Ubuntu's headers are older than the HgiVulkan API used
by OpenUSD 26.05/26.08; the Vulkan loader and `shaderc_combined` remain Ubuntu
packages.

Keep the Windows `-WorkRoot` short. OpenUSD 26.08's Exec examples create deeply
nested MSVC tracking-log paths, so the default is `C:\usd\ovp`.

OpenUSD records an exact Python patch version in `pxrConfig.cmake`. Use
`-Python` to select the same 3.13 installation consumers use; the animu-sphere
cy2026 runtimes use Python 3.13.14.

## Build and export

From the OpenStrata repository root:

```powershell
.\support\publish-openusd-vulkan-runtimes.ps1
```

The script isolates the OpenUSD versions and operating systems into separate
`OST_HOME` stores. It validates the normal OpenStrata runtime contract, then
requires the installed HgiVulkan header, library, runtime plugin registration,
and OpenUSD examples before export. For 26.08 it additionally requires the
OpenExec/ExecIr examples.

Each run writes a new timestamped output directory and a `results.json`
containing the OpenUSD source revision, OpenStrata producer revision, artifact
digest, destination tag, and (after publication) OCI digest.

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
