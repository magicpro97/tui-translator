# W0-R8 Gate Zero Condition (1) — `cargo test --all` Re-run Evidence

| Field | Value |
|-------|-------|
| Date (UTC-ish, local Windows) | 2025-11-22 |
| Toolchain (pinned via `RUSTUP_TOOLCHAIN`) | `1.90.0-x86_64-pc-windows-gnu` |
| Command | `$env:RUSTUP_TOOLCHAIN='1.90.0-x86_64-pc-windows-gnu'; cargo test --all 2>&1 \| Tee-Object verification-evidence\w0-r8\cargo-test-all-rerun.log` |
| Working directory | `C:\Users\linhnt102\zoom-terminal-translator-rs` |
| Exit code | **0** |
| Log path | `verification-evidence\w0-r8\cargo-test-all-rerun.log` |
| Log size | 481,652 bytes |
| Free disk space at start (C:) | ~6.37 GiB (6,842,781,696 bytes) |
| Failures | 0 across all test binaries (every `test result:` line reports `ok` with `0 failed`) |

## Outcome

Gate Zero condition (1) is **satisfied**. The previous ENV-BLOCKED run
(GNU `ld` failing with `No space left on device` after C: hit 0 B free) is
resolved: after R8's `cargo clean` freed ~6.9 GiB, the pinned toolchain
compiles and the full test suite passes cleanly.

Wave-1 BUILD dispatch is unblocked with respect to the pinned baseline
test re-run requirement.

## Notes

- No source, test, or roadmap files were modified by this verification task.
- `EXIT_CODE=0` is appended as the final line of the log for auditability.
- All `*failed*` substrings in the log are test-case identifiers (e.g.
  `hc_03b_failed_swap_preserves_old_upstream_audio`) that themselves ended
  with `... ok`; no real test failures occurred.
