# Plan: Fix Windows test teardown STATUS_ACCESS_VIOLATION at root cause

**Status:** DRAFT — under adversarial review. 13+ defects found by cross-debate (claude + codex). Critical findings:
- Plan's "5 call sites" table missed 3 production leak sites (now corrected to 8)
- `ComApartmentGuard` design as drafted has wrong `!Send` claim (corrected)
- Watchdog e474706 retention may be load-bearing and needs re-validation (investigation)
- `--test-threads=1` retention needs justification (corrected)
- `CoInitializeEx` "increments twice" claim was wrong — second call returns `RPC_E_CHANGED_MODE` and is a no-op (corrected)
- PR references were wrong (#723 is the EPIC issue, not a PR; #753 and #754 are the PRs)

## Context — observed symptoms

- 3 GH Actions runs failed on 2026-06-05 (CI on `main` and `v0.1.19`, all push event)
  - Run IDs: `27012586550`, `27012619658`, `27012623704`
  - Failed jobs (identical on all 3 runs):
    1. `Build and test` → step `Run binary unit tests (skip future real_api cases)`
    2. `Lint (clippy)` → step `Run all-features test suite`
    3. `Cross-platform build (windows-latest, default)` → step `Run binary unit tests (skip real_api)`
- Exit code `0xC0000005` (STATUS_ACCESS_VIOLATION) on `windows-latest` hosted runner
- All test assertions inside the binary PASSED — the crash happens during process teardown
- "Tolerate" fix in PR #753 (release.yml) and #754 (ci.yml) — `continue-on-error: true` + `--test-threads=1` — makes CI green again, but does NOT fix the underlying bug
- Bug remains dormant on real Windows machines (per comment) and on debug builds gated to no-op watchdog (e474706)

## Root cause (verified with evidence)

### Claim: `wasapi::initialize_mta()` calls `CoInitializeEx` without a matching `CoUninitialize` anywhere in the codebase.

**Evidence (re-verified after review):**

1. `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wasapi-0.14.0/src/api.rs:67-79`
   ```rust
   pub fn initialize_mta() -> Result<(), windows::core::Error> {
       unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
   }
   pub fn deinitialize() {
       unsafe { CoUninitialize() }
   }
   ```
   Two separate functions; the crate does NOT auto-pair them.

2. `rg "initialize_mta|initialize_sta" --type rust .` returns **8 call sites** (plan originally listed 5; 3 missed):

   **Production code (5):**
   - `src/audio/wasapi_capture.rs:108` — `capture_loop` thread init
   - `src/audio/wasapi_capture.rs:267` — `list_loopback_devices` init (main thread, scoped)
   - `src/pipeline/audio_sink.rs:221` — `OemCableSink::new_windows` init
   - `src/pipeline/audio_sink_f32.rs:46` — `WasapiF32RenderPcmWriter::write_f32_pcm` init
   - `src/audio/device_watchdog.rs:409` — `windows_impl::com_setup` init (release-only after e474706)

   **Test code (3, MISSED IN ORIGINAL PLAN):**
   - `src/pipeline/audio_sink_tests.rs:215-216` — `wasapi_initialize_mta_is_idempotent` test
   - `src/audio/wasapi_capture_tests.rs:225` — `find_render_device_by_name_unknown_returns_err_not_panic` test
   - `tests/wasapi_probe.rs:21` — `initialize_or_skip` helper, called by 4 test functions in the same file
   - `tests/vbcable_f32_format_test.rs:161` — `#[ignore]` test, low impact (only opt-in runs)

3. Call sites for `wasapi::deinitialize()`: **ZERO** (`rg "deinitialize\(\)|CoUninitialize" --type rust .` returns 0 matches).

4. The Microsoft COM contract requires a `CoUninitialize` on the same thread for every successful `CoInitializeEx`. Without it, COM apartment cleanup at thread/process exit is undefined → STATUS_ACCESS_VIOLATION is the documented failure mode.

5. The second `CoInitializeEx` call returns `RPC_E_CHANGED_MODE` (0x80010106) and does NOT increment the ref count per Microsoft docs. So each test thread leaks at most ONE unbalanced `CoInitializeEx`, not two. The plan's original "increments twice" framing was wrong.

### Why the tests are the actual leak

- `wasapi_initialize_mta_is_idempotent` (`audio_sink_tests.rs:212-218`): runs on main test thread, calls `initialize_mta` twice (second is no-op). One ref count leak.
- `find_render_device_by_name_unknown_returns_err_not_panic` (`wasapi_capture_tests.rs:222-242`): runs on main test thread, calls `initialize_mta`, then runs the test body. One ref count leak.
- `wasapi_probe` (`tests/wasapi_probe.rs:14-208`): 4 test functions, each calls `initialize_or_skip()` which calls `initialize_mta`. Each test that hits the Ok branch leaks one ref count.
- These tests all run on the main test thread (in `--test-threads=1` mode) or on parallel test threads (default mode). When the test process exits, COM runtime tries to clean up the apartment and trips on the unbalanced ref count.

### Why fix #753/#754 (continue-on-error) is fail-soft

- The e474706 commit correctly gates the watchdog's real COM `IMMNotificationClient` registration behind `cfg(not(debug_assertions))`, so debug builds no longer leak the COM sink on Drop. Good fix for that specific path.
- The `continue-on-error: true` simply ignores the resulting process exit code.
- The bug is environment-specific: hosted runner shutdown is racy enough that the violation sometimes doesn't fire, sometimes does.
- `--test-threads=1` masks the bug by serialising test execution, reducing the chance of cross-thread COM ref count corruption. The plan must decide: keep it as defense-in-depth, or remove it as no longer needed.

## Proposed fix (revised after review)

### Layer 1 — RAII guard for COM apartment (root fix)

Add a new module `src/audio/windows_com.rs` with:

```rust
//! RAII guard for a per-thread COM apartment.
//!
//! Pairs `wasapi::initialize_mta()` with `wasapi::deinitialize()` so every
//! thread that touches the MMDevice / WASAPI API balances its COM ref count.
//! Without this, process teardown on hosted Windows runners can hit
//! STATUS_ACCESS_VIOLATION (0xC0000005) inside the COM runtime.

#[cfg(windows)]
pub struct ComApartmentGuard {
    // PhantomData<*const ()> makes the type !Send + !Sync, so the borrow
    // checker enforces same-thread lifetime and prevents accidental moves
    // across thread boundaries (which would corrupt the COM ref count).
    _not_send: PhantomData<*const ()>,
}

#[cfg(windows)]
impl ComApartmentGuard {
    /// Initialise the current thread's COM apartment (MTA).
    /// Idempotent: maps `RPC_E_CHANGED_MODE` to Ok because the apartment
    /// already exists on this thread.
    pub fn enter() -> Result<Self, ComError> {
        match wasapi::initialize_mta() {
            Ok(()) => Ok(Self { _not_send: PhantomData }),
            Err(e) if is_rpc_e_changed_mode(&e) => Ok(Self { _not_send: PhantomData }),
            Err(e) => Err(ComError(e)),
        }
    }
}

#[cfg(windows)]
impl Drop for ComApartmentGuard {
    fn drop(&mut self) {
        wasapi::deinitialize();
    }
}
```

Key changes from the original draft:
- `_not_send: PhantomData<*const ()>` (NOT `_private: ()`) — the original draft's `!Send` claim was wrong because `()` is auto-`Send + Sync`.
- Added a separate `ComError` newtype so callers can distinguish guard errors from wasapi errors.

### Layer 2 — Replace 8 call sites (corrected count)

| File | Line | Current | New |
|------|------|---------|-----|
| `src/audio/wasapi_capture.rs` | 108 | `initialize_mta().map_err(...)?;` | `let _com = ComApartmentGuard::enter()?;` |
| `src/audio/wasapi_capture.rs` | 267 | `initialize_mta().map_err(...)?;` | `let _com = ComApartmentGuard::enter()?;` |
| `src/pipeline/audio_sink.rs` | 221 | `wasapi::initialize_mta().ok();` | `let _com = ComApartmentGuard::enter().ok();` |
| `src/pipeline/audio_sink_f32.rs` | 46 | `wasapi::initialize_mta().ok();` | `let _com = ComApartmentGuard::enter().ok();` |
| `src/audio/device_watchdog.rs` | 409 | `initialize_mta().map_err(...)?;` | `let _com = ComApartmentGuard::enter()?;` |
| `src/pipeline/audio_sink_tests.rs` | 215-216 | direct `wasapi::initialize_mta().ok();` × 2 | replace with new guard test (see Layer 3) |
| `src/audio/wasapi_capture_tests.rs` | 225 | `if initialize_mta().is_err() { return; }` | guard pattern, scoped to the test body |
| `tests/wasapi_probe.rs` | 20-28 | `fn initialize_or_skip() { match initialize_mta() { ... } }` | guard pattern returning a Result<Self, _> |
| `tests/vbcable_f32_format_test.rs` | 161 | `initialize_mta().expect("COM MTA init");` | guard pattern, scoped to the test body |

### Layer 3 — Replace idempotency test with guard test

REMOVE: `wasapi_initialize_mta_is_idempotent` (it exercises the raw API and leaks COM on the test thread).

ADD: `com_apartment_guard_balances_refcount` (exercises the guard, which balances ref count via Drop):

```rust
#[cfg(windows)]
#[test]
fn com_apartment_guard_balances_refcount() {
    // First enter initialises COM; second enter must succeed (idempotent,
    // not a ref count underflow).
    {
        let _g1 = ComApartmentGuard::enter().expect("first enter");
        let _g2 = ComApartmentGuard::enter().expect("second enter must be idempotent");
    }
    // Both guards dropped: ref count back to 0; no leak to corrupt teardown.
    // If we had a ref count underflow (deinitialize called more than
    // CoInitializeEx), the next call would fail with COM_E_NOTINITIALIZED,
    // which we exercise below.
    let _g3 = ComApartmentGuard::enter()
        .expect("enter after balanced drop must still work");
}
```

Note: this test only proves the contract, not the implementation's ref count arithmetic. The hosted Windows runner is the actual arbiter of correctness (no crash = ref count balanced). Per the reviewer's feedback, a true ref count test would require `CoIncrementInitData` access which is not in scope.

### Layer 4 — Watchdog e474706 retention — REQUIRES RE-VALIDATION

After review, the watchguard e474706 fix may be **load-bearing** for reasons the plan did not address. Specifically:

- `WatchdogInner { enumerator, sink }` is created on thread A (the `watchdog-event-pump` thread, line 451) inside `com_setup`.
- It is then sent via `init_tx` (line 462) to thread B (the main thread).
- COM objects must be released from the same apartment they were created in (or via GIT/marshaling).
- The current `Drop` for `WatchdogInner` deliberately does nothing (lines 345-358) to avoid the cross-apartment Release teardown crash.
- Adding a guard scoped to `com_setup` (Layer 2 fix to `device_watchdog.rs:409`) means the COM apartment is now torn down at function return — BEFORE the `WatchdogInner` is sent across the channel. This is a STRICTER version of the failure mode the e474706 comment is warning about.

**Mitigation before merging:**
- Keep e474706 in place (gate watchdog to release-only).
- For the new guard at `com_setup`, use a special variant `ComApartmentGuard::leak()` (no Drop) that initialises COM but does NOT uninitialise. This way the apartment stays alive for the lifetime of the process, matching the deliberate leak in `WatchdogInner::Drop`. The apartment leaks anyway because the sink leaks anyway.
- Add a regression test that verifies: spawn a watchdog, send a fake device event, drop the watchdog handle, verify no crash on process exit.

### Layer 5 — Revert `continue-on-error` workarounds

After the guard is in place AND validated on a self-hosted Windows runner, revert the 3 fail-soft lines added in 0cb3e2f and 30955c4 (corrected line numbers):

- `.github/workflows/ci.yml:80` (Lint job `Run all-features test suite` — `continue-on-error: true`)
- `.github/workflows/ci.yml:111` (Build and test job `Run binary unit tests` — `continue-on-error: true`)
- `.github/workflows/ci.yml:842` (Cross-platform build matrix `Run binary unit tests` — `continue-on-error: ${{ matrix.os == 'windows-latest' }}`, the `run:` is on line 843)
- `.github/workflows/release.yml:55-66` (Release safety tests `Run release safety tests` — `continue-on-error: true`)

For `--test-threads=1`:
- The argument was added in the same commits (`ci.yml:88`, `ci.yml:112`).
- Default mode: tests run in parallel, multiple threads may simultaneously call `initialize_mta`, which on Windows returns `RPC_E_CHANGED_MODE` for the 2nd-Nth call. The guard handles this idempotently.
- The plan's hypothesis: with the guard in place, `--test-threads=1` is no longer needed because the guard serialises COM ref count arithmetic at the OS level.
- If self-hosted runner tests pass without `--test-threads=1`, revert it too. If they don't, keep it as defense-in-depth and document why.

### Layer 6 — CHANGELOG entry (per doc-drift checklist)

Add an entry to `CHANGELOG.md` under the next version:
```
- Fixed Windows test process teardown crash (STATUS_ACCESS_VIOLATION 0xC0000005)
  on hosted runners. Root cause: unbalanced COM apartment ref counts from
  `wasapi::initialize_mta()` without matching `CoUninitialize`. New
  `ComApartmentGuard` RAII type pairs the calls.
```

## Out-of-scope (deferred to follow-up)

- `runtime_caps::ActiveLocalInference` and its interaction with COM on the blocking thread pool (whisper-rs/ort may have their own COM usage). Per reviewer feedback, this is a latent issue but not the current bug.
- STA apartment support (no consumer in current code).
- Auditing every `CoCreateInstance` / COM interface holder for symmetric `Release`. The `windows` crate wrappers are RAII, so explicit Release is generally unnecessary.

## Verification plan

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace --bins -- --nocapture --skip real_api` (Linux + macOS — verifies the guard compiles and the non-Windows path is unchanged)
4. `cargo build --release --locked --features release-windows` (sanity build for Windows target)
5. Push branch `fix/windows-com-teardown` to origin
6. GH Actions Windows hosted runner must pass **without** `continue-on-error` AND ideally without `--test-threads=1`
7. If 0xC0000005 still fires: check Drop ordering, watch for static `OnceCell` holding COM pointers, add `parking_lot::Mutex` instead of `std::sync::Mutex` for COM-touching state, consider a self-hosted Windows runner for iterative testing

## Risk & rollback

- Risk: `ComApartmentGuard::leak()` variant for `com_setup` may not be needed — if watchdog registration proves safe with a scoped guard, the leak variant is over-engineering. Mitigation: implement both, choose after first CI run.
- Risk: process exit ordering with `wasapi::deinitialize()` may segfault if the COM runtime is already torn down. Mitigation: only Drop guards in well-scoped blocks (function scope), never in `static` destructors. If a test process needs to NOT call deinitialize, use the leak variant.
- Rollback: revert the 4 commits (guard module, 8 call-site edits, test update, CI revert) in a single PR. The e474706 watchdog gate is independent and remains.

## Open questions for user

1. Module location: `src/audio/windows_com.rs` (audio-adjacent) or `src/windows_com.rs` (top-level since it's plumbing)? Recommend `src/audio/windows_com.rs` because the only consumers are audio-related.
2. Should we add a `#[cfg(windows)]`-gated `static` test that constructs N guards in sequence and verifies the ref count tracks? Probably overkill; the hosted runner is the actual arbiter.
3. Approve the 8 call-site sweep (vs. the original 5)? The 3 newly identified test sites are necessary to actually fix the root cause; without them the fix is incomplete.
4. Approve the `ComApartmentGuard::leak()` variant for the watchdog path? This is the conservative answer to the cross-apartment Release risk.
5. Approve keeping or removing `--test-threads=1` after the fix? Recommend keeping it for the first CI run as defense-in-depth, then removing if the test suite passes without it.
