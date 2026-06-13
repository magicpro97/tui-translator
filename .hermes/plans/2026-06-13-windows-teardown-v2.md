# Plan v2: STATUS_ACCESS_VIOLATION still fires on hosted `windows-latest` after PR #755 merged

## Context — what we know

PR #755 (`ComApartmentGuard` RAII) merged to main. Self-hosted runner (`LINHPC`,
Windows 11) shows `cargo test --bins` PASS in 1m33s with the fix applied. But
hosted `windows-latest` (Windows Server 2022 image) shows:

```
$ cargo test --bins -- --nocapture --skip real_api
   Compiling tui-translator v0.1.19
   ...
test result: ok. 158 passed; 0 failed; ...
   ... (8 more binary test result lines, all pass) ...
test result: ok. 47 passed; 0 failed; ...
   Running unittests src\main.rs (target\debug\deps\tui_translator-f8696b22d51aca0f.exe)
error: test failed, to rerun pass `--bin tui-translator`
  process didn't exit successfully:
    `...tui_translator-f8696b22d51aca0f.exe --nocapture --skip real_api`
    (exit code: 0xc0000005, STATUS_ACCESS_VIOLATION)
```

All 9 prior test binaries (audio_stability_proof, eval_session, frame_pacing_bench,
llm_mt_bench, mt_bench, provider_benchmark, qa8_slo_gate_checker, quality_benchmark,
run_soak) ran to completion. The crash happens in the 10th binary, which is the
`src/main.rs` test binary — the largest one, with 221 `#[test]` functions. The
crash is at process exit, after all tests inside that binary have passed.

PR #756 (revert `continue-on-error: true`) is now BLOCKING because it would
uncover this crash. We must not merge it until the crash is fixed or we know
the crash is hosted-runner-specific and not reproducible on real machines.

## Hypotheses to verify

H1. **COM ref count is still off by one somewhere.** A test in `main.rs` calls
    a code path that ends up in a thread where `CoInitializeEx` is never paired
    with `CoUninitialize`. The 9 production guard call sites + 3 test sites
    covered in PR #755 do not exhaust the set. The cross-apartment Release
    problem in `device_watchdog` was handled with `leak()` but other COM-using
    modules (e.g. `tracing-subscriber`, `tokio` IO driver, `cpal`) might
    create COM objects via paths we did not wrap.

H2. **Test count differs.** Self-hosted and hosted runners may run different
    numbers of tests because of platform-conditional `#[cfg(test)]` or
    `--all-features` gates. More tests → more chances of a teardown race.

H3. **Hosted runner Windows image is different.** `windows-latest` is
    `windows-2022` (Server 2022 image). Self-hosted is Windows 11. Server
    2022 may have different default behavior for `CoInitializeEx` cleanup
    ordering relative to thread exit, or for `IOCP` completion thread
    teardown (which `mio` / `tokio` use internally).

H4. **A `tokio::runtime::Runtime` built with `.enable_all()` registers
    console signal handlers and `SetConsoleCtrlHandler` on Windows.**
    When the runtime drops at the end of a test, it unregisters the handler.
    If the host's `windows-2022` runner already installed a similar handler
    (e.g. via the GitHub Actions runner process), the unregister races
    with the host's signal delivery and segfaults.

H5. **Test runner thread-pool teardown.** Cargo test by default uses all
    CPU cores via rayon-style thread pool for running tests in parallel.
    On a 4-core hosted runner, multiple threads can be simultaneously
    in the middle of a test, and one of them drops a `DeviceWatchdog`
    whose `WatchdogInner::drop` does the cross-apartment Release while
    another thread is initializing the same watchdog. The race was
    masked by `--test-threads=1`.

## Steps

### Step 1 — Reproduce on self-hosted
Add a job to `.github/workflows/windows-selfhosted-test.yml` that runs ONLY
`src/main.rs` as a test binary, in debug mode, with the same flags as
`ci.yml`'s hosted Windows `Cross-platform build (windows-latest, default)`
job. This is the same binary that crashed on hosted, and the same
configuration self-hosted uses today.

```yaml
test-tui-translator-binary:
  name: Test (tui-translator binary, debug)
  runs-on: self-hosted
  steps:
    - uses: actions/checkout@v4
    - name: cargo test --bin tui-translator (debug)
      shell: pwsh
      run: |
        $env:RUST_BACKTRACE = "1"
        cargo test --bin tui-translator -- --nocapture --skip real_api
```

If this passes on self-hosted → H2/H3 confirmed; the bug is environment-specific.
If this fails on self-hosted → H1 confirmed; we have a missing COM call site to
find.

### Step 2 — Hunt missing COM init
If Step 1 fails, run the test with `RUST_BACKTRACE=full` and check whether
the last test that ran is the one we suspect. Search the code for any
`wasapi`, `IMMDevice`, `MMDevice`, `CoCreateInstance`, `SetConsoleCtrlHandler`,
`tokio::runtime::Builder::new_current_thread`, `tokio::task::spawn_blocking`
that the `ComApartmentGuard` does not already cover. Add the guard.

Most likely candidate to check: `tokio::sync::watch::channel` is called from
`AppState::new()` at line 1985 of `src/main.rs`. The watch channel is
created on the main test thread without a guard. Tokio itself does not
call `CoInitializeEx`, but Tokio's IO driver may dispatch a control
operation to a worker thread that opens a completion port. On Server
2022 the completion port teardown order is different from Windows 11.

### Step 3 — Once a missing call site is found
Apply the same `ComApartmentGuard::enter()` pattern. Push, run the
self-hosted job, wait for green, push to hosted CI.

### Step 4 — Verify on hosted
Cancel the in-progress hosted run for PR #756. Push the new fix. Watch
the hosted `Cross-platform build (windows-latest, default)` job
specifically. It must complete with exit code 0, no `0xc0000005`.

### Step 5 — Merge PR #756
Only after Step 4 passes. PR #756 removes `continue-on-error: true` from
3 hosted-CI jobs. Self-hosted runner will continue to pass (it has no
`continue-on-error` flag on the new self-hosted workflow).

## Failure modes and rollback

- If H1 turns out to be a `tokio` signal-handler race, the fix may
  require upgrading `tokio` to a version that handles Windows console
  handler teardown correctly. That is out of scope for this PR; open a
  follow-up issue.
- If H3 (image-specific) is the only thing that makes hosted crash,
  the right answer is to keep the `--test-threads=1` flag in `ci.yml`
  for the affected matrix jobs (the ones that were also tested with
  the new self-hosted `LINHPC` runner). That reverts the revert
  partially: the self-hosted fix is real, the hosted-side fail-soft
  remains.
- If we cannot get a clean green on hosted within 2-3 iterations, we
  should hold PR #756 and document the hosted-only failure as a known
  issue, then re-evaluate in a follow-up.

## What I will NOT do

- I will not add a `tokio` panic catch-all to the runtime.
- I will not add `SetProcessShutdownParameters` calls.
- I will not work around the crash by setting `KEEP_COMPARTMENT=1` env
  var or similar hacks.
- I will not disable the failing test; the rule from the user is
  "fix the source, do not skip the test".
