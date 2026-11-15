<#
.SYNOPSIS
    Installs the DBPinball command-line interface for the current user.

.DESCRIPTION
    Copies dbpinball.exe into %LOCALAPPDATA%\dbpinball\bin and adds that
    directory to the *user* PATH, so a new terminal resolves `dbpinball`
    without any manual environment editing. No administrator rights are
    required and no machine-wide setting is touched.

    The user PATH is read and written as the raw registry value, without
    expanding %VARIABLES% and without normalising separators, so every existing
    entry is preserved byte for byte. The installer records whether it added the
    entry; uninstall removes the entry only when this installation added it.

.PARAMETER Source
    Path to dbpinball.exe. Defaults to the copy next to this script.

.PARAMETER Uninstall
    Removes the installed binary and, if this installation added it, the PATH
    entry.

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
$StateDir = Join-Path $env:LOCALAPPDATA 'dbpinball'
$StateFile = Join-Path $StateDir 'cli-install-state.json'
$EnvironmentKey = 'HKCU:\Environment'
$MutexName = 'Local\dbpinball-cli-install'

function Write-Step {
    param([string]$Message)
    if (-not $Quiet) { Write-Host $Message }
}

# Serialise install and uninstall so a concurrent run cannot interleave the
# binary copy with the PATH edit.
$mutex = New-Object System.Threading.Mutex($false, $MutexName)
$acquired = $false
try {
    $acquired = $mutex.WaitOne([TimeSpan]::FromSeconds(30))
    if (-not $acquired) { throw 'Another DBPinball CLI install or uninstall is running.' }

    # The raw value, with %VARIABLES% left unexpanded. A read error is fatal:
    # treating it as an empty PATH would let the installer overwrite the value.
    function Get-RawUserPath {
        $key = Get-Item -Path $EnvironmentKey
        try {
            $value = $key.GetValue('Path', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        } catch {
            throw "The user PATH could not be read, so it will not be modified: $($_.Exception.Message)"
        }
        if ($null -eq $value) { return '' }
        return [string]$value
    }

    function Get-UserPathKind {
        $key = Get-Item -Path $EnvironmentKey
        try {
            return $key.GetValueKind('Path')
        } catch {
            return [Microsoft.Win32.RegistryValueKind]::ExpandString
        }
    }

    function Set-RawUserPath {
        param([string]$Value, [Microsoft.Win32.RegistryValueKind]$Kind)
        if ([string]::IsNullOrEmpty($Value)) {
            Remove-ItemProperty -Path $EnvironmentKey -Name Path -ErrorAction SilentlyContinue
            return
        }
        Set-ItemProperty -Path $EnvironmentKey -Name Path -Value $Value -Type $Kind
    }

    function Send-EnvironmentChanged {
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

    # Remove exactly one matching entry, leaving every other entry untouched.
    function Remove-OnePathEntry {
        param([string]$Value, [string]$Entry)
        $normalized = $Entry.TrimEnd('\')
        $entries = @(Split-PathEntries $Value)
        for ($i = 0; $i -lt $entries.Count; $i++) {
            if ([string]::IsNullOrWhiteSpace($entries[$i])) { continue }
            if ($entries[$i].TrimEnd('\') -ieq $normalized) {
                $kept = @()
                for ($j = 0; $j -lt $entries.Count; $j++) {
                    if ($j -ne $i) { $kept += $entries[$j] }
                }
                return ($kept -join ';')
            }
        }
        return $Value
    }

    function Read-State {
        if (-not (Test-Path -LiteralPath $StateFile)) { return $null }
        try {
            return Get-Content -Raw -LiteralPath $StateFile | ConvertFrom-Json
        } catch {
            return $null
        }
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

        $state = Read-State
        $addedByUs = $null -ne $state -and $state.pathEntryAdded -eq $true
        $current = Get-RawUserPath
        if (Test-PathEntry -Value $current -Entry $InstallDir) {
            if ($addedByUs) {
                $updated = Remove-OnePathEntry -Value $current -Entry $InstallDir
                Set-RawUserPath -Value $updated -Kind (Get-UserPathKind)
                Send-EnvironmentChanged
                Write-Step "Removed $InstallDir from the user PATH."
            } else {
                Write-Step "$InstallDir was already on the user PATH before this installation, so it was left in place."
            }
        }
        if (Test-Path -LiteralPath $StateFile) { Remove-Item -LiteralPath $StateFile -Force }
        Write-Step 'DBPinball CLI uninstalled. Open a new terminal to pick up the change.'
        exit 0
    }

    if (-not $Source) {
        $Source = Join-Path $PSScriptRoot 'dbpinball.exe'
    }
    if (-not (Test-Path -LiteralPath $Source)) {
        throw "dbpinball.exe was not found at '$Source'. Pass -Source <path> or run this script from the extracted archive."
    }
    $Source = (Resolve-Path -LiteralPath $Source).Path

    # Prove the file is the DBPinball CLI, not merely an executable that exits 0.
    $reported = (& $Source --version) 2>$null
    if ($LASTEXITCODE -ne 0 -or -not ($reported -match '^dbpinball \d+\.\d+\.\d+')) {
        throw "'$Source --version' did not report a DBPinball version, so it is not the CLI."
    }

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    # Replace atomically so an interrupted copy cannot leave a truncated binary.
    $staged = Join-Path $InstallDir 'dbpinball.exe.new'
    Copy-Item -LiteralPath $Source -Destination $staged -Force
    Move-Item -LiteralPath $staged -Destination $TargetExe -Force
    Write-Step "Installed $reported to $TargetExe"

    $current = Get-RawUserPath
    $previousState = Read-State
    $entryPresent = Test-PathEntry -Value $current -Entry $InstallDir

    # Ownership survives a reinstall: if an earlier run added the entry, a later
    # run must not downgrade that to "pre-existing", or uninstall would leave it
    # behind.
    $addedByUs = $false
    if ($entryPresent) {
        if ($null -ne $previousState -and $previousState.pathEntryAdded -eq $true) {
            $addedByUs = $true
            Write-Step "$InstallDir is already on the user PATH (added by an earlier install)."
        } else {
            Write-Step "$InstallDir is already on the user PATH."
        }
    } else {
        $updated = if ([string]::IsNullOrEmpty($current)) { $InstallDir } else { $current + ';' + $InstallDir }
        Set-RawUserPath -Value $updated -Kind (Get-UserPathKind)
        Send-EnvironmentChanged
        $addedByUs = $true
        Write-Step "Added $InstallDir to the user PATH."
    }

    # Record ownership. If the state cannot be written, undo the PATH edit so a
    # later uninstall is never left guessing.
    try {
        New-Item -ItemType Directory -Path $StateDir -Force | Out-Null
        [ordered]@{
            installDir     = $InstallDir
            pathEntryAdded = $addedByUs
            version        = $reported
            installedAt    = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
        } | ConvertTo-Json | Set-Content -LiteralPath $StateFile -Encoding utf8
    } catch {
        if (-not $entryPresent) {
            Set-RawUserPath -Value $current -Kind (Get-UserPathKind)
            Send-EnvironmentChanged
        }
        throw "The install state could not be recorded, so the PATH change was reverted: $($_.Exception.Message)"
    }

    $installed = & $TargetExe --version
    if ($LASTEXITCODE -ne 0 -or -not ($installed -match '^dbpinball \d+\.\d+\.\d+')) {
        throw "The installed binary at '$TargetExe' did not run."
    }
    Write-Step "Verified: $installed"
    Write-Step 'Open a new terminal and run: dbpinball --version'
}
finally {
    if ($acquired) { $mutex.ReleaseMutex() }
    $mutex.Dispose()
}
