# SPDX-License-Identifier: Apache-2.0
Set-StrictMode -Version Latest

function Assert-RuntimePublicationCli {
    param([Parameter(Mandatory)][object] $Ost)
    & $Ost artifact retain --help | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Runtime publication requires ost v0.23.14 or later with artifact retain support' }
}

function Write-RuntimeMigrationJournal {
    param([string] $Path, [object[]] $Migrations)
    $fullPath = [IO.Path]::GetFullPath($Path)
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($fullPath)) | Out-Null
    $temporary = "$fullPath.$([Guid]::NewGuid().ToString('N')).tmp"
    $document = [ordered]@{ schema = 1; migrations = @($Migrations) }
    [IO.File]::WriteAllText($temporary, (($document | ConvertTo-Json -Depth 12) + [Environment]::NewLine), [Text.UTF8Encoding]::new($false))
    [IO.File]::Move($temporary, $fullPath, $true)
}

function Protect-RuntimePublication {
    param(
        [Parameter(Mandatory)][object] $Ost,
        [Parameter(Mandatory)][string] $Reference,
        [Parameter(Mandatory)][string] $Journal,
        [string] $Reason = 'Runtime leaf republished',
        [string[]] $PreviousReferences = @()
    )
    $history = @()
    if (Test-Path -LiteralPath $Journal -PathType Leaf) {
        $history = @((Get-Content -LiteralPath $Journal -Raw | ConvertFrom-Json).migrations)
    }
    $current = @()
    foreach ($previous in @(@($Reference) + $PreviousReferences | Select-Object -Unique)) {
        $text = (& $Ost artifact resolve $previous --json) -join [Environment]::NewLine
        $exit = $LASTEXITCODE
        $resolved = $text | ConvertFrom-Json
        if ($exit -ne 0) {
            # Only an absent new leaf is allowed. Auth/network failures and
            # missing explicitly supplied old references must stop publication.
            if ($previous -eq $Reference -and $resolved.error.code -eq 'ARTIFACT_REMOTE_NOT_FOUND') { continue }
            throw "Cannot resolve previous runtime reference '$previous': $text"
        }
        $pinned = $resolved.data.resolved.locator
        $retainedText = (& $Ost artifact retain $pinned --json) -join [Environment]::NewLine
        if ($LASTEXITCODE -ne 0) { throw "Cannot retain '$pinned': $retainedText" }
        $retained = $retainedText | ConvertFrom-Json
        if ($retained.data.oci_digest -ne $resolved.data.resolved.oci_digest) { throw 'Retention digest mismatch' }
        $now = [DateTime]::UtcNow
        $current += [pscustomobject][ordered]@{
            previous_tag = $previous
            previous_reference = $pinned
            retention_tag = $retained.data.retention_tag
            retained_at = $now.ToString('o')
            retain_until = $now.AddDays(180).ToString('o')
            replacement_tag = $Reference
            replacement_reference = $null
            replacement_artifact_digest = $null
            reason = $Reason
            state = 'retained-awaiting-publication'
        }
    }
    Write-RuntimeMigrationJournal -Path $Journal -Migrations @($history + $current)
    [pscustomobject]@{ journal = $Journal; history = @($history); current = @($current) }
}

function Complete-RuntimePublication {
    param(
        [Parameter(Mandatory)][object] $Publication,
        [Parameter(Mandatory)][string] $OciDigest,
        [Parameter(Mandatory)][string] $ArtifactDigest
    )
    foreach ($entry in $Publication.current) {
        $repository = $entry.replacement_tag.Substring(0, $entry.replacement_tag.LastIndexOf(':'))
        $entry.replacement_reference = "$repository@$OciDigest"
        $entry.replacement_artifact_digest = $ArtifactDigest
        $entry.state = 'published'
    }
    Write-RuntimeMigrationJournal -Path $Publication.journal -Migrations @($Publication.history + $Publication.current)
}
Export-ModuleMember -Function Assert-RuntimePublicationCli, Protect-RuntimePublication, Complete-RuntimePublication
