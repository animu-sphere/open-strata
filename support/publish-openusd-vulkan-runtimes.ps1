[CmdletBinding()]
param(
    [ValidateSet('26.05', '26.08')]
    [string[]] $Version = @('26.05', '26.08'),

    [ValidateSet('windows', 'linux')]
    [string[]] $Platform = @('windows', 'linux'),

    [ValidateRange(1, 256)]
    [int] $Jobs = [Environment]::ProcessorCount,

    [string] $WorkRoot = 'C:\usd\openusd-vulkan-publish',

    [string] $OutputRoot = 'C:\usd\openusd-vulkan-publish\output',

    [string] $Registry = 'oci://ghcr.io/animu-sphere/openstrata-runtime-cy2026-usd',

    [switch] $Publish,

    [string] $PublishFrom,

    [switch] $VerifyPublished
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:RepositoryRoot = Split-Path -Parent $PSScriptRoot
$script:Validator = Join-Path $PSScriptRoot 'validate-openusd-vulkan-runtime.py'
$script:LinuxBuildScript = Join-Path $PSScriptRoot 'build-openusd-vulkan-linux.sh'
$script:LinuxDockerfile = Join-Path $PSScriptRoot 'openusd-vulkan-runtime-linux.Dockerfile'
$script:Ost = (Get-Command ost -ErrorAction Stop).Source
$script:Git = (Get-Command git -ErrorAction Stop).Source

function Invoke-Checked {
    param(
        [Parameter(Mandatory)]
        [string] $FilePath,

        [Parameter(Mandatory)]
        [string[]] $ArgumentList
    )

    & $FilePath @ArgumentList | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "$FilePath failed with exit code $LASTEXITCODE"
    }
}

function Invoke-OstJson {
    param(
        [Parameter(Mandatory)]
        [string[]] $ArgumentList
    )

    $raw = & $script:Ost @ArgumentList
    if ($LASTEXITCODE -ne 0) {
        throw "ost failed with exit code $LASTEXITCODE"
    }
    $text = $raw -join [Environment]::NewLine
    try {
        return $text | ConvertFrom-Json
    }
    catch {
        throw "ost did not return valid JSON: $text"
    }
}

function Initialize-VsDevEnvironment {
    if (Get-Command cl.exe -ErrorAction SilentlyContinue) {
        return
    }

    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) {
        throw 'cl.exe is unavailable and vswhere.exe was not found'
    }

    $installation = & $vswhere -latest -products '*' `
        -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
        -property installationPath
    if ($LASTEXITCODE -ne 0 -or -not $installation) {
        throw 'Visual Studio with the x64 C++ toolchain was not found'
    }

    $vsDevCmd = Join-Path $installation 'Common7\Tools\VsDevCmd.bat'
    $environmentLines = & cmd.exe /s /c "`"$vsDevCmd`" -arch=x64 -host_arch=x64 >nul && set"
    if ($LASTEXITCODE -ne 0) {
        throw 'VsDevCmd.bat failed'
    }
    foreach ($line in $environmentLines) {
        $parts = $line -split '=', 2
        if ($parts.Count -eq 2) {
            Set-Item -Path "Env:$($parts[0])" -Value $parts[1]
        }
    }
    if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
        throw 'VsDevCmd.bat completed but cl.exe is still unavailable'
    }
}

