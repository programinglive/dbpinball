<#
.SYNOPSIS
    Installs the DBPinball command-line interface for the current user.

.DESCRIPTION
    Copies dbpinball.exe into %LOCALAPPDATA%\dbpinball\bin and adds that
    directory to the *user* PATH, so a new terminal resolves `dbpinball`
    without any manual environment editing. No administrator rights are
    required and no machine-wide setting is touched.

    The script is idempotent: running it again replaces the binary and leaves
    the PATH entry untouched. Existing PATH entries are preserved byte for byte.

.PARAMETER Source
    Path to dbpinball.exe. Defaults to the copy next to this script.

.PARAMETER Uninstall
    Removes the installed binary and the PATH entry.

.PARAMETER Quiet
    Suppresses the progress output.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\install-cli.ps1

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\install-cli.ps1 -Uninstall
#>
[CmdletBinding()]
param(
    [string]$Source,
    [switch]$Uninstall,
    [switch]$Quiet
)

$ErrorActionPreference = 'Stop'

$InstallDir = Join-Path $env:LOCALAPPDATA 'dbpinball\bin'
$TargetExe = Join-Path $InstallDir 'dbpinball.exe'
$EnvironmentKey = 'HKCU:\Environment'

function Write-Step {
    param([string]$Message)
    if (-not $Quiet) { Write-Host $Message }
}

function Get-UserPath {
    $item = Get-ItemProperty -Path $EnvironmentKey -Name Path -ErrorAction SilentlyContinue
    if ($null -eq $item -or $null -eq $item.Path) { return '' }
    return [string]$item.Path
}

function Get-UserPathKind {
    $key = Get-Item -Path $EnvironmentKey
    try {
        return $key.GetValueKind('Path')
    } catch {
        return [Microsoft.Win32.RegistryValueKind]::ExpandString
    }
}

function Set-UserPath {
    param([string]$Value, [Microsoft.Win32.RegistryValueKind]$Kind)
    if ([string]::IsNullOrEmpty($Value)) {
        Remove-ItemProperty -Path $EnvironmentKey -Name Path -ErrorAction SilentlyContinue
        return
    }
    Set-ItemProperty -Path $EnvironmentKey -Name Path -Value $Value -Type $Kind
}

function Send-EnvironmentChanged {
    # Tell running shells and Explorer that the environment changed. A new
    # terminal reads the registry directly, so this is a convenience only.
    if (-not ('DbpinballEnvBroadcast' -as [type])) {
        Add-Type -Namespace '' -Name 'DbpinballEnvBroadcast' -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true, CharSet = System.Runtime.InteropServices.CharSet.Auto)]
public static extern System.IntPtr SendMessageTimeout(System.IntPtr hWnd, uint Msg, System.UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out System.UIntPtr lpdwResult);
'@
    }
    $result = [System.UIntPtr]::Zero
    [void][DbpinballEnvBroadcast]::SendMessageTimeout([System.IntPtr]0xffff, 0x001A, [System.UIntPtr]::Zero, 'Environment', 0x0002, 5000, [ref]$result)
}

function Split-PathEntries {
    param([string]$Value)
    if ([string]::IsNullOrEmpty($Value)) { return @() }
    return $Value -split ';'
}

function Test-PathEntry {
    param([string]$Value, [string]$Entry)
    $normalized = $Entry.TrimEnd('\')
    foreach ($candidate in (Split-PathEntries $Value)) {
        if ([string]::IsNullOrWhiteSpace($candidate)) { continue }
        if ($candidate.TrimEnd('\') -ieq $normalized) { return $true }
    }
    return $false
}

if ($Uninstall) {
    Write-Step "Removing $TargetExe"
    if (Test-Path -LiteralPath $TargetExe) {
        Remove-Item -LiteralPath $TargetExe -Force
    }
    if (Test-Path -LiteralPath $InstallDir) {
        $remaining = Get-ChildItem -LiteralPath $InstallDir -Force
        if (-not $remaining) { Remove-Item -LiteralPath $InstallDir -Force }
    }

    $current = Get-UserPath
    if (Test-PathEntry -Value $current -Entry $InstallDir) {
        $kept = Split-PathEntries $current | Where-Object {
            -not [string]::IsNullOrWhiteSpace($_) -and $_.TrimEnd('\') -ine $InstallDir.TrimEnd('\')
        }
        Set-UserPath -Value ($kept -join ';') -Kind (Get-UserPathKind)
        Send-EnvironmentChanged
        Write-Step "Removed $InstallDir from the user PATH."
    }
    Write-Step 'DBPinball CLI uninstalled. Open a new terminal to pick up the change.'
    exit 0
}

if (-not $Source) {
    $Source = Join-Path $PSScriptRoot 'dbpinball.exe'
}
if (-not (Test-Path -LiteralPath $Source)) {
    throw "dbpinball.exe was not found at '$Source'. Pass -Source <path> or run this script from the extracted archive."
}

$version = (& $Source --version) 2>$null
if ($LASTEXITCODE -ne 0) {
    throw "'$Source --version' failed, so the binary is not usable on this machine."
}

New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
Copy-Item -LiteralPath $Source -Destination $TargetExe -Force
Write-Step "Installed $version to $TargetExe"

$current = Get-UserPath
if (Test-PathEntry -Value $current -Entry $InstallDir) {
    Write-Step "$InstallDir is already on the user PATH."
} else {
    $updated = if ([string]::IsNullOrWhiteSpace($current)) { $InstallDir } else { $current.TrimEnd(';') + ';' + $InstallDir }
    Set-UserPath -Value $updated -Kind (Get-UserPathKind)
    Send-EnvironmentChanged
    Write-Step "Added $InstallDir to the user PATH."
}

$installed = & $TargetExe --version
if ($LASTEXITCODE -ne 0) {
    throw "The installed binary at '$TargetExe' did not run."
}
Write-Step "Verified: $installed"
Write-Step 'Open a new terminal and run: dbpinball --version'
