//! QA8-07 live wiring (Refs #505) — integration tests for the
//! backpressure telemetry hook indirection.
//!
//! The hooks live in `audio::backpressure_hook`, `providers::backpressure_hook`,
//! and `pipeline::backpressure_hook`. Production code under each module
//! calls those hooks unconditionally; `main.rs` installs delegates that
//! forward into `metrics::backpressure::emit::*`. These tests:
//!
//! 1. Cover every branch of `with_retry` (success, recovered, permanent,
//!    exhausted) and assert that the right provider hooks fire.
//! 2. Drive a real `start_fanout` overflow and assert the fanout-drop
//!    hook fires once per dropped chunk.
//! 3. Smoke-test the audio capture / sink hooks via the indirection
//!    layer (the WASAPI loop itself cannot be exercised without
//!    hardware — these helpers are what the loop calls).

#![allow(clippy::needless_return)]
// Tests below hold a `std::sync::Mutex` guard across `await` points to
// serialise mutations of process-global atomic counters. The lock has
// no internal awaits and the runtime is single-threaded per test, so
// this is safe; clippy's general warning does not apply here.
#![allow(clippy::await_holding_lock)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

#[path = "../src/audio/mod.rs"]
mod audio;

#[path = "../src/providers/mod.rs"]
mod providers;

use providers::{with_retry, MtResult, ProviderError};

// ── Atomic counters used as test delegates ────────────────────────────────

static ENQUEUE_CT: AtomicUsize = AtomicUsize::new(0);
static DEQUEUE_CT: AtomicUsize = AtomicUsize::new(0);
static COMPLETE_CT: AtomicUsize = AtomicUsize::new(0);
static RECOVERED_CT: AtomicUsize = AtomicUsize::new(0);
static PERMANENT_CT: AtomicUsize = AtomicUsize::new(0);
static FANOUT_DROP_CT: AtomicUsize = AtomicUsize::new(0);
static AUDIO_CHUNK_CT: AtomicUsize = AtomicUsize::new(0);
static AUDIO_STALL_CT: AtomicUsize = AtomicUsize::new(0);
static SINK_WRITE_CT: AtomicUsize = AtomicUsize::new(0);
static SINK_BYTES: AtomicUsize = AtomicUsize::new(0);
static SINK_UNDERRUN_CT: AtomicUsize = AtomicUsize::new(0);

fn provider_enqueue() {
    ENQUEUE_CT.fetch_add(1, Ordering::SeqCst);
}
fn provider_dequeue() {
    DEQUEUE_CT.fetch_add(1, Ordering::SeqCst);
}
fn provider_complete() {
    COMPLETE_CT.fetch_add(1, Ordering::SeqCst);
}
fn provider_recovered() {
    RECOVERED_CT.fetch_add(1, Ordering::SeqCst);
}
fn provider_permanent() {
    PERMANENT_CT.fetch_add(1, Ordering::SeqCst);
}
fn fanout_drop_d(_slot: usize) {
    FANOUT_DROP_CT.fetch_add(1, Ordering::SeqCst);
}
fn audio_chunk_d(_ns: u64) {
    AUDIO_CHUNK_CT.fetch_add(1, Ordering::SeqCst);
}
fn audio_stall_d() {
    AUDIO_STALL_CT.fetch_add(1, Ordering::SeqCst);
}
fn sink_write_d(bytes: u64, _latency_ns: u64) {
    SINK_WRITE_CT.fetch_add(1, Ordering::SeqCst);
    SINK_BYTES.fetch_add(bytes as usize, Ordering::SeqCst);
}
fn sink_underrun_d() {
    SINK_UNDERRUN_CT.fetch_add(1, Ordering::SeqCst);
}
fn fake_now_ns() -> u64 {
    1_000
}

/// Install hooks exactly once for the whole test binary. `OnceLock`
/// inside each hook module means `install_*` only takes effect the
/// first call; calling from multiple `#[test]` fns is harmless.
fn install_all_hooks() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        audio::backpressure_hook::install_fanout_drop(fanout_drop_d);
        audio::backpressure_hook::install_audio_chunk_at(audio_chunk_d);
        audio::backpressure_hook::install_audio_stall(audio_stall_d);
        audio::backpressure_hook::install_monotonic_now_ns(fake_now_ns);
        providers::backpressure_hook::install_enqueue(provider_enqueue);
        providers::backpressure_hook::install_dequeue_start(provider_dequeue);
        providers::backpressure_hook::install_complete(provider_complete);
        providers::backpressure_hook::install_recovered_error(provider_recovered);
        providers::backpressure_hook::install_permanent_error(provider_permanent);
    });
}

