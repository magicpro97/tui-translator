# Wave 1 — Issue Research Commands and Evidence Transcript

Repo confirmed: `magicpro97/tui-translator` (via `gh repo view --json nameWithOwner`
and `git remote -v` — origin is `ssh://git@ssh.github.com:443/magicpro97/tui-translator.git`).

## Commands attempted

### 1. `gh issue view` (FAILED — GraphQL EOF)

```
foreach $i in 384,450,459,460,461,468,474,476,486,499,500,501,502,503,505,506,509,510:
    gh issue view $i --repo magicpro97/tui-translator --json number,title,url,state,labels,milestone,body
# Result for every issue:
# Post "https://api.github.com/graphql": unexpected EOF
```

All 18 calls failed with the same GraphQL EOF error. Fell back to REST.

### 2. GitHub REST via PowerShell `Invoke-RestMethod` (SUCCESS)

Token obtained in-memory only (`$token = gh auth token`); never printed or persisted.

```powershell
$token = gh auth token
$headers = @{
    Authorization = "Bearer $token"
    "Accept"      = "application/vnd.github+json"
    "User-Agent"  = "wave1-research"
}
$issues = @(384,450,459,460,461,468,474,476,486,499,500,501,502,503,505,506,509,510)
$all = @()
foreach ($i in $issues) {
    $r = Invoke-RestMethod -Uri "https://api.github.com/repos/magicpro97/tui-translator/issues/$i" -Headers $headers -Method GET
    $all += [pscustomobject]@{
        number    = $r.number
        title     = $r.title
        url       = $r.html_url
        state     = $r.state
        labels    = ($r.labels | ForEach-Object { $_.name })
        milestone = $r.milestone.title
        body      = $r.body
    }
}
$all | ConvertTo-Json -Depth 10 |
    Out-File -Encoding UTF8 verification-evidence\waves\wave-1\_issues-raw.json
```

Output (per-issue trace):

```
OK 384
OK 450
OK 459
OK 460
OK 461
OK 468
OK 474
OK 476
OK 486
OK 499
OK 500
OK 501
OK 502
OK 503
OK 505
OK 506
OK 509
OK 510
Saved
Count: 18
```

Saved raw JSON: `verification-evidence/waves/wave-1/_issues-raw.json` (~32 KB, 18 records).

## Inputs cross-referenced

- `verification-evidence/wave-plan.json` (full plan — only read for Wave-1 entries
  via grep; file is 118 KB so not inlined).
- `verification-evidence/waves/wave-1/wave-manifest.json` (read in full;
  per-issue `files_allowed` arrays used as the source of truth for the matrix).
- `verification-evidence/waves/wave-1/files_allowed.txt` (closed allow-list; the
  header confirms that `tests/**` and `benches/**` are implicitly allowed for
  `tests_first` red-mode issues).

## Notes

- All 18 issues are currently `state=open`, `milestone=null`.
- Every issue body has the structured sections used by Opus WBS (Context, Inputs,
  Outputs, Test cases, Acceptance criteria, Dependencies, Opus review gate).
  Quotation in `acceptance-matrix.md` is taken verbatim from those sections.
- No secrets were printed or written. Token only existed in the `$token` variable
  inside the PowerShell session.
