# STEPS: Project board completion and world-class documentation

**Task:** Complete all automation-actionable open project issues, update or create any documentation issues needed, rewrite user documentation to a world-standard level, and add concrete screenshot-style visual guidance.
**Scope:** GitHub issues/project metadata, `.github/steps/`, `.github/agents/`, `README.md`, `USAGE.md`, `PRIVACY.md`, `docs/`, `config.example.json`, `src/bin/`, `src/session/`, `src/audio/`, `src/main.rs`, `tests/`, and safe generated documentation assets.
**Estimated phases:** CLARIFY -> DESIGN -> BUILD -> TEST -> REVIEW -> LOOP-EVAL -> COMMIT/CLOSE

---

## Step 1: CLARIFY - Inventory issues, board status, and blockers

**Goal:** Produce an evidence-backed list of every open issue and classify it as automation-actionable, human-blocked, or post-v1/future-scope.

**Actions:**
1. `gh api --method GET repos/magicpro97/tui-translator/issues -f state=open -f per_page=100` - list open issues without relying on flaky GraphQL.
2. Attempt Project v2 inventory for `magicpro97/2`; if GraphQL fails, record the exact failure and use REST issue labels/issue bodies as the working queue.
3. Read every WBS issue body (#348-#358) and human/post-v1 issue body (#19-#23, #115-#122).
4. Dispatch independent investigation agents for any confidence `< 1.0` classification.

**Done when:** A routing table exists with issue number, status, dependency, owner agent, evidence required, and whether automation may close it.

---

## Step 2: DESIGN - Review decomposition and assign non-overlapping tentacles

**Goal:** Convert the issue inventory into scoped tentacles with no file overlap and explicit evidence gates.

**Actions:**
1. Apply `tentacle-orchestration/references/decomposition-review.md` to this step file.
2. Accept, edit, or reject each planned step.
3. Create or resume tentacles for automation-actionable WBS groups only after dependencies are explicit.
4. Route docs/screenshot work separately from evaluator/runtime work.

**Done when:** Each tentacle has scope, dependency order, test/evidence owner, acceptance signal, and review model (`claude-opus-4.6` or stronger for completed-action review).

---

## Step 3: BUILD - Complete automation-actionable WBS issues

**Goal:** Implement the missing evaluator, pairing, metrics, report, privacy, latest-session, and config/report workflow needed by WBS #348-#358, or prove an issue is already complete on `main`.

**Actions:**
1. For each WBS issue, create RED evidence first: failing parser/metric/report test, missing docs assertion, or missing command output.
2. Implement the smallest code/docs changes needed to make the acceptance criteria pass.
3. Keep human-only and post-v1 issues open unless an evidence-backed automation path exists.
4. Update each GitHub issue with status comments whenever work starts, blocks, or completes.

**Done when:** Every automation-actionable WBS issue has passing evidence or a precise blocker comment explaining why it cannot be closed by automation.

---

## Step 4: BUILD - Rewrite documentation and add screenshot-style guidance

**Goal:** Bring README/USAGE/PRIVACY/docs to a world-standard user guide with concrete visuals and honest release status.

**Actions:**
1. Open documentation tracking issues for any doc/screenshot gaps discovered during inventory.
2. Rewrite user-facing setup and usage docs for non-developers: install, first run, Google key setup, settings selector, audio routing, privacy, measurement reports, troubleshooting.
3. Add concrete screenshot-style images or generated terminal screenshots showing first-run setup, settings choices, subtitle view, metrics panel, measurement report, and virtual mic routing.
4. Ensure all images are safe synthetic captures with no API keys, meeting content, or private paths.

**Done when:** The docs render coherently from a fresh-user perspective and every image has source/provenance and alt text.

---

## Step 5: TEST - Run project gates and docs checks

**Goal:** Prove changed code and documentation are consistent.

**Actions:**
1. `cargo fmt --check`
2. `cargo test --all --locked`
3. `cargo clippy --all-targets --locked -- -D warnings`
4. Run any evaluator/latest-session CLI commands added by WBS work.
5. Run docs/image link checks or targeted tests when available.

**Done when:** All required gates pass, or any pre-existing/human-blocked failure is isolated with evidence.

---

## Step 6: REVIEW - Opus review every completed action

**Goal:** Catch correctness, security, privacy, docs, and overclaiming risks before closing issues.

**Actions:**
1. Dispatch `claude-opus-4.6` review for each completed implementation/docs batch.
2. Fix all substantive findings and re-review until CLEAN.
3. For security/privacy surfaces, run a separate Opus security review plus tool scans where available.

**Done when:** Every completed batch has an Opus review verdict and no unresolved blocking findings.

---

## Step 7: LOOP-EVAL - Decide whether issues remain

**Goal:** Evaluate the overarching request against the live GitHub issue/project state.

**Actions:**
1. Query open issues again with REST and Project v2 where available.
2. Close only issues whose acceptance criteria have direct evidence.
3. Leave human-only/post-v1 issues open only if they are explicitly non-automation-closeable and commented with the blocker.
4. If new gaps appear, create new issues and loop back to Step 2.

**Done when:** No automation-actionable issues remain open; any remaining issues are human-blocked or explicitly future-scope with up-to-date comments.

---

## Step 8: COMMIT/CLOSE - Package the completed wave

**Goal:** Ship the verified changes through a PR and keep the board synchronized.

**Actions:**
1. Commit from the orchestrator session only after gates and Opus review pass.
2. Open a PR, wait for CI/reviewer, fix all comments/failures, then merge if authorized by current task scope.
3. Record `sk learn` before final closeout.

**Done when:** The target branch contains the verified changes, linked issues are closed or blocker-commented, and the local worktree is clean.

---

## Phase Gates

| Phase | Artifact | Status |
|-------|----------|--------|
| CLARIFY | Open issue inventory and automation/human/future classification | ☐ |
| DESIGN | Reviewed decomposition and non-overlapping tentacle map | ☐ |
| BUILD (WBS) | Automation-actionable WBS acceptance criteria satisfied | ☐ |
| BUILD (docs) | Rewritten docs and safe screenshot-style visuals | ☐ |
| TEST | Cargo/docs/evaluator gates pass | ☐ |
| REVIEW | Opus review CLEAN for each completed batch | ☐ |
| LOOP-EVAL | Open issue query shows no automation-actionable issues remain | ☐ |
| COMMIT/CLOSE | PR merged or blocker recorded; `sk learn` recorded | ☐ |
