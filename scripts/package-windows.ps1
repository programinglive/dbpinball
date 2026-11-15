<#
.SYNOPSIS
    Builds the DBPinball Windows release artifacts.

.DESCRIPTION
    Produces, under artifacts\release\v<version>\:

      dbpinball_<version>_x64-setup.exe        Desktop installer (NSIS, per-user)
      dbpinball-cli-<version>-x64.zip          Portable CLI with install-cli.ps1
      SHA256SUMS.txt                           Checksums for every artifact
      provenance.json                          Version, commit, architecture, toolchain

    Nothing is published. The artifacts are local files that CI uploads as a
    workflow artifact; creating a tag or a GitHub release is a separate,
    explicitly authorized action.

.PARAMETER Version
    Overrides the version read from src-tauri\tauri.conf.json.

.PARAMETER SkipDesktop
    Builds only the CLI archive.

.PARAMETER SkipCli
    Builds only the Desktop installer.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\scripts\package-windows.ps1
#>
[CmdletBinding()]
param(
    [string]$Version,
    [switch]$SkipDesktop,
    [switch]$SkipCli
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot
try {
    if (-not $Version) {
        $tauriConfig = Get-Content -Raw (Join-Path $repoRoot 'src-tauri\tauri.conf.json') | ConvertFrom-Json
        $Version = $tauriConfig.version
    }
    if (-not $Version) { throw 'The version could not be determined.' }

    $commit = if ($env:DBPINBALL_BUILD_COMMIT) { $env:DBPINBALL_BUILD_COMMIT } else { (git rev-parse HEAD).Trim() }
    $dirty = (git status --porcelain)
    if ($dirty) {
        Write-Warning 'The working tree is not clean; provenance records the commit plus a dirty flag.'
    }

    $arch = 'x64'
    $outDir = Join-Path $repoRoot "artifacts\release\v$Version"
    # Start from an empty directory: a file left by an earlier run must never
    # travel with this release.
    if (Test-Path -LiteralPath $outDir) {
        Remove-Item -LiteralPath $outDir -Recurse -Force
    }
    New-Item -ItemType Directory -Path $outDir -Force | Out-Null

    $artifacts = @()

    if (-not $SkipCli) {
        Write-Host 'Building the CLI (release)...'
        # The Desktop app and the CLI both produce `dbpinball.exe`. A shared
        # target directory therefore lets one overwrite the other, which
        # silently ships the GUI binary as the CLI. Build the CLI into its own
        # target directory so the two can never collide.
        $cliTarget = Join-Path $repoRoot 'target\cli-dist'
        $previousTarget = $env:CARGO_TARGET_DIR
        $env:CARGO_TARGET_DIR = $cliTarget
        try {
            cargo build -p dbpinball-cli --release --locked
            if ($LASTEXITCODE -ne 0) { throw 'cargo build failed.' }
        }
        finally {
            $env:CARGO_TARGET_DIR = $previousTarget
        }

        $cliExe = Join-Path $cliTarget 'release\dbpinball.exe'
        if (-not (Test-Path -LiteralPath $cliExe)) { throw "Missing $cliExe" }

        # Prove the artifact is the CLI and not the desktop application before
        # it reaches an archive.
        $reported = (& $cliExe --version) 2>$null
        if ($LASTEXITCODE -ne 0 -or -not $reported) {
            throw "The packaged binary did not answer --version; it is not the CLI."
        }
        if ($reported -ne "dbpinball $Version") {
            throw "The CLI reports '$reported' but the release is $Version."
        }
        Write-Host "CLI reports: $reported"

        $stage = Join-Path $repoRoot "artifacts\stage\cli-$Version"
        if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
        New-Item -ItemType Directory -Path $stage -Force | Out-Null
        Copy-Item -LiteralPath $cliExe -Destination (Join-Path $stage 'dbpinball.exe')
        Copy-Item -LiteralPath (Join-Path $repoRoot 'scripts\install-cli.ps1') -Destination $stage
        Copy-Item -LiteralPath (Join-Path $repoRoot 'scripts\CLI-README.txt') -Destination $stage
        # MIT and Apache-2.0 both require the notices to travel with a binary
        # distribution, so the archive carries them.
        Copy-Item -LiteralPath (Join-Path $repoRoot 'LICENSE') -Destination (Join-Path $stage 'LICENSE')
        Copy-Item -LiteralPath (Join-Path $repoRoot 'THIRD_PARTY_NOTICES.md') -Destination $stage

        $zip = Join-Path $outDir "dbpinball-cli-$Version-$arch.zip"
        if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip -Force }
        Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -CompressionLevel Optimal
        $artifacts += $zip
        Write-Host "CLI archive: $zip"
    }

    if (-not $SkipDesktop) {
        Write-Host 'Building the Desktop installer (NSIS)...'
        npm run tauri build -- --bundles nsis
        if ($LASTEXITCODE -ne 0) { throw 'tauri build failed.' }

        $nsisDir = Join-Path $repoRoot 'target\release\bundle\nsis'
        if (-not (Test-Path -LiteralPath $nsisDir)) {
            $nsisDir = Join-Path $repoRoot 'src-tauri\target\release\bundle\nsis'
        }
        # Select the installer for *this* version. A stale installer from an
        # earlier build sits in the same directory, and picking the first match
        # would ship the old binary under the new name.
        $expectedName = "dbpinball_${Version}_${arch}-setup.exe"
        $setup = Get-ChildItem -LiteralPath $nsisDir -Filter $expectedName | Select-Object -First 1
        if (-not $setup) {
            $found = (Get-ChildItem -LiteralPath $nsisDir -Filter '*-setup.exe' | Select-Object -ExpandProperty Name) -join ', '
            throw "No installer named $expectedName in $nsisDir (found: $found). Run the Tauri build for version $Version."
        }
        $setupVersion = (Get-Item -LiteralPath $setup.FullName).VersionInfo.FileVersion
        if ($setupVersion -ne $Version) {
            throw "The installer reports version '$setupVersion' but the release is $Version."
        }
        $target = Join-Path $outDir "dbpinball_${Version}_${arch}-setup.exe"
        Copy-Item -LiteralPath $setup.FullName -Destination $target -Force
        $artifacts += $target
        Write-Host "Desktop installer: $target (FileVersion $setupVersion)"
    }

    $sums = foreach ($artifact in $artifacts) {
        $hash = (Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $(Split-Path -Leaf $artifact)"
    }
    $sumsPath = Join-Path $outDir 'SHA256SUMS.txt'
    Set-Content -LiteralPath $sumsPath -Value ($sums -join "`n") -Encoding ascii
    $artifacts += $sumsPath

    $rustc = (rustc --version) -replace 'rustc ', ''
    $node = (node --version)
    $provenance = [ordered]@{
        product      = 'dbpinball'
        version      = $Version
        commit       = $commit
        dirty        = [bool]$dirty
        architecture = $arch
        platform     = 'windows'
        built_at     = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
        rustc        = $rustc
        node         = $node
        artifacts    = @($artifacts | ForEach-Object { Split-Path -Leaf $_ })
    }
    $provenancePath = Join-Path $outDir 'provenance.json'
    $provenance | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $provenancePath -Encoding utf8
    $artifacts += $provenancePath

    Write-Host ''
    Write-Host "Artifacts in $outDir"
    foreach ($artifact in $artifacts) {
        $size = (Get-Item -LiteralPath $artifact).Length
        Write-Host ("  {0,-45} {1,10} bytes" -f (Split-Path -Leaf $artifact), $size)
    }
}
finally {
    Pop-Location
}
