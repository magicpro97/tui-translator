#Requires -Version 5.1
<#
.SYNOPSIS
    Validates a tui-translator Windows release artifact against its SHA256SUMS file.

.DESCRIPTION
    Verifies that a downloaded tui-translator release zip or installer matches the
    expected checksum from the published SHA256SUMS file, and that the zip contains
    the mandatory files (exe, LICENSE, USAGE.md, LICENSES/ directory).

    Run this script after downloading the release artifacts and before installing.

.PARAMETER ArtifactPath
    Path to the zip or installer .exe to validate.

.PARAMETER ChecksumsPath
    Path to the SHA256SUMS-<tag>.txt file downloaded from the same GitHub Release.

.EXAMPLE
    .\scripts\validate-release-artifact.ps1 `
        -ArtifactPath "tui-translator-v1.0.0-x86_64-pc-windows-msvc.zip" `
        -ChecksumsPath "SHA256SUMS-v1.0.0.txt"
#>
param(
    [Parameter(Mandatory)]
    [string] $ArtifactPath,

    [Parameter(Mandatory)]
    [string] $ChecksumsPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Test-Checksum {
    param([string]$FilePath, [string]$SumsFile)

    $fileName = Split-Path $FilePath -Leaf
    $expectedLine = Get-Content $SumsFile |
                    Where-Object { $_ -match [regex]::Escape($fileName) } |
                    Select-Object -First 1

    if (-not $expectedLine) {
        throw "No checksum entry found for '$fileName' in $SumsFile"
    }

    # BSD-style: "SHA256 (name) = <hash>"
    if ($expectedLine -match 'SHA256 \(.+\) = ([0-9a-f]{64})') {
        $expected = $Matches[1].ToLower()
    } else {
        throw "Unrecognised checksum line format: $expectedLine"
    }

    $actual = (Get-FileHash -Algorithm SHA256 $FilePath).Hash.ToLower()

    if ($actual -ne $expected) {
        throw "CHECKSUM MISMATCH for $fileName`n  expected: $expected`n  actual:   $actual"
    }

    Write-Host "  [PASS] Checksum OK: $fileName"
}

function Test-ZipContents {
    param([string]$ZipPath)

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::OpenRead((Resolve-Path $ZipPath).Path)
    try {
        $entries = $zip.Entries | ForEach-Object { $_.FullName }
        $required = @("tui-translator.exe", "LICENSE", "USAGE.md")
        $missing = @()
        foreach ($req in $required) {
            if (-not ($entries -like "*$req*")) { $missing += $req }
        }
        if (-not ($entries -like "LICENSES/*")) {
            $missing += "LICENSES/ (third-party attributions)"
        }
        if ($missing.Count -gt 0) {
            throw "Zip is missing required files: $($missing -join ', ')"
        }
        Write-Host "  [PASS] Zip contents verified ($($entries.Count) entries)"
    }
    finally {
        $zip.Dispose()
    }
}

# ── Main ───────────────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "tui-translator Release Artifact Validator (JV-18)"
Write-Host "=================================================="
Write-Host ""

if (-not (Test-Path $ArtifactPath)) {
    Write-Error "Artifact not found: $ArtifactPath"
    exit 1
}
if (-not (Test-Path $ChecksumsPath)) {
    Write-Error "SHA256SUMS file not found: $ChecksumsPath"
    exit 1
}

Write-Host "Artifact : $ArtifactPath ($([math]::Round((Get-Item $ArtifactPath).Length / 1MB, 2)) MB)"
Write-Host "Checksums: $ChecksumsPath"
Write-Host ""

try {
    Test-Checksum -FilePath $ArtifactPath -SumsFile $ChecksumsPath

    if ($ArtifactPath -like "*.zip") {
        Test-ZipContents -ZipPath $ArtifactPath
    }

    Write-Host ""
    Write-Host "All validation checks PASSED. The artifact is authentic and complete."
    exit 0
}
catch {
    Write-Host ""
    Write-Host "::error:: Validation FAILED: $_"
    exit 1
}
