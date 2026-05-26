//! Unit tests for `router` (extracted from `router.rs` for the
//! STD-02 module-budget refactor; behavior unchanged).

use super::*;
use tokio::time::{timeout, Duration};

fn make_stream(chunks: Vec<AudioChunk>) -> CaptureStream {
    let (tx, rx) = mpsc::channel(128);
    let info = CaptureInfo {
        device_name: "test-stub".to_string(),
        native_sample_rate: 16_000,
    };
    tokio::spawn(async move {
        for chunk in chunks {
            if tx.send(chunk).await.is_err() {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    });
    CaptureStream { info, receiver: rx }
}

fn make_continuous_stream(value: i16) -> CaptureStream {
    let (tx, rx) = mpsc::channel(128);
    let info = CaptureInfo {
        device_name: "continuous-stub".to_string(),
        native_sample_rate: 16_000,
    };
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(10));
        loop {
            interval.tick().await;
            if tx.send(chunk(value)).await.is_err() {
                break;
            }
        }
    });
    CaptureStream { info, receiver: rx }
}

fn chunk(n: i16) -> AudioChunk {
    AudioChunk::new(vec![n; 160]) // 10 ms at 16 kHz
}

// ── RouterMetrics ────────────────────────────────────────────────────────

#[test]
fn metrics_start_at_zero() {
    let m = RouterMetrics::new();
    assert_eq!(m.dropped_during_swap(), 0);
    assert_eq!(m.swap_count(), 0);
}

#[test]
fn metrics_record_swap_drops_accumulates() {
    let m = RouterMetrics::new();
    m.record_swap_drops(3);
    m.record_swap_drops(7);
    assert_eq!(m.dropped_during_swap(), 10);
}

#[test]
fn metrics_record_swap_increments() {
    let m = RouterMetrics::new();
    m.record_swap();
    m.record_swap();
    assert_eq!(m.swap_count(), 2);
}

#[test]
fn metrics_independent_arcs() {
    let m = RouterMetrics::new();
    let m2 = Arc::clone(&m);
    m.record_swap_drops(5);
    assert_eq!(m2.dropped_during_swap(), 5);
}

// ── CaptureSourceSpec ────────────────────────────────────────────────────

#[test]
fn capture_source_spec_wasapi_default_label() {
    assert_eq!(
        CaptureSourceSpec::Wasapi { device: None }.label(),
        "wasapi:default"
    );
}

#[test]
fn capture_source_spec_wasapi_named_label() {
    assert_eq!(
        CaptureSourceSpec::Wasapi {
            device: Some("Speakers (Realtek)".to_string())
        }
        .label(),
        "wasapi:Speakers (Realtek)"
    );
}

#[test]
fn capture_source_spec_file_label() {
    assert_eq!(
        CaptureSourceSpec::File {
            path: "tests/soak/soak_audio.wav".to_string()
        }
        .label(),
        "file:tests/soak/soak_audio.wav"
    );
}

// ── Router task ──────────────────────────────────────────────────────────

#[tokio::test]
async fn hc_03b_router_delivers_initial_stream() {
    let stream = make_stream(vec![chunk(1), chunk(2), chunk(3)]);
    let (_, mut rx) = start_router(stream, 0.001, None);

    for expected in [1i16, 2, 3] {
        let got = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("no timeout")
            .expect("channel open");
        assert_eq!(got.samples[0], expected);
    }
}

#[tokio::test]
async fn hc_03b_handle_metrics_initially_zero() {
    let stream = make_stream(vec![chunk(42)]);
    let (handle, _rx) = start_router(stream, 0.001, None);
    assert_eq!(handle.metrics().swap_count(), 0);
    assert_eq!(handle.metrics().dropped_during_swap(), 0);
}

#[tokio::test]
async fn hc_03b_no_upstream_router_stays_alive_until_handle_dropped() {
    // Start with an immediately-closed stream → router enters NoUpstream.
    let (tx, rx_inner) = mpsc::channel::<AudioChunk>(1);
    drop(tx); // close the sender immediately
    let stream = CaptureStream {
        info: CaptureInfo {
            device_name: "closed-stub".to_string(),
            native_sample_rate: 16_000,
        },
        receiver: rx_inner,
    };
    let (handle, _rx) = start_router(stream, 0.001, None);
    // Give the router time to detect the closed upstream.
    tokio::time::sleep(Duration::from_millis(80)).await;
    // Router should not have crashed; handle is still alive.
    assert_eq!(handle.metrics().swap_count(), 0);
}