/// `with_retry` tests mutate shared counters and must not race each
/// other. Snapshot counters under this lock before/after each run.
fn provider_test_lock() -> &'static Mutex<()> {
    static M: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    M.get_or_init(|| Mutex::new(()))
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
struct ProviderCounts {
    enqueue: usize,
    dequeue: usize,
    complete: usize,
    recovered: usize,
    permanent: usize,
}

fn snapshot_provider() -> ProviderCounts {
    ProviderCounts {
        enqueue: ENQUEUE_CT.load(Ordering::SeqCst),
        dequeue: DEQUEUE_CT.load(Ordering::SeqCst),
        complete: COMPLETE_CT.load(Ordering::SeqCst),
        recovered: RECOVERED_CT.load(Ordering::SeqCst),
        permanent: PERMANENT_CT.load(Ordering::SeqCst),
    }
}

fn delta(after: ProviderCounts, before: ProviderCounts) -> ProviderCounts {
    ProviderCounts {
        enqueue: after.enqueue - before.enqueue,
        dequeue: after.dequeue - before.dequeue,
        complete: after.complete - before.complete,
        recovered: after.recovered - before.recovered,
        permanent: after.permanent - before.permanent,
    }
}

// ── with_retry coverage ───────────────────────────────────────────────────

#[tokio::test]
async fn with_retry_success_first_try_records_enqueue_and_complete_only() {
    install_all_hooks();
    let _g = provider_test_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let before = snapshot_provider();

    let result: Result<MtResult, ProviderError> = with_retry(|| async {
        Ok(MtResult {
            translated_text: "ok".to_string(),
            detected_source_language: Some("en".into()),
        })
    })
    .await;

    assert!(result.is_ok());
    let d = delta(snapshot_provider(), before);
    assert_eq!(d.enqueue, 1);
    assert_eq!(d.dequeue, 1);
    assert_eq!(d.complete, 1);
    assert_eq!(d.recovered, 0);
    assert_eq!(d.permanent, 0);
}

#[tokio::test]
async fn with_retry_transient_then_success_records_recovered_error() {
    install_all_hooks();
    let _g = provider_test_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let before = snapshot_provider();

    let attempt = std::sync::atomic::AtomicUsize::new(0);
    let result: Result<MtResult, ProviderError> = with_retry(|| async {
        let n = attempt.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            Err(ProviderError::ServiceUnavailable("temporary blip".into()))
        } else {
            Ok(MtResult {
                translated_text: "ok".to_string(),
                detected_source_language: None,
            })
        }
    })
    .await;

    assert!(result.is_ok());
    let d = delta(snapshot_provider(), before);
    assert_eq!(d.enqueue, 1);
    assert_eq!(d.dequeue, 1);
    assert_eq!(d.complete, 1);
    assert_eq!(d.recovered, 1, "transient→Ok must record recovered_error");
    assert_eq!(d.permanent, 0);
}

#[tokio::test]
async fn with_retry_non_transient_error_records_permanent_error_immediately() {
    install_all_hooks();
    let _g = provider_test_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let before = snapshot_provider();

    let attempts = std::sync::atomic::AtomicUsize::new(0);
    let result: Result<MtResult, ProviderError> = with_retry(|| async {
        attempts.fetch_add(1, Ordering::SeqCst);
        Err(ProviderError::InvalidInput("bad request".into()))
    })
    .await;

    assert!(matches!(result, Err(ProviderError::InvalidInput(_))));
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        1,
        "non-transient errors must not be retried"
    );
    let d = delta(snapshot_provider(), before);
    assert_eq!(d.enqueue, 1);
    assert_eq!(d.dequeue, 1);
    assert_eq!(d.complete, 1);
    assert_eq!(d.recovered, 0);
    assert_eq!(d.permanent, 1);
}

#[tokio::test]
async fn with_retry_exhausted_records_permanent_error_not_recovered() {
    install_all_hooks();
    let _g = provider_test_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let before = snapshot_provider();

    // Always fail with a transient error → exhaust retries.
    let result: Result<MtResult, ProviderError> =
        with_retry(|| async { Err(ProviderError::ServiceUnavailable("still down".into())) }).await;

    assert!(matches!(result, Err(ProviderError::ServiceUnavailable(_))));
    let d = delta(snapshot_provider(), before);
    assert_eq!(d.enqueue, 1);
    assert_eq!(d.dequeue, 1);
    assert_eq!(d.complete, 1);
    assert_eq!(d.recovered, 0, "retry exhaustion is NOT a recovered error");
    assert_eq!(d.permanent, 1);
}

