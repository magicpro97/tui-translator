#!/usr/bin/env pwsh
<#
.SYNOPSIS
    I18N-01 (issue #481) missing-key gate for tui-translator locales.

.DESCRIPTION
    Cross-platform PowerShell script that fails CI when:
      1. A `crate::i18n::t("…")` or `crate::i18n::t_arg("…", …)` call
         references a key that is missing from `locales/en-US.ftl`.
      2. A locale shipped in `locales/*.ftl` is missing any key that
         exists in `locales/en-US.ftl`.

    The script intentionally runs *before* the Rust compiler so missing
    keys are caught even when the surrounding code would still compile
    (Fluent lookups happen at runtime).

    Exit codes:
      0  All catalogs are complete.
      1  One or more keys are missing.
      2  Repository layout is invalid (no locales/ or en-US.ftl).
#>

[CmdletBinding()]
param(
    [string]$Root = (Split-Path -Parent $PSScriptRoot)
)

$ErrorActionPreference = 'Stop'

$localesDir = Join-Path $Root 'locales'
$enPath = Join-Path $localesDir 'en-US.ftl'

if (-not (Test-Path $localesDir) -or -not (Test-Path $enPath)) {
    Write-Error "i18n-check: repository layout missing locales/en-US.ftl at $enPath"
    exit 2
}

function Get-FluentKeys {
    param([string]$Path)
    Get-Content -LiteralPath $Path -Encoding UTF8 |
        Where-Object { $_ -match '^[a-zA-Z][a-zA-Z0-9_-]*\s*=' } |
        ForEach-Object {
            ($_ -split '=', 2)[0].Trim()
        } |
        Sort-Object -Unique
}

$enKeys = Get-FluentKeys -Path $enPath
$enSet = [System.Collections.Generic.HashSet[string]]::new([string[]]$enKeys, [System.StringComparer]::Ordinal)

# 1) Verify every t(...) / t_arg(...) reference resolves in en-US.
$callRegex = [System.Text.RegularExpressions.Regex]::new(
    'crate::i18n::t(_arg|_args)?\(\s*"([a-zA-Z][a-zA-Z0-9_-]*)"'
)

$missingFromEn = [System.Collections.Generic.List[string]]::new()
$rustFiles = Get-ChildItem -LiteralPath (Join-Path $Root 'src') -Recurse -Filter *.rs -ErrorAction SilentlyContinue
foreach ($file in $rustFiles) {
    $text = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8
    foreach ($m in $callRegex.Matches($text)) {
        $key = $m.Groups[2].Value
        if (-not $enSet.Contains($key)) {
            $missingFromEn.Add("$($file.FullName): key '$key' not in locales/en-US.ftl")
        }
    }
}

# 2) Verify every shipped catalog has every en-US key.
$missingPerLocale = [System.Collections.Generic.List[string]]::new()
$otherCatalogs = Get-ChildItem -LiteralPath $localesDir -Filter '*.ftl' |
    Where-Object { $_.Name -ne 'en-US.ftl' }

foreach ($cat in $otherCatalogs) {
    $catKeys = Get-FluentKeys -Path $cat.FullName
    $catSet = [System.Collections.Generic.HashSet[string]]::new([string[]]$catKeys, [System.StringComparer]::Ordinal)
    foreach ($k in $enKeys) {
        if (-not $catSet.Contains($k)) {
            $missingPerLocale.Add("$($cat.Name): missing key '$k'")
        }
    }
}

$fail = $false
if ($missingFromEn.Count -gt 0) {
    Write-Host "::error::I18N-01 missing-key check: keys referenced from Rust but absent in en-US:" -ForegroundColor Red
    $missingFromEn | ForEach-Object { Write-Host "  $_" -ForegroundColor Red }
    $fail = $true
}
if ($missingPerLocale.Count -gt 0) {
    Write-Host "::error::I18N-01 missing-key check: shipped catalogs missing en-US keys:" -ForegroundColor Red
    $missingPerLocale | ForEach-Object { Write-Host "  $_" -ForegroundColor Red }
    $fail = $true
}

if ($fail) {
    exit 1
}

Write-Host "i18n-check: $($enKeys.Count) keys in en-US, $($otherCatalogs.Count) other catalog(s) verified." -ForegroundColor Green
exit 0
