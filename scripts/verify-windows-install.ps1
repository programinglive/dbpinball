<#
.SYNOPSIS
    Installs, launches and uninstalls the DBPinball desktop installer.

.DESCRIPTION
    Acceptance harness for the NSIS installer. It performs a silent per-user
    install, verifies the installed executable and its version metadata, starts
    the application once, then runs the uninstaller and verifies that the
    installation is gone.

    It never installs machine-wide and never edits PATH.

.PARAMETER Installer
    Path to dbpinball_<version>_x64-setup.exe.

.PARAMETER KeepInstalled
    Skips the uninstall step, leaving the application installed.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\scripts\verify-windows-install.ps1 -Installer .\artifacts\release\v0.1.6\dbpinball_0.1.6_x64-setup.exe
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [switch]$KeepInstalled
)

$ErrorActionPreference = 'Stop'

function Fail {
    param([string]$Message)
    Write-Host "FAIL: $Message" -ForegroundColor Red
    exit 1
}

function Pass {
    param([string]$Message)
    Write-Host "PASS: $Message" -ForegroundColor Green
}

if (-not (Test-Path -LiteralPath $Installer)) { Fail "installer not found: $Installer" }
$Installer = (Resolve-Path -LiteralPath $Installer).Path
$installerInfo = (Get-Item -LiteralPath $Installer).VersionInfo
Write-Host "Installer: $Installer"
Write-Host "  ProductName: $($installerInfo.ProductName)"
Write-Host "  FileVersion: $($installerInfo.FileVersion)"

$before = @(Get-ChildItem -Path $env:LOCALAPPDATA -Filter 'dbpinball*' -Directory -ErrorAction SilentlyContinue |
    Select-Object -ExpandProperty FullName)

Write-Host 'Installing silently (per-user)...'
$process = Start-Process -FilePath $Installer -ArgumentList '/S' -Wait -PassThru
if ($process.ExitCode -ne 0) { Fail "installer exited with $($process.ExitCode)" }

$installDir = $null
foreach ($candidate in @(
        (Join-Path $env:LOCALAPPDATA 'dbpinball'),
        (Join-Path $env:LOCALAPPDATA 'Programs\dbpinball'),
        (Join-Path $env:LOCALAPPDATA 'DBPinball')
    )) {
    if (Test-Path -LiteralPath (Join-Path $candidate 'dbpinball.exe')) { $installDir = $candidate; break }
}
if (-not $installDir) {
    $found = Get-ChildItem -Path $env:LOCALAPPDATA -Filter 'dbpinball.exe' -Recurse -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($found) { $installDir = $found.DirectoryName }
}
if (-not $installDir) { Fail 'the installer did not place dbpinball.exe under LOCALAPPDATA' }
Pass "installed to $installDir"

$exe = Join-Path $installDir 'dbpinball.exe'
$info = (Get-Item -LiteralPath $exe).VersionInfo
if (-not $info.ProductName) { Fail 'the installed executable has no version metadata' }
Pass "version metadata: $($info.ProductName) $($info.FileVersion)"

$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
$shortcut = Get-ChildItem -Path $startMenu -Filter '*dbpinball*.lnk' -Recurse -ErrorAction SilentlyContinue |
    Select-Object -First 1
if ($shortcut) { Pass "Start Menu shortcut: $($shortcut.FullName)" }
else { Write-Host 'WARN: no Start Menu shortcut found' -ForegroundColor Yellow }

Write-Host 'Launching the application...'
$app = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 6
if ($app.HasExited) { Fail "the application exited immediately with $($app.ExitCode)" }
Pass "application started (pid $($app.Id))"
Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2

if ($KeepInstalled) {
    Pass 'left installed as requested'
    exit 0
}

$uninstaller = Join-Path $installDir 'uninstall.exe'
if (-not (Test-Path -LiteralPath $uninstaller)) { Fail 'no uninstaller was installed' }
Write-Host 'Uninstalling silently...'
$uninstall = Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru
if ($uninstall.ExitCode -ne 0) { Fail "uninstaller exited with $($uninstall.ExitCode)" }
Start-Sleep -Seconds 3

if (Test-Path -LiteralPath $exe) { Fail 'dbpinball.exe still exists after uninstall' }
Pass 'uninstall removed the application'

$after = @(Get-ChildItem -Path $env:LOCALAPPDATA -Filter 'dbpinball*' -Directory -ErrorAction SilentlyContinue |
    Select-Object -ExpandProperty FullName)
$leftover = $after | Where-Object { $before -notcontains $_ }
if ($leftover) { Write-Host "WARN: leftover directories: $($leftover -join ', ')" -ForegroundColor Yellow }

Pass 'desktop installer acceptance complete'