#[tokio::test]
async fn hc_03b_swap_to_bad_file_returns_error_swap_count_zero() {
    let stream = make_stream(vec![chunk(1)]);
    let (handle, _rx) = start_router(stream, 0.001, None);

    let result = timeout(
        Duration::from_secs(3),
        handle.hot_swap(
            CaptureSourceSpec::File {
                path: "nonexistent_hc03b_test.wav".to_string(),
            },
            0.001,
        ),
    )
    .await
    .expect("hot_swap should not time out");

    assert!(result.is_err(), "expected error for missing file");
    assert_eq!(
        handle.metrics().swap_count(),
        0,
        "swap_count must stay 0 on failed swap"
    );
}

#[tokio::test]
async fn hc_03b_downstream_rx_stays_open_after_failed_swap() {
    let stream = make_stream(vec![chunk(1)]);
    let (handle, rx) = start_router(stream, 0.001, None);

    // Wait for initial stream to be consumed.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let _ = handle
        .hot_swap(
            CaptureSourceSpec::File {
                path: "nonexistent.wav".to_string(),
            },
            0.001,
        )
        .await;

    // The downstream receiver should still be open (router task alive).
    assert!(
        !rx.is_closed(),
        "orchestrator receiver must stay open after failed swap"
    );
}

#[tokio::test]
async fn hc_03b_failed_swap_preserves_old_upstream_audio() {
    let stream = make_continuous_stream(7);
    let (handle, mut rx) = start_router(stream, 0.001, None);

    let first = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("initial stream should produce a chunk")
        .expect("router downstream open");
    assert_eq!(first.samples.first().copied(), Some(7));

    let result = timeout(
        Duration::from_secs(3),
        handle.hot_swap(
            CaptureSourceSpec::File {
                path: "nonexistent_hc03b_preserve_old.wav".to_string(),
            },
            0.001,
        ),
    )
    .await
    .expect("failed hot_swap should not time out");
    assert!(result.is_err(), "bad target must fail the swap");

    let after_failure = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("old stream should still produce chunks after failed swap")
        .expect("router downstream open after failed swap");
    assert_eq!(after_failure.samples.first().copied(), Some(7));
    assert_eq!(handle.metrics().swap_count(), 0);
}

#[tokio::test]
async fn hc_03b_steady_state_backpressure_is_not_swap_drop() {
    let stream = make_stream((0..100).map(|n| chunk(n as i16)).collect());
    let (handle, _rx) = start_router(stream, 0.001, None);

    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(
        handle.metrics().dropped_during_swap(),
        0,
        "steady-state downstream backpressure is not a hot-swap drain drop"
    );
}

#[tokio::test]
async fn hc_03b_drain_waits_for_late_old_stream_chunks() {
    let (old_tx, old_rx) = mpsc::channel(1);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let _ = old_tx.send(chunk(9)).await;
    });
    let (downstream_tx, mut downstream_rx) = mpsc::channel(4);
    let metrics = RouterMetrics::new();
    let mut archive = None;

    drain_old(old_rx, &mut archive, &downstream_tx, &metrics).await;

    let drained = downstream_rx
        .try_recv()
        .expect("drain should wait for a late chunk before deadline");
    assert_eq!(drained.samples.first().copied(), Some(9));
    assert_eq!(metrics.dropped_during_swap(), 0);
}

#[test]
fn router_channel_capacity_in_range() {
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

#[tokio::test]
async fn hc_03b_swap_count_increments_on_successful_file_swap() {
    let soak_wav = "tests/soak/soak_audio.wav";
    if !std::path::Path::new(soak_wav).exists() {
        // Skip on environments without the fixture.
        eprintln!("SKIP hc_03b_swap_count_increments: {soak_wav} not found");
        return;
    }
    let stream = make_stream(vec![]);
    let (handle, mut rx) = start_router(stream, 0.001, None);

    let result = timeout(
        Duration::from_secs(5),
        handle.hot_swap(
            CaptureSourceSpec::File {
                path: soak_wav.to_string(),
            },
            0.001,
        ),
    )
    .await
    .expect("hot_swap should not time out");

    assert!(result.is_ok(), "swap to soak_audio.wav should succeed");
    assert_eq!(handle.metrics().swap_count(), 1);

    // Verify chunks are now flowing from the new source.
    let chunk = timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("should receive chunk from new source")
        .expect("channel open");
    assert!(!chunk.samples.is_empty());
}
