#requires -Version 7.0

[CmdletBinding()]
param(
    [ValidateSet('26.05', '26.08')]
    [string[]] $Version = @('26.05', '26.08'),

    [ValidateSet('windows', 'linux')]
    [string[]] $Platform = @('windows', 'linux'),

    [ValidateRange(1, 256)]
    [int] $Jobs = [Environment]::ProcessorCount,

    # OpenUSD 26.08's Exec examples create deeply nested MSVC tlog paths.
    # Keep this default short enough for tools that still observe MAX_PATH.
    [string] $WorkRoot = 'C:\usd\ovp',

    [string] $OutputRoot = 'C:\usd\openusd-vulkan-publish\output',

    [string] $Registry = 'oci://ghcr.io/animu-sphere/openstrata-runtime-cy2026-usd',

    [string] $VulkanSdk = $env:VULKAN_SDK,

    [string] $VulkanVmaInclude,

    [Alias('Python')]
    [string] $PythonExecutable = 'python',

    [ValidatePattern('^3\.13\.\d+$')]
    [string] $ExpectedPythonVersion = '3.13.14',

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
$script:ResolvedVmaInclude = $null
$script:BuildPython = $null
$script:PythonVersion = $null

function Write-Utf8NoBomFile {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][AllowEmptyString()][string] $Content,
        [switch] $Atomic
    )

    $encoding = [Text.UTF8Encoding]::new($false)
    $fullPath = [IO.Path]::GetFullPath($Path)
    if (-not $Atomic) {
        [IO.File]::WriteAllText($fullPath, $Content, $encoding)
        return
    }

    $parent = [IO.Path]::GetDirectoryName($fullPath)
    $leaf = [IO.Path]::GetFileName($fullPath)
    $temporary = Join-Path $parent ".$leaf.tmp-$PID-$([Guid]::NewGuid().ToString('N'))"
    try {
        [IO.File]::WriteAllText($temporary, $Content, $encoding)
        [IO.File]::Move($temporary, $fullPath, $true)
    }
    finally {
        if (Test-Path -LiteralPath $temporary) {
            Remove-Item -LiteralPath $temporary -Force
        }
    }
}

function Write-JsonFile {
    param(
        [Parameter(Mandatory)][object] $Value,
        [Parameter(Mandatory)][string] $Path,
        [ValidateRange(2, 100)][int] $Depth = 20,
        [switch] $Atomic
    )

    $json = $Value | ConvertTo-Json -Depth $Depth
    Write-Utf8NoBomFile -Path $Path -Content ($json + [Environment]::NewLine) -Atomic:$Atomic
}

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
    # The v0.22 implementation is exercised from a 0.21-versioned release
    # branch before the workspace version bump, then by the final 0.22 binary.
    if ($LASTEXITCODE -ne 0 -or $ostVersion -notmatch '^ost 0\.(21|22)\.') {
        throw "ost 0.21.x or 0.22.x is required; found '$ostVersion'"
    }

    $pythonCommand = if (Test-Path -LiteralPath $PythonExecutable) {
        (Resolve-Path -LiteralPath $PythonExecutable).Path
    }
    else {
        (Get-Command $PythonExecutable -ErrorAction Stop).Source
    }
    $pythonVersion = (& $pythonCommand -c 'import sys; print(".".join(map(str, sys.version_info[:3])))') -join ''
    if ($LASTEXITCODE -ne 0 -or $pythonVersion -ne $ExpectedPythonVersion) {
        throw "Python $ExpectedPythonVersion is required; found '$pythonVersion'"
    }
    $script:BuildPython = $pythonCommand
    $script:PythonVersion = $pythonVersion
    $env:PATH = "$(Split-Path -Parent $pythonCommand);$env:PATH"
}

function Assert-CleanRepository {
    $status = @(& $script:Git -C $script:RepositoryRoot status --porcelain --untracked-files=all)
    if ($LASTEXITCODE -ne 0) {
        throw 'could not inspect the OpenStrata producer checkout'
    }
    if ($status.Count -gt 0) {
        throw @"
The OpenStrata producer checkout must be clean before a build.
Uncommitted content would make builder provenance disagree with the code that ran:
$($status -join [Environment]::NewLine)
"@
    }
}

