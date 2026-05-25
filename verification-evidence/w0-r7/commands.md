# w0-r7-jv-overlap — commands log

Read-only research tentacle. No `src/**`, docs, Cargo files, GitHub issues,
or issue comments were modified.

## Inputs consumed

- `verification-evidence/board-snapshot-20260524.json`
  SHA256: `290FB421CDAC960618640169CD36876621ECA9F8E328AF876C3280B7EC19949E`
- `verification-evidence/family-file-map.json`
  SHA256: `4E961DC0282E9E0810505020BDAC45F7B7BF57492AB4D67B7F089A7B0FA9182B`
- GitHub issues #408, #413, #414, #415, #416, #417, #419, #420, #421, #422,
  #423, #424, #425, #426, #427, #428, #429 (17 JV issues) — fetched read-only
  via `gh api repos/magicpro97/tui-translator/issues/<n>`.

## Commands

```powershell
# Determine repo (gh GraphQL was intermittently failing, so audit uses REST only):
gh repo view --json nameWithOwner

# Fetch each JV issue body read-only via REST API; cache JSON under w0-r7/:
New-Item -ItemType Directory -Force verification-evidence\w0-r7
$issues = 408,413,414,415,416,417,419,420,421,422,423,424,425,426,427,428,429
foreach($i in $issues){
  $out = "verification-evidence\w0-r7\issue-$i.json"
  if(-not (Test-Path $out)){
    gh api repos/magicpro97/tui-translator/issues/$i | Out-File -Encoding utf8 $out
  }
}

# Hash the inputs to lock the audit baseline:
Get-FileHash verification-evidence\board-snapshot-20260524.json -Algorithm SHA256
Get-FileHash verification-evidence\family-file-map.json -Algorithm SHA256

# Extract the union of concrete repo paths mentioned in each JV body:
foreach($i in $issues){
  $j = Get-Content "verification-evidence\w0-r7\issue-$i.json" -Raw | ConvertFrom-Json
  $body = $j.body
  $matches = [regex]::Matches($body, '(?<![A-Za-z0-9_./-])(src/[A-Za-z0-9_/.\-*]+|Cargo\.toml|Cargo\.lock|\.github/workflows/[A-Za-z0-9_.\-]+\.ya?ml|docs/[A-Za-z0-9_/.\-]+\.md|README\.md|config\.example\.json|config\.json|models/[A-Za-z0-9_/.\-]+|verification-evidence/[A-Za-z0-9_/.\-*]+|tests/[A-Za-z0-9_/.\-]+|benches/[A-Za-z0-9_/.\-]+|\.gitignore|\.github/[A-Za-z0-9_/.\-]+)')
  $set = @{}; foreach($m in $matches){$set[$m.Value]=1}
  Write-Output ("=== #"+$i+": "+$j.title)
  foreach($k in ($set.Keys | Sort-Object)){ Write-Output "  $k" }
}
```

## Method

1. Each of the 17 JV issue bodies was fetched read-only and stored under
   `verification-evidence/w0-r7/issue-<n>.json`.
2. The set of concrete repo paths mentioned in each body was extracted by
   regex (src/**, Cargo.*, .github/workflows/*.yml, docs/**, README.md,
   packaging/**, config*.json, verification-evidence/**).
3. Where the body listed only the JV-only benchmark binary
   (`src/bin/mt_bench.rs`) but the title strongly implied additional surfaces
   (JV-11 → src/providers/local/mt.rs + src/providers/mt/routing.rs; JV-14
   → src/tui/mod.rs + src/tui/frame_pacer.rs; JV-18 → packaging/windows/
   tui-translator.iss; JV-19 → .github/workflows/release.yml), the implied
   files were recorded in `files_implied_by_title_and_body` and treated as
   collision candidates. This is conservative — it predicts more collisions
   than the body text alone.
4. The union touch surface was intersected with the W0-R5
   `concrete_src_ownership`, `shared_within_family`, and `shared_paths`
   ladders from `verification-evidence/family-file-map.json`.
5. A path is BLOCKING if (a) an in-scope issue claims it in
   `concrete_src_ownership` or (b) it is in a serialized writers list and JV
   is not in that list. A path is ADDITIVE-SAFE if the in-scope merge rule
   explicitly accommodates additive writers (Cargo.toml). A path is UNOWNED
   if no in-scope issue claims it (src/main.rs, src/bin/mt_bench.rs,
   config.example.json, docs/evidence/dm-07-fps.md).

## Findings (summary)

- `overlap = true`. 9 BLOCKING overlaps across src/config/mod.rs,
  src/providers/{mod.rs,local,mt,google}, .github/workflows/release.yml,
  packaging/windows/tui-translator.iss, src/tui/{mod.rs,frame_pacer.rs}.
- 6 NON-BLOCKING overlaps (Cargo.toml additive, src/main.rs unowned,
  src/bin/mt_bench.rs JV-only, config.example.json unowned,
  docs/evidence/dm-07-fps.md additive, docs/** additive).
- R6 recommendation: Option A — keep JV deferred from R6 implementation
  waves. JV is OUT OF BOARD SCOPE today (not in the 82-row W0-R1 snapshot).
  Admitting JV mid-cycle requires a human board-hygiene decision (add to
  project #2 then re-run W0-R1 and W0-R5).

## Output

- `verification-evidence/jv-overlap-audit.json` — R7 audit artifact
  (schema_version 1, overlap=true, 9 BLOCKING + 6 NON-BLOCKING entries,
  readiness=true with caveats).
- `verification-evidence/w0-r7/issue-<n>.json` — cached read-only JV issue
  bodies (×17).
- `verification-evidence/w0-r7/commands.md` — this file.

## Outputs NOT produced

- No edits to `src/**`, `docs/**`, `Cargo.toml`, `Cargo.lock`, GitHub issue
  bodies, issue labels, comments, or project state.
- No edits to `verification-evidence/board-snapshot-20260524.json` — the
  W0-R7 audit produces a standalone artifact (`jv-overlap-audit.json`); no
  factual W0-R7 note had to be threaded into the snapshot.
- No edits to `verification-evidence/family-file-map.json`.
