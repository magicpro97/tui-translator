#!/usr/bin/env pwsh
# W0-R3: Independent GraphQL ProjectV2 count reconciliation.
#
# Approach (intentionally distinct from W0-R2):
#   * Invoke-RestMethod against https://api.github.com/graphql (no gh api graphql).
#   * Single combined query per page that fetches BOTH the Status field value
#     AND the underlying Issue/PR content + labels in one round-trip.
#   * Pagination via pageInfo.endCursor until hasNextPage == false.
#   * Token sourced from `gh auth token`; only used in the Authorization header
#     and never echoed.
#
# Output: verification-evidence/board-count-graphql-20260524.json with
# total_project_items / todo_open / todo_closed_excluded / ai_actionable /
# human_gated_subset, an SHA-256 hash over the sorted Todo issue-number list,
# and a per-issue list suitable for diffing against the W0-R1 snapshot.

[CmdletBinding()]
param(
    [string] $Login   = 'magicpro97',
    [int]    $Number  = 2,
    [string] $RepoOwner = 'magicpro97',
    [string] $RepoName  = 'tui-translator',
    [string] $OutFile = (Join-Path $PSScriptRoot '..\board-count-graphql-20260524.json')
)

$ErrorActionPreference = 'Stop'

$humanGatedSet = @(19, 115, 116, 117, 118, 119, 120, 121, 122, 366)

$token = (& gh auth token).Trim()
if ([string]::IsNullOrWhiteSpace($token)) {
    throw 'gh auth token returned empty; cannot authenticate.'
}

$headers = @{
    Authorization = "bearer $token"
    'User-Agent'  = 'tui-translator-w0r3'
    Accept        = 'application/vnd.github+json'
}

$query = @'
query($login:String!, $number:Int!, $after:String) {
  user(login:$login) {
    projectV2(number:$number) {
      title
      items(first:100, after:$after) {
        pageInfo { hasNextPage endCursor }
        totalCount
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
            ... on Issue {
              number
              title
              state
              url
              repository { nameWithOwner }
              labels(first:50) { nodes { name } }
            }
            ... on PullRequest {
              number
              title
              state
              url
              repository { nameWithOwner }
            }
            ... on DraftIssue { title }
          }
        }
      }
    }
  }
}
'@

$allNodes = New-Object System.Collections.Generic.List[object]
$cursor   = $null
$pages    = 0
$totalCount = $null

do {
    $pages++
    $vars = @{ login = $Login; number = $Number; after = $cursor }
    $body = @{ query = $query; variables = $vars } | ConvertTo-Json -Depth 12 -Compress
    $resp = Invoke-RestMethod -Uri 'https://api.github.com/graphql' -Method Post `
        -Headers $headers -Body $body -ContentType 'application/json'
    if ($resp.errors) {
        throw "GraphQL errors on page $pages : $($resp.errors | ConvertTo-Json -Depth 6 -Compress)"
    }
    $items = $resp.data.user.projectV2.items
    if ($null -eq $totalCount) { $totalCount = $items.totalCount }
    foreach ($n in $items.nodes) { $allNodes.Add($n) }
    $cursor = $items.pageInfo.endCursor
    $hasNext = [bool]$items.pageInfo.hasNextPage
} while ($hasNext)

function Get-StatusValue {
    param($node)
    foreach ($fv in $node.fieldValues.nodes) {
        if ($fv.__typename -eq 'ProjectV2ItemFieldSingleSelectValue' -and $fv.field.name -eq 'Status') {
            return $fv.name
        }
    }
    return $null
}

$todoOpen     = New-Object System.Collections.Generic.List[object]
$todoClosed   = New-Object System.Collections.Generic.List[object]
$doneCount    = 0
$noneCount    = 0
$nonIssueTodo = New-Object System.Collections.Generic.List[object]

foreach ($n in $allNodes) {
    $status = Get-StatusValue -node $n
    switch ($status) {
        'Todo' {
            $c = $n.content
            if ($null -eq $c) { continue }
            if ($c.__typename -eq 'Issue') {
                if ($c.state -eq 'OPEN') {
                    $todoOpen.Add([pscustomobject]@{
                        number = [int]$c.number
                        title  = $c.title
                        url    = $c.url
                        labels = @($c.labels.nodes | ForEach-Object { $_.name })
                    })
                } else {
                    $todoClosed.Add([pscustomobject]@{
                        number = [int]$c.number
                        title  = $c.title
                        state  = $c.state
                        url    = $c.url
                    })
                }
            } else {
                $nonIssueTodo.Add($c)
            }
        }
        'Done'    { $doneCount++ }
        default   { $noneCount++ }
    }
}

$todoOpenSorted = $todoOpen | Sort-Object number
$todoNumbers = @($todoOpenSorted | ForEach-Object { $_.number })

$humanGated = @($todoNumbers | Where-Object { $humanGatedSet -contains $_ }) | Sort-Object
$aiActionable = @($todoNumbers | Where-Object { -not ($humanGatedSet -contains $_) }) | Sort-Object

# SHA-256 hash over the canonical sorted Todo issue-number list (newline joined).
$hashInput = ($todoNumbers -join "`n")
$sha = [System.Security.Cryptography.SHA256]::Create()
$bytes = [System.Text.Encoding]::UTF8.GetBytes($hashInput)
$hashHex = ([BitConverter]::ToString($sha.ComputeHash($bytes))) -replace '-', ''
$sha.Dispose()

$result = [pscustomobject]@{
    schema_version = 1
    generated_at   = (Get-Date).ToUniversalTime().ToString('o')
    source = [pscustomobject]@{
        method      = 'Invoke-RestMethod -> https://api.github.com/graphql'
        project     = "user:$Login projectV2 number:$Number"
        repository  = "$RepoOwner/$RepoName"
        pages       = $pages
        independent_of = 'w0-r2 (no gh api graphql, no shared script)'
    }
    counts = [pscustomobject]@{
        total_project_items   = $totalCount
        status_done           = $doneCount
        status_todo_open      = $todoOpen.Count
        todo_closed_excluded  = $todoClosed.Count
        status_none           = $noneCount
        ai_actionable         = $aiActionable.Count
        human_gated_subset    = $humanGated.Count
    }
    expected = [pscustomobject]@{
        total = 82
        ai_actionable = 72
        human_gated_subset = 10
        status = 'Todo'
    }
    reconciliation = [pscustomobject]@{
        total_matches_expected         = ($todoOpen.Count -eq 82)
        ai_actionable_matches_expected = ($aiActionable.Count -eq 72)
        human_gated_matches_expected   = ($humanGated.Count -eq 10)
        todo_closed_excluded_matches   = ($todoClosed.Count -eq 1)
    }
    todo_open_issue_numbers      = $todoNumbers
    todo_open_issue_numbers_sha256 = $hashHex
    human_gated_numbers          = $humanGated
    ai_actionable_numbers        = $aiActionable
    todo_closed_excluded         = $todoClosed
    non_issue_todo               = $nonIssueTodo
}

$result | ConvertTo-Json -Depth 10 | Out-File -FilePath $OutFile -Encoding utf8
Write-Host "Wrote $OutFile"
Write-Host ("Pages: {0}  TotalItems: {1}  Done: {2}  TodoOpen: {3}  TodoClosed: {4}  None: {5}" -f `
    $pages, $totalCount, $doneCount, $todoOpen.Count, $todoClosed.Count, $noneCount)
Write-Host ("ai_actionable={0}  human_gated={1}  sha256={2}" -f $aiActionable.Count, $humanGated.Count, $hashHex)
