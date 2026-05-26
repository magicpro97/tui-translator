//! QA8-07 cancellation telemetry wiring (Refs #505).
//!
//! Asserts that `pipeline::cancellation_hook::{issue, exit}` —
//! invoked from the real cancel/shutdown sites in `main.rs` and the
//! orchestrator loop — correctly forward into
//! `metrics::backpressure::emit::{cancellation_issue,
//! cancellation_exit}` and produce the expected counter / histogram
//! deltas on the active `BackpressureTelemetry` instance.
//!
//! The hook delegates use `OnceLock`, so all tests in this file share
//! a single installation of the production delegates and serialise
//! through `serial_lock()` to avoid racing the global emit slot.

#![allow(clippy::needless_return)]
#![allow(unused_imports)]

use std::sync::{Arc, Mutex, OnceLock};
use std::thread::sleep;
use std::time::Duration;

// Tests under `tests/` are separate compilation units; mirror the
// `#[path]` pattern used by `qa8_07_live_wiring.rs` to access
// internal modules.
#[path = "../src/pipeline/cancellation_hook.rs"]
mod cancellation_hook;
#[path = "../src/metrics/mod.rs"]
mod metrics;

use metrics::backpressure::{emit, BackpressureTelemetry, BackpressureThresholds};

/// Mutex that serialises every test in this file. The hook delegate
/// slots are `OnceLock`-backed and may only accept one installer per
/// process; each test below installs the *production* delegates so
/// the second installer call is a harmless no-op rather than a
/// silent override.
fn serial_lock() -> &'static Mutex<()> {
    static M: OnceLock<Mutex<()>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(()))
}

/// Install the production delegates (the same ones `main.rs` wires).
/// Idempotent on subsequent calls thanks to `OnceLock`.
fn install_production_delegates() {
    cancellation_hook::install_issue(emit::cancellation_issue);
    cancellation_hook::install_exit(emit::cancellation_exit);
    cancellation_hook::install_monotonic_now_ns(emit::monotonic_now_ns);
}

#[test]
fn issue_then_exit_records_cancellation_telemetry_via_emit() {
    let _g = serial_lock().lock().unwrap_or_else(|p| p.into_inner());
    install_production_delegates();

    let emit_guard = emit::test_lock().lock().unwrap_or_else(|p| p.into_inner());
    let telemetry = Arc::new(BackpressureTelemetry::new());
    emit::install(Arc::clone(&telemetry));

    cancellation_hook::__reset_for_tests();
    cancellation_hook::issue();
    // Two observers — slot A and slot B in production. Sleep between
    // observations so the histogram receives strictly positive
    // latencies from the real monotonic clock.
    sleep(Duration::from_millis(2));
    cancellation_hook::exit();
    sleep(Duration::from_millis(2));
    cancellation_hook::exit();

    assert_eq!(
        telemetry.cancellation.issued(),
        1,
        "cancellation_issue must increment once per issuance"
    );
    assert_eq!(
        telemetry.cancellation.observed(),
        2,
        "both orchestrator observers must record an exit"
    );
    assert_eq!(
        telemetry.cancellation.histogram().count(),
        2,
        "cancellation latency histogram must record each observed exit"
    );

    // The snapshot JSON must surface the same numbers so QA8-05
    // (issue #503) sees the cancellation section populated.
    let snap = telemetry.snapshot_json(BackpressureThresholds::default());
    let cancel = snap.get("cancellation").expect("cancellation section");
    assert_eq!(cancel.get("issued").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(cancel.get("observed").and_then(|v| v.as_u64()), Some(2));
    assert_eq!(
        cancel
            .get("latency")
            .and_then(|l| l.get("count"))
            .and_then(|v| v.as_u64()),
        Some(2),
        "snapshot latency histogram count must match observed exits"
    );

    emit::uninstall();
    cancellation_hook::__reset_for_tests();
    drop(emit_guard);
}

#[test]
fn exit_without_issue_is_a_noop_on_telemetry() {
    let _g = serial_lock().lock().unwrap_or_else(|p| p.into_inner());
    install_production_delegates();

    let emit_guard = emit::test_lock().lock().unwrap_or_else(|p| p.into_inner());
    let telemetry = Arc::new(BackpressureTelemetry::new());
    emit::install(Arc::clone(&telemetry));

    // Reset the issue-timestamp slot so the next `exit()` is treated
    // as "no cancellation in flight" — covers the natural channel-
    // close branch where the orchestrator loop exits because
    // `audio_rx.recv()` returned `None` rather than a true cancel.
    cancellation_hook::__reset_for_tests();
    cancellation_hook::exit();
    cancellation_hook::exit();

    assert_eq!(
        telemetry.cancellation.issued(),
        0,
        "no issue() ⇒ no issuance counter increment"
    );
    assert_eq!(
        telemetry.cancellation.observed(),
        0,
        "exit() must short-circuit when no cancellation is in flight"
    );
    assert_eq!(
        telemetry.cancellation.histogram().count(),
        0,
        "no spurious latency samples on natural shutdown"
    );

    emit::uninstall();
    drop(emit_guard);
}

#[test]
fn second_issuance_resets_latency_baseline() {
    let _g = serial_lock().lock().unwrap_or_else(|p| p.into_inner());
    install_production_delegates();

    let emit_guard = emit::test_lock().lock().unwrap_or_else(|p| p.into_inner());
    let telemetry = Arc::new(BackpressureTelemetry::new());
    emit::install(Arc::clone(&telemetry));

    cancellation_hook::__reset_for_tests();
    cancellation_hook::issue();
    sleep(Duration::from_millis(1));
    cancellation_hook::exit();

    // Simulated session 2: a fresh issuance updates the baseline so
    // the next exit's latency is measured from the new issuance, not
    // from a stale timestamp left in the static slot.
    cancellation_hook::issue();
    sleep(Duration::from_millis(1));
    cancellation_hook::exit();

    assert_eq!(telemetry.cancellation.issued(), 2);
    assert_eq!(telemetry.cancellation.observed(), 2);
    assert_eq!(telemetry.cancellation.histogram().count(), 2);

    emit::uninstall();
    cancellation_hook::__reset_for_tests();
    drop(emit_guard);
}
