# Code Style & Architecture Conventions

> **Status:** Authoritative for v0.4.0+.  
> **Scope:** every new Rust file, every refactor of existing code, every CI gate.  
> **Enforcement:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `scripts/check-file-size.sh`, and the 3-agent adversarial review (ADR-0010 §Review plan).  
> **Reviewers (claude-code, codex, opencode)** cite this document by section number when rejecting a PR.

## Why this document exists

The project shipped its first commit in 2022 and now sits at ~80 k LOC of non-test code and ~19 k LOC of tests, in 250+ `.rs` files.  As of 2026-06-21 the five largest files are:

| LOC    | File |
|--------|------|
| 7 682  | `src/main.rs` |
| 5 453  | `src/config/mod.rs` |
| 5 374  | `src/tui/mod.rs` |
| 4 285  | `src/pipeline/mod.rs` |
| 2 446  | `src/bin/eval_session.rs` |

The pattern is unsustainable: each new feature added a 200-500 LOC block to `main.rs` or `config/mod.rs`.  The result is files that no one wants to read top-to-bottom, and where every new bug fix has to touch three or more of them.

This document codifies the practices the project has been converging on (the `*_tests.rs` sibling-file pattern, the `pub use` re-export pattern, the `Arc<...>`-wrapped field pattern in `OrchestratorContext`) and adds the missing enforcement so they actually hold.

The goal is **not** to refactor the existing 7 k LOC `main.rs` in one PR.  It is to make sure that **every new file** we add from v0.4.0 onward is bounded, testable, and well-named — and to chip away at the existing files in the same direction over the next few PRs.

## The seven layers of enforcement

We protect code quality in **seven layers**, each with a single owner.  When a layer fires, the fix is local — no need to escalate.

| # | Layer | Tool / File | Owner |
|---|-------|------------|-------|
| 1 | Formatting | `cargo fmt` (rustfmt.toml) | `rustfmt` |
| 2 | Lints | `cargo clippy --all-targets -- -D warnings` (clippy.toml) | `clippy` |
| 3 | Doc coverage | `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps` | `rustdoc` |
| 4 | File size | `scripts/check-file-size.sh` (new in v0.4.0) | CI script |
| 5 | Function / arg / complexity | `clippy.toml` thresholds | `clippy` |
| 6 | Test ratio | `cargo-llvm-cov` report in `scripts/check-coverage.sh` (new in v0.4.0) | CI script |
| 7 | Human / adversarial review | claude-code, codex, opencode (ADR-0010) | humans + AI agents |

Layers 1-3 already run in CI.  Layers 4 and 6 are new in v0.4.0.  Layer 5's thresholds exist but are warnings, not errors — they become errors in v0.4.0.  Layer 7 is the new 3-agent review.

---

## 1. File organisation

### 1.1 Size limit (layer 4)

| File class | Max LOC | Rationale |
|------------|---------|-----------|
| Module entry (`mod.rs`) | **1 500** | One file, one concern.  If the `mod.rs` is past 1 k, split into sub-modules. |
| Module file (`foo.rs`) | **1 500** | Same. |
| Test sibling (`foo_tests.rs`) | **unbounded** | Tests are easier to read when grouped; not subject to the limit. |
| Integration test (`tests/<name>.rs`) | **2 000** | The `#[path]` include pattern means these are pseudo-modules. |
| CLI subcommand (`src/bin/<name>.rs`) | **500** | A CLI subcommand is a thin wrapper; if it grows, extract logic. |
| `main.rs` | **2 500** | The dispatch table; exempt from the 1.5 k cap but no larger. |

The CI script `scripts/check-file-size.sh` enforces these.  A PR that adds a new file past the cap is rejected with a pointer to the closest sub-module the author should have used.

### 1.2 Naming

- **Files:** `snake_case.rs`, no abbreviations beyond standard (`config`, `provider`, `stt`, `mt`, `tts`, `wss`).
- **Module directories:** plural if they hold variants (`providers/`, `tests/`), singular if they hold one concern (`audio/`, `metrics/`).
- **Test files:** `<subject>_tests.rs` as a sibling of the file under test, OR an inline `#[cfg(test)] mod tests { ... }` block at the bottom of the file.  Pick one per file, never both.
- **CLI subcommands:** `src/bin/<verb>_<noun>.rs` (e.g. `eval_session.rs`, `mt_bench.rs`).

### 1.3 Sub-module split

When `mod.rs` exceeds 1 k LOC, split by **concern**, not by file size:

