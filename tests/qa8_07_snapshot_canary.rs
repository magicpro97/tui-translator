//! QA8-07 (#505) — end-to-end snapshot canary.
//!
//! This is the **hook-path** e2e proof for the live-wiring PR (#545).
//! It installs a real [`BackpressureTelemetry`] sink via
//! `metrics::backpressure::emit::install`, wires each `*::backpressure_hook`
//! indirection module to the real `emit::*` helpers (exactly as
//! `src/main.rs` does at startup), drives a deterministic sequence of
//! synthetic events through the production hook entry points
//! (`audio::backpressure_hook::*`, `providers::backpressure_hook::*`,
//! `pipeline::backpressure_hook::*`), and then takes a
//! [`BackpressureTelemetry::snapshot_json`].
//!
//! It then asserts that audio / provider / sink counters are non-zero
//! and writes the snapshot to
//! `verification-evidence/qa8/QA8-07-snapshot-canary.json` so reviewers
//! can inspect the artifact.
//!
//! ## Honest limitation
//!
//! This canary exercises the **hook indirection path only**. The real
//! WASAPI capture loop (`src/audio/wasapi_capture.rs`) cannot be spun
//! up without Windows audio hardware, and the test does not claim to
//! prove the live capture binary itself emits telemetry. Closing that
//! gap is part of the QA8-05 (#503) runner consumption and the 30-minute
//! calibration soak that keep #505 open.

#![allow(clippy::needless_return)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[path = "../src/metrics/backpressure/mod.rs"]
mod backpressure;

#[path = "../src/audio/mod.rs"]
mod audio;

#[path = "../src/providers/mod.rs"]
mod providers;

#[path = "../src/pipeline/backpressure_hook.rs"]
mod pipeline_hook;

use backpressure::{BackpressureTelemetry, BackpressureThresholds};

/// Install every `*::backpressure_hook` indirection so it forwards into
/// the real `metrics::backpressure::emit` registry. Mirrors the wiring
/// in `src/main.rs`. Safe to call from multiple `#[test]`s — each hook
/// uses `OnceLock::set` internally and ignores duplicate installs.
fn install_real_delegates() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        // Audio.
        audio::backpressure_hook::install_fanout_drop(backpressure::emit::fanout_drop);
        audio::backpressure_hook::install_audio_chunk_at(backpressure::emit::audio_chunk_at);
        audio::backpressure_hook::install_audio_stall(backpressure::emit::audio_capture_stall);
        audio::backpressure_hook::install_monotonic_now_ns(backpressure::emit::monotonic_now_ns);

        // Providers.
        providers::backpressure_hook::install_enqueue(backpressure::emit::provider_enqueue);
        providers::backpressure_hook::install_dequeue_start(
            backpressure::emit::provider_dequeue_start,
        );
        providers::backpressure_hook::install_complete(backpressure::emit::provider_complete);
        providers::backpressure_hook::install_recovered_error(
            backpressure::emit::provider_recovered_error,
        );
        providers::backpressure_hook::install_permanent_error(
            backpressure::emit::provider_permanent_error,
        );

        // Pipeline sink.
        pipeline_hook::install_sink_write(backpressure::emit::sink_write);
        pipeline_hook::install_sink_underrun(backpressure::emit::sink_underrun);
    });
}

#[tokio::test]
async fn snapshot_canary_records_nonzero_audio_provider_sink_counters() {
    // Serialise against other tests that mutate the global emit slot.
    let _g = backpressure::emit::test_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());

    // Fresh telemetry sink for this canary.
    let telemetry = Arc::new(BackpressureTelemetry::new());
    backpressure::emit::install(Arc::clone(&telemetry));
    install_real_delegates();

    // ── Drive the production hook entry points ───────────────────────────

    // Audio: simulate three chunk arrivals with a non-zero inter-arrival
    // delay so the jitter histogram observes at least one sample, plus
    // one explicit capture stall and one fanout drop.
    let base = audio::backpressure_hook::monotonic_now_ns();
    audio::backpressure_hook::audio_chunk_at(base);
    audio::backpressure_hook::audio_chunk_at(base + 32_000_000); // +32 ms
    audio::backpressure_hook::audio_chunk_at(base + 64_000_000); // +64 ms
    audio::backpressure_hook::audio_capture_stall();
    audio::backpressure_hook::fanout_drop(/* slot = */ 1);

    // Providers: a clean call (enqueue → dequeue → complete) and one
    // permanent error so both counters move off zero.
    providers::backpressure_hook::enqueue();
    providers::backpressure_hook::dequeue_start();
    providers::backpressure_hook::complete();
    providers::backpressure_hook::enqueue();
    providers::backpressure_hook::dequeue_start();
    providers::backpressure_hook::permanent_error();
    providers::backpressure_hook::complete();

    // Sink: two writes + one underrun.
    pipeline_hook::sink_write(1024, Duration::from_micros(400).as_nanos() as u64);
    pipeline_hook::sink_write(2048, Duration::from_micros(620).as_nanos() as u64);
    pipeline_hook::sink_underrun();

    // ── Take a snapshot via the *installed* telemetry sink ───────────────
    let active = backpressure::emit::try_clone().expect("telemetry sink installed");
    let snap = active.snapshot_json(BackpressureThresholds::PRODUCTION);

    // ── Assert each domain advanced past zero ────────────────────────────
    let audio_obj = snap
        .get("audio_capture")
        .expect("audio_capture present in snapshot");
    let provider_obj = snap.get("provider").expect("provider present in snapshot");
    let sink_obj = snap.get("sink").expect("sink present in snapshot");

    // Audio: at least the stall counter and the chunk count must be > 0.
    let stall_count = audio_obj
        .get("stall_count")
        .and_then(|v| v.as_u64())
        .expect("audio_capture.stall_count is u64");
    assert!(stall_count >= 1, "audio stall counter must be non-zero");

    // Provider: enqueue / complete / permanent_errors all moved.
    let enq = provider_obj
        .get("enqueued_total")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let perm = provider_obj
        .get("permanent_errors")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(
        enq >= 2 && perm >= 1,
        "provider counters must be non-zero (enq={enq}, perm={perm}); raw={provider_obj}"
    );

    // Sink: at least one underrun and one fanout drop / write must be visible.
    let underruns = sink_obj
        .get("underruns")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let fanout_drops = sink_obj
        .get("fanout_drops")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(
        underruns >= 1 && fanout_drops >= 1,
        "sink counters must be non-zero (underruns={underruns}, fanout_drops={fanout_drops}); raw={sink_obj}"
    );

    // ── Persist the artifact ─────────────────────────────────────────────
    let mut out = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    out.push("verification-evidence");
    out.push("qa8");
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("warning: could not create evidence dir: {e}");
    }
    out.push("QA8-07-snapshot-canary.json");
    let pretty = serde_json::to_string_pretty(&snap).unwrap_or_else(|_| snap.to_string());
    if let Err(e) = std::fs::write(&out, pretty) {
        eprintln!("warning: could not write snapshot artifact to {out:?}: {e}");
    }

    // Clean up the global slot so other tests aren't surprised.
    backpressure::emit::uninstall();
}