// ── fanout drop hook ──────────────────────────────────────────────────────

#[tokio::test]
async fn fanout_overflow_drives_fanout_drop_hook() {
    use audio::fanout::{fanout_loop, FanoutDropCounters, FANOUT_SLOT_CAPACITY, SLOT_B};
    use audio::AudioChunk;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    install_all_hooks();

    // Build a fanout with slot A drained as fast as possible and slot B
    // saturated so the overflow path fires.
    let large_cap = FANOUT_SLOT_CAPACITY * 4;
    let (source_tx, source_rx) = mpsc::channel::<AudioChunk>(large_cap);
    let (tx_a, mut rx_a) = mpsc::channel::<AudioChunk>(large_cap);
    // Hold rx_b at default capacity but never drain it → triggers Full.
    let (tx_b, _rx_b_stalled) = mpsc::channel::<AudioChunk>(FANOUT_SLOT_CAPACITY);
    let counters = Arc::new(FanoutDropCounters::default());

    let drainer = tokio::spawn(async move { while rx_a.recv().await.is_some() {} });
    let fanout_handle = tokio::spawn(fanout_loop(source_rx, tx_a, tx_b, counters.clone()));

    let before_drops = FANOUT_DROP_CT.load(Ordering::SeqCst);
    let total = FANOUT_SLOT_CAPACITY + 16;
    for _ in 0..total {
        source_tx
            .send(AudioChunk::new(vec![0i16; 16]))
            .await
            .expect("send");
    }
    drop(source_tx);
    fanout_handle.await.expect("fanout_loop completes");
    drainer.abort();
    let _ = drainer.await;

    let observed = FANOUT_DROP_CT.load(Ordering::SeqCst) - before_drops;
    let expected = (total - FANOUT_SLOT_CAPACITY) as u64;
    let primitive = counters.drops(SLOT_B);
    assert_eq!(
        primitive, expected,
        "primitive fanout counter mirrors expectation"
    );
    assert_eq!(
        observed as u64, primitive,
        "fanout-drop hook fires once per primitive drop"
    );
}

// ── audio capture / sink hooks ────────────────────────────────────────────

#[test]
fn audio_capture_hooks_route_through_indirection() {
    install_all_hooks();

    let chunks_before = AUDIO_CHUNK_CT.load(Ordering::SeqCst);
    let stalls_before = AUDIO_STALL_CT.load(Ordering::SeqCst);

    // The WASAPI capture loop itself cannot run without hardware. Drive
    // the same hook entry points it calls and confirm they reach the
    // installed delegate.
    audio::backpressure_hook::audio_chunk_at(audio::backpressure_hook::monotonic_now_ns());
    audio::backpressure_hook::audio_chunk_at(audio::backpressure_hook::monotonic_now_ns());
    audio::backpressure_hook::audio_capture_stall();

    assert_eq!(
        AUDIO_CHUNK_CT.load(Ordering::SeqCst) - chunks_before,
        2,
        "two chunk hook calls must reach the delegate"
    );
    assert_eq!(
        AUDIO_STALL_CT.load(Ordering::SeqCst) - stalls_before,
        1,
        "one stall hook call must reach the delegate"
    );
}

// ── pipeline sink hooks (separate hook module — install locally) ─────────

#[path = "../src/pipeline/backpressure_hook.rs"]
mod pipeline_hook;

#[test]
fn pipeline_sink_hooks_route_through_indirection() {
    pipeline_hook::install_sink_write(sink_write_d);
    pipeline_hook::install_sink_underrun(sink_underrun_d);

    let writes_before = SINK_WRITE_CT.load(Ordering::SeqCst);
    let bytes_before = SINK_BYTES.load(Ordering::SeqCst);
    let underruns_before = SINK_UNDERRUN_CT.load(Ordering::SeqCst);

    pipeline_hook::sink_write(512, Duration::from_micros(750).as_nanos() as u64);
    pipeline_hook::sink_underrun();

    assert_eq!(SINK_WRITE_CT.load(Ordering::SeqCst) - writes_before, 1);
    assert_eq!(SINK_BYTES.load(Ordering::SeqCst) - bytes_before, 512);
    assert_eq!(
        SINK_UNDERRUN_CT.load(Ordering::SeqCst) - underruns_before,
        1
    );
}