| Pattern | Example | Rationale |
|---------|---------|-----------|
| Per stage | `pipeline/orchestrator/`, `pipeline/swap/`, `pipeline/reconnect/` | Each stage has its own state machine. |
| Per kind | `providers/cloud/`, `providers/local/`, `providers/llm/` | Each vendor/family is independent. |
| Per layer | `metrics/backpressure/`, `metrics/cost/`, `metrics/latency/` | Each metric type has its own collection. |

When the concern is "this function is too long", **don't** split.  Refactor the function.  When the concern is "this file mixes two stages of a pipeline", **do** split.

---

## 2. Function / type design

### 2.1 Size limits (layer 5)

| Rule | Threshold | Source |
|------|-----------|--------|
| Function body LOC | **80** | `clippy.toml` `too-many-lines-threshold` (was warn, v0.4.0 → deny) |
| Cognitive complexity | **15** | `clippy.toml` `cognitive-complexity-threshold` (was warn, v0.4.0 → deny) |
| Function arguments | **7** | `clippy.toml` `too-many-arguments-threshold` (was warn, v0.4.0 → deny) |
| Struct fields | **20** | new in v0.4.0; rationale: `OrchestratorContext` is the cautionary tale |
| Enum variants | **10** | new in v0.4.0; rationale: huge enums force wide `match` arms |
| `match` arms per `match` | **8** | new in v0.4.0; rationale: large matches are the most common source of "I forgot to handle this case" bugs |

These are **deny** in v0.4.0.  A PR that introduces a 90-line function is rejected with: "split per the convention in `docs/architecture/CODE_STYLE.md` §2.1".

### 2.2 When to extract a function

Extract a function when:

