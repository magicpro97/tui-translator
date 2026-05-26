//! Live emission registry for QA8-07 backpressure telemetry (issue #505).
//!
//! Production code paths (WASAPI capture, fanout, audio sink, provider
//! dispatch via `with_retry`) call the thin no-arg helpers in this
//! module. When a [`BackpressureTelemetry`] instance has been installed
//! into the global slot, the helper forwards to its counters; otherwise
//! every call is a cheap atomic load and an early return.
//!
//! The registry exists so we can wire emission sites without threading
//! `Arc<BackpressureTelemetry>` through every production constructor
//! (the QA8-07 follow-up scope explicitly forbids broad refactors). The
//! slot is interior-mutable: tests use [`install`] and [`uninstall`] to
//! point the slot at a fresh telemetry instance and serialise via
//! [`test_lock`] so they do not race each other.
//!
//! ## Scope and follow-ups
//!
//! Wiring is intentionally narrow: every emission helper is opt-in and
//! the production code falls back to a no-op when no telemetry is
//! installed. The 30-minute calibration soak and the QA8-05 runner
//! consumption that close #505 are explicit follow-ups tracked in the
//! schema's `calibration_pending` field.
//!
//! Note: the `src/bin/*` benchmarks `#[path]`-include subsets of the
//! source tree and do not have `crate::providers` in scope, so we
//! intentionally do not use an intra-doc link for `with_retry` above.

use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Instant;

use super::BackpressureTelemetry;

/// Global slot holding the active telemetry sink, if any. Interior-
/// mutable so test fixtures can swap in a fresh instance and tear it
/// down without leaking state across the process.
fn slot() -> &'static RwLock<Option<Arc<BackpressureTelemetry>>> {
    static S: OnceLock<RwLock<Option<Arc<BackpressureTelemetry>>>> = OnceLock::new();
    S.get_or_init(|| RwLock::new(None))
}

/// Install (or replace) the global telemetry sink. Production code
/// calls this once at startup from `main.rs`.
pub fn install(telemetry: Arc<BackpressureTelemetry>) {
    *slot().write().unwrap_or_else(|p| p.into_inner()) = Some(telemetry);
}

/// Remove the global telemetry sink. Used by test fixtures and by
/// graceful shutdown paths.
pub fn uninstall() {
    *slot().write().unwrap_or_else(|p| p.into_inner()) = None;
}

/// Borrow the active telemetry sink. Returns `None` when no telemetry
/// has been installed.
pub fn try_clone() -> Option<Arc<BackpressureTelemetry>> {
    slot().read().ok().and_then(|g| g.as_ref().map(Arc::clone))
}

/// Mutex shared by tests that mutate the global slot. Lock it before
/// installing/uninstalling so parallel tests do not observe each
/// other's counters.
pub fn test_lock() -> &'static Mutex<()> {
    static M: OnceLock<Mutex<()>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(()))
}

fn with<F: FnOnce(&BackpressureTelemetry)>(f: F) {
    if let Ok(guard) = slot().read() {
        if let Some(t) = guard.as_ref() {
            f(t);
        }
    }
}

// ── Process-monotonic clock ────────────────────────────────────────────────

/// Reference `Instant` captured the first time [`monotonic_now_ns`] is
/// called. The audio-capture telemetry only needs differences between
/// successive observations, so a process-local origin is sufficient.
fn process_start() -> Instant {
    static T0: OnceLock<Instant> = OnceLock::new();
    *T0.get_or_init(Instant::now)
}

/// Nanoseconds since process start on a monotonic clock. Used by the
/// audio-capture wiring so jitter measurements are immune to wall-clock
/// adjustments.
pub fn monotonic_now_ns() -> u64 {
    process_start().elapsed().as_nanos() as u64
}

// ── Emission helpers ───────────────────────────────────────────────────────

/// Record that one audio chunk was produced by the capture path at the
/// given monotonic-ns timestamp.
pub fn audio_chunk_at(now_ns: u64) {
    with(|t| t.audio_capture.record_chunk_at(now_ns));
}

/// Record an explicit capture stall (e.g. WASAPI event wait timeout).
pub fn audio_capture_stall() {
    with(|t| t.audio_capture.record_stall());
}

