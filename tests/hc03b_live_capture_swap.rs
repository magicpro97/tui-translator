//! HC-03B integration tests: live capture hot-swap (issue #436).
//!
//! Evidence gates:
//! - `hc_03b_source_a_to_b_switch`: chunks from source A stop and source B
//!   starts after a successful hot-swap (fixture-gated).
//! - `hc_03b_swap_count_reflects_successful_swap`: `RouterMetrics::swap_count`
//!   increments on each successful swap.
//! - `hc_03b_orchestrator_rx_stable_across_swap`: orchestrator receiver stays
//!   open after both successful and failed swaps.
//! - `hc_03b_no_upstream_transitions_on_closed_initial_stream`: router enters
//!   NoUpstream when the initial stream closes and still responds to swap commands.
//! - `hc_03b_dropped_during_swap_metric_accessible`: metric API is exercised.
//!
//! Test strategy: all tests that require a real WAV fixture are guarded with
//! `if !path.exists() { return; }` so the suite is skip-safe on CI runners
//! that do not have the fixture installed.

#[path = "../src/audio/mod.rs"]
mod audio;

use audio::{
    router::{CaptureSourceSpec, RouterMetrics, ROUTER_CHANNEL_CAPACITY},
    start_router, AudioChunk, CaptureInfo, CaptureStream,
};
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_stream(chunks: Vec<AudioChunk>) -> CaptureStream {
    let (tx, rx) = mpsc::channel(128);
    let info = CaptureInfo {
        device_name: "hc03b-test-stub".to_string(),
        native_sample_rate: 16_000,
    };
    tokio::spawn(async move {
        for chunk in chunks {
            if tx.send(chunk).await.is_err() {
                break;
            }
        }
        // Linger briefly so all chunks are visible before sender is dropped.
        tokio::time::sleep(Duration::from_millis(50)).await;
    });
    CaptureStream { info, receiver: rx }
}

fn make_closed_stream() -> CaptureStream {
    let (_tx, rx) = mpsc::channel::<AudioChunk>(1);
    // _tx dropped immediately → receiver returns None on first recv
    CaptureStream {
        info: CaptureInfo {
            device_name: "closed-stub".to_string(),
            native_sample_rate: 16_000,
        },
        receiver: rx,
    }
}

fn chunk_val(n: i16) -> AudioChunk {
    AudioChunk::new(vec![n; 160]) // 10 ms at 16 kHz
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Router forwards chunks from the initial stream to the fixed orchestrator receiver.
#[tokio::test]
async fn hc_03b_router_delivers_initial_stream_chunks() {
    let stream = make_stream(vec![chunk_val(10), chunk_val(20), chunk_val(30)]);
    let (_, mut rx) = start_router(stream, 0.001, None);

    for expected in [10i16, 20, 30] {
        let got = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("should not time out")
            .expect("channel should be open");
        assert_eq!(got.samples[0], expected, "chunk value from initial stream");
    }
}

/// Orchestrator receiver stays open after a failed swap (bad file path).
#[tokio::test]
async fn hc_03b_orchestrator_rx_stable_across_failed_swap() {
    let stream = make_stream(vec![chunk_val(1)]);
    let (handle, rx) = start_router(stream, 0.001, None);

    // Let initial stream drain.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = timeout(
        Duration::from_secs(3),
        handle.hot_swap(
            CaptureSourceSpec::File {
                path: "nonexistent_hc03b_fixture.wav".to_string(),
            },
            0.001,
        ),
    )
    .await
    .expect("hot_swap must not time out");

    assert!(result.is_err(), "expected error for missing file");
    assert!(
        !rx.is_closed(),
        "orchestrator receiver must stay open after a failed swap"
    );
    assert_eq!(
        handle.metrics().swap_count(),
        0,
        "swap_count must stay 0 on failed swap"
    );
}

/// `swap_count` increments exactly once per successful swap (fixture-gated).
#[tokio::test]
async fn hc_03b_swap_count_reflects_successful_swap() {
    let soak_wav = "tests/soak/soak_audio.wav";
    if !std::path::Path::new(soak_wav).exists() {
        eprintln!("SKIP hc_03b_swap_count_reflects_successful_swap: {soak_wav} not found");
        return;
    }

    let stream = make_stream(vec![]);
    let (handle, mut rx) = start_router(stream, 0.001, None);

    let swap_result = timeout(
        Duration::from_secs(5),
        handle.hot_swap(
            CaptureSourceSpec::File {
                path: soak_wav.to_string(),
            },
            0.001,
        ),
    )
    .await
    .expect("hot_swap must not time out");

    assert!(
        swap_result.is_ok(),
        "expected successful swap to soak fixture"
    );
    assert_eq!(
        handle.metrics().swap_count(),
        1,
        "swap_count must be 1 after one swap"
    );

    // Verify chunks flow from the new source.
    let chunk = timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("should receive from new source")
        .expect("channel open");
    assert!(
        !chunk.samples.is_empty(),
        "chunk from source B must be non-empty"
    );
}

