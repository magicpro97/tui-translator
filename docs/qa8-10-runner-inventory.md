# QA8-10 — Runner inventory (Wave 7)

**Tracking issue:** [#508](https://github.com/magicpro97/tui-translator/issues/508)
**Status:** Initial inventory; macOS / Linux / self-hosted enablement deferred.
**Owner of acceptance:** repo maintainer (acceptance criteria: "Runner-inventory
ADR owner is recorded before the ADR is accepted").

This document is the authoritative companion to
`.github/workflows/soak-ladder.yml`. It records what runners exist today, what
the soak ladder expects of each, and the work-breakdown items that must
complete before each gap closes.

---

## 1. Runner matrix

| OS / arch       | Provider          | Labels required           | Soak rung availability                     | Status (Wave 7)            |
|-----------------|-------------------|----------------------------|--------------------------------------------|----------------------------|
| Windows 11 x64  | GitHub-hosted     | `windows-latest`           | smoke, calibration, nightly                | ✅ Enabled                  |
| Windows 11 x64  | Self-hosted       | `[self-hosted, windows, soak-8h-hardware]` | weekly-8h               | ⏸ Not registered yet        |
| macOS 13 x64    | GitHub-hosted     | `macos-13`                 | smoke, calibration, nightly, weekly-8h     | 🔒 Blocked by macOS WBS    |
| macOS 14 arm64  | GitHub-hosted     | `macos-14`                 | smoke, calibration, nightly, weekly-8h     | 🔒 Blocked by macOS WBS    |
| Ubuntu 24.04    | GitHub-hosted     | `ubuntu-latest`            | smoke, calibration, nightly, weekly-8h     | 🔒 Blocked by Linux WBS    |

Legend:
* ✅ — runner is online and the soak ladder may dispatch real jobs.
* ⏸ — runner *contract* exists in the workflow, but no runner currently
  advertises the required label set. Jobs targeted at that label stay queued
  until the workflow timeout expires; the release gate continues to block
  promotion because no fresh evidence appears for that platform.
* 🔒 — runner is intentionally *not* enabled in this wave. The soak-ladder
  workflow emits a documented "SKIPPED — blocked by WBS" job so the absence
  is visible to anyone reading CI before they reach
  `.github/workflows/release-gate.yml`.

## 2. Soak ladder rungs and which runners they need

| Rung          | Duration | Runners that must produce evidence for an RC/GA gate |
|---------------|----------|-------------------------------------------------------|
| smoke         | 10 min   | windows-latest                                        |
| calibration   | 30 min   | windows-latest                                        |
| nightly       | 1 h      | windows-latest                                        |
| weekly-8h     | 8 h      | self-hosted windows + macOS + Linux                   |

The release-gate workflow currently demands
`windows-latest,ubuntu-latest,macos-latest` per-platform evidence (see
`required_platforms` input default in `.github/workflows/release-gate.yml`).
Two of those three rows are intentionally blocked in Wave 7; the gate is
expected to block any RC/GA promotion until they are unblocked.

## 3. WBS work-items blocking each gap

* **windows-self-hosted (8-hour runner)**
  * Blocking issue: [#503](https://github.com/magicpro97/tui-translator/issues/503) (QA8-05 runner v2). Held until Group C.
  * Exit criteria: a runner registered with labels
    `[self-hosted, windows, soak-8h-hardware]`; runner power policy disables
    sleep; storage retention covers `verification-evidence/release/<tag>/`.
* **macOS**
  * Blocking ADR: `docs/adr/xplat-01-cross-platform-audio-hal.md`
    (cross-platform audio HAL must land before macOS soaks are meaningful).
  * Additional dependency: macOS WBS (separate epic). The soak-ladder skip
    job references this row so the dependency is discoverable from CI alone.
  * Exit criteria: audio HAL ADR accepted; macOS-13 + macOS-14 hosted runners
    enabled; first calibration run green for two consecutive days.
* **Linux**
  * Blocking WBS: Linux platform enablement (separate epic).
  * Exit criteria: PulseAudio/PipeWire capture path verified;
    `ubuntu-latest` enabled in `soak-ladder.yml` and `ci.yml`; first
    calibration run green for two consecutive days.

## 4. How the release gate consumes runner status

`.github/workflows/release-gate.yml` requires the following bundle for any
RC / GA decision:

```
verification-evidence/release/<release-tag>/
  manifest.json
  per-platform/<platform>/nfr.json
  per-platform/<platform>/soak.json
  per-platform/<platform>/crashes/*.json   (optional; presence ⇒ block)
  review.md
```

Until the macOS and Linux runners light up, no `per-platform/macos-latest/*`
or `per-platform/ubuntu-latest/*` files will be produced by the soak ladder.
The release gate therefore blocks at `validate-bundle` with a
`platforms_missing` value covering both rows. This is the intended Wave 7
behaviour — the system fails closed.

## 5. Owner-approved exception path

When (and only when) the maintainer explicitly approves an exception (for
example a hot-fix RC that cannot wait for macOS enablement), the path is:

1. Open a comment on the release PR with the exception scope, expiry date,
   and the name of the approver (must be a maintainer).
2. Run `release-gate.yml` via `workflow_dispatch` with
   `required_platforms=windows-latest` (drops macOS / Linux temporarily).
3. Attach the run URL and the maintainer's comment URL to the release PR.
4. File a follow-up issue to restore the default `required_platforms` list
   no later than the next RC cycle.

This procedure intentionally lives only in this document; the workflow does
not auto-approve any exception.

## 6. Verification trail for this ADR

* Soak ladder workflow: `.github/workflows/soak-ladder.yml` (Wave 7).
* Release gate workflow: `.github/workflows/release-gate.yml` (Wave 1).
* Acceptance matrix row for #508: pending Wave 7 acceptance — this document
  is the runner-inventory ADR referenced by that row.
* See also: `docs/qa8-12-release-gate-orchestration.md` for how Opus / NFR
  reviews map onto the evidence bundle this inventory feeds.
