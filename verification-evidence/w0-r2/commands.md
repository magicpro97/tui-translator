# w0-r2 board-count-rest — commands and provenance

Tentacle: `w0-r2-count-rest`
Date: 2026-05-24
Operator: delegated Opus worker (autopilot)
Independence goal: re-derive Project #2 Todo counts via a path other than the
hand-rolled GraphQL pulls used in W0-R1 and (anticipated) W0-R3.

## 1. Primary attempted path — `gh project item-list` (failed)

```powershell
gh project item-list 2 --owner magicpro97 --limit 500 --format json `
    --query "-status:Done"
# stderr: Post "https://api.github.com/graphql": unexpected EOF
# exit code: 1
```

Same command retried 4× with `--query "-status:Done"` and 4× without; further
retries at `--limit 50/100/200/500` (no query) all failed with the identical
error and exit code 1.

| Limit | Query filter | Result | stderr |
| ---: | --- | --- | --- |
| 500 | `-status:Done` | FAIL exit=1 | `unexpected EOF` |
| 500 | (none) | FAIL exit=1 | `unexpected EOF` |
| 200 | (none) | FAIL exit=1 | `unexpected EOF` |
| 100 | (none) | FAIL exit=1 | `unexpected EOF` |
| 50  | (none) | FAIL exit=1 | `unexpected EOF` |

Root cause: `gh project item-list` is implemented on top of
`api.github.com/graphql` under the hood, so it reproduces the same EOF defect
that W0-R1 documented for the raw `gh api graphql` path on this host. GitHub
provides no Projects v2 REST endpoint, so the gh CLI has no non-GraphQL
implementation to fall back to. The CLI path therefore cannot serve as the
independent reconciliation for this tentacle.

## 2. Fallback path — Issues REST API (succeeded)

Rationale: ProjectsV2 itself has no REST surface, but the underlying Issues
REST API can independently answer the questions that matter for reconciling
W0-R1's counts:

1. Are all 82 snapshot rows real OPEN issues in `magicpro97/tui-translator`?
2. Is #95 (the excluded mis-parked row) genuinely closed?
3. Are the 10 human-gated rows actually labelled `needs: human-reviewer`, and
   is the wider population of that label consistent with W0-R1's documented
   exclusions?

Repository nameWithOwner was derived from the `url` field on snapshot rows
(`https://github.com/magicpro97/tui-translator/issues/N`) — note the snapshot
is for `magicpro97/tui-translator`, not the local working-tree directory name
`zoom-terminal-translator-rs`.

### 2a. All open issues in the repo

```powershell
foreach ($state in 'open','closed') {
  $page = 1
  do {
    gh api "repos/magicpro97/tui-translator/issues?state=$state&per_page=100&page=$page" `
       --jq '[.[] | select(.pull_request | not) | .number]'
    # ... accumulate, stop when page returns <100 items
  } while ($page -lt 30)
}
```

Results:

| State  | Pages walked | Issue rows (PRs excluded) |
| ------ | ---: | ---: |
| open   | 1 | 99 |
| closed | 1 | 55 |

Files emitted:

- `verification-evidence/w0-r2/rest-open-issue-numbers-20260524.txt`
- `verification-evidence/w0-r2/rest-closed-issue-numbers-20260524.txt`

### 2b. Cross-check the 82 snapshot rows

```powershell
$snap = Get-Content verification-evidence/board-snapshot-20260524.json -Raw | ConvertFrom-Json
$snapNums   = $snap.rows | ForEach-Object { [int]$_.number }
$openNums   = Get-Content verification-evidence/w0-r2/rest-open-issue-numbers-20260524.txt   | ForEach-Object { [int]$_ }
$closedNums = Get-Content verification-evidence/w0-r2/rest-closed-issue-numbers-20260524.txt | ForEach-Object { [int]$_ }
```

| Check | Expected | Observed |
| --- | ---: | ---: |
| snap rows count | 82 | 82 |
| snap rows ∈ REST open set | 82 | 82 |
| snap rows ∈ REST closed set | 0 | 0 |
| REST open NOT in snap | 17 | 17 (unclassifiable via REST — see Limitations) |

### 2c. Confirm #95 closed mis-park

```powershell
gh api repos/magicpro97/tui-translator/issues/95 --jq '{number,state,title}'
# {
#   "number": 95,
#   "state": "closed",
#   "title": "[P0] WP-15.02 — The CI job runs on windows-latest …"
# }
```

Independently corroborates W0-R1's exclusion of #95 from the open Todo count.

### 2d. Human-gated subset cross-check

```powershell
gh api "repos/magicpro97/tui-translator/issues?state=open&labels=needs:%20human-reviewer&per_page=100" `
    --jq '[.[] | select(.pull_request | not) | {number,title,state,labels:[.labels[].name]}]'
```

→ `verification-evidence/w0-r2/rest-needs-human-reviewer-20260524.json` (14 rows).

| Set | Numbers |
| --- | --- |
| Snapshot `human_gate=true` (10) | 19, 115, 116, 117, 118, 119, 120, 121, 122, 366 |
| REST `needs: human-reviewer` open (14) | 19, 20, 21, 23, 115, 116, 117, 118, 119, 120, 121, 122, 366, 429 |
| In snap subset but NOT REST-labelled | (none) |
| REST-labelled but NOT in snap subset | 20, 21, 23, 429 |

The 4 REST-extras are accounted for:

- #20, #21, #23 — Post-v1 EPICs (WP-20 gRPC STT, WP-21 per-process audio,
  WP-23 Ollama). Present in the snapshot with `human_gate=false`, per W0-R1's
  documented rationale that these epics gate kickoff rather than per-iteration
  human action.
- #429 — JV-21 release-candidate gate; verified NOT present in the snapshot
  (i.e., not on Project #2 Todo column at all).

## 3. Reconciled counts (REST-corroborated)

| Quantity | W0-R1 snapshot | This REST reconciliation | Match? |
| --- | ---: | ---: | --- |
| `todo_open` (open issues on Project Todo column) | 82 | 82 | ✅ |
| `ai_actionable` | 72 | 72 (82 − 10) | ✅ |
| `human_gated_subset` | 10 | 10 (all ⊂ REST `needs: human-reviewer` open set) | ✅ |
| `todo_closed_excluded` (mis-parked #95) | 1 | 1 (REST confirms CLOSED) | ✅ |

## 4. Limitations (non-negotiable)

- GitHub provides **no REST API for Projects v2** items or column/Status
  membership. This tentacle therefore cannot, by design, independently
  re-derive *which* open repo issues sit on Project #2 Todo column. The 17
  REST open issues missing from the snapshot remain unclassified by this
  path; only the GraphQL route (W0-R3) can reach them.
- The REST cross-check proves the snapshot's row set is real, open, and
  free of closed-issue contamination, and that the human-gated subset is a
  documented subset of REST-confirmed `needs: human-reviewer` issues. It does
  **not** re-prove project membership.
- `gh project item-list` failed deterministically (4/4 limits tested) with
  `unexpected EOF` on this host. No transient retry recovered.

## 5. No source files modified

This tentacle modified only:

- `verification-evidence/board-count-rest-20260524.json` (new)
- `verification-evidence/w0-r2/commands.md` (this file)
- `verification-evidence/w0-r2/rest-open-issue-numbers-20260524.txt`
- `verification-evidence/w0-r2/rest-closed-issue-numbers-20260524.txt`
- `verification-evidence/w0-r2/rest-needs-human-reviewer-20260524.json`

No `src/**`, docs, Cargo files, GitHub issues, or PR comments were touched.
