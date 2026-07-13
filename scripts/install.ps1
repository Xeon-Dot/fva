# Install FVA from GitHub Releases.
#
# Usage:
#   irm https://raw.githubusercontent.com/Xeon-Dot/fva/main/scripts/install.ps1 | iex
#   .\scripts\install.ps1 [-Version v0.2.0] [-InstallDir $env:LOCALAPPDATA\Programs\fva\bin]
#
# Environment:
#   FVA_VERSION    Pin release tag (e.g. v0.2.0)
#   FVA_INSTALL_DIR Destination directory
#   FVA_REPO       GitHub repo slug (default: Xeon-Dot/fva)

[CmdletBinding()]
param(
    [string]$Version = $env:FVA_VERSION,
    [string]$InstallDir = $(if ($env:FVA_INSTALL_DIR) { $env:FVA_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\fva\bin" }),
    [string]$Repo = $(if ($env:FVA_REPO) { $env:FVA_REPO } else { "Xeon-Dot/fva" }),
    [switch]$NoPathUpdate
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$BinaryName = "fva.exe"

function Write-Step([string]$Message) {
    Write-Host "==> $Message"
}

function Get-Architecture {
    $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
    switch ($arch) {
        "X64" { return "amd64" }
        "Arm64" { return "arm64" }
        default { throw "unsupported CPU architecture: $arch" }
    }
}

function Get-AssetUrl([string]$FileName) {
    if ($Version) {
        return "https://github.com/$Repo/releases/download/$Version/$FileName"
    }
    return "https://github.com/$Repo/releases/latest/download/$FileName"
}

function Test-FileSha256([string]$Path, [string]$Expected) {
    $actual = (Get-FileHash -Path $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $Expected.ToLowerInvariant()) {
        throw "checksum mismatch for $(Split-Path -Leaf $Path)"
    }
}

function Add-ToUserPath([string]$Directory) {
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ([string]::IsNullOrWhiteSpace($userPath)) {
        $userPath = ""
    }

    $parts = $userPath -split ";" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    if ($parts -contains $Directory) {
        return
    }

    $newPath = if ($parts.Count -gt 0) { ($parts + $Directory) -join ";" } else { $Directory }
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    $env:Path = "$env:Path;$Directory"
    Write-Step "added $Directory to user PATH (restart terminal if command is not found)"
}

$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("fva-install-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

try {
    $arch = Get-Architecture
    $artifact = "fva-windows-$arch.zip"

    Write-Step "downloading $artifact"
    $archivePath = Join-Path $tempDir $artifact
    Invoke-WebRequest -Uri (Get-AssetUrl $artifact) -OutFile $archivePath -UseBasicParsing

    Write-Step "downloading checksums"
    $sumsPath = Join-Path $tempDir "SHA256SUMS.txt"
    try {
        Invoke-WebRequest -Uri (Get-AssetUrl "SHA256SUMS.txt") -OutFile $sumsPath -UseBasicParsing
        $expected = (Get-Content $sumsPath | Where-Object { $_ -match [regex]::Escape($artifact) } | ForEach-Object { ($_ -split '\s+', 2)[0] } | Select-Object -First 1)
        if (-not $expected) {
            throw "checksum entry not found for $artifact"
        }
        Test-FileSha256 -Path $archivePath -Expected $expected
        Write-Step "checksum verified"
    }
    catch {
        Write-Step "checksum file unavailable or invalid; skipping verification"
    }

    Write-Step "extracting archive"
    $extractDir = Join-Path $tempDir "extract"
    Expand-Archive -Path $archivePath -DestinationPath $extractDir -Force

    $sourceBinary = Join-Path $extractDir $BinaryName
    if (-not (Test-Path $sourceBinary)) {
        throw "binary not found in archive: $BinaryName"
    }

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    $destBinary = Join-Path $InstallDir $BinaryName
    Copy-Item -Path $sourceBinary -Destination $destBinary -Force

    Write-Step "installed $destBinary"

    if (-not $NoPathUpdate) {
        Add-ToUserPath -Directory $InstallDir
    }
    else {
        Write-Step "skipped PATH update"
    }

    Write-Step "done"
    & $destBinary --version
}
finally {
    Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
}