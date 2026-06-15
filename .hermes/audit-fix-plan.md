# tui-translator Audit-Fix Plan

Source: 4-CLI white-box audit (claude-code, codex, opencode, copilot)
via 9router `trt/MiniMax-M3`. 14 raw findings → 7 distinct user
stories, 7 issues, 7 PRs (1 file per PR per user convention).

## Issues (GitHub)

| # | Issue | Area | Pri | Level | Files | Confidence |
|---|-------|------|-----|-------|-------|------------|
| #797 | zip-slip path traversal | providers,local | P0 | atomic | model_download_archive.rs | 0.95 |
| #798 | Windows reserved-name (epic) | session,audio | P0 | epic | recorder.rs, archive.rs, recorder_writer.rs, storage/mod.rs | 0.97 |
| #799 | prune_session_dirs over-broad | session | P0 | atomic | recorder_writer.rs | 0.90 |
| #800 | sentence_aggregator UTF-8 | audio | P1 | atomic | sentence_aggregator.rs | 0.80 |
| #801 | is_valid_path doc-drift | session,docs | P2 | atomic | recorder_writer.rs, storage/mod.rs | 0.95 |
| #802 | current_segment_bytes init | session | P2 | atomic | recorder_writer.rs | 0.85 |
| #803 | epoch_secs_to_ymd range | session | P3 | atomic | storage/mod.rs | 0.90 |

## Branch strategy

`main` (clean) → 7 fix branches → squash-merge back to `main` after
each PR is green.

Branch name pattern: `audit/<issue#>-<slug>`

## PR execution order (P0 first, P3 last)

### Phase 0 — prep
- [ ] Verify `cargo build` green on `main`
- [ ] Verify `cargo test` green on `main`
- [ ] Verify `cargo clippy --all-targets -- -D warnings` clean
- [ ] Verify `cargo fmt --check` clean
- [ ] Baseline coverage: `cargo llvm-cov --html` → save to
      `.hermes/coverage/baseline/`

### Phase 1 — #797 zip-slip (P0 atomic)
- [ ] Branch: `audit/797-zip-slip`
- [ ] Edit: `src/providers/local/model_download_archive.rs`
  - Replace `target.parent().canonicalize().unwrap_or_else(...)` with
    `fs::create_dir_all(target.parent())` THEN `canonicalize()`.
  - Use `Path::starts_with` on the **canonicalized** `dest_dir`.
- [ ] Tests: add `rejects_path_traversal` and
  `accepts_valid_archive` unit tests with `.tar.bz2` fixtures.
- [ ] Verify: `cargo test -p tui-translator --lib providers::local`
- [ ] Per-file 100% coverage on `model_download_archive.rs`
- [ ] PR: body includes `Confidence: 0.95` and
  `### Opus review evidence` section
- [ ] Squash-merge
- [ ] Close #797 with PR link

### Phase 2 — #799 prune_session_dirs (P0 atomic)
- [ ] Branch: `audit/799-prune-session-dirs`
- [ ] Edit: `src/session/recorder_writer.rs:391`
  - Check for session marker (e.g. `meta.json` with version field)
    before counting a subdir as a session.
  - Skip non-session dirs with a debug log.
- [ ] Tests: add `prune_skips_non_session_dir` and
  `prune_deletes_old_sessions`.
- [ ] Per-file 100% coverage
- [ ] PR + squash-merge → close #799

### Phase 3 — #798 Windows reserved-name epic (P0 epic, 4 sites)
This is the biggest fix. Split into 2 sub-PRs to keep per-file
coverage gates passing:
- [ ] Sub-PR A: `audit/798a-reserved-helper`
  - Edit: `src/storage/mod.rs` — add `is_reserved_device_name(s) ->
    bool` (case-insensitive on Windows), with full unit tests.
  - Per-file 100% on `storage/mod.rs`
  - PR + squash-merge
- [ ] Sub-PR B: `audit/798b-reserved-callers`
  - Edit: `src/session/recorder.rs:166` (use new helper)
  - Edit: `src/audio/archive.rs:225` (use new helper)
  - Edit: `src/session/recorder_writer.rs:281` (use new helper;
    remove the local `is_valid_path_component` mirror in favour
    of `storage::validate_path_component` — overlaps with #801)
  - Per-file 100% on each touched file
  - PR + squash-merge → close #798

### Phase 4 — #800 sentence_aggregator (P1 atomic)
- [ ] Branch: `audit/800-sentence-aggregator`
- [ ] Edit: `src/pipeline/sentence_aggregator.rs:158, :165`
  - Guard concatenation on `!held.text.is_empty() && !text.is_empty()`.
  - Verify the concat result is valid UTF-8; skip emission if not.
- [ ] Tests: 4 unit tests (see issue body)
- [ ] Per-file 100%
- [ ] PR + squash-merge → close #800

### Phase 5 — P2 (#801, #802)
- [ ] Branch: `audit/801-path-drift`
  - Edit: delete local mirror in `recorder_writer.rs`, route
    callers to `storage::validate_path_component`.
  - Edit: `storage/mod.rs:76` to actually reject `\` and `:` on
    Windows.
  - Per-file 100% on both
  - PR + squash-merge → close #801
- [ ] Branch: `audit/802-segment-bytes`
  - Edit: `recorder_writer.rs:73` → init to 0.
  - Test: assert `current_segment_bytes == N` after one write.
  - Per-file 100%
  - PR + squash-merge → close #802

### Phase 6 — #803 epoch_secs_to_ymd (P3)
- [ ] Branch: `audit/803-epoch-range`
  - Edit: `storage/mod.rs:508` → use
    `time::OffsetDateTime::from_unix_timestamp` (full i64 range).
  - Tests: `0`, known date, `-1` pre-1970, `i64::MAX` boundary.
  - Per-file 100%
  - PR + squash-merge → close #803

### Phase 7 — final verify
- [ ] Re-run 4 full audits (claude-code, codex, opencode, copilot)
  on `main` post-fix.
- [ ] Re-extract findings → expect 0 high, 0 medium, 0 low from
  the 7 clusters above.
- [ ] Re-post leaderboard verbose to Discord.
- [ ] Re-baseline coverage: `cargo llvm-cov --html` → save to
  `.hermes/coverage/post-fix/`
- [ ] Diff: confirm no per-file coverage regression.

## Status update protocol (per memory rule)

After each PR squash-merge:
1. `gh issue close <N> --comment "Fixed in #<PR>"`
2. `gh pr comment <PR> --body "Closes #<issue>"`
3. Update this plan file: tick the checkbox, change status to
   `done` for that issue.
4. Post Discord summary: "✅ #<issue> closed: <title>"

## Per-PR verification commands (run before push)

```
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test -p tui-translator --lib <module>::<file>
cargo llvm-cov --html --no-run -- --include-pattern 'src/<file>.rs'
```

## Out of scope (deferred)

- Multi-archive-format zip-slip (`.zip`, `.tar.gz`)
- Retention policy for sessions
- Recursive scan in `prune_session_dirs`
- Time-zone handling in `epoch_secs_to_ymd`
