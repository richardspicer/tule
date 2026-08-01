[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ExpectedRustVersion = '1.97.1'
$ExpectedNodeVersion = '24.18.0'
$ExpectedPnpmVersion = '11.4.0'
$BootstrapNodeRequirement = 'Node.js 22.13+ (22.x) or Node.js 24.x with Corepack'
$WebView2ClientId = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'

$script:Results = [System.Collections.Generic.List[object]]::new()
$script:HasConfigurationError = $false

function Add-Result {
    param(
        [Parameter(Mandatory)]
        [ValidateSet('PASS', 'WARN', 'FAIL')]
        [string]$Status,

        [Parameter(Mandatory)]
        [string]$Name,

        [Parameter(Mandatory)]
        [string]$Found,

        [Parameter(Mandatory)]
        [string]$Required,

        [string]$Remediation = '',

        [switch]$ConfigurationError
    )

    $script:Results.Add([pscustomobject]@{
            Status      = $Status
            Name        = $Name
            Found       = $Found
            Required    = $Required
            Remediation = $Remediation
        })

    if ($ConfigurationError -and $Status -eq 'FAIL') {
        $script:HasConfigurationError = $true
    }
}

function Get-JsonProperty {
    param(
        [Parameter(Mandatory)]
        [object]$InputObject,

        [Parameter(Mandatory)]
        [string]$Name
    )

    $property = $InputObject.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }

    return $property.Value
}

function Get-ExternalCommand {
    param([Parameter(Mandatory)][string]$Name)

    return @(Get-Command -Name $Name -All -ErrorAction SilentlyContinue |
            Where-Object { $_.CommandType -in @('Application', 'ExternalScript') }) |
        Select-Object -First 1
}

function Find-RustCommand {
    param([Parameter(Mandatory)][string]$Name)

    $command = Get-ExternalCommand -Name ("{0}.exe" -f $Name)
    if ($null -eq $command) {
        $command = Get-ExternalCommand -Name $Name
    }
    if ($null -ne $command) {
        return [pscustomobject]@{
            Command             = $command
            FromStandardUserBin = $false
        }
    }

    if (-not [string]::IsNullOrWhiteSpace($env:USERPROFILE)) {
        $candidate = Join-Path $env:USERPROFILE (".cargo\bin\{0}.exe" -f $Name)
        if (Test-Path -LiteralPath $candidate) {
            return [pscustomobject]@{
                Command             = $candidate
                FromStandardUserBin = $true
            }
        }
    }

    return $null
}