function Assert-CommonPrerequisites {
    $ostVersion = (& $script:Ost --version) -join ''
    if ($LASTEXITCODE -ne 0 -or $ostVersion -notmatch '^ost 0\.21\.') {
        throw "ost 0.21.x is required; found '$ostVersion'"
    }

    $pythonVersion = (& python -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")') -join ''
    if ($LASTEXITCODE -ne 0 -or $pythonVersion -ne '3.13') {
        throw "Python 3.13 is required; found '$pythonVersion'"
    }
}

function Assert-WindowsPrerequisites {
    Initialize-VsDevEnvironment
    if (-not $env:VULKAN_SDK -or -not (Test-Path -LiteralPath $env:VULKAN_SDK)) {
        throw 'VULKAN_SDK must point to an installed Vulkan SDK'
    }
}

function ConvertTo-WslPath {
    param([Parameter(Mandatory)][string] $WindowsPath)

    $path = (& wsl.exe --exec wslpath -a $WindowsPath) -join ''
    if ($LASTEXITCODE -ne 0 -or -not $path) {
        throw "could not convert path for WSL: $WindowsPath"
    }
    return $path.Trim()
}

function Assert-LinuxPrerequisites {
    if (-not (Get-Command wsl.exe -ErrorAction SilentlyContinue)) {
        throw 'WSL2 is required for the Linux runtime build'
    }
    Invoke-Checked wsl.exe @('docker', 'version')
}

function Get-OpenUsdSource {
    param(
        [Parameter(Mandatory)][string] $OpenUsdVersion
    )

    $sourceRoot = Join-Path $WorkRoot 'sources'
    $source = Join-Path $sourceRoot "OpenUSD-$($OpenUsdVersion.Replace('.', ''))"
    New-Item -ItemType Directory -Force -Path $sourceRoot | Out-Null
    $tag = "v$OpenUsdVersion"

    if (-not (Test-Path -LiteralPath (Join-Path $source '.git'))) {
        Invoke-Checked $script:Git @(
            'clone', '--branch', $tag, '--depth', '1',
            'https://github.com/PixarAnimationStudios/OpenUSD.git', $source
        )
    }
    else {
        $dirty = (& $script:Git -C $source status --porcelain) -join ''
        if ($LASTEXITCODE -ne 0 -or $dirty) {
            throw "OpenUSD source checkout is dirty: $source"
        }
        Invoke-Checked $script:Git @(
            '-C', $source, 'fetch', '--depth', '1', 'origin',
            "refs/tags/${tag}:refs/tags/${tag}"
        )
        Invoke-Checked $script:Git @('-C', $source, 'checkout', '--detach', $tag)
    }

    return $source
}

function New-BuildMetadata {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][string] $SourceRevision,
        [Parameter(Mandatory)][string] $OpenUsdVersion,
        [Parameter(Mandatory)][string] $TargetPlatform,
        [Parameter(Mandatory)][string] $OpenStrataRevision
    )

    $metadata = [ordered]@{
        source = [ordered]@{
            repository = 'https://github.com/PixarAnimationStudios/OpenUSD'
            revision = $SourceRevision
        }
        builder = [ordered]@{
            id = "https://github.com/animu-sphere/open-strata/blob/$OpenStrataRevision/support/publish-openusd-vulkan-runtimes.ps1"
            identity = [ordered]@{
                host = $env:COMPUTERNAME
                pipeline = 'openusd-vulkan-runtime'
                git_ref = "v$OpenUsdVersion"
                platform = $TargetPlatform
            }
        }
    }
    $metadata | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Path -Encoding utf8
}

function Build-WindowsRuntime {
    param(
        [Parameter(Mandatory)][string] $OpenUsdVersion,
        [Parameter(Mandatory)][string] $RunRoot,
        [Parameter(Mandatory)][string] $OpenStrataRevision
    )

    $slug = "$OpenUsdVersion-windows-x86_64"
    $output = Join-Path $RunRoot $slug
    $dist = Join-Path $output 'dist'
    $ostHome = Join-Path $WorkRoot "ost-home-$($OpenUsdVersion.Replace('.', ''))-windows"
    $source = Get-OpenUsdSource $OpenUsdVersion
    New-Item -ItemType Directory -Force -Path $output | Out-Null
    if (Test-Path -LiteralPath $dist) {
        throw "refusing to overwrite existing dist directory: $dist"
    }

    $env:OST_HOME = $ostHome
    Invoke-Checked $script:Ost @(
        'runtime', 'pull', 'cy2026',
        '--profile', 'usd',
        '--build', $source,
        '--jobs', "$Jobs",
        '--force',
        '--build-arg', '--vulkan',
        '--build-arg', '--examples',
        '--build-arg', '--python-install-dir=lib/python'
    )
    Invoke-Checked $script:Ost @('runtime', 'validate', 'cy2026', '--profile', 'usd')

    $runtimeRoot = Join-Path $ostHome 'runtimes\openstrata-cy2026-windows-x86_64-py313-usd'
    $validationPath = Join-Path $output 'feature-validation.json'
    $validation = & python $script:Validator $runtimeRoot `
        --version $OpenUsdVersion --platform windows
    if ($LASTEXITCODE -ne 0) {
        throw "OpenUSD feature validation failed for $slug"
    }
    $validation | Set-Content -LiteralPath $validationPath -Encoding utf8

    $sourceRevision = (& $script:Git -C $source rev-parse HEAD) -join ''
    if ($LASTEXITCODE -ne 0) {
        throw "could not resolve the OpenUSD revision for $source"
    }
    $metadataPath = Join-Path $output 'build-metadata.json'
    New-BuildMetadata $metadataPath $sourceRevision $OpenUsdVersion `
        'windows-x86_64' $OpenStrataRevision

    $export = Invoke-OstJson @(
        'runtime', 'export', 'cy2026',
        '--profile', 'usd',
        '--dist', $dist,
        '--build-metadata', $metadataPath,
        '--jobs', "$Jobs",
        '--json'
    )
    $export | ConvertTo-Json -Depth 20 |
        Set-Content -LiteralPath (Join-Path $output 'export.json') -Encoding utf8

    return [pscustomobject][ordered]@{
        version = $OpenUsdVersion
        platform = 'windows'
        tag = "$Registry`:$slug"
        source_revision = $sourceRevision.Trim()
        runtime_root = $runtimeRoot
        dist = $dist
        store_home = $ostHome
        artifact_digest = $export.data.digest
        oci_digest = $null
    }
}

