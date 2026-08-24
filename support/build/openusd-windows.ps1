#requires -Version 7.0
param(
    [Parameter(Mandatory)][ValidateSet('26.05', '26.08')][string] $Version,
    [Parameter(Mandatory)][ValidateSet('core', 'gl', 'vulkan')][string] $Variant,
    [Parameter(ValueFromRemainingArguments)][string[]] $Remaining
)

$publisher = Join-Path (Split-Path -Parent $PSScriptRoot) 'publish-openusd-runtimes.ps1'
& $publisher -Version $Version -Variant $Variant @Remaining
exit $LASTEXITCODE
