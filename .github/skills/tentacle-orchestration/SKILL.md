---
name: tentacle-orchestration
description: >
  Break complex tasks into scoped parallel work units for multi-agent execution in the
  tui-translator Rust project. Enforces Opus Leader Council (dev-leader, test-leader,
  qa-leader) consensus before any implementation, infinite confidence-gate loop until
  certainty = 1.0, cross-platform quality over speed, and self-healing obstacle resolution.
  Always use task-step-generator first as a reviewed planning scaffold, then adapt the
  reviewed steps into tentacles. Use when a task spans multiple modules or layers, needs
  agent delegation, or the user says "orchestrate", "multi-agent", "parallel agents",
  "tentacle", "swarm", "leader", or "Opus council".
---

# Tentacle Orchestration — tui-translator

Break a complex task into scoped work units ("tentacles"), enrich each with context,
then dispatch agents in parallel. Results persist in files so nothing is lost between
agent boundaries.

Adapted from the [OctoGent](https://github.com/hesamsheikh/octogent) tentacle pattern,
customized for a **multi-platform Rust TUI app** where **quality always beats speed**
and **Opus-class leaders must reach consensus before any implementation begins**.

> **Relationship with strict-tdd-workflow**: Tentacle is the **orchestrator** (splits
> work), strict-tdd is the **executor** (runs inside each implementation/fix tentacle).
> For single-module tasks, skip tentacle and use strict-tdd directly.
>
> **Relationship with task-step-generator**: `task-step-generator` is the **planning
> scaffold**. Run it before creating tentacles, then review and edit the generated steps.
> Do not copy generated steps blindly.

---

## Planning Discipline

Use this sequence before creating any tentacle:

1. Generate a step file with `task-step-generator` (`.github/steps/<task-slug>.md`).
2. Review the generated step file with `references/decomposition-review.md`.
3. Record what was accepted, edited, and rejected before dispatching agents.
4. Convert only the reviewed steps into non-overlapping tentacles and atomic todos.

Why: decomposition and checklists reduce avoidable cognitive load, but generated plans
can anchor on the first plausible split. Treat the step file as a draft planning
artifact, not as authority.

---

## Decision Confidence Gate

Before creating, dispatching, merging, deleting, or closing tentacles, verify the
routing/plan confidence. Any confidence below `1.0` is **not** "good enough"; it means
the orchestrator is still guessing. Split the noisy point into focused research concerns
and dispatch independent research or validation agents first.

**Required behavior when confidence `< 1.0`:**

1. Stop all implementation/deletion/merge decisions for the uncertain scope.
2. Split the ambiguity into atomic questions: task type, scope boundaries, dependencies,
   acceptance evidence, and affected systems.
3. Dispatch research/validation tentacles or sub-agents on the strongest available model
   (`claude-opus-4.7`; fallback to newest opus-class available).
4. Record evidence and rejected alternatives in the tentacle `handoff.md` or the
   conductor `research_gate` artifact.
5. Continue **only** after the synthesized decision reaches confidence `1.0`, or after
   an explicit user override is recorded with its rationale.

**There is no partial confidence.** 0.9 means one critical unknown; that unknown must be
resolved by a research tentacle before proceeding. Loop indefinitely if needed — shipping
wrong work is always more expensive than delaying for certainty.

Why this gate exists: low-confidence orchestration creates the worst kind of parallelism —
many agents confidently doing the wrong work. Research-first decomposition is cheaper than
unwinding a bad swarm.

---

## ⚔️ Opus Leader Council

This project enforces a **Leader Council** pattern before any implementation tentacle is
dispatched. Three Opus-class leader agents must convene on every non-trivial task:

| Leader | Role | Model | Profile |
|--------|------|-------|---------|
| **dev-leader** | Architecture, Rust correctness, async safety, provider traits | `claude-opus-4.7` | `.github/agents/dev-leader.agent.md` |
| **test-leader** | TDD strategy, coverage gates, evidence contracts, regression harness | `claude-opus-4.7` | `.github/agents/test-leader.agent.md` |
| **qa-leader** | Cross-platform quality, ISO 25010/29119, acceptance criteria, release gates | `claude-opus-4.7` | `.github/agents/qa-leader.agent.md` |

### Council Protocol

```
Phase 0 (Clarify) → Council convenes:
  1. dev-leader  raises implementation risks and proposes scope boundaries.
  2. test-leader raises testing blind spots and proposes evidence contracts.
  3. qa-leader   raises cross-platform and QA risks, proposes gate criteria.
  4. All three   cross-review each other's proposals.
  5. If any leader reports confidence < 1.0 on any point:
       → dispatch a research tentacle (Opus model) to resolve it.
       → loop back to step 1 with the research findings.
  6. Only when all three leaders sign off with confidence = 1.0:
       → proceed to Phase 1 (Plan).
```

**Dispatch the council in parallel** (three simultaneous tentacles):

```bash
sk tentacle create council-dev   --scope "src/**/*.rs" --desc "dev-leader review" --profile dev-leader --briefing
sk tentacle create council-test  --scope "tests/**/*"  --desc "test-leader review" --profile test-leader --briefing
sk tentacle create council-qa    --scope "docs/qa8/**" --desc "qa-leader review" --profile qa-leader --briefing

# Swarm all three in parallel
sk tentacle swarm council-dev   --agent-type general-purpose --model claude-opus-4.7 --briefing
sk tentacle swarm council-test  --agent-type general-purpose --model claude-opus-4.7 --briefing
sk tentacle swarm council-qa    --agent-type general-purpose --model claude-opus-4.7 --briefing
```

**Leaders cross-discuss**: each leader's `handoff.md` is fed as input to the next
leader's research tentacle until all three reach `confidence = 1.0`.

### Self-Healing Loop Rule

If any leader or implementation tentacle encounters a blocker:

| Situation | Action |
|-----------|--------|
| Technical unknown | Create research tentacle (Opus model), loop back to council |
| API/platform uncertainty | Create spike tentacle, produce evidence, loop back |
| Cross-platform incompatibility | Decompose into platform-specific tentacles, NOT block |
| Test failure root cause unclear | Create `tui-rust-code-reviewer` tentacle to investigate |
| Security concern | Create `tui-security-auditor` tentacle before proceeding |
| Performance concern | Create `tui-soak-monitor` tentacle to gather baseline |
| No known solution | Document in `handoff.md` with all attempted approaches, escalate to human |

> **Never** mark a tentacle `BLOCKED` and abandon it — obstacles are attacked, not
> skipped. The only acceptable `BLOCKED` is "requires human decision with evidence
> attached." Even then, provide three alternative approaches the human can choose from.

---

## ⛔ Platform Quality Gate

This project targets **Windows 10/11, macOS, and Linux**. Quality always takes priority
over speed. Every feature must pass all platform gates before close.

| Platform | Status | Audio backend | CI requirement |
|----------|--------|---------------|----------------|
| Windows 10/11 | ✅ Production | WASAPI loopback | Must pass `cargo test --all` on GNU toolchain |
| macOS | 🔧 In-progress | CoreAudio/BlackHole → ScreenCaptureKit | Stub compiles, CI matrix runs |
| Linux | 🔧 In-progress | PipeWire → PulseAudio | Stub compiles, CI matrix runs |

**Cross-platform verification rule**: Before closing any implementation tentacle:

```bash
# Windows (native)
cargo check --target x86_64-pc-windows-gnu
cargo test --all --target x86_64-pc-windows-gnu

# macOS cross-check (CI or local)
cargo check --target aarch64-apple-darwin 2>/dev/null || echo "STUB: macOS stub compiles"

# Linux cross-check (CI or local)
cargo check --target x86_64-unknown-linux-gnu 2>/dev/null || echo "STUB: Linux stub compiles"
```

Any platform compilation failure **blocks** the tentacle close — even if Windows passes.

---

## When to use

| Scope | Approach |
|-------|----------|
| 1–2 files, single concern | Direct work — no tentacle needed |
| 3+ files, single module | Optional — tentacle helps track but not required |
| 3+ files, multiple modules | **Tentacle required** — decompose into scoped units |
| Multi-phase with agent delegation | **Tentacle required** — each delegated agent gets a tentacle |
| Bug investigation, multiple hypotheses | Tentacle recommended — one tentacle per hypothesis |
| Cross-platform parity work | **Council + tentacle required** — dev + test + qa leaders all must agree |
| Any confidence < 1.0 | **Research tentacle required** — Opus model, loop until 1.0 |

**Not a good fit:** strictly sequential single-file tasks, limited token budget, trivial edits.

---

## Sub-agent Guardrails

These guardrails apply to dispatched sub-agents. The **commit restriction is enforced
at the git level** when hooks are installed; all other items are conventions reinforced
by prompt context.

**Why git hooks, not preToolUse alone:** When the orchestrator dispatches a sub-agent
via the `task()` tool, the platform does not guarantee that `preToolUse` hooks from the
parent `hooks.json` propagate into the sub-agent's context window. Git hooks
(`pre-commit`, `pre-push`) are filesystem-level and fire for any git process regardless
of which agent spawned it — they are the reliable enforcement surface.

**How enforcement works:**
1. `tentacle.py create` generates a UUID `tentacle_id` stored in the tentacle's
   `meta.json`. If the requested name directory already exists, `create` auto-resolves
   the collision by creating `<name>-<uuid[:8]>` — the slug is printed and must be used
   for all subsequent commands. `tentacle.py swarm` reads `tentacle_id` from `meta.json`
   and writes an HMAC-signed marker file at
   `~/.copilot/markers/dispatched-subagent-active` containing `active_tentacles` entries
   of the form `{"name": ..., "ts": ..., "git_root": ..., "tentacle_id": ...}`.
   **Primary deduplication key: `tentacle_id`** (when present).
2. `hooks/pre-commit` and `hooks/pre-push` call `hooks/check_subagent_marker.py`, which
   blocks the git operation when the marker is present, auth-valid, within its 4-hour
   TTL, **and the entry's `git_root` matches the repo running the git command**.
3. `hooks/rules/subagent_guard.py` provides a secondary `preToolUse` intercept for the
   orchestrator session (defense-in-depth only — not the primary path).
4. `sk tentacle complete <name>` reads `tentacle_id` from `meta.json` and removes only
   the matching marker entry; the marker is deleted when `active_tentacles` becomes empty.

**Install the git hooks** (once per repository):

```bash
sk install --install-git-hooks
# fallback: python3 ~/.copilot/tools/install.py --install-git-hooks
```

| Convention | What to do |
|------------|-----------|
| **Commit restriction** | Do not run `git commit` or `git push`. Git hooks block both while the marker is active. |
| **Stay in scope** | Do not edit files outside your tentacle's declared `scope`. Escalate gaps. |
| **Escalate, don't expand** | Write gaps to `handoff.md` and stop. The orchestrator decides. |
| **No over-implementation** | Implement only what your todos specify. |
| **Handoff before stopping** | Always write structured handoff with `--status` before stopping. |
| **No platform `create` for reports** | Use `tentacle.py handoff` to persist output. `create` is not guaranteed in all agent runtimes. |
| **Rust conventions** | `cargo fmt` before touching any `.rs` file. No `unwrap()`/`expect()` outside tests. Use `tracing::` not `println!`. Every `pub` item gets `///` doc comment. |

---

## Anti-patterns

- ❌ SQL/markdown todos only for multi-agent work → agents lose scope isolation and CONTEXT.md
- ❌ Launching sub-agents without `swarm` prompt → agent gets no scope, constraints, or key files
- ❌ Skipping `--briefing` on create → past mistakes not injected into CONTEXT.md
- ❌ Skipping `complete` before `delete` → learnings from handoff.md lost permanently
- ❌ Overlapping tentacle scopes → agents overwrite each other's work
- ❌ Creating tentacles directly from intuition without a generated-and-reviewed step file
- ❌ Copying `task-step-generator` output blindly without checking dependencies and evidence fit
- ❌ Skipping the runtime bundle on multi-agent work → agents lose file-backed context
- ❌ Sub-agent commits or pushes → blocked by git hooks when installed
- ❌ Sub-agent edits files outside declared scope → silent conflicts with other parallel agents
- ❌ Treating confidence `< 1.0` as acceptable → always run Opus research first
- ❌ Skipping `install.py --install-git-hooks` → git-level guard inactive
- ❌ Accepting sub-agent claims of "tests pass" without running commands → unverified claims are not evidence
- ❌ Closing a tentacle `DONE` with no verification evidence → treated as `AMBIGUOUS`
- ❌ Marking a tentacle `BLOCKED` without attempting self-healing approaches → attack obstacles, don't abandon
- ❌ Skipping Opus Leader Council for non-trivial work → council exists to prevent wrong implementations
- ❌ Skipping cross-platform `cargo check` gates → Windows-only pass ≠ project pass
- ❌ Using `unwrap()` or `expect()` in production code → violates project Rust conventions
- ❌ Using `println!` in production code → use `tracing::info!` / `tracing::warn!` etc.
- ❌ Adding platform-specific GUI/web code → out of scope for this Rust TUI app

---

## Core concept

A **tentacle** is a scoped work unit stored as files:

```
.octogent/tentacles/<name>/
├── CONTEXT.md    ← What the agent needs to know (scope, constraints, key files)
├── todo.md       ← Checkbox items — each is a delegation unit
├── handoff.md    ← Agent writes results here when done
├── meta.json     ← Metadata (scope, status, timestamps)
└── bundle/       ← Runtime context artifacts (created by dispatch by default)
    ├── manifest.json
    ├── session-metadata.md
    ├── recall-pack.json
    ├── briefing.md
    ├── instructions.md
    └── skills.md
```

The octopus metaphor: one orchestrator (you), multiple tentacles (agents), each
handling a distinct code region.

<example>
**Task:** Add PipeWire audio capture for Linux

**Council phase (parallel):**
- `council-dev`   → reviews `src/audio/` trait boundaries, proposes backend HAL shape
- `council-test`  → designs test fixture strategy for PipeWire simulation
- `council-qa`    → maps to LINUX-WBS issues, defines acceptance criteria matrix

**Implementation tentacles (after council signs off at confidence 1.0):**
- `audio-pipewire-backend`  → scope: `src/audio/backend/linux.rs` — PipeWire capture impl
- `audio-pipewire-tests`    → scope: `tests/fixtures/linux_audio/` — simulation fixtures
- `audio-cross-platform`    → scope: `src/audio/mod.rs` — HAL trait and cfg gates

Each tentacle is independent, non-overlapping. The orchestrator merges results after all
three pass: `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test --all`,
cross-platform `cargo check`.
</example>

---

## Internal workflow

The workflow has 5 phases: **Clarify → Plan → Execute → Verify → Close**.

Clarification is the most important phase. A bug found in spec costs 1x to fix.
Found in code: 10x. Found in production: 100x. Never skip this phase — time invested
here prevents entire categories of downstream waste.

### Phase 0: Clarify Spec (Steps 0.0–0.5)

This phase takes a raw specification and makes it implementation-ready through iterative
Q&A. No planning or coding happens until the spec is CLEAN.

- **Step 0.0** (optional): Co-author the spec when user has no written spec
- **Steps 0.1–0.4**: Analyze spec against 8 quality dimensions, generate Spec Health
  Report, iterative refinement until CLEAN
- **Step 0.5**: Reader Testing — verify a fresh agent (no context) can correctly
  understand the spec

For the full process, see `references/spec-clarification.md`.

**Gate**: Planning on an unclear spec produces incorrect decomposition, wasted agent
work, and rework. Never proceed to Phase 1 until the spec is CLEAN and reader-tested.

**After spec is CLEAN → convene Opus Leader Council** (see `## Opus Leader Council`).
All three leaders must reach `confidence = 1.0` before Phase 1 begins.

### Phase 1: Plan

Use the CLEAN spec and council findings to inform decomposition.

#### Plan A: Generate a step file

Use `task-step-generator` before creating tentacles:

```text
Generate a step file for this task. Include CLARIFY, RED evidence/test strategy for
implementation or fixes, BUILD, TEST, REVIEW, LOOP-EVAL when iteration is likely,
and COMMIT/CLOSE. All cross-platform targets (Windows/macOS/Linux) must have explicit
gate steps.
```

Save to `.github/steps/<task-slug>.md`.

#### Plan B: Review and edit the generated steps

Apply `references/decomposition-review.md`. At minimum, verify:

- acceptance signal is observable,
- RED evidence/test strategy exists before implementation,
- dependencies are ordered before parallel work,
- steps are small enough to review in one context,
- only independent work is parallelized,
- each evidence-producing step names logs/screenshots/hashes or equivalent artifacts,
- each step maps to the correct agent type/model available in this project,
- cross-platform `cargo check` gates are present for every implementation step.

Do not proceed until the reviewed plan clearly states accepted, edited, and rejected steps.

#### Plan C: Decompose into modules (agent mapping)

| Tentacle type | agent_type | Model | Scope pattern |
|--------------|-----------|-------|---------------|
| Council: Architecture review | `general-purpose` | `claude-opus-4.7` | `src/**/*.rs` |
| Council: Test strategy | `general-purpose` | `claude-opus-4.7` | `tests/**/*` |
| Council: QA & cross-platform | `general-purpose` | `claude-opus-4.7` | `docs/qa8/**`, `docs/adr/**` |
| Research (confidence < 1.0) | `general-purpose` | `claude-opus-4.7` | Relevant scope |
| Audio backend | `general-purpose` | `claude-sonnet-4.6` | `src/audio/**/*.rs` |
| Provider implementation | `general-purpose` | `claude-sonnet-4.6` | `src/providers/**/*.rs` |
| Pipeline orchestration | `general-purpose` | `claude-sonnet-4.6` | `src/pipeline/**/*.rs` |
| TUI / display | `general-purpose` | `claude-sonnet-4.6` | `src/tui/**/*.rs` |
| Config / hot-reload | `general-purpose` | `claude-sonnet-4.6` | `src/config/**/*.rs` |
| i18n / locales | `general-purpose` | `claude-sonnet-4.6` | `src/i18n/**/*.rs`, `locales/**` |
| Metrics / session | `general-purpose` | `claude-sonnet-4.6` | `src/metrics/**/*.rs`, `src/session/**/*.rs` |
| Integration tests | `general-purpose` | `claude-sonnet-4.6` | `tests/**/*.rs` |
| Crash / stability | `crash-root-cause` | `claude-sonnet-4.6` | Crash evidence, `src/audio/**`, `src/pipeline/**` |
| Security audit | `tui-security-auditor` | `claude-sonnet-4.6` | `src/**`, `config.json` surface |
| Code review (Rust/async) | `tui-rust-code-reviewer` | `claude-sonnet-4.6` | `src/**/*.rs` |
| Soak / stability gate | `tui-soak-monitor` | `claude-sonnet-4.6` | `tests/soak/**`, evidence artifacts |
| NFR verification | `nfr-verification-gate` | `claude-sonnet-4.6` | Evidence artifacts, verification reports |
| Packaging / CI | `general-purpose` | `claude-sonnet-4.6` | `.github/workflows/**`, `packaging/**` |

#### Plan D: Create tentacles

```bash
sk tentacle create <module-name> \
  --scope "<file-patterns>" \
  --profile "<profile-id>" \
  --desc "<short description>" \
  --briefing
# fallback: python3 ~/.copilot/tools/tentacle.py create <module-name> ...
```

Use `--profile` with one of the agent names from the mapping table above.
Always include `--briefing` to inject past mistakes and patterns.

#### Plan E: Add todos

```bash
sk tentacle todo <name> add "<specific, atomic task>"
# fallback: python3 ~/.copilot/tools/tentacle.py todo <name> add "<task>"
```

Each todo = one deliverable — testable, reviewable, completable in isolation.
For Rust tasks, each todo should include the expected `cargo test` filter.

#### Plan F: Enrich CONTEXT.md

Read reference files with `view`, then edit CONTEXT.md to add:

- **Step-plan review**: source step file (`.github/steps/<slug>.md`), accepted/edited/rejected steps, dependency order, and evidence contract
- **Council findings**: dev-leader, test-leader, qa-leader summaries and confidence verdicts
- **What exists**: describe current code in the scope area
- **Key files**: full paths to reference files the agent needs
- **Constraints**: Rust edition, error handling (`anyhow`/`thiserror`), async rules (`tokio`), no `unwrap()`/`expect()`, `tracing::` not `println!`, `///` on all `pub` items, cross-platform `cfg` gates
- **Cross-platform targets**: which platform stubs must still compile

CONTEXT.md template for this project:

```markdown
## Agent Context — <tentacle-name>

### Step-plan review
- Source: `.github/steps/<task-slug>.md`
- Accepted steps: <list>
- Edited steps: <list with rationale>
- Rejected steps: <list with rationale>
- Dependency order: <ordered list>
- Evidence contract: <what must be in handoff.md>

### Council findings
- dev-leader: <confidence verdict, key risks, scope boundaries>
- test-leader: <confidence verdict, test strategy, evidence contracts>
- qa-leader: <confidence verdict, cross-platform gates, acceptance criteria>

### What exists
<description of current code in scope>

### Key files
- `src/providers/mod.rs` — Provider traits (SttProvider, TranslationProvider, TtsProvider)
- `src/audio/mod.rs` — Audio capture HAL
- `src/pipeline/mod.rs` — Pipeline orchestration
- `src/config/mod.rs` — Config loading and hot-reload
- `.github/copilot-instructions.md` — Project conventions
- `Cargo.toml` — Workspace and feature flags

### Constraints (Rust)
- Edition: Rust 2021; MSRV rust-version = 1.88
- Error handling: `anyhow` for app errors; `thiserror` for provider errors
- Async: `tokio` only; avoid `std::thread::spawn` except for WASAPI callbacks
- No `unwrap()`/`expect()` outside tests and `main`
- No `println!` in production paths — use `tracing::info!` / `tracing::warn!`
- `cargo fmt` and `cargo clippy -- -D warnings` must be clean
- `///` doc comment on every `pub` item
- Unit test in same file for every new pure function
- No `std::process::exit` — return `Err` from `main`

### Cross-platform targets
- Windows: primary — must pass `cargo test --all`
- macOS: stub must compile on `aarch64-apple-darwin` or equivalent
- Linux: stub must compile on `x86_64-unknown-linux-gnu` or equivalent
```

---

### Phase 2: Execute (Steps 5–6)

#### Step 5: Dispatch agents (swarm)

```bash
sk tentacle swarm <name> --agent-type <type> --model <model> --briefing
sk tentacle swarm <name> --output parallel --briefing
sk tentacle dispatch <name> --agent-type <type> --model <model> --briefing
# fallback: python3 ~/.copilot/tools/tentacle.py swarm/dispatch <name> ...
```

Use `--model claude-opus-4.7` for council and research tentacles.
Use `--model claude-sonnet-4.6` for implementation tentacles.

Every implementation or bug-fix tentacle must execute the strict-TDD loop internally:
define or reproduce the failing evidence first, make the smallest change, then prove the
same criterion turns green.

#### Step 6: Monitor progress

```bash
sk tentacle status
sk tentacle show <name>
```

If a tentacle returns `BLOCKED`:
1. Read `handoff.md` — it must contain attempted approaches and evidence.
2. Create a research tentacle (Opus model) targeting the specific unknown.
3. Feed research output back as additional CONTEXT.md content.
4. Re-dispatch the original tentacle with updated context.
5. Loop until resolved.

---

### Phase 3: Verify (Steps 7–12)

Every step here catches a different class of agent error. For detailed gate descriptions,
see `references/verification-gates.md`.

| Gate | tui-translator command | Skip when |
|------|----------------------|-----------|
| **Format** | `cargo fmt --all -- --check` | Never skip |
| **Build** | `cargo build --all-targets` | Never skip |
| **Lint** | `cargo clippy --all-targets -- -D warnings` | Never skip |
| **Test** | `cargo test --all` | Never skip |
| **Cross-platform check (macOS)** | `cargo check --target aarch64-apple-darwin` | Never skip if platform stub touched |
| **Cross-platform check (Linux)** | `cargo check --target x86_64-unknown-linux-gnu` | Never skip if platform stub touched |
| **Review** | `tui-rust-code-reviewer` agent tentacle | Never skip for `src/**` changes |
| **Docs** | `cargo doc --no-deps 2>&1 \| grep -E "warning\|error"` | Internal refactors only |
| **Security** | `cargo audit && cargo deny check` | Never skip for provider/config/audio changes |
| **QA audit** | `nfr-verification-gate` agent tentacle | Low-risk doc-only changes only |

**Evidence requirement:** Each gate must produce concrete, recorded output before
being marked as passed. Run the commands yourself and attach or reference the output.
A gate is only passed when you hold the proof, not when the sub-agent says it is.

---

### Phase 3.5: Goal Evaluation Loop

After all verification gates pass, evaluate whether the overarching goal is met before
proceeding to commit and close. This is the **loop-until-verified** phase.

```bash
# Run the goal's success-criteria check and persist the result
sk tentacle verify <name> "<success-criteria-command>" --label "goal-eval"
# fallback: python3 ~/.copilot/tools/tentacle.py verify <name> ...
```

**Decision logic:**

| Result | Action |
|--------|--------|
| Goal met — all success criteria satisfied | Proceed to Phase 4 (Commit + Close) |
| Goal partially met — remaining gaps identified | Return to Phase 1 (Plan), create new tentacles for gaps |
| Goal blocked — external dependency | Write gap to handoff, surface to user with 3 alternative approaches |

**Rules:**
1. Success criteria must be defined **before** dispatching tentacles (in Phase 1).
2. Evaluation is the **orchestrator's responsibility** — sub-agents do not loop.
3. When looping, create **new tentacles** for remaining gaps; never re-open completed ones.
4. Record evidence for every evaluation using `tentacle.py verify`.
5. Do not infer goal status from handoff prose alone — run the success-criteria command.

---

### Phase 4: Commit + Close (Steps 13–17)

#### Step 13: Commit after each completed phase (orchestrator only)

```bash
git add -A && git commit -m "feat(<scope>): <phase description>

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

Commit after each major phase: shared/foundation → parallel batch → verification.

**Commit restriction:** Sub-agents must NOT run `git commit` or `git push`. Git hooks
block both while the `dispatched-subagent-active` marker is fresh.

#### Step 14: Runtime verification

Build passing ≠ app works. After all tentacles are merged:

```bash
# Windows: full build and run
cargo build --release
cargo test --all

# Quick smoke test (if binary can run)
cargo run -- --help 2>&1 | head -5

# Soak gate (for stability/performance changes)
sk tentacle swarm soak-gate --agent-type tui-soak-monitor --model claude-sonnet-4.6 --briefing
```

#### Step 15: Complete and learn

```bash
sk tentacle complete <name>
```

Only call `complete` after all verification gates pass. This marks all todos done and
auto-extracts learnings from `handoff.md` into long-term knowledge.

#### Step 16: Resume a tentacle

```bash
sk tentacle resume <name>             # Refresh briefing, mark active
sk tentacle resume <name> --no-briefing  # Skip briefing injection
```

#### Step 17: Cleanup

```bash
sk tentacle delete <name>
```

---

## Verification summary

| Gate | tui-translator command | Mandatory |
|------|----------------------|-----------|
| Format | `cargo fmt --all -- --check` | ✅ Always |
| Build | `cargo build --all-targets` | ✅ Always |
| Lint | `cargo clippy --all-targets -- -D warnings` | ✅ Always |
| Test | `cargo test --all` | ✅ Always |
| macOS cross-check | `cargo check --target aarch64-apple-darwin` | ✅ If platform touched |
| Linux cross-check | `cargo check --target x86_64-unknown-linux-gnu` | ✅ If platform touched |
| Rust code review | `tui-rust-code-reviewer` agent | ✅ For `src/**` changes |
| Security | `cargo audit && cargo deny check` | ✅ Provider/config/audio changes |
| NFR gate | `nfr-verification-gate` agent | ✅ Stability/performance work |
| Docs | `cargo doc --no-deps 2>&1 \| grep -E "warning\|error"` | Internal refactors only |

---

## CLI reference

```bash
# Create tentacles
tentacle.py create <name> --scope "<paths>" --desc "<desc>" --briefing
tentacle.py create <name> --scope "<paths>" --profile <agent> --desc "<desc>" --briefing

# Add todos
tentacle.py todo <name> add "<task>"

# Dispatch (always use --briefing)
tentacle.py swarm <name> --agent-type general-purpose --model claude-opus-4.7 --briefing   # Opus council/research
tentacle.py swarm <name> --agent-type general-purpose --model claude-sonnet-4.6 --briefing  # Implementation
tentacle.py swarm <name> --agent-type tui-rust-code-reviewer --briefing                     # Code review
tentacle.py swarm <name> --agent-type tui-security-auditor --briefing                       # Security
tentacle.py swarm <name> --agent-type tui-soak-monitor --briefing                           # Soak gate
tentacle.py swarm <name> --agent-type nfr-verification-gate --briefing                      # NFR gate
tentacle.py swarm <name> --agent-type crash-root-cause --briefing                           # Crash analysis
tentacle.py swarm <name> --output parallel --briefing                                       # One worker per todo
sk tentacle swarm <name> --output json --briefing                                           # JSON + bundle_path

# Monitor
sk tentacle status
sk tentacle show <name>

# Handoff
sk tentacle handoff <name> "<summary>" --status DONE --changed-file <path> --learn
sk tentacle handoff <name> "<summary>" --status BLOCKED --learn    # Must include attempted approaches

# Goal tracking
sk tentacle goal init --title "<goal title>" [--desc "<goal description>"]
sk tentacle goal link <name>
sk tentacle goal eval --decision continue|pause|complete|abandon
sk tentacle goal status [--format text|json]

# Verify
sk tentacle verify <name> "<command>" --label "goal-eval"

# Lifecycle
sk tentacle resume <name>
sk tentacle complete <name>
sk tentacle delete <name>

# Fallback (when sk is unavailable)
python3 ~/.copilot/tools/tentacle.py <cmd> <args>
```

---

## Tips

1. **Invest in CONTEXT.md** — 2–3 minutes writing good context saves 10 minutes of agent confusion
2. **Opus council first** — always convene dev-leader + test-leader + qa-leader before implementation
3. **Research beats guessing** — `confidence < 1.0` means dispatch an Opus research tentacle, not implement
4. **Keep todos atomic** — each item = one testable deliverable with a `cargo test` filter
5. **No scope overlap** — overlapping scopes cause agents to overwrite each other's work
6. **Complete before delete** — `complete` saves learnings; `delete` alone loses them
7. **Commit after each phase** — uncommitted code is lost if the session crashes
8. **Cross-platform on every PR** — `cargo check` for Windows + macOS + Linux even when you only changed Windows code
9. **Attack obstacles** — `BLOCKED` tentacles get a research tentacle, not abandonment
10. **Run `cargo fmt` before touching any `.rs` file** — prevents formatter noise in diffs
11. **⚠️ Commit restriction** — Sub-agents must not run `git commit`/`git push`. Git hooks block both while the marker is fresh.
12. **Council evidence propagates** — paste the council leaders' `handoff.md` confidence verdicts into each implementation tentacle's CONTEXT.md
13. **Infinite loop is acceptable** — looping until `confidence = 1.0` is the correct behavior; shipping uncertain work is not

---

## ⛔ Workflow Integration

This project uses a phase-gate delivery model described in
`.github/copilot-instructions.md`. Tentacle orchestration maps to the delivery phases
as follows:

| Delivery Phase | Tentacle role | Notes |
|----------------|--------------|-------|
| Phase 0: TUI placeholder | Direct work | Too small for tentacles |
| Phase 1: WASAPI audio | Audio backend tentacle(s) | Council required for HAL design |
| Phase 2: Google STT | Provider tentacle(s) | Council required for trait boundary |
| Phase 3: Google Translation | Provider tentacle(s) | Council required |
| Phase 4: Full v1 (TTS, cost, live controls) | Multi-tentacle swarm | Council + full verification suite |
| Phase 5: Post-v1 validation gates | NFR/soak/security tentacles | Use specialist agents |
| Phase 6: Azure/Ollama providers | Provider tentacle(s) per backend | Council required for new provider traits |

**Phase-gate stubs rule** (from project conventions): Code that belongs to a future
phase must be written as a stub that `bail!("not yet implemented (Phase N)")` rather
than compiling away entirely. Implementation tentacles must include this stub pattern
in their CONTEXT.md constraints.

Tentacle orchestration runs **within** a delivery phase, after the phase is approved and
before the phase-gate commit. The orchestrator commits after all tentacles in a phase
pass verification.

---

## Reference docs

- `~/.copilot/tools/skills/tentacle-orchestration/references/` — canonical reference material
- `~/.copilot/tools/skills/tentacle-orchestration/references/decomposition-review.md`
- `~/.copilot/tools/skills/tentacle-orchestration/references/verification-gates.md`
- `~/.copilot/tools/skills/tentacle-orchestration/references/spec-clarification.md`
- `~/.copilot/tools/skills/tentacle-orchestration/references/cli-reference.md`
- `.github/agents/` — project-specific agent profiles
- `.github/copilot-instructions.md` — Rust conventions and phase-gate rules
- `docs/adr/` — Architecture Decision Records
- `docs/qa8/` — 8-hour stability QA documentation
