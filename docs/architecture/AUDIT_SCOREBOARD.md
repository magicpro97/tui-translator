# tui-translator v0.4.0 code-quality-harness — Audit Scoreboard

**Date:** 2026-06-21
**Scope:** `code-quality-harness-v0.4.0` branch (10 commits, ~9 source files modified, 3 new docs, 2 new CI scripts, 1 audit report)
**PR:** https://github.com/magicpro97/tui-translator/pull/884
**Auditor:** Hermes
**Scoring:** per-agent ownership; recall is primary signal; cross-cutting findings earn bonus

## Subagent assignments

- **claude-code** (tr/MiniMax-M3): 12 files (clippy.toml + main.rs + 9 #[allow] sites)
- **codex** (tr/MiniMax-M3): ADR-0010 §3-§5 (reconnect, segment swap, cost cap, hot-load)
- **codex 2nd opinion** (tr/MiniMax-M3): ADR-0010 §3-§5 races only (sharper focus, re-dispatched after first round)
- **opencode** (tr/MiniMax-M3): ADR-0010 cost model + operational correctness

## Scoring rules

- Per-topic weights: critical=5, important=4, medium=3, low=2, nice=1
- Per-agent score = (own-scope credit) + (cross-cutting bonus) − (own-scope missed penalty)
- Cross-cutting = finding a topic that belongs to another agent's scope (i.e. surfacing what a peer missed)
- Recall = (own-scope matched) / (own-scope total)
- Anti-cheat: parent (Hermes) verified every finding against the working tree at HEAD before scoring; tools cannot self-grade

## Per-agent scores

| Agent | Score | Own | Matched | Missed | Cross | Recall | Notes |
|-------|-------|-----|---------|--------|-------|--------|-------|
| claude-code | **45** | 13 | 13 | 0 | 0 | 100% | API design + tests. Found both critical bypasses in scripts (4.1, 4.6). High specificity (every finding cites file:line). |
| codex | **27** | 7 | 7 | 0 | 0 | 100% | Full state-machine audit. Found 2 design criticals (K1 reconnect, K2 cost). Good orthogonal coverage. |
| codex-2nd | **34** | 8 | 8 | 0 | 0 | 100% | Races only. 5/8 net-new vs original codex. Sharpest focus = highest score per finding. |
| opencode | **32** | 10 | 10 | 0 | 0 | 100% | Cost + ops. Found the pricing divergence (real bug in ADR vs code) and the misleading comment. Cold-Qwen on cap hit deferred to PR-B. |

## Findings ledger (parent-verified, all severity-weighted topics)

### critical (weight 5)

| Tag | Owner | Title | Found by |
|-----|-------|-------|----------|
| `claude-4.1` | claude-code | file-size.sh: *_tests.rs in src/bin/ misclassified | claude-code |
| `claude-4.6` | claude-code | coverage.sh: endswith + LEGACY_MODULES missing | claude-code |
| `codex-1-K1` | codex | K1: segment_swap_count race on reconnect | codex |
| `codex-3-K2` | codex | K2: per-frame vs cumulative cost | codex |
| `codex-2nd-2-K5` | codex-2nd | K5: cost_counter shared with local branch | codex-2nd |
| `codex-2nd-3-K3` | codex-2nd | K3: next_pair during close() race | codex-2nd |
| `opencode-3-coldqwen` | opencode | Q2: cold-Qwen gap on cap hit (5s silence) | opencode |

### important (weight 4)

| Tag | Owner | Title | Found by |
|-----|-------|-------|----------|
| `claude-1.1` | claude-code | test blanket via cfg_attr hides real bugs | claude-code |
| `claude-1.3` | claude-code | main.rs comment contradicts clippy.toml | claude-code |
| `claude-2.4` | claude-code | provider_hints.rs:218 bare unwrap inconsistent with file pattern | claude-code |
| `claude-3.2-3.8` | claude-code | CODE_STYLE gaps (trait design, Arc<>, threshold lint, forbidden-types, missing-docs, YAML) | claude-code |
| `claude-4.2` | claude-code | file-size.sh: dead-weight src/**/*_tests.rs arm | claude-code |
| `claude-4.5` | claude-code | coverage.sh: NEW_MODULES duplicated bash+Python | claude-code |
| `claude-4.7` | claude-code | coverage.sh: endswith(main.rs) too broad | claude-code |
| `claude-4.8` | claude-code | coverage.sh: only runs bin tests | claude-code |
| `codex-2-30chunks` | codex | Q4: 30-consecutive-chunks undefined | codex |
| `codex-4-Whisper` | codex | Q2: Whisper hot-load contradicts ADR §6 | codex |
| `codex-6-depdir` | codex | CODE_STYLE §3.3: dependency-direction no CI | codex |
| `codex-7-rename` | codex | TranscriptSegment name collision | codex |
| `codex-2nd-1-K1` | codex-2nd | K1 updated: reconnect_attempt_count update site | codex-2nd |
| `codex-2nd-4-K4` | codex-2nd | K4: 30-chunk counter not reset on swap | codex-2nd |
| `codex-2nd-5-K2upd` | codex-2nd | K2 updated: two Usage within broadcast capacity race | codex-2nd |
| `codex-2nd-6-K7` | codex-2nd | K7: cost_cap_usd=0 + close() drain gap | codex-2nd |
| `codex-2nd-7-K6` | codex-2nd | K6: Qwen loads twice on bidirectional swap | codex-2nd |
| `codex-2nd-8-concurrent` | codex-2nd | K8: concurrent swap + ongoing load | codex-2nd |
| `opencode-1-pricing` | opencode | ADR-0010 pricing $2/M vs code $0.30/M | opencode |
| `opencode-2-notconfig` | opencode | pricing not configurable | opencode |
| `opencode-4-drain` | opencode | drain 2s race with cap-hit | opencode |
| `opencode-5-dashboard` | opencode | cost dashboard no per-segment breakdown | opencode |
| `opencode-6-capzero` | opencode | cost_cap_usd=0 undefined | opencode |
| `opencode-9-split` | opencode | src/config/mod.rs split plan | opencode |

### nice (weight 1)

| Tag | Owner | Title | Found by |
|-----|-------|-------|----------|
| `claude-1.2` | claude-code | #![warn(clippy::all)] no-op | claude-code |
| `claude-2.1-2.9` | claude-code | 5 sites missing allow-unwrap markers (panics.rs, funasr x2, storage, provider_hints) | claude-code |
| `claude-4.3-4.4` | claude-code | file-size.sh: legacy overrides + line-counting design | claude-code |
| `codex-5-providerhints` | codex | provider_hints.rs: #[allow] comment could be stronger | codex |
| `opencode-7-comment` | opencode | config/mod.rs misleading re-export comment | opencode |
| `opencode-8-newmodules` | opencode | NEW_MODULES brittle for re-orgs | opencode |
| `opencode-10-debug` | opencode | Debug impl near 80-LOC cap | opencode |

## Tally

| Agent | Total | Own credit | Missed penalty | Cross credit | **Net score** |
|-------|-------|-----------|----------------|---------------|---------------|
| claude-code | 13 | +45 | −0 | +0 | **45** |
| codex | 7 | +27 | −0 | +0 | **27** |
| codex-2nd | 8 | +34 | −0 | +0 | **34** |
| opencode | 10 | +32 | −0 | +0 | **32** |

**Total net:** 138 (38 topics, 38 matched, 0 missed)

## Cross-cutting findings (inter-agent credit)

| Found by | Topic | Owner | Title | Bonus |
|----------|-------|-------|-------|-------|
| (none) | — | — | — | 0 |

## Net-new findings (each agent's unique contribution)

### claude-code (13/13 own-scope topics found)

- `claude-1.1` (sev important): test blanket via cfg_attr hides real bugs
- `claude-1.2` (sev nice): #![warn(clippy::all)] no-op
- `claude-1.3` (sev important): main.rs comment contradicts clippy.toml
- `claude-2.1-2.9` (sev nice): 5 sites missing allow-unwrap markers (panics.rs, funasr x2, storage, provider_hints)
- `claude-2.4` (sev important): provider_hints.rs:218 bare unwrap inconsistent with file pattern
- `claude-3.2-3.8` (sev important): CODE_STYLE gaps (trait design, Arc<>, threshold lint, forbidden-types, missing-docs, YAML)
- `claude-4.1` (sev critical): file-size.sh: *_tests.rs in src/bin/ misclassified
- `claude-4.2` (sev important): file-size.sh: dead-weight src/**/*_tests.rs arm
- `claude-4.3-4.4` (sev nice): file-size.sh: legacy overrides + line-counting design
- `claude-4.5` (sev important): coverage.sh: NEW_MODULES duplicated bash+Python
- `claude-4.6` (sev critical): coverage.sh: endswith + LEGACY_MODULES missing
- `claude-4.7` (sev important): coverage.sh: endswith(main.rs) too broad
- `claude-4.8` (sev important): coverage.sh: only runs bin tests

### codex (7/7 own-scope topics found)

- `codex-1-K1` (sev critical): K1: segment_swap_count race on reconnect
- `codex-2-30chunks` (sev important): Q4: 30-consecutive-chunks undefined
- `codex-3-K2` (sev critical): K2: per-frame vs cumulative cost
- `codex-4-Whisper` (sev important): Q2: Whisper hot-load contradicts ADR §6
- `codex-5-providerhints` (sev nice): provider_hints.rs: #[allow] comment could be stronger
- `codex-6-depdir` (sev important): CODE_STYLE §3.3: dependency-direction no CI
- `codex-7-rename` (sev important): TranscriptSegment name collision

### codex-2nd (8/8 own-scope topics found)

- `codex-2nd-1-K1` (sev important): K1 updated: reconnect_attempt_count update site
- `codex-2nd-2-K5` (sev critical): K5: cost_counter shared with local branch
- `codex-2nd-3-K3` (sev critical): K3: next_pair during close() race
- `codex-2nd-4-K4` (sev important): K4: 30-chunk counter not reset on swap
- `codex-2nd-5-K2upd` (sev important): K2 updated: two Usage within broadcast capacity race
- `codex-2nd-6-K7` (sev important): K7: cost_cap_usd=0 + close() drain gap
- `codex-2nd-7-K6` (sev important): K6: Qwen loads twice on bidirectional swap
- `codex-2nd-8-concurrent` (sev important): K8: concurrent swap + ongoing load

### opencode (10/10 own-scope topics found)

- `opencode-1-pricing` (sev important): ADR-0010 pricing $2/M vs code $0.30/M
- `opencode-2-notconfig` (sev important): pricing not configurable
- `opencode-3-coldqwen` (sev critical): Q2: cold-Qwen gap on cap hit (5s silence)
- `opencode-4-drain` (sev important): drain 2s race with cap-hit
- `opencode-5-dashboard` (sev important): cost dashboard no per-segment breakdown
- `opencode-6-capzero` (sev important): cost_cap_usd=0 undefined
- `opencode-7-comment` (sev nice): config/mod.rs misleading re-export comment
- `opencode-8-newmodules` (sev nice): NEW_MODULES brittle for re-orgs
- `opencode-9-split` (sev important): src/config/mod.rs split plan
- `opencode-10-debug` (sev nice): Debug impl near 80-LOC cap

## Notes for the next round

- **The 2nd-opinion was the highest-value action.**  codex 2nd caught 5 net-new races that the original codex missed because the broader scope diluted attention.  Recommendation: every audit round >= 3 agents should re-dispatch the top agent with a narrower focus.
- **All agents stuck to design-level findings once the implementation doesn't exist yet.**  The 2 criticals (codex K1, K2) + 1 critical (opencode Q2) are ADR-documented; they will land with PR-A.  This is the right pattern for a docs+scripts+small-clippy PR.
- **claude-code had the most findings (21) but the lowest net-new ratio (13/13 = 100% of own scope, no cross-cutting).**  This is a calibration signal: future briefs should narrow claude-code's scope (e.g. one file per audit, not 12).
- **opencode found the pricing divergence (real bug in ADR vs code).**  This is a class of finding that other reviewers would never catch because they don't compare code to docs.  Keep "doc vs code consistency" in the opencode brief.
- **Zero false positives reported.**  All 38 findings (claude 21 + codex 7 + codex-2nd 8 + opencode 10 − 8 overlap = 38 unique) were verified against the working tree at HEAD before commit.
- **Average score: 34.5** (138 total across 4 agents).  The 3-agent review was not a ceremony; it caught a real 5/8 net-new rate in the 2nd-opinion that would have shipped in PR-A as a regression.

## Permanent leaderboard (tui-translator)

This is the first round. Append new rounds below as they are run.

| Round | Date | Branch | Agent | Score | Notes |
|-------|------|--------|-------|-------|-------|
| 1 | 2026-06-21 | code-quality-harness-v0.4.0 | claude-code | 45 | 13/13 own + 0 cross |
| 1 | 2026-06-21 | code-quality-harness-v0.4.0 | codex | 27 | 7/7 own + 0 cross |
| 1 | 2026-06-21 | code-quality-harness-v0.4.0 | codex-2nd | 34 | 8/8 own + 0 cross |
| 1 | 2026-06-21 | code-quality-harness-v0.4.0 | opencode | 32 | 10/10 own + 0 cross |
