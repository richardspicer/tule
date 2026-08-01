[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-ExternalCommandPath {
    param(
        [Parameter(Mandatory)]
        [string[]]$Names,

        [string]$Fallback = ''
    )

    foreach ($name in $Names) {
        $command = Get-Command -Name $name -ErrorAction SilentlyContinue |
            Where-Object { $_.CommandType -in @('Application', 'ExternalScript') } |
            Select-Object -First 1
        if ($null -ne $command) {
            return $command.Source
        }
    }

    if (-not [string]::IsNullOrWhiteSpace($Fallback) -and
        (Test-Path -LiteralPath $Fallback)) {
        return $Fallback
    }

    return $null
}

function Invoke-CheckedCommand {
    param(
        [Parameter(Mandatory)]
        [string]$Label,

        [Parameter(Mandatory)]
        [string]$Command,

        [string[]]$Arguments = @()
    )

    Write-Output ''
    Write-Output ("==> {0}" -f $Label)
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw ("{0} failed with exit code {1}." -f $Label, $LASTEXITCODE)
    }
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$userProfilePath = if ([string]::IsNullOrWhiteSpace($env:USERPROFILE)) {
    [Environment]::GetFolderPath('UserProfile')
}
else {
    $env:USERPROFILE
}
$cargoFallback = Join-Path $userProfilePath '.cargo\bin\cargo.exe'
$cargo = Get-ExternalCommandPath -Names @('cargo.exe', 'cargo') -Fallback $cargoFallback
$corepack = Get-ExternalCommandPath -Names @('corepack.cmd', 'corepack')

if ($null -eq $cargo) {
    throw 'Cargo was not found. Run scripts\doctor.ps1 for the exact remediation.'
}
if ($null -eq $corepack) {
    throw 'Corepack was not found. Run scripts\doctor.ps1 for the exact remediation.'
}

$previousLocation = Get-Location
try {
    Set-Location -LiteralPath $repoRoot

    Invoke-CheckedCommand -Label 'Frontend formatting' -Command $corepack `
        -Arguments @('pnpm', '--filter', '@tule/desktop', 'format:check')
    Invoke-CheckedCommand -Label 'Rust formatting' -Command $cargo `
        -Arguments @('fmt', '--all', '--', '--check')
    Invoke-CheckedCommand -Label 'Frontend lint' -Command $corepack `
        -Arguments @('pnpm', '--filter', '@tule/desktop', 'lint')
    Invoke-CheckedCommand -Label 'Frontend type check' -Command $corepack `
        -Arguments @('pnpm', '--filter', '@tule/desktop', 'typecheck')
    Invoke-CheckedCommand -Label 'Frontend tests' -Command $corepack `
        -Arguments @('pnpm', '--filter', '@tule/desktop', 'test')
    Invoke-CheckedCommand -Label 'Rust Clippy' -Command $cargo `
        -Arguments @('clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings')
    Invoke-CheckedCommand -Label 'Rust tests' -Command $cargo `
        -Arguments @('test', '--workspace', '--locked')

    Write-Output ''
    Write-Output 'Tule checks passed.'
}
finally {
    Set-Location -LiteralPath $previousLocation.Path
}
