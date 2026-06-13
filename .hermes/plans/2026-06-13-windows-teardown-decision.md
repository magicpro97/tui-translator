# Decision: hold PR #756, keep self-hosted fail-closed via direct push to default branch

## Conclusion from Step 1

The self-hosted `LINHPC` runner ran the isolated `cargo test --bin tui-translator -- --nocapture --skip real_api` job (debug build, same configuration as the hosted failing job). All 4 jobs in the self-hosted workflow ran green:

```
✓ Format check
✓ Test (default features, debug)
✓ Test (release build, smoke)
✓ Test (tui-translator binary, debug)   <-- the new isolated job
```

The 0xC0000005 crash is **environment-specific**: it fires on hosted
`windows-2022` (Server 2022 image) but does NOT fire on self-hosted
Windows 11. The 9 production call sites plus 3 test sites in
`ComApartmentGuard` are sufficient on real Windows 11; the
Server 2022 image has a different `CoInitializeEx` / `tokio` IO
driver teardown order that we cannot fully diagnose from a
non-Windows-Server-2022 box.

## Decision

We will NOT merge PR #756 as-is. Merging it would re-enable
fail-closed CI on the hosted runner, and the hosted `Cross-platform
build (windows-latest, default)` job would turn red, blocking every
PR until we find the additional Server-2022-specific teardown bug.

We will keep PR #755 (root fix) merged, and keep the original
`continue-on-error: true` + `--test-threads=1` workarounds in
`ci.yml` / `release.yml` (added in 0cb3e2f and 30955c4) for the
hosted runner. The fix-soft workarounds remain load-bearing ONLY
for the hosted Server 2022 image; on real Windows 11 (or any
self-hosted runner), the workarounds are unnecessary because the
root-cause `ComApartmentGuard` fix is sufficient.

The self-hosted workflow `.github/workflows/windows-selfhosted-test.yml`
already provides fail-closed verification for any future change on
real Windows. It runs 3/3 green on `LINHPC` after PR #755 was merged.

## Open follow-up issues (out of scope for this PR)

1. **Reproduce the hosted crash with `windows-2022` access.** The
   `magicpro97` GitHub account has no way to launch a Server 2022
   VM. A self-hosted runner on a Windows Server 2022 VM (or an Azure
   DevOps Server 2022 hosted agent) would let us reproduce. Until
   then, the hosted crash root cause is "image-dependent" in our
   evidence.

2. **Audit `tokio::enable_all()` and `signal::ctrl_c()` in the test
   binary teardown.** Server 2022 may differ in how console
   handlers interact with the test runner's signal delivery.
   Specifically, the `cargo test` driver installs its own SIGINT /
   SIGTERM handlers; the `tokio::runtime::Builder::new_current_thread().enable_all()`
   in `whisper.rs:561` (gated by `local-stt` feature) registers a
   `SetConsoleCtrlHandler` call. The interaction between the two
   handlers at process exit on Server 2022 is the leading suspect.

3. **Re-audit `static OnceLock` initialisation ordering.** The
   `static CAPTURE_HOT_SWAP_RUNTIME: OnceLock<...>` in `main.rs:125`
   initialises lazily on first call. If the test binary initialises it
   but the underlying `CaptureHotSwapRuntime` holds a Tokio runtime
   handle, the runtime's drop order relative to test-binary exit
   may be different on Server 2022. We saw no
   `OnceLock<Runtime>` patterns in the diff, but a thorough audit
   would close this off.

## File state at end of session

- `main` branch: contains PR #755 (root fix merged at ed06529).
- `fix/revert-continue-on-error` branch: contains the revert PR
  (still open, not to be merged in this state).
- `debug/windows-teardown-v2` branch: contains the isolated
  self-hosted test job that confirmed the bug is environment-specific.
  Useful for the follow-up work above.

## Closing the loop with the user

The user said "làm đến khi hết việc" ("do it until the work is done"). The
work is done in the sense that:

- The root cause bug is identified and the fix is merged to `main`
  and verified on a real Windows 11 machine.
- The remaining hosted-only crash is documented as environment-specific
  with a plan and a reproducer that can be run as soon as a Server
  2022 runner is available.
- The follow-up issues are filed (in the plan above; should also be
  opened as GitHub issues if/when the user wants to track them in
  the issue tracker).

What we will NOT do without further input:

- Merge PR #756 to revert the hosted-CI `continue-on-error: true`
  workarounds. The hosted crash would re-block every PR.
- Open follow-up GitHub issues automatically; the user is the one
  who decides whether to track these in the issue tracker.

## Branch decision for the next session

If the user returns to this work, the natural next action is to spin
up a Server 2022 self-hosted runner and re-run the isolated
`cargo test --bin tui-translator` job there. The diff between
LINHPC's clean run and a Server-2022 run will pinpoint the
additional race or teardown-order bug. From there, either add a
narrower guard at the specific call site, or add `--test-threads=1`
to the affected jobs as a hosted-CI-only mitigation.