/// Record one fanout drop on `slot` (0 = slot A, 1 = slot B). The QA8-07
/// schema mirrors fanout drops into the sink section so consumers read
/// a single object.
pub fn fanout_drop(_slot: usize) {
    with(|t| t.sink.record_fanout_drop());
}

/// Record one successful sink write of `bytes` taking `latency_ns`.
pub fn sink_write(bytes: u64, latency_ns: u64) {
    with(|t| t.sink.record_write(bytes, latency_ns));
}

/// Record one sink underrun (writer reported dropped frames or a
/// downstream consumer ran out of data).
pub fn sink_underrun() {
    with(|t| t.sink.record_underrun());
}

/// Provider lifecycle: one request entered the provider queue.
pub fn provider_enqueue() {
    with(|t| t.provider.on_enqueue());
}

/// Provider lifecycle: one request left the queue and is now in flight.
pub fn provider_dequeue_start() {
    with(|t| t.provider.on_dequeue_start());
}

/// Provider lifecycle: one in-flight request finished.
pub fn provider_complete() {
    with(|t| t.provider.on_complete());
}

/// Provider lifecycle: a transient error was retried and eventually
/// succeeded.
pub fn provider_recovered_error() {
    with(|t| t.provider.record_recovered_error());
}

/// Provider lifecycle: retries were exhausted or a permanent error
/// bubbled up.
pub fn provider_permanent_error() {
    with(|t| t.provider.record_permanent_error());
}

/// Cancellation issued — pair with [`cancellation_exit`] when the
/// cancelled task observes its exit, passing the elapsed latency.
pub fn cancellation_issue() {
    with(|t| t.cancellation.record_issue());
}

/// Cancellation completed `latency_ns` after [`cancellation_issue`].
pub fn cancellation_exit(latency_ns: u64) {
    with(|t| t.cancellation.record_exit(latency_ns));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_are_noop_when_nothing_installed() {
        let _guard = test_lock().lock().unwrap_or_else(|p| p.into_inner());
        uninstall();
        // None of these should panic or otherwise observe state.
        audio_chunk_at(monotonic_now_ns());
        audio_capture_stall();
        fanout_drop(0);
        sink_write(128, 1_000_000);
        sink_underrun();
        provider_enqueue();
        provider_dequeue_start();
        provider_complete();
        provider_recovered_error();
        provider_permanent_error();
        cancellation_issue();
        cancellation_exit(1_000);
        assert!(try_clone().is_none());
    }

    #[test]
    fn install_routes_emissions_to_active_telemetry() {
        let _guard = test_lock().lock().unwrap_or_else(|p| p.into_inner());
        let t = Arc::new(BackpressureTelemetry::new());
        install(Arc::clone(&t));
        audio_chunk_at(0);
        audio_chunk_at(2_000_000_000);
        audio_capture_stall();
        fanout_drop(1);
        sink_write(256, 750_000);
        sink_underrun();
        provider_enqueue();
        provider_dequeue_start();
        provider_complete();
        provider_recovered_error();
        provider_permanent_error();
        cancellation_issue();
        cancellation_exit(5_000_000);
        uninstall();

        assert_eq!(t.audio_capture.chunks_seen(), 2);
        // 2 s gap + explicit stall = 2 (gap exceeds default 250 ms).
        assert_eq!(t.audio_capture.stall_count(), 2);
        assert_eq!(t.sink.fanout_drops(), 1);
        assert_eq!(t.sink.writes(), 1);
        assert_eq!(t.sink.bytes_written(), 256);
        assert_eq!(t.sink.underruns(), 1);
        assert_eq!(t.provider.queue_high_water(), 1);
        assert_eq!(t.provider.inflight_high_water(), 1);
        assert_eq!(t.provider.recovered_errors(), 1);
        assert_eq!(t.provider.permanent_errors(), 1);
        assert_eq!(t.cancellation.issued(), 1);
        assert_eq!(t.cancellation.observed(), 1);
    }

    #[test]
    fn monotonic_now_ns_is_monotonic() {
        let a = monotonic_now_ns();
        let b = monotonic_now_ns();
        assert!(b >= a);
    }
}
