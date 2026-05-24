# Evidence — Issue #509 — QA8-11 issue-hygiene workflow

## Wave-1 T0 gate
- `workflow_dry_run` red mode (per acceptance-matrix.md row #509).
- Required: actionlint pass + successful `workflow_dispatch` run URL.

## Local validation log

### RED skeleton (workflow_dispatch only, single noop job)
```
$ actionlint .github/workflows/issue-hygiene.yml
$ echo EXIT=$?
EXIT=0
```
RED skeleton lint-clean (no issues reported).

### GREEN full workflow (issues + schedule + workflow_dispatch; 3 jobs)
See `actionlint-green.log` (empty body, exit code 0) and `yaml-parse.log`.

```
$ actionlint .github/workflows/issue-hygiene.yml
$ echo EXIT=$?
EXIT=0
```

`yaml-parse.log` confirms PyYAML loads the file and lists:
- triggers: `issues`, `schedule`, `workflow_dispatch`
- jobs: `enforce-priority-label`, `sync-project-priority`, `weekly-audit`

actionlint version: see `actionlint-version.txt` (1.7.7, windows/amd64).

## Successful workflow_dispatch run URL
Cannot be recorded from this tentacle — the orchestrator owns VCS
(no `git commit` / `git push` from sub-agents per envelope rules).
After the orchestrator merges this branch, dispatch via:

```
gh workflow run "Issue hygiene (QA8-11)" --ref <branch>
gh run list  --workflow "Issue hygiene (QA8-11)" --limit 1
```

…and append the resulting run URL to this file.

## Permissions / secrets posture (least privilege)
- Top-level `permissions: { contents: read }`.
- `enforce-priority-label` widens to `issues: write` (label add + comment).
- `sync-project-priority` keeps `contents: read`; uses optional
  `PROJECT_TOKEN` PAT secret with `project` scope. Gracefully skips when
  absent (mirrors `contract-weekly.yml` skip pattern).
- `weekly-audit` widens to `issues: read` only.
- No edits to other workflows, no use of `pull-requests`, `actions`, or
  `id-token` scopes.

## Scope check
Allowed-files list (from final-dispatch-authorization §1 / acceptance-matrix):
- `.github/workflows/issue-hygiene.yml` — created in this tentacle.

Evidence files (this directory) live under `verification-evidence/` per
the envelope's "Record validation logs under verification-evidence/"
directive and are not part of the workflow's code-scope allow-list.
