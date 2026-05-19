#Requires -Version 5.1
[CmdletBinding()]
param(
    [switch]$Json,
    [string]$ReleaseHashPath,
    [string]$SmokeLogPath
)

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Checks = New-Object System.Collections.Generic.List[object]

function Add-Check {
    param(
        [string]$Id,
        [string]$Path,
        [bool]$Passed,
        [string]$Detail
    )

    $Checks.Add([pscustomobject]@{
        id = $Id
        path = $Path
        passed = $Passed
        detail = $Detail
    }) | Out-Null
}

function Resolve-RepoPath {
    param([string]$Path)
    if ([System.IO.Path]::IsPathRooted($Path)) {
        return $Path
    }
    return (Join-Path $RepoRoot $Path)
}

function Read-RepoFile {
    param([string]$RelativePath)
    $FullPath = Resolve-RepoPath $RelativePath
    if (-not (Test-Path $FullPath)) {
        return $null
    }
    Get-Content -Raw -Path $FullPath
}

function Assert-FileContains {
    param(
        [string]$Id,
        [string]$RelativePath,
        [string[]]$Needles
    )

    $Content = Read-RepoFile $RelativePath
    if ($null -eq $Content) {
        Add-Check $Id $RelativePath $false "missing file"
        return
    }

    foreach ($Needle in $Needles) {
        if (-not $Content.Contains($Needle)) {
            Add-Check $Id $RelativePath $false "missing text: $Needle"
            return
        }
    }
    Add-Check $Id $RelativePath $true "present"
}

$EvidenceFiles = @(
    @{ Id = "VMIC-B1"; Path = "verification-evidence\vmic\VMIC-B1-format-negotiation.json"; Terms = @('"issue": "#321"', '"status": "pass"', '"convert_i16_pcm"', '"CpalDeviceFormatProvider"') },
    @{ Id = "VMIC-B2"; Path = "verification-evidence\vmic\VMIC-B2-oem-registry.json"; Terms = @('"issue": 322', '"status": "pass"', '"virtual_device_patterns"', '"vendor_binary_dependency": false') },
    @{ Id = "VMIC-B3"; Path = "verification-evidence\vmic\VMIC-B3-production-path-decision.md"; Terms = @('OEM/commercial cable production implementation in VMIC-B4', 'NO-GO** for project-owned custom driver implementation', 'No pure user-mode microphone endpoint path is accepted') },
    @{ Id = "VMIC-B4"; Path = "verification-evidence\vmic\VMIC-B4-production-sink-roundtrip.json"; Terms = @('"issue": "#324"', '"status": "pass"', '"sink": "OemCableSink"', '"human_acceptance_required": false', '"zoom_or_teams_required": false') }
)

foreach ($Evidence in $EvidenceFiles) {
    Assert-FileContains $Evidence.Id $Evidence.Path $Evidence.Terms
}

Assert-FileContains "REPORT-A8" "verification-evidence\vmic\VMIC-A8-mvp-readiness-report.md" @(
    "GO for MVP release",
    "GO for production phase",
    "No human acceptance step is required",
    "VMIC-A8-release-sha256.txt"
)

Assert-FileContains "REPORT-B5" "verification-evidence\vmic\VMIC-B5-production-readiness-report.md" @(
    "GO for production release checkpoint",
    "All VMIC-B production child issues are closed",
    "unsigned application executable",
    "VMIC-B5-release-sha256.txt",
    "VMIC-B5-smoke-log.txt",
    "No manual Zoom/Teams/human acceptance remains in the required path"
)

Assert-FileContains "CODE-ProductionSink" "src\pipeline\audio_sink.rs" @(
    "pub struct OemCableSink",
    "impl AudioSink for OemCableSink",
    "run_memory_production_sink_roundtrip"
)

Assert-FileContains "DOC-ProductionLimitations" "docs\12-virtual-mic-setup.md" @(
    "Supported production path and limitations",
    "OEM/commercial virtual cable",
    "does not create a Windows microphone endpoint by itself",
    "unsigned application executable",
    "pipeline signs it",
    "No manual Zoom or Teams acceptance is required"
)

Assert-FileContains "CI-ProductionGate" ".github\workflows\ci.yml" @(
    "VMIC-B5 production readiness",
    "scripts\check-vmic-production-evidence.ps1",
    "VMIC-B5-release-sha256.txt",
    "VMIC-B5-smoke-log.txt",
    "VMIC-B4 production sink round-trip"
)

if ($ReleaseHashPath) {
    Assert-FileContains "RELEASE-HASH" $ReleaseHashPath @(
        "sha256=",
        "bytes=",
        "unsigned=true",
        "driver_bundled=false"
    )
}

if ($SmokeLogPath) {
    Assert-FileContains "SMOKE-LOG" $SmokeLogPath @(
        "--list-audio-devices",
        "--list-capture-devices"
    )
}

$Failed = @($Checks | Where-Object { -not $_.passed })
$Status = if ($Failed.Count -eq 0) { "pass" } else { "fail" }
$Result = [pscustomobject]@{
    schema_version = 1
    issue = "#325"
    status = $Status
    checked_at = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    checks = $Checks
}

if ($Json) {
    $Result | ConvertTo-Json -Depth 6
} else {
    "VMIC production evidence check: $Status"
    $Checks | Format-Table id, passed, path, detail -AutoSize
}

if ($Failed.Count -gt 0) {
    exit 1
}
