#requires -Version 7.0

[CmdletBinding()]
param(
    [ValidateSet('usd', 'lookdev')][string] $Profile = 'usd',
    [ValidateSet('26.05', '26.08')][string[]] $Version = @('26.05', '26.08'),
    [ValidateSet('core', 'gl', 'vulkan', 'metal')][string[]] $Variant = @('core', 'gl', 'vulkan', 'metal'),
    [ValidateRange(1, 256)][int] $Jobs = [Environment]::ProcessorCount,
    [string] $WorkRoot = (Join-Path $PSScriptRoot '.openusd-runtime-work'),
    [string] $Registry,
    [switch] $Publish,
    [switch] $VerifyPublished,
    [switch] $PlanOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$matrixPath = Join-Path $PSScriptRoot 'openusd-runtime-matrix.json'
if ($Profile -eq 'lookdev') {
    $matrixPath = Join-Path $PSScriptRoot 'openusd-lookdev-runtime-matrix.json'
    if (-not $PSBoundParameters.ContainsKey('Version')) { $Version = @('26.08') }
    if (-not $PSBoundParameters.ContainsKey('Variant')) { $Variant = if ($IsMacOS) { @('metal') } else { @('gl') } }
}
if (-not $Registry) { $Registry = "oci://ghcr.io/animu-sphere/openstrata-runtime-cy2026-$Profile" }
$planner = Join-Path $PSScriptRoot 'plan-openusd-runtimes.py'
$validator = Join-Path $PSScriptRoot 'validate-openusd-runtime.py'
$repositoryRoot = Split-Path -Parent $PSScriptRoot

$hostOs = if ($IsWindows) { 'windows' } elseif ($IsMacOS) { 'macos' } else { 'linux' }
$hostArch = if ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq 'Arm64') { 'arm64' } else { 'x86_64' }
$python = (Get-Command python -ErrorAction Stop).Source
$planArguments = @($planner, $matrixPath, '--host', $hostOs, '--arch', $hostArch)
foreach ($itemVersion in $Version) { $planArguments += @('--version', $itemVersion) }
foreach ($itemVariant in $Variant) { $planArguments += @('--variant', $itemVariant) }
$planText = (& $python @planArguments) -join [Environment]::NewLine
if ($LASTEXITCODE -ne 0) { throw 'canonical OpenUSD runtime planning failed' }
$plannedLeaves = @(($planText | ConvertFrom-Json).jobs)
if ($PlanOnly) {
    $planText
    return
}

$ost = (Get-Command ost -ErrorAction Stop).Source
$git = (Get-Command git -ErrorAction Stop).Source
$producerStatus = @(& $git -C $repositoryRoot status --porcelain --untracked-files=all)
if ($LASTEXITCODE -ne 0) { throw 'could not inspect the OpenStrata producer checkout' }
if ($producerStatus.Count -gt 0) {
    throw "OpenStrata producer checkout is dirty; commit or remove these changes before publishing:`n$($producerStatus -join [Environment]::NewLine)"
}
$openStrataRevision = (& $git -C $repositoryRoot rev-parse HEAD) -join ''
if ($LASTEXITCODE -ne 0) { throw 'could not resolve the OpenStrata revision' }
$openStrataRevision = $openStrataRevision.Trim()
[IO.Directory]::CreateDirectory([IO.Path]::GetFullPath($WorkRoot)) | Out-Null

function Invoke-Checked([string] $File, [string[]] $Arguments) {
    & $File @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$File failed with exit code $LASTEXITCODE" }
}

