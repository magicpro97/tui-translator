#Requires -Version 5.1
[CmdletBinding()]
param(
    [switch]$Json
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

function Read-RepoFile {
    param([string]$RelativePath)
    $FullPath = Join-Path $RepoRoot $RelativePath
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
    @{ Id = "VMIC-00"; Path = "verification-evidence\vmic\VMIC-00-audio-sink-report.json"; Terms = @('"status": "pass"', '"issue": 312') },
    @{ Id = "VMIC-A1"; Path = "verification-evidence\vmic\VMIC-A1-device-probe.json"; Terms = @('"status": "pass"', '"issue": 313') },
    @{ Id = "VMIC-A2"; Path = "verification-evidence\vmic\VMIC-A2-config-schema.json"; Terms = @('"status": "pass"', '"issue": 314') },
    @{ Id = "VMIC-A3"; Path = "verification-evidence\vmic\VMIC-A3-routing-report.json"; Terms = @('"status": "pass"', '"issue": 315') },
    @{ Id = "VMIC-A4"; Path = "verification-evidence\vmic\VMIC-A4-settings-pty.json"; Terms = @('"status": "pass"', '"issue": 316') },
    @{ Id = "VMIC-A5"; Path = "verification-evidence\vmic\VMIC-A5-status-render.json"; Terms = @('"status": "pass"', '"issue": 317') },
    @{ Id = "VMIC-A6"; Path = "verification-evidence\vmic\VMIC-A6-vbcable-ci-report.json"; Terms = @('"status": "pass"', '"memory_pcm"', '"real_virtual_cable"') },
    @{ Id = "VMIC-A7"; Path = "verification-evidence\vmic\VMIC-A7-docs-check.json"; Terms = @('"status": "pass"', '"issue": "#319"', '"T1"', '"T2"', '"T3"') }
)

foreach ($Evidence in $EvidenceFiles) {
    Assert-FileContains $Evidence.Id $Evidence.Path $Evidence.Terms
}

Assert-FileContains "CODE-AudioSink" "src\pipeline\audio_sink.rs" @(
    "pub trait AudioSink",
    "pub struct MockAudioSink"
)
Assert-FileContains "CODE-TtsRouting" "src\config\mod.rs" @(
    "pub enum TtsRouting",
    "Speakers",
    "VirtualMic",
    "Both",
    "pub virtual_mic_device"
)
Assert-FileContains "CODE-MultiSink" "src\pipeline\playback.rs" @(
    "pub struct PlaybackRoutePlan",
    "PlaybackSinkTarget::VirtualMic",
    "fn play_to_audio_sinks"
)
Assert-FileContains "CODE-VirtualCableCI" "src\audio\vbcable_ci.rs" @(
    "run_memory_pcm_tier",
    "TierEvidence",
    "latency"
)
Assert-FileContains "CODE-VirtualCableProbe" "src\bin\vbcable_ci_probe.rs" @(
    "real_virtual_cable",
    "run_real_virtual_cable_tier"
)
Assert-FileContains "REPORT-A8" "verification-evidence\vmic\VMIC-A8-mvp-readiness-report.md" @(
    "GO for MVP release",
    "GO for production phase",
    "No human acceptance step is required",
    "OEM/DriverSink",
    "without modifying STT/MT/TTS orchestration"
)

$Failed = @($Checks | Where-Object { -not $_.passed })
$Status = if ($Failed.Count -eq 0) { "pass" } else { "fail" }
$Result = [pscustomobject]@{
    schema_version = 1
    issue = "#320"
    status = $Status
    checked_at = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    checks = $Checks
}

if ($Json) {
    $Result | ConvertTo-Json -Depth 6
} else {
    "VMIC MVP evidence check: $Status"
    $Checks | Format-Table id, passed, path, detail -AutoSize
}

if ($Failed.Count -gt 0) {
    exit 1
}