function Build-LinuxImage {
    param([Parameter(Mandatory)][string] $OpenStrataRevision)

    $repo = ConvertTo-WslPath $script:RepositoryRoot
    $dockerfile = ConvertTo-WslPath $script:LinuxDockerfile
    $image = "openstrata-openusd-vulkan-builder:$($OpenStrataRevision.Substring(0, 12))"
    Invoke-Checked wsl.exe @(
        'docker', 'build',
        '--file', $dockerfile,
        '--tag', $image,
        $repo
    )
    return $image
}

function Build-LinuxRuntime {
    param(
        [Parameter(Mandatory)][string] $OpenUsdVersion,
        [Parameter(Mandatory)][string] $RunRoot,
        [Parameter(Mandatory)][string] $OpenStrataRevision,
        [Parameter(Mandatory)][string] $Image
    )

    $slug = "$OpenUsdVersion-linux-x86_64"
    $output = Join-Path $RunRoot $slug
    New-Item -ItemType Directory -Force -Path $output | Out-Null
    $runRootWsl = ConvertTo-WslPath $RunRoot
    $repoWsl = ConvertTo-WslPath $script:RepositoryRoot
    $volume = "openstrata-openusd-vulkan-$($OpenUsdVersion.Replace('.', ''))"
    Invoke-Checked wsl.exe @('docker', 'volume', 'create', $volume)
    Invoke-Checked wsl.exe @(
        'docker', 'run', '--rm',
        '--volume', "${volume}:/work",
        '--volume', "${repoWsl}:/src/open-strata:ro",
        '--volume', "${runRootWsl}:/out",
        $Image,
        'bash', '/src/open-strata/support/build-openusd-vulkan-linux.sh',
        $OpenUsdVersion, "$Jobs", $slug, $OpenStrataRevision
    )

    $exportPath = Join-Path $output 'export.json'
    $export = Get-Content -LiteralPath $exportPath -Raw | ConvertFrom-Json
    $dist = Join-Path $output 'dist'
    $metadata = Get-Content -LiteralPath (Join-Path $output 'build-metadata.json') -Raw |
        ConvertFrom-Json
    $publisherHome = Join-Path $WorkRoot 'publisher-home'
    $env:OST_HOME = $publisherHome
    Invoke-Checked $script:Ost @('artifact', 'import', $dist)

    return [pscustomobject][ordered]@{
        version = $OpenUsdVersion
        platform = 'linux'
        tag = "$Registry`:$slug"
        source_revision = $metadata.source.revision
        runtime_root = $null
        dist = $dist
        store_home = $publisherHome
        artifact_digest = $export.data.digest
        oci_digest = $null
    }
}

function Assert-PublishCredentials {
    if (-not $env:OST_REGISTRY_USER -or -not $env:OST_REGISTRY_PASSWORD) {
        throw 'Publish requires OST_REGISTRY_USER and OST_REGISTRY_PASSWORD with GHCR package write access'
    }
}