function Invoke-ReadOnlyCommand {
    param(
        [Parameter(Mandatory)]
        [object]$Command,

        [string[]]$Arguments = @(),

        [Parameter(Mandatory)]
        [string]$WorkingDirectory,

        [hashtable]$Environment = @{}
    )

    $previousEnvironment = @{}
    $previousLocation = Get-Location
    $previousErrorActionPreference = $ErrorActionPreference

    try {
        foreach ($key in $Environment.Keys) {
            $previousEnvironment[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
            [Environment]::SetEnvironmentVariable($key, [string]$Environment[$key], 'Process')
        }

        Set-Location -LiteralPath $WorkingDirectory
        $global:LASTEXITCODE = 0
        # Windows PowerShell 5.1 wraps native stderr as non-terminating ErrorRecord
        # objects. Capture those records and trust the native exit code instead.
        $ErrorActionPreference = 'Continue'
        $output = @(& $Command @Arguments 2>&1 | ForEach-Object { $_.ToString() })
        $exitCode = if ($null -eq $LASTEXITCODE) { 0 } else { [int]$LASTEXITCODE }

        return [pscustomobject]@{
            ExitCode = $exitCode
            Output   = $output
        }
    }
    catch {
        return [pscustomobject]@{
            ExitCode = 9001
            Output   = @('The command could not be executed.')
        }
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
        Set-Location -LiteralPath $previousLocation.Path
        foreach ($key in $Environment.Keys) {
            if ($null -eq $previousEnvironment[$key]) {
                Remove-Item -LiteralPath "Env:$key" -ErrorAction SilentlyContinue
            }
            else {
                [Environment]::SetEnvironmentVariable($key, [string]$previousEnvironment[$key], 'Process')
            }
        }
    }
}

function Get-WindowsArchitecture {
    $architecture = $null

    try {
        $architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    }
    catch {
        $architecture = if ($env:PROCESSOR_ARCHITEW6432) {
            $env:PROCESSOR_ARCHITEW6432
        }
        else {
            $env:PROCESSOR_ARCHITECTURE
        }
    }

    switch -Regex ($architecture.ToUpperInvariant()) {
        '^(X64|AMD64)$' {
            return [pscustomobject]@{
                RustTarget  = 'x86_64-pc-windows-msvc'
                VcComponent = 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64'
                SdkArch     = 'x64'
                CompilerBin = @('Hostx64\x64', 'Hostarm64\x64')
            }
        }
        '^(ARM64|AARCH64)$' {
            return [pscustomobject]@{
                RustTarget  = 'aarch64-pc-windows-msvc'
                VcComponent = 'Microsoft.VisualStudio.Component.VC.Tools.ARM64'
                SdkArch     = 'arm64'
                CompilerBin = @('Hostarm64\arm64', 'Hostx64\arm64')
            }
        }
        default { return $null }
    }
}

function Find-VsWhere {
    $candidates = [System.Collections.Generic.List[string]]::new()
    $command = Get-ExternalCommand -Name 'vswhere.exe'
    if ($null -ne $command) {
        $candidates.Add($command.Source)
    }

    foreach ($base in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
        if (-not [string]::IsNullOrWhiteSpace($base)) {
            $candidates.Add((Join-Path $base 'Microsoft Visual Studio\Installer\vswhere.exe'))
        }
    }

    return @($candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -Unique) |
        Select-Object -First 1
}

function Get-VsWhereInstances {
    param(
        [Parameter(Mandatory)][string]$VsWhere,
        [string[]]$RequiredComponents = @()
    )

    $arguments = @('-all', '-products', '*')
    if ($RequiredComponents.Count -gt 0) {
        $arguments += '-requires'
        $arguments += $RequiredComponents
    }
    $arguments += @('-format', 'json', '-utf8')

    $result = Invoke-ReadOnlyCommand -Command $VsWhere -Arguments $arguments -WorkingDirectory $script:RepoRoot
    if ($result.ExitCode -ne 0 -or $result.Output.Count -eq 0) {
        return @()
    }

    try {
        return @((($result.Output -join [Environment]::NewLine) | ConvertFrom-Json) |
                Where-Object { (Get-JsonProperty -InputObject $_ -Name 'isComplete') -eq $true })
    }
    catch {
        return @()
    }
}

function Get-KnownVisualStudioRoots {
    $roots = [System.Collections.Generic.List[string]]::new()

    foreach ($base in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
        if ([string]::IsNullOrWhiteSpace($base)) {
            continue
        }

        $visualStudioRoot = Join-Path $base 'Microsoft Visual Studio'
        if (-not (Test-Path -LiteralPath $visualStudioRoot)) {
            continue
        }

        foreach ($release in @(Get-ChildItem -LiteralPath $visualStudioRoot -Directory -ErrorAction SilentlyContinue)) {
            foreach ($product in @(Get-ChildItem -LiteralPath $release.FullName -Directory -ErrorAction SilentlyContinue)) {
                $roots.Add($product.FullName)
            }
        }
    }

    return @($roots | Select-Object -Unique)
}

function Test-VisualStudioToolFiles {
    param(
        [Parameter(Mandatory)][string]$InstallationPath,
        [Parameter(Mandatory)][string[]]$CompilerBin
    )

    $msbuildFound = @(
        (Join-Path $InstallationPath 'MSBuild\Current\Bin\MSBuild.exe'),
        (Join-Path $InstallationPath 'MSBuild\Current\Bin\amd64\MSBuild.exe')
    ) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1

    $compilerFound = $false
    $toolRoot = Join-Path $InstallationPath 'VC\Tools\MSVC'
    if (Test-Path -LiteralPath $toolRoot) {
        foreach ($toolVersion in @(Get-ChildItem -LiteralPath $toolRoot -Directory -ErrorAction SilentlyContinue)) {
            foreach ($relativeBin in $CompilerBin) {
                $bin = Join-Path (Join-Path $toolVersion.FullName 'bin') $relativeBin
                if ((Test-Path -LiteralPath (Join-Path $bin 'cl.exe')) -and
                    (Test-Path -LiteralPath (Join-Path $bin 'link.exe'))) {
                    $compilerFound = $true
                    break
                }
            }

            if ($compilerFound) {
                break
            }
        }
    }

    return ($null -ne $msbuildFound -and $compilerFound)
}

function Find-WindowsSdkVersion {
    param([Parameter(Mandatory)][string]$Architecture)

    $sdkRoots = [System.Collections.Generic.List[string]]::new()
    foreach ($registryPath in @(
            'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows Kits\Installed Roots',
            'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\WOW6432Node\Microsoft\Windows Kits\Installed Roots'
        )) {
        try {
            $kitsRoot = (Get-ItemProperty -LiteralPath $registryPath -Name 'KitsRoot10' -ErrorAction Stop).KitsRoot10
            if (-not [string]::IsNullOrWhiteSpace($kitsRoot)) {
                $sdkRoots.Add($kitsRoot)
            }
        }
        catch {
            # Continue to the remaining read-only discovery locations.
        }
    }

    foreach ($base in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
        if (-not [string]::IsNullOrWhiteSpace($base)) {
            $sdkRoots.Add((Join-Path $base 'Windows Kits\10'))
        }
    }

    foreach ($sdkRoot in @($sdkRoots | Select-Object -Unique)) {
        $libRoot = Join-Path $sdkRoot 'Lib'
        if (-not (Test-Path -LiteralPath $libRoot)) {
            continue
        }

        $versions = @(Get-ChildItem -LiteralPath $libRoot -Directory -ErrorAction SilentlyContinue |
                Sort-Object Name -Descending)
        foreach ($version in $versions) {
            $kernelLibrary = Join-Path $version.FullName ("um\{0}\kernel32.lib" -f $Architecture)
            $windowsHeader = Join-Path $sdkRoot ("Include\{0}\um\Windows.h" -f $version.Name)
            $resourceCompiler = Join-Path $sdkRoot ("bin\{0}\{1}\rc.exe" -f $version.Name, $Architecture)

            if ((Test-Path -LiteralPath $kernelLibrary) -and
                (Test-Path -LiteralPath $windowsHeader) -and
                (Test-Path -LiteralPath $resourceCompiler)) {
                return $version.Name
            }
        }
    }

    return $null
}

function Get-WebView2RegistryVersions {
    $registryPaths = @(
        "Registry::HKEY_LOCAL_MACHINE\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\$WebView2ClientId",
        "Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\EdgeUpdate\Clients\$WebView2ClientId",
        "Registry::HKEY_CURRENT_USER\Software\Microsoft\EdgeUpdate\Clients\$WebView2ClientId"
    )
    $versions = [System.Collections.Generic.List[string]]::new()

    foreach ($registryPath in $registryPaths) {
        try {
            $value = [string](Get-ItemProperty -LiteralPath $registryPath -Name 'pv' -ErrorAction Stop).pv
            $parsed = $null
            if ([Version]::TryParse($value, [ref]$parsed) -and $parsed -gt [Version]'0.0.0.0') {
                $versions.Add($parsed.ToString())
            }
        }
        catch {
            # A missing registry view is expected on some Windows architectures.
        }
    }

    return @($versions | Select-Object -Unique)
}

function Get-WebView2FileVersions {
    $versions = [System.Collections.Generic.List[string]]::new()

    foreach ($base in @(${env:ProgramFiles(x86)}, $env:ProgramFiles, $env:LOCALAPPDATA)) {
        if ([string]::IsNullOrWhiteSpace($base)) {
            continue
        }

        $applicationRoot = Join-Path $base 'Microsoft\EdgeWebView\Application'
        if (-not (Test-Path -LiteralPath $applicationRoot)) {
            continue
        }

        foreach ($versionDirectory in @(Get-ChildItem -LiteralPath $applicationRoot -Directory -ErrorAction SilentlyContinue)) {
            $parsed = $null
            if ([Version]::TryParse($versionDirectory.Name, [ref]$parsed) -and
                (Test-Path -LiteralPath (Join-Path $versionDirectory.FullName 'msedgewebview2.exe'))) {
                $versions.Add($parsed.ToString())
            }
        }
    }

    return @($versions | Select-Object -Unique)
}

function Write-DoctorReport {
    Write-Output ''
    Write-Output 'Tule Windows environment doctor'
    Write-Output '---------------------------------'
    foreach ($result in $script:Results) {
        Write-Output ("[{0}] {1}: {2} (required: {3})" -f
            $result.Status, $result.Name, $result.Found, $result.Required)
    }

    $actionable = @($script:Results | Where-Object {
            $_.Status -ne 'PASS' -and -not [string]::IsNullOrWhiteSpace($_.Remediation)
        })
    if ($actionable.Count -gt 0) {
        Write-Output ''
        Write-Output 'Remediation'
        Write-Output '-----------'
        foreach ($result in $actionable) {
            Write-Output ("- {0}: {1}" -f $result.Name, $result.Remediation)
        }
    }

    $passCount = @($script:Results | Where-Object { $_.Status -eq 'PASS' }).Count
    $warnCount = @($script:Results | Where-Object { $_.Status -eq 'WARN' }).Count
    $failCount = @($script:Results | Where-Object { $_.Status -eq 'FAIL' }).Count
    Write-Output ''
    Write-Output ("Summary: {0} passed, {1} warnings, {2} failed." -f $passCount, $warnCount, $failCount)
}

try {
    $script:RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path

    if ($env:OS -ne 'Windows_NT') {
        Add-Result -Status 'FAIL' -Name 'Operating system' -Found 'non-Windows host' `
            -Required 'Windows' -Remediation 'Run this doctor on the Windows development workstation.' `
            -ConfigurationError
        throw 'Unsupported operating system.'
    }

    $platform = Get-WindowsArchitecture
    if ($null -eq $platform) {
        Add-Result -Status 'FAIL' -Name 'Windows architecture' -Found 'unsupported architecture' `
            -Required 'x64 or ARM64' -Remediation 'Use a supported 64-bit Windows architecture.' `
            -ConfigurationError
        throw 'Unsupported Windows architecture.'
    }
    Add-Result -Status 'PASS' -Name 'Windows architecture' -Found $platform.RustTarget `
        -Required 'an MSVC Windows target'

    $rustToolchainPath = Join-Path $script:RepoRoot 'rust-toolchain.toml'
    $requiredRustComponents = @()
    if (-not (Test-Path -LiteralPath $rustToolchainPath)) {
        Add-Result -Status 'FAIL' -Name 'Rust pin' -Found 'rust-toolchain.toml is missing' `
            -Required $ExpectedRustVersion -Remediation 'Restore the committed rust-toolchain.toml file.' `
            -ConfigurationError
    }
    else {
        $rustToolchainText = Get-Content -LiteralPath $rustToolchainPath -Raw
        if ($rustToolchainText -notmatch '(?m)^\s*channel\s*=\s*"([^"]+)"') {
            Add-Result -Status 'FAIL' -Name 'Rust pin' -Found 'channel is missing or malformed' `
                -Required $ExpectedRustVersion -Remediation 'Set an exact channel in rust-toolchain.toml.' `
                -ConfigurationError
        }
        elseif ($Matches[1] -ne $ExpectedRustVersion) {
            Add-Result -Status 'FAIL' -Name 'Rust pin' -Found $Matches[1] -Required $ExpectedRustVersion `
                -Remediation 'Restore the repository Rust pin.' -ConfigurationError
        }
        else {
            Add-Result -Status 'PASS' -Name 'Rust pin' -Found $Matches[1] -Required $ExpectedRustVersion
        }

        if ($rustToolchainText -match '(?ms)^\s*components\s*=\s*\[(.*?)\]') {
            $componentBlock = $Matches[1]
            $requiredRustComponents = @([regex]::Matches($componentBlock, '"([^"]+)"') |
                    ForEach-Object { $_.Groups[1].Value })
        }

        $missingComponentPins = @('rustfmt', 'clippy' | Where-Object { $_ -notin $requiredRustComponents })
        if ($missingComponentPins.Count -gt 0) {
            Add-Result -Status 'FAIL' -Name 'Rust component pins' `
                -Found ("missing {0}" -f ($missingComponentPins -join ', ')) `
                -Required 'rustfmt and clippy' -Remediation 'Restore the committed Rust component pins.' `
                -ConfigurationError
        }
        else {
            Add-Result -Status 'PASS' -Name 'Rust component pins' `
                -Found ($requiredRustComponents -join ', ') -Required 'rustfmt and clippy'
        }
    }

    $nodePinPath = Join-Path $script:RepoRoot '.node-version'
    if (-not (Test-Path -LiteralPath $nodePinPath)) {
        Add-Result -Status 'FAIL' -Name 'Node pin' -Found '.node-version is missing' `
            -Required $ExpectedNodeVersion -Remediation 'Restore the committed .node-version file.' `
            -ConfigurationError
    }
    else {
        $nodePin = (Get-Content -LiteralPath $nodePinPath -Raw).Trim()
        if ($nodePin -eq $ExpectedNodeVersion) {
            Add-Result -Status 'PASS' -Name 'Node pin' -Found $nodePin -Required $ExpectedNodeVersion
        }
        else {
            Add-Result -Status 'FAIL' -Name 'Node pin' -Found $nodePin -Required $ExpectedNodeVersion `
                -Remediation 'Restore the repository Node pin.' -ConfigurationError
        }
    }

    $packageCandidates = @(
        (Join-Path $script:RepoRoot 'package.json'),
        (Join-Path $script:RepoRoot 'apps\desktop\package.json')
    ) | Where-Object { Test-Path -LiteralPath $_ }
    $packageManagerEntries = [System.Collections.Generic.List[object]]::new()
    foreach ($packagePath in $packageCandidates) {
        try {
            $packageJson = Get-Content -LiteralPath $packagePath -Raw | ConvertFrom-Json
            $packageManager = Get-JsonProperty -InputObject $packageJson -Name 'packageManager'
            if (-not [string]::IsNullOrWhiteSpace([string]$packageManager)) {
                $packageManagerEntries.Add([pscustomobject]@{
                        Path = $packagePath
                        Spec = [string]$packageManager
                        Json = $packageJson
                    })
            }
        }
        catch {
            Add-Result -Status 'FAIL' -Name 'Package manifest' -Found 'invalid JSON' `
                -Required 'valid package.json' -Remediation 'Repair the committed package manifest.' `
                -ConfigurationError
        }
    }

    $rootPackagePath = Join-Path $script:RepoRoot 'package.json'
    $rootPackageEntry = @($packageManagerEntries |
            Where-Object { $_.Path -eq $rootPackagePath } |
            Select-Object -First 1)
    if ($rootPackageEntry.Count -eq 0) {
        Add-Result -Status 'FAIL' -Name 'Node runtime policy' `
            -Found 'root package.json policy is missing' `
            -Required ("engines and devEngines pinned to Node.js {0} and pnpm {1}" -f $ExpectedNodeVersion, $ExpectedPnpmVersion) `
            -Remediation 'Restore the committed root package.json runtime policy.' -ConfigurationError
    }
    else {
        $rootPackageJson = $rootPackageEntry[0].Json
        $engines = Get-JsonProperty -InputObject $rootPackageJson -Name 'engines'
        $devEngines = Get-JsonProperty -InputObject $rootPackageJson -Name 'devEngines'
        $engineNode = if ($null -eq $engines) { $null } else { Get-JsonProperty -InputObject $engines -Name 'node' }
        $enginePnpm = if ($null -eq $engines) { $null } else { Get-JsonProperty -InputObject $engines -Name 'pnpm' }
        $runtimePolicy = if ($null -eq $devEngines) { $null } else { Get-JsonProperty -InputObject $devEngines -Name 'runtime' }
        $runtimeName = if ($null -eq $runtimePolicy) { $null } else { Get-JsonProperty -InputObject $runtimePolicy -Name 'name' }
        $runtimeVersion = if ($null -eq $runtimePolicy) { $null } else { Get-JsonProperty -InputObject $runtimePolicy -Name 'version' }
        $runtimeOnFail = if ($null -eq $runtimePolicy) { $null } else { Get-JsonProperty -InputObject $runtimePolicy -Name 'onFail' }

        if ([string]$engineNode -eq $ExpectedNodeVersion -and
            [string]$enginePnpm -eq $ExpectedPnpmVersion -and
            [string]$runtimeName -eq 'node' -and
            [string]$runtimeVersion -eq $ExpectedNodeVersion -and
            [string]$runtimeOnFail -eq 'download') {
            Add-Result -Status 'PASS' -Name 'Node runtime policy' `
                -Found ("Node.js {0}, pnpm {1}, managed-runtime download" -f $ExpectedNodeVersion, $ExpectedPnpmVersion) `
                -Required 'exact engines and devEngines.runtime pins'
        }
        else {
            Add-Result -Status 'FAIL' -Name 'Node runtime policy' `
                -Found 'engines or devEngines.runtime is missing or mismatched' `
                -Required ("Node.js {0}, pnpm {1}, and onFail=download" -f $ExpectedNodeVersion, $ExpectedPnpmVersion) `
                -Remediation 'Restore the committed root engines and devEngines.runtime policy.' -ConfigurationError
        }
    }

    $packageWorkingDirectory = Join-Path $script:RepoRoot 'apps\desktop'
    $packageManagerConfigured = $false
    if ($packageManagerEntries.Count -eq 0) {
        Add-Result -Status 'FAIL' -Name 'pnpm pin' -Found 'packageManager is missing' `
            -Required ("pnpm@{0}" -f $ExpectedPnpmVersion) `
            -Remediation 'Restore the committed packageManager field.' -ConfigurationError
    }
    else {
        $distinctSpecs = @($packageManagerEntries.Spec | Select-Object -Unique)
        if ($distinctSpecs.Count -gt 1) {
            Add-Result -Status 'FAIL' -Name 'pnpm pin' -Found 'conflicting packageManager values' `
                -Required ("pnpm@{0}" -f $ExpectedPnpmVersion) `
                -Remediation 'Keep one consistent pnpm packageManager pin.' -ConfigurationError
        }
        elseif ($distinctSpecs[0] -notmatch '^pnpm@(\d+\.\d+\.\d+)(\+sha(?:224|256|384|512)\..+)?$' -or
            $Matches[1] -ne $ExpectedPnpmVersion) {
            Add-Result -Status 'FAIL' -Name 'pnpm pin' -Found $distinctSpecs[0] `
                -Required ("pnpm@{0}" -f $ExpectedPnpmVersion) `
                -Remediation 'Restore the repository pnpm pin.' -ConfigurationError
        }
        else {
            $packageManagerConfigured = $true
            $packageWorkingDirectory = Split-Path -Parent $packageManagerEntries[0].Path
            if ([string]::IsNullOrWhiteSpace($Matches[2])) {
                Add-Result -Status 'WARN' -Name 'pnpm pin' -Found $distinctSpecs[0] `
                    -Required ("pnpm@{0} with an integrity hash preferred" -f $ExpectedPnpmVersion) `
                    -Remediation 'Add the Corepack integrity hash when the packageManager field is next refreshed.'
            }
            else {
                Add-Result -Status 'PASS' -Name 'pnpm pin' -Found ("pnpm@{0} with integrity hash" -f $ExpectedPnpmVersion) `
                    -Required ("pnpm@{0}" -f $ExpectedPnpmVersion)
            }
        }
    }

    $git = Get-ExternalCommand -Name 'git.exe'
    if ($null -eq $git) {
        $git = Get-ExternalCommand -Name 'git'
    }
    if ($null -eq $git) {
        Add-Result -Status 'FAIL' -Name 'Git' -Found 'not found on PATH' -Required 'Git for Windows' `
            -Remediation 'Install Git for Windows, restart the terminal, and rerun the doctor.'
    }
    else {
        $gitVersionResult = Invoke-ReadOnlyCommand -Command $git -Arguments @('--version') `
            -WorkingDirectory $script:RepoRoot
        $gitText = $gitVersionResult.Output -join ' '
        if ($gitVersionResult.ExitCode -eq 0 -and $gitText -match 'git version (\d+\.\d+\.\d+)') {
            Add-Result -Status 'PASS' -Name 'Git' -Found $Matches[1] -Required 'an available Git version'
        }
        else {
            Add-Result -Status 'FAIL' -Name 'Git' -Found 'version check failed' -Required 'Git for Windows' `
                -Remediation 'Repair Git for Windows, restart the terminal, and rerun the doctor.'
        }
    }

    $rustupDiscovery = Find-RustCommand -Name 'rustup'
    $rustcDiscovery = Find-RustCommand -Name 'rustc'
    $cargoDiscovery = Find-RustCommand -Name 'cargo'
    $rustup = if ($null -eq $rustupDiscovery) { $null } else { $rustupDiscovery.Command }
    $rustc = if ($null -eq $rustcDiscovery) { $null } else { $rustcDiscovery.Command }
    $cargo = if ($null -eq $cargoDiscovery) { $null } else { $cargoDiscovery.Command }
    $fallbackRustCommands = @()
    if ($null -ne $rustupDiscovery -and $rustupDiscovery.FromStandardUserBin) {
        $fallbackRustCommands += 'rustup'
    }
    if ($null -ne $rustcDiscovery -and $rustcDiscovery.FromStandardUserBin) {
        $fallbackRustCommands += 'rustc'
    }
    if ($null -ne $cargoDiscovery -and $cargoDiscovery.FromStandardUserBin) {
        $fallbackRustCommands += 'cargo'
    }
    if ($fallbackRustCommands.Count -gt 0) {
        Add-Result -Status 'WARN' -Name 'Rust command PATH' `
            -Found ("found {0} in the standard per-user Rust directory" -f ($fallbackRustCommands -join ', ')) `
            -Required 'Rust commands available on PATH in a fresh terminal' `
            -Remediation 'Restart the terminal so the rustup proxy directory is inherited on PATH.'
    }
    $rustToolchainInstalled = $false
    $rustToolchainName = "${ExpectedRustVersion}-$($platform.RustTarget)"
    $rustupEnvironment = @{
        RUSTUP_AUTO_INSTALL = '0'
    }
    if ($null -eq $rustup) {
        Add-Result -Status 'FAIL' -Name 'rustup' -Found 'not found on PATH' -Required 'rustup' `
            -Remediation 'Install rustup with the MSVC host, restart the terminal, and rerun the doctor.'
    }
    else {
        $rustupVersionResult = Invoke-ReadOnlyCommand -Command $rustup -Arguments @('--version') `
            -WorkingDirectory $script:RepoRoot -Environment $rustupEnvironment
        $rustupText = $rustupVersionResult.Output -join ' '
        if ($rustupVersionResult.ExitCode -eq 0 -and $rustupText -match 'rustup\s+(\d+\.\d+\.\d+)') {
            Add-Result -Status 'PASS' -Name 'rustup' -Found $Matches[1] -Required 'rustup'
        }
        else {
            Add-Result -Status 'FAIL' -Name 'rustup' -Found 'version check failed' -Required 'working rustup' `
                -Remediation 'Repair rustup, restart the terminal, and rerun the doctor.'
        }

        $toolchainListResult = Invoke-ReadOnlyCommand -Command $rustup -Arguments @('toolchain', 'list') `
            -WorkingDirectory $script:RepoRoot -Environment $rustupEnvironment
        if ($toolchainListResult.ExitCode -eq 0) {
            $installedToolchains = @($toolchainListResult.Output | ForEach-Object {
                    ($_ -replace '\s+\(.*\)\s*$', '').Trim()
                })
            $rustToolchainInstalled = $rustToolchainName -in $installedToolchains
        }

        if ($rustToolchainInstalled) {
            Add-Result -Status 'PASS' -Name 'Rust toolchain' -Found $rustToolchainName `
                -Required $rustToolchainName
        }
        else {
            Add-Result -Status 'FAIL' -Name 'Rust toolchain' -Found 'not installed' `
                -Required $rustToolchainName `
                -Remediation ("Run 'rustup toolchain install {0} --profile minimal --component rustfmt,clippy', then rerun the doctor." -f $ExpectedRustVersion)
        }
    }

    if ($null -eq $rustc) {
        Add-Result -Status 'FAIL' -Name 'rustc' -Found 'not found on PATH' -Required $ExpectedRustVersion `
            -Remediation 'Ensure the rustup proxy directory is on PATH, restart the terminal, and rerun the doctor.'
    }
    elseif ($rustToolchainInstalled) {
        $rustcVersionResult = Invoke-ReadOnlyCommand -Command $rustc -Arguments @('-vV') `
            -WorkingDirectory $script:RepoRoot -Environment $rustupEnvironment
        $rustcText = $rustcVersionResult.Output -join [Environment]::NewLine
        $releaseMatch = [regex]::Match($rustcText, '(?m)^release:\s*(\S+)\s*$')
        $hostMatch = [regex]::Match($rustcText, '(?m)^host:\s*(\S+)\s*$')
        if ($rustcVersionResult.ExitCode -eq 0 -and
            $releaseMatch.Success -and $releaseMatch.Groups[1].Value -eq $ExpectedRustVersion -and
            $hostMatch.Success -and $hostMatch.Groups[1].Value -eq $platform.RustTarget) {
            Add-Result -Status 'PASS' -Name 'rustc' `
                -Found ("{0} ({1})" -f $releaseMatch.Groups[1].Value, $hostMatch.Groups[1].Value) `
                -Required ("{0} ({1})" -f $ExpectedRustVersion, $platform.RustTarget)
        }
        else {
            Add-Result -Status 'FAIL' -Name 'rustc' -Found 'version or host mismatch' `
                -Required ("{0} ({1})" -f $ExpectedRustVersion, $platform.RustTarget) `
                -Remediation 'Remove PATH shadowing and let the repository rust-toolchain.toml select the MSVC toolchain.'
        }
    }
    else {
        Add-Result -Status 'FAIL' -Name 'rustc' -Found 'not run; pinned toolchain is absent' `
            -Required $ExpectedRustVersion -Remediation 'Install the pinned Rust toolchain first.'
    }

    if ($null -eq $cargo) {
        Add-Result -Status 'FAIL' -Name 'Cargo' -Found 'not found on PATH' -Required $ExpectedRustVersion `
            -Remediation 'Ensure the rustup proxy directory is on PATH, restart the terminal, and rerun the doctor.'
    }
    elseif ($rustToolchainInstalled) {
        $cargoVersionResult = Invoke-ReadOnlyCommand -Command $cargo -Arguments @('--version') `
            -WorkingDirectory $script:RepoRoot -Environment $rustupEnvironment
        $cargoText = $cargoVersionResult.Output -join ' '
        $managedCargoResult = Invoke-ReadOnlyCommand -Command $rustup `
            -Arguments @('run', $rustToolchainName, 'cargo', '--version') `
            -WorkingDirectory $script:RepoRoot -Environment $rustupEnvironment
        $managedCargoText = $managedCargoResult.Output -join ' '
        if ($cargoVersionResult.ExitCode -eq 0 -and $managedCargoResult.ExitCode -eq 0 -and
            $cargoText.Trim() -eq $managedCargoText.Trim() -and
            $cargoText -match '^cargo\s+(\d+\.\d+\.\d+)') {
            Add-Result -Status 'PASS' -Name 'Cargo' -Found $Matches[1] `
                -Required ("Cargo bundled with Rust {0}" -f $ExpectedRustVersion)
        }
        else {
            Add-Result -Status 'FAIL' -Name 'Cargo' -Found 'does not match the pinned toolchain' `
                -Required ("Cargo bundled with Rust {0}" -f $ExpectedRustVersion) `
                -Remediation 'Remove PATH shadowing and let rustup provide Cargo for the pinned toolchain.'
        }
    }
    else {
        Add-Result -Status 'FAIL' -Name 'Cargo' -Found 'not run; pinned toolchain is absent' `
            -Required $ExpectedRustVersion -Remediation 'Install the pinned Rust toolchain first.'
    }

    if ($null -ne $rustup -and $rustToolchainInstalled) {
        $targetListResult = Invoke-ReadOnlyCommand -Command $rustup `
            -Arguments @('target', 'list', '--installed', '--toolchain', $rustToolchainName) `
            -WorkingDirectory $script:RepoRoot -Environment $rustupEnvironment
        if ($targetListResult.ExitCode -eq 0 -and $platform.RustTarget -in $targetListResult.Output) {
            Add-Result -Status 'PASS' -Name 'Rust MSVC target' -Found $platform.RustTarget `
                -Required $platform.RustTarget
        }
        else {
            Add-Result -Status 'FAIL' -Name 'Rust MSVC target' -Found 'not installed' `
                -Required $platform.RustTarget `
                -Remediation ("Run 'rustup target add {0} --toolchain {1}', then rerun the doctor." -f $platform.RustTarget, $ExpectedRustVersion)
        }

        $componentListResult = Invoke-ReadOnlyCommand -Command $rustup `
            -Arguments @('component', 'list', '--installed', '--toolchain', $rustToolchainName) `
            -WorkingDirectory $script:RepoRoot -Environment $rustupEnvironment
        $missingComponents = @()
        if ($componentListResult.ExitCode -eq 0) {
            foreach ($component in $requiredRustComponents) {
                $escapedComponent = [regex]::Escape($component)
                if (-not @($componentListResult.Output | Where-Object { $_ -match "^$escapedComponent(?:-|$)" })) {
                    $missingComponents += $component
                }
            }
        }
        else {
            $missingComponents = $requiredRustComponents
        }

        if ($missingComponents.Count -eq 0) {
            Add-Result -Status 'PASS' -Name 'Rust components' -Found ($requiredRustComponents -join ', ') `
                -Required ($requiredRustComponents -join ', ')
        }
        else {
            Add-Result -Status 'FAIL' -Name 'Rust components' -Found ("missing {0}" -f ($missingComponents -join ', ')) `
                -Required ($requiredRustComponents -join ', ') `
                -Remediation ("Run 'rustup component add {0} --toolchain {1}', then rerun the doctor." -f ($missingComponents -join ' '), $ExpectedRustVersion)
        }
    }

    $node = Get-ExternalCommand -Name 'node.exe'
    if ($null -eq $node) {
        $node = Get-ExternalCommand -Name 'node'
    }
    $hostNodeVersion = $null
    if ($null -ne $node) {
        $nodeVersionResult = Invoke-ReadOnlyCommand -Command $node -Arguments @('--version') `
            -WorkingDirectory $script:RepoRoot
        $nodeText = $nodeVersionResult.Output -join ' '
        if ($nodeVersionResult.ExitCode -eq 0 -and $nodeText -match '^v(\d+\.\d+\.\d+)') {
            $hostNodeVersion = $Matches[1]
        }
    }

    $managedNodePath = Join-Path $script:RepoRoot 'node_modules\.bin\node.exe'
    $managedNodePresent = Test-Path -LiteralPath $managedNodePath
    $managedNodeVersion = $null
    if ($managedNodePresent) {
        $managedNodeResult = Invoke-ReadOnlyCommand -Command $managedNodePath -Arguments @('--version') `
            -WorkingDirectory $script:RepoRoot
        $managedNodeText = $managedNodeResult.Output -join ' '
        if ($managedNodeResult.ExitCode -eq 0 -and $managedNodeText -match '^v(\d+\.\d+\.\d+)') {
            $managedNodeVersion = $Matches[1]
        }
    }

    $managedNodeExact = $managedNodeVersion -eq $ExpectedNodeVersion
    $hostNodeExact = $hostNodeVersion -eq $ExpectedNodeVersion
    $hostNodeParsed = $null
    $hostNodeCompatible = $false
    if ($null -ne $hostNodeVersion -and [Version]::TryParse($hostNodeVersion, [ref]$hostNodeParsed)) {
        $hostNodeCompatible = (($hostNodeParsed.Major -eq 22 -and $hostNodeParsed -ge [Version]'22.13.0') -or
            $hostNodeParsed.Major -eq 24)
    }
    if ($managedNodeExact) {
        Add-Result -Status 'PASS' -Name 'Project Node.js' `
            -Found ("{0} (repository-managed)" -f $managedNodeVersion) -Required $ExpectedNodeVersion
    }
    elseif ($managedNodePresent) {
        $foundManagedNode = if ($null -eq $managedNodeVersion) {
            'repository-managed runtime could not be executed'
        }
        else {
            "${managedNodeVersion} (repository-managed)"
        }
        Add-Result -Status 'FAIL' -Name 'Project Node.js' -Found $foundManagedNode `
            -Required $ExpectedNodeVersion `
            -Remediation ("While online, rerun 'corepack pnpm@{0} install --frozen-lockfile' to provision Node.js {1}." -f $ExpectedPnpmVersion, $ExpectedNodeVersion)
    }
    elseif ($hostNodeExact) {
        Add-Result -Status 'PASS' -Name 'Project Node.js' `
            -Found ("{0} (host)" -f $hostNodeVersion) -Required $ExpectedNodeVersion
    }
    else {
        $foundProjectNode = if ($null -eq $hostNodeVersion) { 'exact runtime is not installed' } else { "${hostNodeVersion} (host only)" }
        Add-Result -Status 'FAIL' -Name 'Project Node.js' -Found $foundProjectNode `
            -Required $ExpectedNodeVersion `
            -Remediation ("Install Node.js {0}, or run the first locked pnpm install while online to provision the managed runtime." -f $ExpectedNodeVersion)
    }

    if ($null -eq $hostNodeVersion) {
        Add-Result -Status 'FAIL' -Name 'Bootstrap Node.js' -Found 'not found or unusable on PATH' `
            -Required $BootstrapNodeRequirement `
            -Remediation 'Install a compatible host Node.js runtime, restart the terminal, and rerun the doctor.'
    }
    elseif (-not $hostNodeCompatible) {
        Add-Result -Status 'FAIL' -Name 'Bootstrap Node.js' -Found $hostNodeVersion `
            -Required $BootstrapNodeRequirement `
            -Remediation 'Install a supported Node.js 22 or 24 LTS runtime with Corepack, restart the terminal, and rerun the doctor.'
    }
    elseif ($hostNodeExact) {
        Add-Result -Status 'PASS' -Name 'Bootstrap Node.js' -Found $hostNodeVersion `
            -Required $BootstrapNodeRequirement
    }
    else {
        $bootstrapRemediation = if ($managedNodeExact) {
            'No host-runtime change is required while the repository-managed Node.js runtime passes.'
        }
        else {
            ("Provision the managed Node.js {0} runtime during the first online install, or install that exact version on the host." -f $ExpectedNodeVersion)
        }
        Add-Result -Status 'WARN' -Name 'Bootstrap Node.js' `
            -Found ("{0} is compatible; project commands use Node.js {1}" -f $hostNodeVersion, $ExpectedNodeVersion) `
            -Required $BootstrapNodeRequirement `
            -Remediation $bootstrapRemediation
    }

    $corepackEnvironment = @{
        COREPACK_ENABLE_NETWORK         = '0'
        COREPACK_ENABLE_AUTO_PIN        = '0'
        COREPACK_DEFAULT_TO_LATEST      = '0'
        COREPACK_ENABLE_DOWNLOAD_PROMPT = '0'
        COREPACK_ENV_FILE               = '0'
        CI                              = '1'
        NO_UPDATE_NOTIFIER              = '1'
    }
    $corepack = Get-ExternalCommand -Name 'corepack.cmd'
    if ($null -eq $corepack) {
        $corepack = Get-ExternalCommand -Name 'corepack'
    }
    if ($null -eq $corepack) {
        Add-Result -Status 'FAIL' -Name 'Corepack' -Found 'not found on PATH' -Required 'Corepack' `
            -Remediation 'Install or enable the Corepack supplied with Node.js, restart the terminal, and rerun the doctor.'
    }
    else {
        $corepackVersionResult = Invoke-ReadOnlyCommand -Command $corepack -Arguments @('--version') `
            -WorkingDirectory $packageWorkingDirectory -Environment $corepackEnvironment
        $corepackText = $corepackVersionResult.Output -join ' '
        if ($corepackVersionResult.ExitCode -eq 0 -and $corepackText -match '(\d+\.\d+\.\d+)') {
            Add-Result -Status 'PASS' -Name 'Corepack' -Found $Matches[1] -Required 'working Corepack'
        }
        else {
            Add-Result -Status 'FAIL' -Name 'Corepack' -Found 'version check failed' -Required 'working Corepack' `
                -Remediation 'Repair Corepack, restart the terminal, and rerun the doctor.'
        }

        if ($packageManagerConfigured) {
            $managedPnpmResult = Invoke-ReadOnlyCommand -Command $corepack -Arguments @('pnpm', '--version') `
                -WorkingDirectory $packageWorkingDirectory -Environment $corepackEnvironment
            $managedPnpmText = $managedPnpmResult.Output -join ' '
            if ($managedPnpmResult.ExitCode -eq 0 -and $managedPnpmText -match '(\d+\.\d+\.\d+)' -and
                $Matches[1] -eq $ExpectedPnpmVersion) {
                Add-Result -Status 'PASS' -Name 'Corepack pnpm' -Found $Matches[1] -Required $ExpectedPnpmVersion
            }
            else {
                Add-Result -Status 'FAIL' -Name 'Corepack pnpm' -Found 'pinned version is not available offline' `
                    -Required $ExpectedPnpmVersion `
                    -Remediation 'While online, hydrate the pinned pnpm version from the repository root, then rerun the doctor.'
            }
        }
    }

    $pnpm = Get-ExternalCommand -Name 'pnpm.cmd'
    if ($null -eq $pnpm) {
        $pnpm = Get-ExternalCommand -Name 'pnpm'
    }
    if ($null -eq $pnpm) {
        Add-Result -Status 'PASS' -Name 'Direct pnpm command' `
            -Found 'not present on PATH and not required' `
            -Required ("optional; repository commands use Corepack pnpm {0}" -f $ExpectedPnpmVersion)
    }
    else {
        Add-Result -Status 'WARN' -Name 'Direct pnpm command' `
            -Found 'present but intentionally not executed; repository commands are unaffected' `
            -Required ("optional; repository commands use Corepack pnpm {0}" -f $ExpectedPnpmVersion) `
            -Remediation ("Use 'corepack pnpm@{0}' for this repository; the direct pnpm command does not need to be inspected or changed." -f $ExpectedPnpmVersion)
    }

    $vsWhere = Find-VsWhere
    $registeredInstances = @()
    $allInstanceRoots = [System.Collections.Generic.List[string]]::new()
    if ($null -ne $vsWhere) {
        $registeredInstances = @(Get-VsWhereInstances -VsWhere $vsWhere -RequiredComponents @(
                'Microsoft.VisualStudio.Workload.VCTools',
                $platform.VcComponent
            ))
        foreach ($instance in @(Get-VsWhereInstances -VsWhere $vsWhere)) {
            $installationPath = [string](Get-JsonProperty -InputObject $instance -Name 'installationPath')
            if (-not [string]::IsNullOrWhiteSpace($installationPath)) {
                $allInstanceRoots.Add($installationPath)
            }
        }
    }
    foreach ($knownRoot in @(Get-KnownVisualStudioRoots)) {
        $allInstanceRoots.Add($knownRoot)
    }

    $registeredToolInstance = $null
    foreach ($instance in $registeredInstances) {
        $installationPath = [string](Get-JsonProperty -InputObject $instance -Name 'installationPath')
        if (-not [string]::IsNullOrWhiteSpace($installationPath) -and
            (Test-VisualStudioToolFiles -InstallationPath $installationPath -CompilerBin $platform.CompilerBin)) {
            $registeredToolInstance = $instance
            break
        }
    }

    if ($null -ne $registeredToolInstance) {
        $displayName = [string](Get-JsonProperty -InputObject $registeredToolInstance -Name 'displayName')
        $installationVersion = [string](Get-JsonProperty -InputObject $registeredToolInstance -Name 'installationVersion')
        if ([string]::IsNullOrWhiteSpace($displayName)) {
            $displayName = 'Visual Studio C++ tools'
        }
        Add-Result -Status 'PASS' -Name 'Visual Studio C++ tools' `
            -Found ("{0} {1}" -f $displayName, $installationVersion).Trim() `
            -Required 'Desktop development with C++ and the matching MSVC compiler'
    }
    else {
        $fallbackToolsFound = $false
        foreach ($installationPath in @($allInstanceRoots | Select-Object -Unique)) {
            if (Test-VisualStudioToolFiles -InstallationPath $installationPath -CompilerBin $platform.CompilerBin) {
                $fallbackToolsFound = $true
                break
            }
        }

        if ($fallbackToolsFound) {
            Add-Result -Status 'WARN' -Name 'Visual Studio C++ tools' `
                -Found 'compiler files found; workload registration was not confirmed' `
                -Required 'Desktop development with C++ and the matching MSVC compiler' `
                -Remediation 'Open Visual Studio Installer and confirm that Desktop development with C++ and the matching compiler component are selected.'
        }
        else {
            Add-Result -Status 'FAIL' -Name 'Visual Studio C++ tools' -Found 'not found or incomplete' `
                -Required 'Desktop development with C++ and the matching MSVC compiler' `
                -Remediation 'Install or modify Visual Studio Build Tools with Desktop development with C++ and the matching MSVC compiler component.'
        }
    }

    $windowsSdkVersion = Find-WindowsSdkVersion -Architecture $platform.SdkArch
    if ($null -ne $windowsSdkVersion) {
        Add-Result -Status 'PASS' -Name 'Windows SDK' -Found $windowsSdkVersion `
            -Required ("headers, libraries, and resource compiler for {0}" -f $platform.SdkArch)
    }
    else {
        Add-Result -Status 'FAIL' -Name 'Windows SDK' -Found 'not found or incomplete' `
            -Required ("headers, libraries, and resource compiler for {0}" -f $platform.SdkArch) `
            -Remediation 'Use Visual Studio Installer to add a supported Windows SDK to the C++ workload.'
    }

    $webViewRegistryVersions = @(Get-WebView2RegistryVersions)
    $webViewFileVersions = @(Get-WebView2FileVersions)
    $matchingWebViewVersions = @($webViewRegistryVersions |
            Where-Object { $_ -in $webViewFileVersions } |
            Sort-Object { [Version]$_ } -Descending)
    if ($matchingWebViewVersions.Count -gt 0) {
        Add-Result -Status 'PASS' -Name 'WebView2 Runtime' -Found $matchingWebViewVersions[0] `
            -Required 'registered Evergreen WebView2 Runtime with matching files'
    }
    elseif ($webViewRegistryVersions.Count -gt 0 -and $webViewFileVersions.Count -gt 0) {
        Add-Result -Status 'FAIL' -Name 'WebView2 Runtime' `
            -Found 'registered versions and runtime files do not match' `
            -Required 'registered Evergreen WebView2 Runtime with matching files' `
            -Remediation 'Repair or update the Evergreen WebView2 Runtime, then rerun the doctor.'
    }
    elseif ($webViewRegistryVersions.Count -gt 0) {
        Add-Result -Status 'FAIL' -Name 'WebView2 Runtime' `
            -Found 'registered runtime has no matching executable' `
            -Required 'registered Evergreen WebView2 Runtime with matching files' `
            -Remediation 'Repair or update the Evergreen WebView2 Runtime, then rerun the doctor.'
    }
    elseif ($webViewFileVersions.Count -gt 0) {
        Add-Result -Status 'FAIL' -Name 'WebView2 Runtime' `
            -Found 'runtime files exist, but the Evergreen Runtime is not registered' `
            -Required 'registered Evergreen WebView2 Runtime with matching files' `
            -Remediation 'Install or repair the Evergreen WebView2 Runtime, then rerun the doctor.'
    }
    else {
        Add-Result -Status 'FAIL' -Name 'WebView2 Runtime' -Found 'not found' `
            -Required 'registered Evergreen WebView2 Runtime with matching files' `
            -Remediation 'Install the Microsoft Evergreen WebView2 Runtime, then rerun the doctor.'
    }
}
catch {
    if (-not $script:HasConfigurationError) {
        Add-Result -Status 'FAIL' -Name 'Doctor execution' -Found 'an internal check failed' `
            -Required 'all checks to complete' `
            -Remediation 'Review the script and rerun it; no installation or update was attempted.' `
            -ConfigurationError
    }
}

Write-DoctorReport

$failed = @($script:Results | Where-Object { $_.Status -eq 'FAIL' }).Count -gt 0
if ($script:HasConfigurationError) {
    exit 2
}
if ($failed) {
    exit 1
}
exit 0