function Assert-WindowsPrerequisites {
    Initialize-VsDevEnvironment
    if (-not $VulkanSdk -or -not (Test-Path -LiteralPath $VulkanSdk)) {
        throw 'VULKAN_SDK must point to an installed Vulkan SDK'
    }
    $env:VULKAN_SDK = (Resolve-Path -LiteralPath $VulkanSdk).Path

    $sdkInclude = Join-Path $env:VULKAN_SDK 'Include'
    $vmaHeader = Join-Path $sdkInclude 'vma\vk_mem_alloc.h'
    if (Test-Path -LiteralPath $vmaHeader) {
        $script:ResolvedVmaInclude = $sdkInclude
    }
    else {
        if (-not $VulkanVmaInclude) {
            throw @"
The selected Vulkan SDK has no Include\vma\vk_mem_alloc.h.
Pass -VulkanVmaInclude with an include root containing vma\vk_mem_alloc.h.
"@
        }
        $resolvedInclude = (Resolve-Path -LiteralPath $VulkanVmaInclude).Path
        if (-not (Test-Path -LiteralPath (Join-Path $resolvedInclude 'vma\vk_mem_alloc.h'))) {
            throw "VMA header is missing under: $resolvedInclude"
        }
        $script:ResolvedVmaInclude = $resolvedInclude
        # CMake's generated Visual Studio projects do not reliably inherit a
        # late INCLUDE edit. CL is MSVC's supported process-wide extra-options
        # channel and reaches every cl.exe launched by MSBuild.
        $existingCl = $env:CL
        $env:CL = ("/I`"$resolvedInclude`" $existingCl").Trim()
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
                vulkan_sdk = if ($TargetPlatform -eq 'windows-x86_64') {
                    Split-Path -Leaf $env:VULKAN_SDK
                }
                else {
                    'distribution-libvulkan-dev'
                }
                vma_include = if ($TargetPlatform -eq 'windows-x86_64') {
                    $script:ResolvedVmaInclude
                }
                else {
                    '/usr/include'
                }
                python = $script:PythonVersion
            }
        }
    }
    Write-JsonFile -Value $metadata -Path $Path -Depth 6
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
    # build_usd.py decodes CMake/MSVC output through Python's locale. On a
    # Japanese Windows host, UTF-8 diagnostics otherwise fail under CP932 after
    # a successful compile.
    $env:PYTHONUTF8 = '1'
    $env:PYTHONIOENCODING = 'utf-8'
    $buildArguments = @(
        'runtime', 'pull', 'cy2026',
        '--profile', 'usd',
        '--build', $source,
        '--jobs', "$Jobs",
        '--force',
        '--build-arg', '--vulkan',
        '--build-arg', '--examples'
    )
    if ($OpenUsdVersion -eq '26.08') {
        $buildArguments += @('--build-arg', '--python-install-dir=lib/python')
    }
    Invoke-Checked $script:Ost $buildArguments
    Invoke-Checked $script:Ost @('runtime', 'validate', 'cy2026', '--profile', 'usd')

    $runtimeRoot = Join-Path $ostHome 'runtimes\openstrata-cy2026-windows-x86_64-py313-usd'
    $validationPath = Join-Path $output 'feature-validation.json'
    $validation = & $script:BuildPython $script:Validator $runtimeRoot `
        --version $OpenUsdVersion --platform windows
    if ($LASTEXITCODE -ne 0) {
        throw "OpenUSD feature validation failed for $slug"
    }
    Write-Utf8NoBomFile -Path $validationPath -Content (
        ($validation -join [Environment]::NewLine) + [Environment]::NewLine
    )

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
        '--slim',
        '--jobs', "$Jobs",
        '--json'
    )
    Write-JsonFile -Value $export -Path (Join-Path $output 'export.json')

    return [pscustomobject][ordered]@{
        version = $OpenUsdVersion
        platform = 'windows'
        tag = "$Registry`:$slug"
        source_revision = $sourceRevision.Trim()
        runtime_root = $runtimeRoot
        dist = $dist
        store_home = $ostHome
        python_version = $script:PythonVersion
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
        '--build-arg', "PYTHON_VERSION=$ExpectedPythonVersion",
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
    # The suffix separates CMake caches created with Ubuntu's older headers
    # from the pinned Vulkan 1.4.350/VMA 3.4.0 builder generation.
    $volume = "openstrata-openusd-vulkan-$($OpenUsdVersion.Replace('.', ''))-vk14350"
    Invoke-Checked wsl.exe @('docker', 'volume', 'create', $volume)
    Invoke-Checked wsl.exe @(
        'docker', 'run', '--rm',
        '--volume', "${volume}:/work",
        '--volume', "${repoWsl}:/src/open-strata:ro",
        '--volume', "${runRootWsl}:/out",
        $Image,
        'bash', '/src/open-strata/support/build-openusd-vulkan-linux.sh',
        $OpenUsdVersion, "$Jobs", $slug, $OpenStrataRevision, $ExpectedPythonVersion
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
        python_version = $metadata.builder.identity.python
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
        [object[]] $Results,

        [Parameter(Mandatory)]
        [object] $ResultsDocument,

        [Parameter(Mandatory)]
        [string] $ResultsPath
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
        $result.oci_digest = $push.data.oci_digest
        $ResultsDocument['published_at'] = [DateTime]::UtcNow.ToString('o')
        # A later tag may fail after this one has already moved. Journal each
        # successful push so the durable record never lags the public registry.
        Write-JsonFile -Value $ResultsDocument -Path $ResultsPath -Atomic

        $resolved = Invoke-OstJson @('artifact', 'resolve', $result.tag, '--json')
        if ($push.data.oci_digest -ne $resolved.data.resolved.oci_digest) {
            throw "tag resolution mismatch after publishing $($result.tag)"
        }
    }
}

function Verify-PublishedResults {
    param(
        [Parameter(Mandatory)]
        [object[]] $Results
    )

    $savedUser = $env:OST_REGISTRY_USER
    $savedPassword = $env:OST_REGISTRY_PASSWORD
    $savedToken = $env:OST_REGISTRY_TOKEN
    try {
        Remove-Item Env:OST_REGISTRY_USER -ErrorAction SilentlyContinue
        Remove-Item Env:OST_REGISTRY_PASSWORD -ErrorAction SilentlyContinue
        Remove-Item Env:OST_REGISTRY_TOKEN -ErrorAction SilentlyContinue
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
        if ($null -ne $savedToken) {
            $env:OST_REGISTRY_TOKEN = $savedToken
        }
    }
}

Assert-CommonPrerequisites
if ($PublishFrom) {
    $sourceDocument = Get-Content -LiteralPath $PublishFrom -Raw | ConvertFrom-Json
    $results = @($sourceDocument.runtimes)
    $resultsDocument = [ordered]@{
        openstrata_revision = $sourceDocument.openstrata_revision
        created_at = $sourceDocument.created_at
        published_at = $null
        options = $sourceDocument.options
        runtimes = $results
    }
    Publish-Results $results $resultsDocument $PublishFrom
    if ($VerifyPublished) {
        Verify-PublishedResults $results
    }
    Write-JsonFile -Value $resultsDocument -Path $PublishFrom -Atomic
    Write-Host "Updated publish results: $PublishFrom"
    return
}

Assert-CleanRepository
$openStrataRevision = (& $script:Git -C $script:RepositoryRoot rev-parse HEAD) -join ''
if ($LASTEXITCODE -ne 0) {
    throw 'could not resolve the OpenStrata revision'
}
$openStrataRevision = $openStrataRevision.Trim()

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

$resultsPath = Join-Path $runRoot 'results.json'
$builderOptions = [ordered]@{}
if ($Platform -contains 'windows') {
    $builderOptions['windows'] = [ordered]@{
        python = $script:PythonVersion
        vulkan_sdk = $VulkanSdk
        vulkan_vma_include = $script:ResolvedVmaInclude
    }
}
if ($Platform -contains 'linux') {
    $linuxResult = $results | Where-Object platform -eq 'linux' | Select-Object -First 1
    $builderOptions['linux'] = [ordered]@{
        environment = 'wsl2-docker'
        image = $linuxImage
        python = $linuxResult.python_version
        vulkan_sdk = 'headers+utility-1.4.350+vma-3.4.0+ubuntu-24.04-loader+shaderc'
    }
}
$resultsDocument = [ordered]@{
    openstrata_revision = $openStrataRevision
    created_at = [DateTime]::UtcNow.ToString('o')
    published_at = $null
    options = [ordered]@{
        versions = $Version
        platforms = $Platform
        jobs = $Jobs
        registry = $Registry
        builders = $builderOptions
        vulkan = $true
        examples = $true
        layout_profile = 'sdk'
        slim_export = $true
    }
    runtimes = $results
}
Write-JsonFile -Value $resultsDocument -Path $resultsPath -Atomic

if ($Publish) {
    Publish-Results $results $resultsDocument $resultsPath
}
if ($VerifyPublished) {
    Verify-PublishedResults $results
}
Write-JsonFile -Value $resultsDocument -Path $resultsPath -Atomic

Write-Host "Build results: $resultsPath"
foreach ($result in $results) {
    Write-Host "$($result.version) $($result.platform): artifact=$($result.artifact_digest) oci=$($result.oci_digest)"
}