function Publish-Results {
    param(
        [Parameter(Mandatory)]
        [object[]] $Results
    )

    Assert-PublishCredentials
    foreach ($result in $Results) {
        $env:OST_HOME = $result.store_home
        if ($result.platform -eq 'linux') {
            Invoke-Checked $script:Ost @('artifact', 'import', $result.dist)
        }
        $push = Invoke-OstJson @(
            'artifact', 'push', $result.artifact_digest, $result.tag, '--json'
        )
        $resolved = Invoke-OstJson @('artifact', 'resolve', $result.tag, '--json')
        if ($push.data.oci_digest -ne $resolved.data.resolved.oci_digest) {
            throw "tag resolution mismatch after publishing $($result.tag)"
        }
        $result.oci_digest = $push.data.oci_digest
    }
}

function Verify-PublishedResults {
    param(
        [Parameter(Mandatory)]
        [object[]] $Results
    )

    $savedUser = $env:OST_REGISTRY_USER
    $savedPassword = $env:OST_REGISTRY_PASSWORD
    try {
        Remove-Item Env:OST_REGISTRY_USER -ErrorAction SilentlyContinue
        Remove-Item Env:OST_REGISTRY_PASSWORD -ErrorAction SilentlyContinue
        foreach ($result in $Results) {
            if (-not $result.oci_digest) {
                throw "no OCI digest is recorded for $($result.tag)"
            }
            $env:OST_HOME = Join-Path $WorkRoot (
                "verify-$($result.version.Replace('.', ''))-$($result.platform)"
            )
            $repository = $result.tag.Substring(0, $result.tag.LastIndexOf(':'))
            $pinned = "$repository@$($result.oci_digest)"
            Invoke-Checked $script:Ost @(
                'artifact', 'pull', $pinned,
                '--expect-artifact', $result.artifact_digest,
                '--require-kind', 'runtime'
            )
        }
    }
    finally {
        if ($null -ne $savedUser) {
            $env:OST_REGISTRY_USER = $savedUser
        }
        if ($null -ne $savedPassword) {
            $env:OST_REGISTRY_PASSWORD = $savedPassword
        }
    }
}

Assert-CommonPrerequisites
$openStrataRevision = (& $script:Git -C $script:RepositoryRoot rev-parse HEAD) -join ''
if ($LASTEXITCODE -ne 0) {
    throw 'could not resolve the OpenStrata revision'
}
$openStrataRevision = $openStrataRevision.Trim()

if ($PublishFrom) {
    $results = @(
        Get-Content -LiteralPath $PublishFrom -Raw |
            ConvertFrom-Json |
            Select-Object -ExpandProperty runtimes
    )
    Publish-Results $results
    if ($VerifyPublished) {
        Verify-PublishedResults $results
    }
    $resultsDocument = [ordered]@{
        openstrata_revision = $openStrataRevision
        published_at = [DateTime]::UtcNow.ToString('o')
        runtimes = $results
    }
    $resultsDocument | ConvertTo-Json -Depth 20 |
        Set-Content -LiteralPath $PublishFrom -Encoding utf8
    Write-Host "Updated publish results: $PublishFrom"
    return
}

$runId = '{0}-{1}' -f [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ'), $openStrataRevision.Substring(0, 12)
$runRoot = Join-Path $OutputRoot $runId
New-Item -ItemType Directory -Force -Path $runRoot | Out-Null

$results = @()
if ($Platform -contains 'windows') {
    Assert-WindowsPrerequisites
    foreach ($item in $Version) {
        $results += Build-WindowsRuntime $item $runRoot $openStrataRevision
    }
}

if ($Platform -contains 'linux') {
    Assert-LinuxPrerequisites
    $linuxImage = Build-LinuxImage $openStrataRevision
    foreach ($item in $Version) {
        $results += Build-LinuxRuntime $item $runRoot $openStrataRevision $linuxImage
    }
}

if ($Publish) {
    Publish-Results $results
}
if ($VerifyPublished) {
    Verify-PublishedResults $results
}

$resultsPath = Join-Path $runRoot 'results.json'
$resultsDocument = [ordered]@{
    openstrata_revision = $openStrataRevision
    created_at = [DateTime]::UtcNow.ToString('o')
    options = [ordered]@{
        versions = $Version
        platforms = $Platform
        jobs = $Jobs
        registry = $Registry
        vulkan = $true
        examples = $true
        full_export = $true
    }
    runtimes = $results
}
$resultsDocument | ConvertTo-Json -Depth 20 |
    Set-Content -LiteralPath $resultsPath -Encoding utf8

Write-Host "Build results: $resultsPath"
foreach ($result in $results) {
    Write-Host "$($result.version) $($result.platform): artifact=$($result.artifact_digest) oci=$($result.oci_digest)"
}