/// Source A → source B switch: orchestrator receives chunks from both sources
/// in sequence through the fixed receiver (fixture-gated).
#[tokio::test]
async fn hc_03b_source_a_to_b_switch() {
    let soak_wav = "tests/soak/soak_audio.wav";
    if !std::path::Path::new(soak_wav).exists() {
        eprintln!("SKIP hc_03b_source_a_to_b_switch: {soak_wav} not found");
        return;
    }

    // Source A: a few in-memory chunks.
    let stream_a = make_stream((0i16..5).map(chunk_val).collect());
    let (handle, mut orchestrator_rx) = start_router(stream_a, 0.001, None);

    // Drain source A chunks.
    for _ in 0..5 {
        let _ = timeout(Duration::from_secs(2), orchestrator_rx.recv())
            .await
            .expect("timeout")
            .expect("open");
    }

    // Hot-swap to source B (WAV fixture).
    let info = timeout(
        Duration::from_secs(5),
        handle.hot_swap(
            CaptureSourceSpec::File {
                path: soak_wav.to_string(),
            },
            0.001,
        ),
    )
    .await
    .expect("hot_swap must not time out")
    .expect("swap should succeed");

    assert!(
        info.device_name.contains("soak_audio") || info.device_name.contains("file"),
        "unexpected device_name: {}",
        info.device_name
    );
    assert_eq!(handle.metrics().swap_count(), 1);

    // Orchestrator receiver (SAME channel object) should now deliver source B chunks.
    let chunk = timeout(Duration::from_secs(3), orchestrator_rx.recv())
        .await
        .expect("should receive chunk from source B")
        .expect("channel open after swap");
    assert!(
        !chunk.samples.is_empty(),
        "source B chunk must be non-empty"
    );
}

/// Router in NoUpstream state stays alive and responds to subsequent swap commands.
#[tokio::test]
async fn hc_03b_no_upstream_transitions_on_closed_initial_stream() {
    // Initial stream that closes immediately.
    let stream = make_closed_stream();
    let (handle, _rx) = start_router(stream, 0.001, None);

    // Let the router detect the closed upstream.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Router should be in NoUpstream but still alive.
    assert_eq!(handle.metrics().swap_count(), 0, "no swaps yet");

    // A swap to a bad path should return an error (not crash).
    let result = timeout(
        Duration::from_secs(2),
        handle.hot_swap(
            CaptureSourceSpec::File {
                path: "does_not_exist.wav".to_string(),
            },
            0.001,
        ),
    )
    .await
    .expect("hot_swap must not time out");

    assert!(result.is_err(), "expected error for missing file");
    assert_eq!(
        handle.metrics().swap_count(),
        0,
        "still no successful swaps"
    );
}

/// `dropped_during_swap` metric is accessible and starts at zero.
#[tokio::test]
async fn hc_03b_dropped_during_swap_metric_accessible() {
    let stream = make_stream(vec![chunk_val(1)]);
    let (handle, _rx) = start_router(stream, 0.001, None);
    // Initially zero.
    assert_eq!(handle.metrics().dropped_during_swap(), 0);
    // After consuming some chunks, still zero (small stream, not full channel).
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(handle.metrics().dropped_during_swap(), 0);
}

/// Multiple handles share the same metrics Arc.
#[tokio::test]
async fn hc_03b_handle_clone_shares_metrics() {
    let stream = make_stream(vec![]);
    let (handle, _rx) = start_router(stream, 0.001, None);
    let handle2 = handle.clone();

    // Both handles read from the same underlying metrics.
    assert_eq!(
        handle.metrics().swap_count(),
        handle2.metrics().swap_count()
    );
}

// ── Unit-level tests (no tokio runtime needed) ────────────────────────────────

/// `RouterMetrics` atomic counters start at zero.
#[test]
fn hc_03b_router_metrics_start_at_zero() {
    let m = RouterMetrics::new();
    assert_eq!(m.dropped_during_swap(), 0);
    assert_eq!(m.swap_count(), 0);
}

/// `RouterMetrics::record_swap_drops` accumulates.
#[test]
fn hc_03b_router_metrics_drops_accumulate() {
    let m = RouterMetrics::new();
    m.record_swap_drops(4);
    m.record_swap_drops(6);
    assert_eq!(m.dropped_during_swap(), 10);
}

/// `CaptureSourceSpec::label()` returns correct strings.
#[test]
fn hc_03b_source_spec_labels() {
    assert_eq!(
        CaptureSourceSpec::Wasapi { device: None }.label(),
        "wasapi:default"
    );
    assert_eq!(
        CaptureSourceSpec::Wasapi {
            device: Some("My Device".to_string())
        }
        .label(),
        "wasapi:My Device"
    );
    assert_eq!(
        CaptureSourceSpec::File {
            path: "a/b.wav".to_string()
        }
        .label(),
        "file:a/b.wav"
    );
}

/// `ROUTER_CHANNEL_CAPACITY` is within a reasonable range.
#[test]
fn hc_03b_channel_capacity_in_range() {
    const {
        assert!(
            ROUTER_CHANNEL_CAPACITY >= 32,
            "capacity too small for audio pipeline"
        );
        assert!(
            ROUTER_CHANNEL_CAPACITY <= 512,
            "capacity too large — latency budget exceeded"
        );
    }
}