$results = foreach ($job in $plannedLeaves) {
    $slug = $job.tag
    $runRoot = Join-Path (Join-Path $WorkRoot $Profile) $slug
    $source = Join-Path $WorkRoot "OpenUSD-$($job.openusd.Replace('.', ''))"
    $ostHome = Join-Path $runRoot 'ost-home'
    $dist = Join-Path $runRoot 'dist'
    [IO.Directory]::CreateDirectory($runRoot) | Out-Null
    if (-not (Test-Path -LiteralPath (Join-Path $source '.git'))) {
        Invoke-Checked $git @('clone', '--branch', "v$($job.openusd)", '--depth', '1', 'https://github.com/PixarAnimationStudios/OpenUSD.git', $source)
    } else {
        Invoke-Checked $git @('-C', $source, 'fetch', '--depth', '1', 'origin', "refs/tags/v$($job.openusd):refs/tags/v$($job.openusd)")
        Invoke-Checked $git @('-C', $source, 'checkout', '--detach', "v$($job.openusd)")
    }
    # build_usd.py imports from build_scripts/, so running one leaf leaves a
    # __pycache__ behind and the next leaf of the same OpenUSD version would fail
    # the check below. Clearing untracked residue first keeps the check meaning
    # "the tracked tree is exactly the tag" instead of weakening it.
    Invoke-Checked $git @('-C', $source, 'clean', '-xfdq')
    if ((& $git -C $source status --porcelain)) { throw "OpenUSD source checkout is dirty: $source" }
    $env:OST_HOME = $ostHome
    $pull = @('runtime', 'pull', 'cy2026', '--profile', $Profile, '--build', $source, '--openusd-variant', $job.variant, '--jobs', "$Jobs", '--force')
    if ($hostOs -eq 'macos') {
        $pull += @('--sdk', "$($job.sdk)", '--deployment-target', "$($job.deployment_target)")
    }
    Invoke-Checked $ost $pull
    $runtimeName = Get-ChildItem -LiteralPath (Join-Path $ostHome 'runtimes') -Directory | Select-Object -First 1
    if (-not $runtimeName) { throw "runtime output was not created for $slug" }
    if ($Profile -eq 'lookdev') {
        # USD's installer does not carry its UI Python dependencies. Ship them
        # with the runtime so a clean consumer needs only platform CPython.
        $pythonDirectory = @('lib/python', 'lib/site-packages', 'Lib/site-packages', 'lib/python3.13/site-packages') | ForEach-Object { Join-Path $runtimeName.FullName $_ } | Where-Object { Test-Path -LiteralPath (Join-Path $_ 'pxr') -PathType Container } | Select-Object -First 1
        if (-not $pythonDirectory) { throw 'lookdev runtime has no CPython 3.13 package directory' }
        Invoke-Checked $python @('-m', 'pip', 'install', '--target', $pythonDirectory, 'PySide6==6.8.3', 'PyOpenGL==3.1.9')
        if ($hostOs -eq 'macos') {
            foreach ($driver in @('libqsqlmimer.dylib', 'libqsqlodbc.dylib', 'libqsqlpsql.dylib')) {
                $path = Join-Path $pythonDirectory "PySide6/Qt/plugins/sqldrivers/$driver"
                if (Test-Path -LiteralPath $path -PathType Leaf) { Remove-Item -LiteralPath $path }
            }
        }
    }
    Invoke-Checked $ost @('runtime', 'validate', 'cy2026', '--profile', $Profile)
    Invoke-Checked $python @($validator, $runtimeName.FullName, '--version', $job.openusd, '--variant', $job.variant, '--platform', $hostOs, '--arch', $hostArch, '--profile', $Profile)
    $sourceRevision = ((& $git -C $source rev-parse HEAD) -join '').Trim()
    $metadata = [ordered]@{
        source = [ordered]@{ repository = 'https://github.com/PixarAnimationStudios/OpenUSD'; revision = $sourceRevision }
        builder = [ordered]@{
            id = "https://github.com/animu-sphere/open-strata/blob/$openStrataRevision/support/publish-openusd-runtimes.ps1"
            identity = [ordered]@{ matrix = "support/$(Split-Path -Leaf $matrixPath)"; profile = $Profile; leaf = $slug; host = "$hostOs-$hostArch" }
        }
    }
    # `runtime pull --force` above rebuilds this leaf unconditionally, so the
    # export has to be repeatable too; `runtime export` refuses a non-empty
    # --dist, which made every re-run fail on the first already-exported leaf.
    if (Test-Path -LiteralPath $dist) {
        $resolvedDist = [IO.Path]::GetFullPath($dist)
        $resolvedWork = [IO.Path]::GetFullPath($WorkRoot).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
        if (-not $resolvedDist.StartsWith($resolvedWork, [StringComparison]::OrdinalIgnoreCase)) { throw 'dist must remain inside WorkRoot' }
        Remove-Item -LiteralPath $resolvedDist -Recurse -Force
    }
    $metadataPath = Join-Path $runRoot 'build-metadata.json'
    [IO.File]::WriteAllText($metadataPath, (($metadata | ConvertTo-Json -Depth 8) + [Environment]::NewLine), [Text.UTF8Encoding]::new($false))
    $exportText = (& $ost runtime export cy2026 --profile $Profile --dist $dist --build-metadata $metadataPath --slim --jobs $Jobs --json) -join [Environment]::NewLine
    if ($LASTEXITCODE -ne 0) { throw "runtime export failed for $slug" }
    $export = $exportText | ConvertFrom-Json
    $artifactDigest = $export.data.digest
    $ociDigest = $null
    if ($Publish) {
        $pushText = (& $ost artifact push $artifactDigest "$Registry`:$slug" --json) -join [Environment]::NewLine
        if ($LASTEXITCODE -ne 0) { throw "artifact push failed for $slug" }
        $ociDigest = ($pushText | ConvertFrom-Json).data.oci_digest
    }
    if ($VerifyPublished) {
        if (-not $ociDigest) { throw '-VerifyPublished requires -Publish in the same run' }
        $verifyHome = Join-Path $runRoot "verify-home-$([Guid]::NewGuid().ToString('N'))"
        $savedUser = $env:OST_REGISTRY_USER
        $savedPassword = $env:OST_REGISTRY_PASSWORD
        $savedToken = $env:OST_REGISTRY_TOKEN
        try {
            Remove-Item Env:OST_REGISTRY_USER -ErrorAction SilentlyContinue
            Remove-Item Env:OST_REGISTRY_PASSWORD -ErrorAction SilentlyContinue
            Remove-Item Env:OST_REGISTRY_TOKEN -ErrorAction SilentlyContinue
            $env:OST_HOME = $verifyHome
            Invoke-Checked $ost @('artifact', 'pull', "$Registry@$ociDigest", '--expect-artifact', $artifactDigest, '--require-kind', 'runtime')
        }
        finally {
            if ($null -eq $savedUser) { Remove-Item Env:OST_REGISTRY_USER -ErrorAction SilentlyContinue } else { $env:OST_REGISTRY_USER = $savedUser }
            if ($null -eq $savedPassword) { Remove-Item Env:OST_REGISTRY_PASSWORD -ErrorAction SilentlyContinue } else { $env:OST_REGISTRY_PASSWORD = $savedPassword }
            if ($null -eq $savedToken) { Remove-Item Env:OST_REGISTRY_TOKEN -ErrorAction SilentlyContinue } else { $env:OST_REGISTRY_TOKEN = $savedToken }
            $env:OST_HOME = $ostHome
        }
    }
    [ordered]@{ tag = $slug; artifact_digest = $artifactDigest; oci_digest = $ociDigest; source_revision = $sourceRevision }
}

[ordered]@{
    schema = 1
    matrix = "support/$(Split-Path -Leaf $matrixPath)"
    openstrata_revision = $openStrataRevision
    runtimes = @($results)
} | ConvertTo-Json -Depth 8