1. The body exceeds the line limit.
2. The body is a long `if` / `else if` chain that conceptually has a name.
3. The body is called from two or more sites (DRY).
4. The body has a comment explaining what it does (the comment is the function's name).

Do **not** extract a function when:

1. The body is 3-5 lines and called once.
2. Extraction would force the caller to thread new arguments through.
3. The function would need a `&mut` borrow that the caller cannot spare.

### 2.3 When to extract a type

Extract a type when:

1. A function takes 4+ parameters of the same kind (e.g. four `&str` arguments that always travel together).
2. Two functions return the same shape of data.
3. A `HashMap<String, ...>` is being passed around — the key set is the implicit type, make it explicit.
4. A boolean parameter is being passed (`fn foo(verbose: bool)` is a smell — make it `fn foo_verbose()` and `fn foo_quiet()` or take a config struct).

### 2.4 Async signatures

- `async fn` must return `Result<T, _>` unless the operation is truly infallible.
- `async fn` must not be a thin wrapper over a synchronous function.  If the only await is a single `block_in_place`, it should be sync.
- Trait methods that are async must document the cancellation semantics: "this future can be dropped mid-execution without side effects" or "this future must be awaited to completion".

---

## 3. Module structure

### 3.1 Public API surface

Each module exposes a **public API** via `pub` items + a `pub use` re-export block at the top of `mod.rs`.  The re-export block is the contract; everything else is implementation detail.

Example (from `src/providers/cloud/mod.rs`):

```rust
pub mod config;
pub mod gemini_live_translate;
pub mod orchestrator;
pub mod protocol;

#[allow(unused_imports)]
pub use config::{CloudConfig, CloudVendor};
#[allow(unused_imports)]
pub use gemini_live_translate::{
    build_setup_public, GeminiLiveTranslateProvider, GEMINI_LIVE_TRANSLATE_MODEL,
};
#[allow(unused_imports)]
pub use protocol::{CloudStreamEvent, SetupMessage, TranslationStyle, UsageStats};
```

The `#[allow(unused_imports)]` is intentional: the re-exports are part of the public API even when no internal code uses them in the current build.  Integration tests that include the module without exercising every item would otherwise fail `cargo clippy -- -D warnings`.

### 3.2 Visibility

- `pub` — part of the module's public API.  Re-exported at the top of `mod.rs`.
- `pub(crate)` — used across the crate but not re-exported.  Examples: `OrchestratorContext`, `RuntimeSttProvider`.
- `pub(super)` — used by the parent module's siblings.  Rare; prefer `pub(crate)`.
- Default (private) — implementation detail.  Tests in sibling files access via `super::foo` or `pub(crate)`.

A `pub` item that is **not** in the `pub use` re-export block at the top of `mod.rs` is a code-review red flag.  Either it should be re-exported (it is part of the API) or it should be `pub(crate)` (internal).

### 3.3 The dependency-direction rule (new in v0.4.0)

Lower-layer modules must not import from higher-layer modules:

```
main          ─┐
              ├─► pipeline ─► providers
tui           ─┘
audio ─► pipeline ─► providers
metrics       (no module imports; consumed by everyone)
```

Concretely:

- `pipeline` can import from `providers`, `audio`, `metrics`, `tui`.  It must not import from `main`.
- `providers` can import from `metrics`.  It must not import from `pipeline`, `tui`, `main`.
- `tui` can import from `pipeline`, `audio`, `metrics`, `providers`.  It must not import from `main`.
- `audio` can import from `metrics`.  It must not import from `pipeline`, `tui`, `main`.
- `metrics` is the bottom: imports nothing from the project (only std + external crates).

Violations are caught by the 3-agent review in v0.4.0 and by a future CI script (`scripts/check-dependency-direction.sh`).

A v0.4.0 concrete example: the `cloud_session` field added to `OrchestratorContext` in PR-A is fine because `OrchestratorContext` is owned by `pipeline`.  But `pipeline/mod.rs` must not `use crate::providers::cloud::CloudStreamSession;` to construct one — that construction lives in `main.rs` and is **injected** into the context at startup.

---

## 4. Error handling

### 4.1 The `anyhow` vs `thiserror` rule

- `thiserror` for **library-level** errors: provider impls, pipeline stages, config parsers.  These errors are part of the public API and need a stable, machine-readable shape.
- `anyhow` for **top-level** glue: `main.rs`, CLI subcommands, integration tests.  These are human-facing and benefit from chained context.

The boundary is the layer rule: anything inside `src/pipeline/`, `src/providers/`, `src/audio/`, `src/tui/` is library-level and uses `thiserror`.  `src/main.rs`, `src/bin/*`, `src/cloud_setup_cli.rs`, and `tests/*` use `anyhow`.

### 4.2 Error chain

Every `Result` returned across a layer boundary must include context.  Use `.context("…")` / `with_context(|| format!(…))` from `anyhow`, or the equivalent `#[source]` chain in `thiserror`.

Anti-pattern: returning `Result<T, ProviderError>` with no context, then catching it 5 calls up and printing `"Error: <display>"`.  The user sees no actionable information.

### 4.3 Logging errors

Errors that are **handled** (logged, swallowed, or replaced) get `tracing::error!` or `tracing::warn!`.  Errors that **propagate** get no log; the top-level handler logs once.

---

## 5. Testing

### 5.1 Co-location

Each `.rs` file (or sub-module) has a **test file or block** next to it.  Pick one:

- **Sibling file** (`foo.rs` + `foo_tests.rs`) when the tests are large (> 200 LOC) or when `foo.rs` is a library that gets reused.
- **Inline block** (`#[cfg(test)] mod tests { ... }` at the bottom of `foo.rs`) when the tests are small or when they need access to private items.

### 5.2 Public API test ratio (layer 6)

Every public function or method in a non-`bin` module must have **at least one** test.  The CI gate `scripts/check-coverage.sh` (v0.4.0, new) walks each module and counts the `#[test]` items in the matching test file/block; if the ratio of tested-public-items to total-public-items drops below **0.6**, the gate fails.

Items exempt from the count: `#[cfg(test)]`-only items, `pub use` re-exports, items in `mod.rs` that are themselves `pub mod` re-declarations.

### 5.3 What to test

- **Every public function**: at least one happy-path test.
- **Every error variant**: at least one test that triggers it.
- **Every state transition**: at least one test that walks through the transition.
- **Every `#[derive(...)]` on a public type**: at least one round-trip test (serialize → deserialize).

### 5.4 What not to test

- Trivial getters / setters.
- `#[derive(Clone)]` on a struct (the macro is the test).
- Re-exports.
- `pub use` aliases.

### 5.5 Test names

Test names read as a sentence: `validate_rejects_empty_target_language`, not `test_invalid_1`.  The format is:

```
<subject>_<verb>_<expected outcome>[_<condition>]
```

Examples:

- `run_audio_pump_emits_session_finish_on_shutdown`
- `validate_rejects_empty_target_language`
- `cloud_segment_p50_latency_tracks_recent_chunks`

---

## 6. Documentation

### 6.1 Doc comments

- Every `pub` item gets `///` doc comment.  No exceptions.
- `pub(crate)` items get a `///` comment only when their behaviour is non-obvious.
- Module-level `//!` is required for every `mod.rs`.

### 6.2 Doc-comment style

- First line is a single sentence, period-terminated, no preamble.
- Blank line, then the rationale or context.
- Code blocks in doc comments must compile (use ` ```text ` for non-Rust blocks, ` ```rust ` for Rust snippets that should compile, ` ```rust,no_run ` for snippets that should not execute).

Anti-pattern: a 30-line doc comment with no structure.  If the doc comment needs a table, an ASCII diagram, or section headers, the function is probably too long — refactor.

### 6.3 The "why" comment

Comments explain **why**, not **what**.  The `what` is the code; the `why` is the comment.

```rust
// BAD: increment i by 1
i += 1;

// GOOD: skip past the leading 4-byte length prefix
i += 4;
```

---

## 7. Dependencies

### 7.1 New external crates

A new external crate requires:

1. Mentioned in the PR description with a one-sentence rationale.
2. Listed in `Cargo.toml` with the minimal feature set needed.
3. Approved by the 3-agent review (one reviewer veto is enough to block).

### 7.2 Vendoring

We do **not** vendor (commit a local copy of) third-party crates.  Cargo's `Cargo.lock` provides reproducibility.  Vendoring is a security/audit concern best handled at the org level, not in the project repo.

### 7.3 Forbidden crates

These crates are forbidden in the project, with rationale:

| Crate | Why forbidden |
|-------|---------------|
| `unwrap` in library code | Forces a panic on the user; the only acceptable place for `unwrap` is in test code. |
| `lazy_static` | Use `OnceLock` from std (Rust 1.70+). |
| `parking_lot` | The std `Mutex` is fast enough for our hot-path.  Reach for `parking_lot` only after a benchmark proves `std::sync::Mutex` is the bottleneck. |
| `tokio::spawn_blocking` inside a hot path | Defeats the purpose of async.  Use it only for `std::fs` operations that have no async equivalent. |

`unwrap()` is **allowed** in test code (it is the idiomatic way to surface test failures) and in `main.rs` at the very top of `fn main` (where panicking on a programmer error is acceptable).

---

## 8. CI gates summary

| Gate | Command | Source | Failure mode |
|------|---------|--------|--------------|
| Format | `cargo fmt --all -- --check` | `rustfmt.toml` | CI red, `cargo fmt` to fix |
| Lint | `cargo clippy --all-targets -- -D warnings` | `clippy.toml` | CI red |
| Doc | `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps` | `rustdoc` | CI red |
| File size | `scripts/check-file-size.sh` | new in v0.4.0 | CI red, lists files over cap |
| Test ratio | `scripts/check-coverage.sh` | new in v0.4.0 | CI red, lists modules under 0.6 |
| 3-agent review | manual + claude-code + codex + opencode | ADR-0010 | PR cannot merge until each reviewer signs off |

The new v0.4.0 scripts (`check-file-size.sh`, `check-coverage.sh`) are added in PR-A alongside the `OrchestratorContext: Clone` refactor.  Until those PRs land, layers 4 and 6 are advisory: a 3-agent review can still reject a PR for being too large or under-tested, but CI will not fail.

---

## 9. Migration from v0.3.x to v0.4.0

The existing code is **not** retroactively refactored to match this document.  Instead:

1. **Every new file** added from v0.4.0 onward follows the document.
2. **Every refactor** of an existing file uses the refactor as an opportunity to split the file if it is over the cap.
3. The `scripts/check-file-size.sh` script has an allow-list of legacy files (e.g. `src/main.rs`, `src/config/mod.rs`) with their current sizes as the cap.  Each refactor that splits one of these legacy files removes the corresponding allow-list entry.

The allow-list lives in `scripts/check-file-size.sh` and is updated only by the PR that does the refactor.  It is a one-line YAML.

Example allow-list entry:

```yaml
legacy_overrides:
  - path: src/main.rs
    current_loc: 7682
    target_loc: 2500
    tracked_in: ADR-0010 §PR-A
```

A PR that splits `main.rs` to under 2 500 LOC removes the entry.

---

## 10. References

- ADR-0008-rev1: Adopt Gemini 3.5 Live Translate — `/docs/research/cloud-streaming-2026/adr/0008-rev1-adopt-gemini-live-translate.md`
- ADR-0010: Wire the cloud streaming branch — `/docs/research/cloud-streaming-2026/adr/0010-wire-cloud-into-pipeline.md` (uses this document as its coding standard)
- `rustfmt.toml`, `clippy.toml` at the repo root
- `CONTRIBUTING.md` (the contributor-facing summary; this document is the developer-facing reference)
- Rust API guidelines: <https://rust-lang.github.io/api-guidelines/>
- "Cognitive complexity" definition: <https://www.sonarsource.com/docs/CognitiveComplexity.pdf>
