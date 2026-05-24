# w0-r1 board-snapshot — commands and provenance

Tentacle: `w0-r1-board-snapshot`
Date: 2026-05-24
Operator: delegated Opus worker (autopilot)

## Raw artifact health (pre-existing files)

Inspected `verification-evidence/board-snapshot-raw/*.json`:

| File | Size (bytes) | Status |
| --- | ---: | --- |
| `jv-not-on-board-20260524.json` | 3588 | OK — JV-family issues not yet attached to Project #2 |
| `project2-items-20260524.json` | 232068 | OK — 329 items, no Status field |
| `project2-page1-20260524.json` | 108450 | OK — first GraphQL page, no Status field |

None of the pre-existing artifacts were zero-byte or 8-byte; no silent deletion or
replacement was required. However, the pre-existing pulls did **not** include
`fieldValues` (Project Status column), so the Status partitioning needed for the
snapshot was unobtainable from them. Fresh pulls were appended (not replacing)
to retain the original evidence.

## Fresh GraphQL pulls (with fieldValues / Status)

Performed via the documented fallback (`gh auth token` + PowerShell
`Invoke-RestMethod` → `https://api.github.com/graphql`) because `gh api graphql`
intermittently returns `unexpected EOF` for paged ProjectV2 queries on this host.

Query (paginated, 100 items per page):

```graphql
query($login:String!, $number:Int!, $after:String) {
  user(login:$login) {
    projectV2(number:$number) {
      items(first:100, after:$after) {
        pageInfo { hasNextPage endCursor }
        nodes {
          id
          type
          fieldValues(first:30) {
            nodes {
              __typename
              ... on ProjectV2ItemFieldSingleSelectValue {
                name
                field { ... on ProjectV2SingleSelectField { name } }
              }
            }
          }
          content {
            __typename
            ... on Issue { number title state url repository { nameWithOwner } labels(first:50) { nodes { name } } }
            ... on DraftIssue { title }
            ... on PullRequest { number title state url repository { nameWithOwner } }
          }
        }
      }
    }
  }
}
```

Variables: `{ login: "magicpro97", number: 2 }`.

| Page | After cursor | Nodes | hasNext | Output |
| --- | --- | ---: | --- | --- |
| 1 | (null) | 100 | true | `project2-fieldvalues-page1-20260524.json` (219,265 B) |
| 2 | `Y3Vyc29yOnYyOpKrMDAwMDAwMDAuMDHOCxbvYg==` | 100 | true | `project2-fieldvalues-page2-20260524.json` (231,842 B) |
| 3 | (from page 2) | 100 | true | `project2-fieldvalues-page3-20260524.json` (212,737 B) |
| 4 | (from page 3) | 29 | false | `project2-fieldvalues-page4-20260524.json` (61,189 B) |

Total items returned: 329 (matches `totalCount`).

PowerShell command shape (token never printed, only used in header):

```powershell
$token = gh auth token
$headers = @{ Authorization = "bearer $token"; "User-Agent" = "tui-translator-w0r1" }
$body = @{ query = $query; variables = @{ login = "magicpro97"; number = 2; after = $cursor } } |
    ConvertTo-Json -Depth 10 -Compress
$r = Invoke-RestMethod -Uri "https://api.github.com/graphql" -Method Post `
    -Headers $headers -Body $body -ContentType "application/json"
$r | ConvertTo-Json -Depth 20 | Out-File $outFile -Encoding utf8
```

Exit status: all four pulls returned HTTP 200 with non-empty `data.user.projectV2.items.nodes`.
No `errors` array was present in any response.

## Status partitioning

Computed from the four `fieldvalues-page*` files:

| Status | Count |
| --- | ---: |
| Done | 246 |
| Todo | 83 |
| (none) | 0 |
| **Total** | **329** |

Of the 83 Todo rows, **1 is CLOSED but mis-parked in the Todo column**:

- `#95 [P0] WP-15.02 — The CI job runs on windows-latest …` — state=CLOSED.

This issue is **excluded** from the snapshot (`todo_closed_excluded: 1` in the
counts block), leaving **82 open Todo rows**. A board-cleanup task should move
#95 to Done; the snapshot records the discrepancy under `notes` rather than
silently dropping it.

## Human-gated subset definition (10 rows)

Defined as the WP-19 acceptance-testing epic plus its atomic Layer-5 review and
scheduling sub-issues (all carry the `needs: human-reviewer` label and require a
named human to perform the work):

- #19 `[EPIC] WP-19 — Human Acceptance Testing on Real Hardware (Verification Layer 5)`
- #115 `WP-19.02 — Conduct L5-1 through L5-6 using real hardware`
- #116 `WP-19.03 — Run L5-1 subtitle accuracy review`
- #117 `WP-19.04 — Run L5-2 bilingual translation review`
- #118 `WP-19.05 — Validate L5-3 translated audio toggle`
- #119 `WP-19.06 — Verify L5-4 terminal compatibility matrix`
- #120 `WP-19.07 — Run L5-5 non-developer onboarding review`
- #121 `WP-19.08 — Run L5-6 live meeting readability review`
- #122 `WP-19.09 — Sign and date the acceptance log`
- #366 `verification: recruit and schedule named Layer 5 human reviewers`

The remaining 3 `needs: human-reviewer` items (#20, #21, #23) are
post-v1 epics that gate kickoff rather than per-iteration human action; they
remain in the AI-actionable 72 with `human_gate=false` but their child issues
will set it where appropriate in future iterations.

## Snapshot file

- Output: `verification-evidence/board-snapshot-20260524.json` (135,929 B)
- Schema fields per row: `number, title, labels, url, family, human_gate,`
  `confidence, acceptance, files_touched_hint, needs_human_clarification`
- Counts: `todo_open=82, ai_actionable=72, human_gated_subset=10`

Heuristic derivations (recorded in `notes`):

- `family` parsed from title prefix (`WP-XX`, `WP-XX.YY`, `JV-XX`, `VMIC-*`).
- `files_touched_hint` mapped from `area: *` / `provider: *` labels.
- `confidence` = `HIGH` for atomic with at least one area label,
  `MEDIUM` for epics, `LOW` otherwise.
- `acceptance` is a placeholder pointer to the issue body; full per-issue
  acceptance text should be ingested in a follow-up R2 pass.
- `needs_human_clarification` flips to `true` when the title was truncated
  (ends in `…`) or is under 25 characters.

No secrets, tokens, or PII appear in any emitted file. The `gh auth token`
value was passed only via the in-process `$headers` hashtable and never echoed.
