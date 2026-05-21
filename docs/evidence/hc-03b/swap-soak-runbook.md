# HC-03B Swap Soak Runbook

**Issue:** [#436 feat(hc-03-b): wire live capture hot-swap into orchestrator](https://github.com/magicpro97/tui-translator/issues/436)
**Branch:** `feat/hc-03b-live-capture-hot-swap`
**Author:** Copilot

---

## 1. What was implemented

`src/audio/router.rs` — new `CaptureRouter` task providing channel indirection
so `run_orchestrator` keeps a fixed `mpsc::Receiver<AudioChunk>` while the
upstream capture source is hot-swapped at runtime without an app restart.

Key design decisions:
| Decision | Value |
|---|---|
| `ROUTER_CHANNEL_CAPACITY` | 64 frames |
| `DRAIN_TIMEOUT_MS` | 200 ms |
| biased select (swap first) | prevents swap starvation during high-throughput audio |
| archive-before-orchestrator | `forward_chunk` writes to archive before `try_send` |
| graceful shutdown drain | handle drop triggers `recv().await` loop to flush in-flight chunks |

---

## 2. CI evidence (automated, run locally)

```
cargo fmt --all -- --check     → clean (exit 0)
cargo clippy --all-targets -- -D warnings  → clean (exit 0)
cargo test --test hc03b_live_capture_swap  → 133 passed; 0 failed (exit 0)
```

### Test coverage summary

| Test | Type | Gate |
|---|---|---|
| `hc_03b_router_delivers_initial_stream` | unit (tokio) | chunks forwarded from initial stream |
| `hc_03b_handle_metrics_initially_zero` | unit | metrics start at 0 |
| `hc_03b_no_upstream_router_stays_alive_until_handle_dropped` | unit | NoUpstream state is stable |
| `hc_03b_downstream_rx_stays_open_after_failed_swap` | unit | receiver survives error |
| `hc_03b_swap_count_increments_on_successful_file_swap` | unit (fixture-gated) | swap_count++ |
| `hc_03b_router_delivers_initial_stream_chunks` | integration | E2E forward |
| `hc_03b_orchestrator_rx_stable_across_failed_swap` | integration | receiver open after error |
| `hc_03b_swap_count_reflects_successful_swap` | integration (fixture-gated) | fixture-gated |
| `hc_03b_source_a_to_b_switch` | integration (fixture-gated) | A→B hot-swap with fixed rx |
| `hc_03b_no_upstream_transitions_on_closed_initial_stream` | integration | NoUpstream + bad swap |
| `hc_03b_dropped_during_swap_metric_accessible` | integration | dropped_during_swap = 0 |
| `hc_03b_handle_clone_shares_metrics` | integration | Arc metrics shared |
| `hc_03b_router_metrics_start_at_zero` | unit (no runtime) | atomic init |
| `hc_03b_router_metrics_drops_accumulate` | unit (no runtime) | accumulation |
| `hc_03b_source_spec_labels` | unit (no runtime) | CaptureSourceSpec::label() |
| `hc_03b_channel_capacity_in_range` | const assert | 32 ≤ capacity ≤ 512 |

---

## 3. Soak test procedure (manual, requires real WASAPI device)

> ⚠️ **Tool unavailable in CI**: live WASAPI loopback swap cannot be performed
> in CI runners because no audio render device is present.  The commands below
> are for manual validation on a developer machine with VB-Cable or equivalent
> virtual device installed.

### 3a. Start the app

```powershell
cargo run --release
```

Wait until the TUI shows the subtitle panel and "Capturing from: <device>" in
the status bar.

### 3b. Trigger a hot-swap via `config.json` edit

While the app is running, open `config.json` in a separate editor and change
`capture_device` from the current value (or `null`) to a different device name
(e.g., `"CABLE Output (VB-Audio Virtual Cable)"`).  Save the file.

The app's config-watcher detects the change and calls
`CaptureRouterHandle::hot_swap(CaptureSourceSpec::Wasapi { device: Some("...") }, ...)`.

### 3c. Expected behaviour

1. The subtitle panel continues to render (no black-screen restart).
2. The status bar shows the new device name within ≤ 500 ms.
3. `RouterMetrics::swap_count()` increments exactly once per swap.
4. `RouterMetrics::dropped_during_swap()` stays 0 for low-latency paths
   (may increment under extreme CPU load — acceptable per spec).

### 3d. 30-minute soak

Run the app for 30 minutes while issuing a hot-swap every 5 minutes (6 total).
Check for:
- No crash / panic in the terminal
- `dropped_during_swap` ≤ 10 per swap cycle (spec: bounded, not zero)
- RSS growth ≤ 20 MB over the soak (no unbounded channel growth)

---

## 4. Borrow-checker design note

`run_router` keeps `current: Option<mpsc::Receiver<AudioChunk>>` as a **local
variable**, not a struct field.  The `poll_action` async fn takes independent
`&mut` references to `swap_rx` and `current`.  When `poll_action` returns an
owned `RouterAction`, all borrows are released and the caller can freely mutate
both.  This is the canonical solution to the tokio `select!` borrow-conflict
pattern.

---

## 5. Remaining limitations

- Live WASAPI device swap requires an actual audio render device — not testable
  in CI.  Covered by the manual soak procedure above.
- `silence_threshold` is per-swap (passed in `SwapRequest`); the initial stream's
  threshold comes from `start_router`'s parameter which is currently unused
  (`_silence_threshold`).  Future work: apply threshold to the initial stream too.
