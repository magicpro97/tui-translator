# Wave W1 — Baseline hashes generation method

This document records the exact method used to populate
`verification-evidence/waves/wave-1/baseline-hashes.json` at wave start,
before any T0 implementation tentacle was dispatched.

## Environment

- Repository: `zoom-terminal-translator-rs`
- Host OS: Windows 10/11 (PowerShell 5.1+)
- Working directory: repository root
- Branch: `main`
- Generator role: Opus state/evidence agent (orchestrator delegated; no
  substantive implementation work performed in this step)

## Inputs

- `verification-evidence/waves/wave-1/files_allowed.txt` — closed allow-list
  for Wave W1 (lines starting with `#` and blank lines are ignored).
- `git rev-parse HEAD` on `main` at wave start.
- Current UTC timestamp.

## Procedure

1. Capture the current commit and timestamp:

   ```powershell
   $head = (git rev-parse HEAD).Trim()
   $ts   = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
   ```

2. Parse the allow-list, skipping comments and blanks. Convert forward
   slashes to backslashes for Windows path probing:

   ```powershell
   $lines = Get-Content verification-evidence\waves\wave-1\files_allowed.txt
   foreach ($l in $lines) {
     $t = $l.Trim()
     if ($t -eq '' -or $t.StartsWith('#')) { continue }
     $winPath = $t -replace '/','\'
     # ... probe + hash, see step 3 ...
   }
   ```

3. For each allow-list entry, probe the worktree and record either the
   SHA-256 of the file or an explicit `exists_pre=false` placeholder:

   ```powershell
   if (Test-Path -LiteralPath $winPath -PathType Leaf) {
     $h = (Get-FileHash -LiteralPath $winPath -Algorithm SHA256).Hash.ToLower()
     # -> { path, exists_pre=true,  sha256_pre=$h }
   } else {
     # -> { path, exists_pre=false, sha256_pre=null, note="absent at wave start; to be created by implementation tentacle" }
   }
   ```

   Hash algorithm: SHA-256, lowercase hex, no separators. Matches
   `sha256sum` / `Get-FileHash -Algorithm SHA256` output (lowercased).

4. Assemble the JSON object with the required schema fields and write it
   to `verification-evidence/waves/wave-1/baseline-hashes.json` (UTF-8):

   ```powershell
   $obj | ConvertTo-Json -Depth 6 |
     Set-Content -LiteralPath verification-evidence\waves\wave-1\baseline-hashes.json -Encoding UTF8
   ```

## Schema fields written

- `schema_version` — `1`
- `wave` — `"W1"`
- `note` — human description of pre/post semantics
- `generated_at` — UTC ISO-8601 timestamp at wave start
- `main_head_sha` — output of `git rev-parse HEAD`
- `files_allowed_source` — `"waves/wave-1/files_allowed.txt"` (unchanged)
- `t0_authorised_issues` — issues authorised by
  `verification-evidence/waves/wave-1/final-dispatch-authorization.md`
  for the T0 cohort
- `deferred_issues` — `[384]` (deferred per final authorisation; still on
  the allow-list, not removed)
- `files[]` — one entry per non-comment allow-list line, in the order they
  appear in `files_allowed.txt`, with:
  - `path` (forward-slash, as written in the allow-list)
  - `exists_pre` (bool)
  - `sha256_pre` (lowercase hex; `null` when `exists_pre=false`)
  - `note` (only on missing entries)

## Reproduction (single block)

```powershell
cd C:\Users\linhnt102\zoom-terminal-translator-rs
$lines = Get-Content verification-evidence\waves\wave-1\files_allowed.txt
$files = @()
foreach ($l in $lines) {
  $t = $l.Trim()
  if ($t -eq '' -or $t.StartsWith('#')) { continue }
  $winPath = $t -replace '/','\'
  if (Test-Path -LiteralPath $winPath -PathType Leaf) {
    $h = (Get-FileHash -LiteralPath $winPath -Algorithm SHA256).Hash.ToLower()
    $files += [ordered]@{ path=$t; exists_pre=$true; sha256_pre=$h }
  } else {
    $files += [ordered]@{ path=$t; exists_pre=$false; sha256_pre=$null; note="absent at wave start; to be created by implementation tentacle" }
  }
}
$head = (git rev-parse HEAD).Trim()
$ts   = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
$obj = [ordered]@{
  schema_version = 1
  wave = "W1"
  note = "Pre-implementation baseline. Populated by Opus state agent at wave start..."
  generated_at = $ts
  main_head_sha = $head
  files_allowed_source = "waves/wave-1/files_allowed.txt"
  t0_authorised_issues = @(450,459,460,461,468,474,476,486,499,500,509,510)
  deferred_issues = @(384)
  files = $files
}
$obj | ConvertTo-Json -Depth 6 |
  Set-Content -LiteralPath verification-evidence\waves\wave-1\baseline-hashes.json -Encoding UTF8
```

## Verification

To re-verify a single existing file's recorded hash:

```powershell
Get-FileHash -LiteralPath src\metrics\snapshot.rs -Algorithm SHA256
```

To verify the recorded `main_head_sha` matches the worktree:

```powershell
git rev-parse HEAD
```

## Secrets

No secrets, tokens, or credentials are used or recorded by this procedure.
All inputs are public repository state and local file hashes.

## Anomalies / notes

- The allow-list contains 32 file entries; 10 exist on `main` at wave
  start, 22 are absent and are expected to be created by authorised T0/T1
  implementation tentacles.
- Issue **#384** is deferred per the final dispatch authorisation. Its
  associated paths remain on `files_allowed.txt` and therefore in
  `files[]` (we do not mutate the allow-list); `deferred_issues` is
  recorded as a soft marker so the wave-close diff gate can distinguish
  "absent because deferred" from "absent and never created".
- `baseline-hashes.json` and `baseline-hashes-commands.md` themselves
  live under `verification-evidence/waves/wave-1/` and are **not** on the
  Wave W1 implementation allow-list. This is intentional: they are
  state/evidence artifacts owned by the orchestrator (Opus) layer, not
  by implementation tentacles, and must not be edited inside an
  implementation issue's diff.
