# w0-r3 board-count-graphql — commands and provenance

Tentacle: `w0-r3-count-graphql`
Date: 2026-05-24
Operator: delegated Opus worker (autopilot)

## Objective

Independent GraphQL ProjectV2 reconciliation of the W0-R1 Todo-column snapshot.
This run must not share code path with W0-R2; the only inputs reused are the
W0-R1 expected counts (82 / 72 / 10) and the human-gated issue set definition.

## Method

PowerShell script `verification-evidence/w0-r3/fetch-board-graphql.ps1`:

* Auth: `gh auth token` is captured into an in-process variable and only sent
  in the `Authorization: bearer …` header. The token is never echoed, written
  to disk, or passed on the command line.
* Transport: `Invoke-RestMethod -Uri https://api.github.com/graphql -Method Post`.
  `gh api graphql` was avoided per the W0-R1 EOF caveat and to keep the code
  path independent from W0-R2.
* Query: single combined GraphQL document per page that pulls both the
  `Status` single-select field value and the `Issue/PullRequest/DraftIssue`
  content (number / title / state / url / labels) in one round-trip.
* Pagination: `items(first:100, after:$after)` driven by
  `pageInfo.hasNextPage` / `pageInfo.endCursor`.
* Project: `user(login:"magicpro97").projectV2(number:2)`
  (= `magicpro97/tui-translator` project board #2).

## Pagination receipts

| Page | Nodes | hasNextPage |
| ---: | ---: | --- |
| 1 | 100 | true |
| 2 | 100 | true |
| 3 | 100 | true |
| 4 | 29  | false |
| **Total** | **329** | — |

`totalCount` reported by GraphQL = 329 (matches the sum of the four pages).

## Status partitioning (computed locally from the GraphQL response)

| Status | Count |
| --- | ---: |
| Done | 246 |
| Todo (issues, state = OPEN) | 82 |
| Todo (issues, state = CLOSED, excluded) | 1 |
| (none / other) | 0 |
| **Total** | **329** |

The single CLOSED-but-Todo row is issue **#95**, matching the W0-R1 finding
verbatim. It is captured under `todo_closed_excluded` in
`board-count-graphql-20260524.json` and is **not** counted toward the 82.

## Reconciliation against W0-R1

| Expected (W0-R1) | Observed (W0-R3) | Match |
| --- | --- | --- |
| `total = 82` (open Todo) | 82 | ✅ |
| `ai_actionable = 72` | 72 | ✅ |
| `human_gated_subset = 10` (#19,115–122,366) | 10 (#19,115–122,366) | ✅ |
| `todo_closed_excluded = 1` (#95) | 1 (#95) | ✅ |
| Status field = `Todo` | `Todo` | ✅ |

## Hash / issue-list reconciliation

* Canonical form: ascending integer list of open Todo issue numbers, joined
  by `\n` (LF), UTF-8 bytes, SHA-256.
* W0-R1 snapshot rows hashed in the same canonical form.
* W0-R3 result: `3ACD5BFD9051F2E9E01AA9B7F991D26F69E05E9D45B299A261F297C1FA6ED1BC`
* W0-R1 snapshot: `3ACD5BFD9051F2E9E01AA9B7F991D26F69E05E9D45B299A261F297C1FA6ED1BC`
* Set diff: `only_in_w0_r1 = []`, `only_in_w0_r3 = []`.

Hashes are byte-identical; the 82-issue set is identical between the two
independent code paths.

## PowerShell command shape (token redacted)

```powershell
$token   = (gh auth token).Trim()
$headers = @{ Authorization = "bearer $token"; 'User-Agent' = 'tui-translator-w0r3' }
$body    = @{ query = $query; variables = @{ login = 'magicpro97'; number = 2; after = $cursor } } |
    ConvertTo-Json -Depth 12 -Compress
$resp = Invoke-RestMethod -Uri 'https://api.github.com/graphql' -Method Post `
    -Headers $headers -Body $body -ContentType 'application/json'
```

No `errors` array was present on any of the four responses.

## Output files

* `verification-evidence/w0-r3/fetch-board-graphql.ps1` — independent fetch script.
* `verification-evidence/w0-r3/commands.md` — this provenance log.
* `verification-evidence/board-count-graphql-20260524.json` — reconciliation
  artifact (counts, expected, reconciliation flags, todo issue numbers,
  SHA-256 hash, human-gated / ai-actionable splits, closed-excluded record,
  W0-R1 cross-check block).

No `src/**`, docs, Cargo, GitHub-issue, or GitHub-comment files were modified.
No secrets are present in any emitted file.
