# SPDX-License-Identifier: Apache-2.0
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot '../runtime-publication.psm1') -Force
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ("ost-publication-" + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($testRoot) | Out-Null
$global:PublicationTestMode = 'existing'
$fakeOst = {
    $action = $args[1]
    $global:LASTEXITCODE = 0
    if ($args[-1] -eq '--help') {
        if ($global:PublicationTestMode -eq 'old-cli') { $global:LASTEXITCODE = 2 }
        return
    }
    if ($action -eq 'resolve') {
        if ($global:PublicationTestMode -eq 'missing' -or $global:PublicationTestMode -eq 'denied') {
            $global:LASTEXITCODE = 4
            $code = if ($global:PublicationTestMode -eq 'missing') { 'ARTIFACT_REMOTE_NOT_FOUND' } else { 'ARTIFACT_AUTH_DENIED' }
            return (@{ error = @{ code = $code } } | ConvertTo-Json -Compress)
        }
        return (@{ data = @{ resolved = @{ locator = 'oci://fixture/owner/runtime@sha256:old'; oci_digest = 'sha256:old' } } } | ConvertTo-Json -Depth 5 -Compress)
    }
    if ($global:PublicationTestMode -eq 'retain-failed') {
        $global:LASTEXITCODE = 4
        return '{"error":{"code":"ARTIFACT_AUTH_DENIED"}}'
    }
    return (@{ data = @{ oci_digest = 'sha256:old'; retention_tag = 'oci://fixture/owner/runtime:retain-sha256-old' } } | ConvertTo-Json -Depth 5 -Compress)
}
function Assert([bool] $Condition, [string] $Message) { if (-not $Condition) { throw $Message } }
try {
    Assert-RuntimePublicationCli -Ost $fakeOst
    $global:PublicationTestMode = 'old-cli'
    $failed = $false
    try { Assert-RuntimePublicationCli -Ost $fakeOst } catch { $failed = $true }
    Assert $failed 'an old CLI must be rejected before starting a runtime publication'
    $global:PublicationTestMode = 'existing'
    $journal = Join-Path $testRoot 'migrations.json'
    $publication = Protect-RuntimePublication -Ost $fakeOst -Reference 'oci://fixture/owner/runtime:new' -Journal $journal -Reason 'variant tags' -PreviousReferences @('oci://fixture/owner/runtime:old')
    $pending = Get-Content -LiteralPath $journal -Raw | ConvertFrom-Json
    Assert ($pending.migrations.Count -eq 2) 'both the current and old-family tag must be retained'
    Assert ($pending.migrations[0].state -eq 'retained-awaiting-publication') 'old pin must be journaled before push'
    Assert (([DateTime]$pending.migrations[0].retain_until - [DateTime]$pending.migrations[0].retained_at).TotalDays -ge 180) 'retention window must be at least 180 days'
    Complete-RuntimePublication -Publication $publication -OciDigest 'sha256:new' -ArtifactDigest 'sha256:archive'
    $completed = Get-Content -LiteralPath $journal -Raw | ConvertFrom-Json
    Assert ($completed.migrations[1].replacement_reference -eq 'oci://fixture/owner/runtime@sha256:new') 'replacement pin must be journaled'
    Assert ($completed.migrations[1].reason -eq 'variant tags') 'migration reason must be retained'
    $global:PublicationTestMode = 'missing'
    $first = Protect-RuntimePublication -Ost $fakeOst -Reference 'oci://fixture/owner/runtime:first' -Journal (Join-Path $testRoot 'first.json')
    Assert ($first.current.Count -eq 0) 'initial publication can have no previous leaf'
    $failed = $false
    try { Protect-RuntimePublication -Ost $fakeOst -Reference 'oci://fixture/owner/runtime:new' -PreviousReferences @('oci://fixture/owner/runtime:old') -Journal (Join-Path $testRoot 'lost.json') | Out-Null } catch { $failed = $true }
    Assert $failed 'missing explicit old references must stop publication'
    foreach ($mode in @('denied', 'retain-failed')) {
        $global:PublicationTestMode = $mode
        $failed = $false
        try { Protect-RuntimePublication -Ost $fakeOst -Reference 'oci://fixture/owner/runtime:new' -Journal (Join-Path $testRoot "$mode.json") | Out-Null } catch { $failed = $true }
        Assert $failed 'auth and retention failures must stop publication'
        Assert (-not (Test-Path -LiteralPath (Join-Path $testRoot "$mode.json"))) 'failed retention must not be recorded as successful'
    }
    Write-Output 'Runtime publication retention/journal tests passed.'
} finally {
    # Exclusively created root inside the system temp directory.
    Remove-Item -LiteralPath $testRoot -Recurse -Force
    Remove-Variable PublicationTestMode -Scope Global
}
# Expected fake CLI failures must not leak into the CI shell's exit status.
$global:LASTEXITCODE = 0
