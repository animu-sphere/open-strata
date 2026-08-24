#requires -Version 7.0

[CmdletBinding()]
param(
    [ValidateSet('26.05', '26.08')][string[]] $Version = @('26.05', '26.08'),
    [ValidateSet('core', 'gl', 'vulkan', 'metal')][string[]] $Variant = @('core', 'gl', 'vulkan', 'metal'),
    [ValidateRange(1, 256)][int] $Jobs = [Environment]::ProcessorCount,
    [string] $WorkRoot = (Join-Path $PSScriptRoot '.openusd-runtime-work'),
    [string] $Registry = 'oci://ghcr.io/animu-sphere/openstrata-runtime-cy2026-usd',
    [switch] $Publish,
    [switch] $VerifyPublished,
    [switch] $PlanOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$matrixPath = Join-Path $PSScriptRoot 'openusd-runtime-matrix.json'
$validator = Join-Path $PSScriptRoot 'validate-openusd-runtime.py'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$matrix = Get-Content -LiteralPath $matrixPath -Raw | ConvertFrom-Json

$hostOs = if ($IsWindows) { 'windows' } elseif ($IsMacOS) { 'macos' } else { 'linux' }
$hostArch = if ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq 'Arm64') { 'arm64' } else { 'x86_64' }
$cell = @($matrix.cells | Where-Object { $_.os -eq $hostOs -and $_.arch -eq $hostArch })
if ($cell.Count -ne 1) {
    throw "the canonical matrix has no unique producer cell for $hostOs-$hostArch"
}
$allowed = @($cell[0].variants)
$selectedVariants = @($Variant | Where-Object { $allowed -contains $_ })
if ($selectedVariants.Count -eq 0) {
    throw "none of the selected variants apply to $hostOs-$hostArch (allowed: $($allowed -join ', '))"
}

$plannedLeaves = foreach ($itemVersion in $Version) {
    foreach ($itemVariant in $selectedVariants) {
        [ordered]@{
            openusd = $itemVersion
            profile = 'usd'
            variant = $itemVariant
            os = $hostOs
            arch = $hostArch
            examples_required = $itemVariant -ne 'core'
            tag = "$itemVersion-$itemVariant-$hostOs-$hostArch"
        }
    }
}
if ($PlanOnly) {
    [ordered]@{ schema = 1; jobs = @($plannedLeaves) } | ConvertTo-Json -Depth 8
    return
}

$ost = (Get-Command ost -ErrorAction Stop).Source
$git = (Get-Command git -ErrorAction Stop).Source
$python = (Get-Command python -ErrorAction Stop).Source
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
    $runRoot = Join-Path $WorkRoot $slug
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
    if ((& $git -C $source status --porcelain)) { throw "OpenUSD source checkout is dirty: $source" }
    $env:OST_HOME = $ostHome
    $pull = @('runtime', 'pull', 'cy2026', '--profile', 'usd', '--build', $source, '--openusd-variant', $job.variant, '--jobs', "$Jobs", '--force')
    if ($hostOs -eq 'macos') {
        $pull += @('--sdk', "$($cell[0].sdk)", '--deployment-target', "$($cell[0].deployment_target)")
    }
    Invoke-Checked $ost $pull
    Invoke-Checked $ost @('runtime', 'validate', 'cy2026', '--profile', 'usd')
    $runtimeName = Get-ChildItem -LiteralPath (Join-Path $ostHome 'runtimes') -Directory | Select-Object -First 1
    if (-not $runtimeName) { throw "runtime output was not created for $slug" }
    Invoke-Checked $python @($validator, $runtimeName.FullName, '--version', $job.openusd, '--variant', $job.variant, '--platform', $hostOs, '--arch', $hostArch)
    $sourceRevision = ((& $git -C $source rev-parse HEAD) -join '').Trim()
    $metadata = [ordered]@{
        source = [ordered]@{ repository = 'https://github.com/PixarAnimationStudios/OpenUSD'; revision = $sourceRevision }
        builder = [ordered]@{
            id = "https://github.com/animu-sphere/open-strata/blob/$openStrataRevision/support/publish-openusd-runtimes.ps1"
            identity = [ordered]@{ matrix = 'support/openusd-runtime-matrix.json'; leaf = $slug; host = "$hostOs-$hostArch" }
        }
    }
    $metadataPath = Join-Path $runRoot 'build-metadata.json'
    [IO.File]::WriteAllText($metadataPath, (($metadata | ConvertTo-Json -Depth 8) + [Environment]::NewLine), [Text.UTF8Encoding]::new($false))
    $exportText = (& $ost runtime export cy2026 --profile usd --dist $dist --build-metadata $metadataPath --slim --jobs $Jobs --json) -join [Environment]::NewLine
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
    matrix = 'support/openusd-runtime-matrix.json'
    openstrata_revision = $openStrataRevision
    runtimes = @($results)
} | ConvertTo-Json -Depth 8
