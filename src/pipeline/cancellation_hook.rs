//! Indirection layer between pipeline cancel/shutdown sites and the
//! QA8-07 backpressure cancellation-latency counters
//! (`metrics::backpressure::emit::cancellation_issue` /
//! `cancellation_exit`).
//!
//! See issue #505 (QA8-07). The orchestrator runs as a long-lived
//! Tokio task; "cancellation" in the QA8-07 sense is the graceful
//! shutdown handshake described in `pipeline::mod` — the caller flips
//! `OrchestratorContext::shutdown` and the orchestrator loop observes
//! it and breaks. This hook records:
//!
//! * `issue()` — invoked by `main.rs` immediately after it stores
//!   `true` into `orchestrator_shutdown`. Captures a monotonic
//!   timestamp into a process-static `AtomicU64` and forwards to the
//!   installed `cancellation_issue` delegate (no-op if unset).
//! * `exit()` — invoked by each orchestrator instance when it
//!   observes the shutdown flag and is about to break out of its
//!   loop. Computes the elapsed latency against the stored timestamp
//!   and forwards to the installed `cancellation_exit` delegate.
//!
//! The wiring is intentionally narrow (no broad refactor): the
//! orchestrator already polls `ctx.shutdown` on every iteration, and
//! `main.rs` already centralises the shutdown signal. A static
//! timestamp slot is sufficient because there is exactly one
//! cancellation issuance per process; multiple orchestrator slots
//! (A / B) each emit one exit so the cancellation histogram records
//! both per-slot latencies.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

static ISSUE: OnceLock<fn()> = OnceLock::new();
static EXIT: OnceLock<fn(u64)> = OnceLock::new();
static MONO_NS: OnceLock<fn() -> u64> = OnceLock::new();

/// Timestamp (monotonic ns) at which the most recent cancellation was
/// issued. Zero means "no cancellation in flight" — `exit()` then
/// short-circuits so non-cancellation orchestrator exits (audio
/// channel close in tests, etc.) do not emit a spurious latency.
static ISSUE_AT_NS: AtomicU64 = AtomicU64::new(0);

/// Install the delegate invoked on cancellation issuance.
pub fn install_issue(f: fn()) {
    let _ = ISSUE.set(f);
}

/// Install the delegate invoked on observed cancellation exit. The
/// argument is the elapsed nanoseconds since `issue()`.
pub fn install_exit(f: fn(u64)) {
    let _ = EXIT.set(f);
}

/// Install the monotonic-ns clock used to time the cancellation
/// handshake. Production wiring uses
/// `metrics::backpressure::emit::monotonic_now_ns`.
pub fn install_monotonic_now_ns(f: fn() -> u64) {
    let _ = MONO_NS.set(f);
}

/// Record that a cancellation/shutdown signal was issued. Safe to
/// call when no delegate has been installed (no-op fallback).
#[inline]
pub fn issue() {
    if let Some(now) = MONO_NS.get() {
        ISSUE_AT_NS.store(now(), Ordering::Relaxed);
    }
    if let Some(f) = ISSUE.get() {
        f();
    }
}

/// Record that a cancellation/shutdown was observed by a task. The
/// elapsed latency is measured against the timestamp stored by
/// `issue()`. If `issue()` was never called (or the timestamp slot
/// is unavailable) this is a no-op so we never emit a spurious
/// latency for natural channel-close exits.
#[inline]
pub fn exit() {
    let t0 = ISSUE_AT_NS.load(Ordering::Relaxed);
    if t0 == 0 {
        return;
    }
    let latency_ns = match MONO_NS.get() {
        Some(now) => now().saturating_sub(t0),
        None => 0,
    };
    if let Some(f) = EXIT.get() {
        f(latency_ns);
    }
}

/// Clear the in-flight cancellation timestamp. Test-only — production
/// code never resets the slot because the process exits shortly after
/// the cancellation handshake completes.
#[doc(hidden)]
pub fn __reset_for_tests() {
    ISSUE_AT_NS.store(0, Ordering::Relaxed);
}

// Note: integration coverage for this module lives in
// `tests/qa8_07_cancellation_emit.rs`. Unit tests deliberately
// avoided here because the file is `#[path]`-included by that
// integration test, and inner `#[test]` items would claim the
// `OnceLock` delegate slots before the integration test could
// install the production delegates.
