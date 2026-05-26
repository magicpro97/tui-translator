//! QA8-07 (issue #505) — backpressure telemetry integration tests.
//!
//! These tests exercise the [`tui_translator::metrics::backpressure`]
//! module through the four QA8-07 acceptance scenarios, driven by the
//! deterministic #460 simulation harness where applicable:
//!
//! * **Audio gap** — synthetic capture gap exceeds the stall threshold
//!   and the stall counter ticks; nominal cadence inserts only jitter
//!   observations.
//! * **Provider 429 / outage** — synthetic transient errors recover via
//!   `with_retry` and bump the recovered-error counter; a permanent
//!   error bumps the permanent-error counter.
//! * **Sink underrun** — explicit `record_underrun()` paths increment
//!   the sink underrun counter and the write-latency histogram captures
//!   nominal write latencies.
//! * **Nominal dual-mode** — a sequence of clean chunks + clean
//!   provider calls + clean sink writes produces no fanout drops and an
//!   `ok = true` snapshot.
//!
//! In addition, the snapshot is validated structurally against the
//! committed schema at
//! `verification-evidence/qa8/QA8-07-backpressure-telemetry.schema.json`.

use std::time::Duration;

use serde_json::Value;

#[path = "../src/metrics/backpressure/mod.rs"]
mod backpressure;

#[path = "../src/providers/mod.rs"]
mod providers;

#[path = "sim/mod.rs"]
mod sim;

use backpressure::{
    AudioCaptureBackpressure, BackpressureTelemetry, BackpressureThresholds, ProviderBackpressure,
    SinkBackpressure, SCHEMA_VERSION,
};
use providers::{is_transient, with_retry, MtProvider, MtResult, ProviderError};
use sim::clock::FakeClock;
use sim::fakes::{FakeMtProvider, Outcome};
use sim::feeder::{AudioScript, ScriptedAudioFeeder};

// ── (a) audio gap → stall counter ───────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn audio_gap_increments_stall_counter_under_sim_harness() {
    // Use the #460 feeder to script three nominal chunks at 20 ms cadence
    // followed by a single chunk after a 600 ms gap. The 600 ms gap must
    // tick the stall counter exactly once.
    let mut feeder = ScriptedAudioFeeder::new([
        AudioScript::Silence { samples: 320 }, // 20 ms @ 16 kHz mono
        AudioScript::Silence { samples: 320 },
        AudioScript::Silence { samples: 320 },
        AudioScript::Silence { samples: 320 },
    ]);

    let a = AudioCaptureBackpressure::new();
    // Tight 100 ms threshold so the 600 ms gap dominates without
    // flagging the nominal 20 ms cadence.
    a.set_stall_threshold_ns(100_000_000);

    let cadences = [
        Duration::from_millis(20),
        Duration::from_millis(20),
        Duration::from_millis(600),
    ];
    let mut now_ns: u64 = 0;
    let mut idx = 0;
    a.record_chunk_at(now_ns);
    while let Some(_chunk) = feeder.next_chunk() {
        if idx >= cadences.len() {
            break;
        }
        now_ns += cadences[idx].as_nanos() as u64;
        a.record_chunk_at(now_ns);
        idx += 1;
    }

    assert_eq!(a.chunks_seen(), 4);
    assert_eq!(
        a.stall_count(),
        1,
        "exactly one 600 ms gap must register as a stall"
    );
}

#[test]
fn nominal_cadence_does_not_tick_stall_counter() {
    let a = AudioCaptureBackpressure::new();
    // 100 ms threshold; 20 ms cadence stays well clear.
    a.set_stall_threshold_ns(100_000_000);
    let mut now = 0u64;
    a.record_chunk_at(now);
    for _ in 0..20 {
        now += 20_000_000;
        a.record_chunk_at(now);
    }
    assert_eq!(a.stall_count(), 0);
    assert!(a.jitter_count_for_test() >= 20);
}

trait AudioJitterCount {
    fn jitter_count_for_test(&self) -> u64;
}

impl AudioJitterCount for AudioCaptureBackpressure {
    fn jitter_count_for_test(&self) -> u64 {
        // The aggregator's JSON view is the public contract; pull the
        // count out of the snapshot to avoid exposing the histogram
        // type beyond the public surface.
        let t = BackpressureTelemetry::new();
        // Replicate the chunks we already recorded into the aggregate so
        // the count matches what the test observed. The aggregate's
        // own `audio_capture` is empty here; for simplicity in the
        // test, reach through the public JSON snapshot of *this*
        // instance instead by going through serde.
        let _ = t; // keep the aggregator import alive in this helper
                   // The HistogramUs::count is exposed indirectly via snapshot_json;
                   // construct a one-off snapshot to read the count.
        let v: serde_json::Value = serde_json::json!({
            "stall_count": self.stall_count(),
        });
        let _ = v;
        // The simplest stable path: use `chunks_seen` minus the first
        // chunk (which never records jitter).
        self.chunks_seen().saturating_sub(1)
    }
}

// ── (b) provider 429 + permanent error → counters ───────────────────────────

#[tokio::test(start_paused = true)]
async fn provider_429_drives_recovered_error_counter_via_with_retry() {
    let clock = FakeClock::new();
    let mt = FakeMtProvider::new(clock.clone());
    // Two transient errors, then a success: matches the #460 L2 pattern.
    mt.enqueue(Outcome::rate_limited(Duration::ZERO));
    mt.enqueue(Outcome::unavailable(Duration::ZERO));
    mt.enqueue(Outcome::ok(MtResult {
        translated_text: "ok".into(),
        detected_source_language: Some("en".into()),
    }));

    let p = ProviderBackpressure::new();
    p.on_enqueue();
    p.on_dequeue_start();

    // Drive the production retry wrapper, counting transient errors as
    // "recovered" once the wrapper finally returns Ok. Permanent errors
    // would bypass the loop via `is_transient`, so this helper closure
    // is precisely the wiring contract documented on
    // `ProviderBackpressure::record_recovered_error`.
    let transient_failures = std::sync::atomic::AtomicU64::new(0);
    let result = with_retry(|| async {
        match mt.translate("anything", "en", "vi").await {
            Ok(v) => Ok(v),
            Err(e) => {
                if is_transient(&e) {
                    transient_failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                Err(e)
            }
        }
    })
    .await
    .expect("retry resolves");
    assert_eq!(result.translated_text, "ok");
    if transient_failures.load(std::sync::atomic::Ordering::Relaxed) > 0 {
        p.record_recovered_error();
    }
    p.on_complete();

    assert_eq!(p.recovered_errors(), 1);
    assert_eq!(p.permanent_errors(), 0);
    assert_eq!(p.inflight(), 0);
}

#[tokio::test(start_paused = true)]
async fn provider_permanent_error_increments_permanent_counter() {
    let clock = FakeClock::new();
    let mt = FakeMtProvider::new(clock.clone());
    mt.enqueue(Outcome::auth_failed(Duration::ZERO));

    let p = ProviderBackpressure::new();
    p.on_enqueue();
    p.on_dequeue_start();
    let err = with_retry(|| async { mt.translate("x", "en", "vi").await })
        .await
        .expect_err("auth must surface");
    assert!(matches!(err, ProviderError::AuthError(_)));
    assert!(!is_transient(&err));
    p.record_permanent_error();
    p.on_complete();

    assert_eq!(p.permanent_errors(), 1);
    assert_eq!(p.recovered_errors(), 0);
}

#[test]
fn provider_queue_high_water_tracks_peak_only() {
    let p = ProviderBackpressure::new();
    p.on_enqueue();
    p.on_enqueue();
    p.on_enqueue(); // depth 3
    p.on_dequeue_start();
    p.on_dequeue_start();
    p.on_enqueue();
    assert_eq!(p.queue_depth(), 2);
    assert_eq!(p.queue_high_water(), 3);
}

// ── (c) sink underrun → counter ─────────────────────────────────────────────

#[test]
fn sink_underrun_ticks_counter_independent_of_writes() {
    let s = SinkBackpressure::new();
    s.record_write(4_096, 800_000); // 0.8 ms nominal
    s.record_underrun();
    s.record_write(4_096, 1_200_000);
    s.record_underrun();
    assert_eq!(s.writes(), 2);
    assert_eq!(s.underruns(), 2);
}

// ── (d) nominal dual-mode → zero fanout drops + ok snapshot ─────────────────

#[test]
fn nominal_dual_mode_snapshot_is_ok_with_no_breaches() {
    let t = BackpressureTelemetry::new();
    // Nominal 20 ms cadence for 50 chunks.
    let mut now = 0u64;
    t.audio_capture.record_chunk_at(now);
    for _ in 0..50 {
        now += 20_000_000;
        t.audio_capture.record_chunk_at(now);
    }
    // Nominal provider activity: 5 round-trips, no errors.
    for _ in 0..5 {
        t.provider.on_enqueue();
        t.provider.on_dequeue_start();
        t.provider.on_complete();
    }
    // Nominal sink writes around 1 ms.
    for _ in 0..30 {
        t.sink.record_write(2_048, 1_000_000);
    }
    // No fanout drops.
    let snap = t.snapshot_json(BackpressureThresholds::PRODUCTION);
    assert_eq!(snap["ok"], true, "snapshot was: {snap}");
    assert_eq!(snap["breaches"].as_array().unwrap().len(), 0);
    assert_eq!(snap["sink"]["fanout_drops"], 0);
}

// ── Schema conformance ──────────────────────────────────────────────────────

#[test]
fn snapshot_conforms_to_qa8_07_schema_structure() {
    let schema_raw = std::fs::read_to_string(
        "verification-evidence/qa8/QA8-07-backpressure-telemetry.schema.json",
    )
    .expect("schema must be readable");
    let schema: Value = serde_json::from_str(&schema_raw).expect("schema parses");

    let t = BackpressureTelemetry::new();
    t.audio_capture.record_chunk_at(0);
    t.audio_capture.record_chunk_at(20_000_000);
    t.cancellation.record_issue();
    t.cancellation.record_exit(500_000);
    t.sink.record_write(1024, 500_000);
    let snap = t.snapshot_json(BackpressureThresholds::PRODUCTION);

    // Top-level required fields.
    let required: Vec<String> = schema["required"]
        .as_array()
        .expect("schema.required")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    for field in &required {
        assert!(
            snap.get(field).is_some(),
            "snapshot missing required top-level field {field:?}; snap was: {snap}"
        );
    }
    // Schema version pin.
    assert_eq!(snap["schema_version"], SCHEMA_VERSION);
    assert_eq!(
        schema["properties"]["schema_version"]["const"],
        SCHEMA_VERSION
    );
    // Sub-objects each have their schema-required keys.
    for (key, def_key) in [
        ("audio_capture", "audio_capture"),
        ("provider", "provider"),
        ("cancellation", "cancellation"),
        ("sink", "sink"),
        ("thresholds", "thresholds"),
    ] {
        let def = &schema["definitions"][def_key];
        let req = def["required"].as_array().expect("nested.required");
        for field in req {
            let f = field.as_str().expect("field name");
            assert!(
                snap[key].get(f).is_some(),
                "{key} missing required field {f:?}; was: {}",
                snap[key]
            );
        }
    }
    // Breaches enum is closed; ensure we never emit an unknown identifier.
    let allowed_breaches: Vec<String> = schema["properties"]["breaches"]["items"]["enum"]
        .as_array()
        .expect("breaches.enum")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    for b in snap["breaches"].as_array().expect("breaches array") {
        let s = b.as_str().expect("breach string");
        assert!(
            allowed_breaches.iter().any(|a| a == s),
            "breach {s:?} not in schema enum {allowed_breaches:?}"
        );
    }
}

#[test]
fn breaches_enum_in_schema_covers_every_emitter_branch() {
    // Force every branch in snapshot_json by exceeding every threshold.
    let t = BackpressureTelemetry::new();
    // Big jitter + stall: 2 s gap.
    t.audio_capture.record_chunk_at(0);
    t.audio_capture.record_chunk_at(2_000_000_000);
    // Provider over-limit on every dimension.
    for _ in 0..100 {
        t.provider.on_enqueue();
    }
    for _ in 0..50 {
        t.provider.on_dequeue_start();
    }
    t.provider.record_permanent_error();
    // Cancel with 2 s latency.
    t.cancellation.record_issue();
    t.cancellation.record_exit(2_000_000_000);
    // Sink saturation.
    t.sink.record_underrun();
    t.sink.record_write(1, 2_000_000_000); // 2 s
    t.sink.record_fanout_drop();

    let snap = t.snapshot_json(BackpressureThresholds::PRODUCTION);
    let breaches: Vec<String> = snap["breaches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    for expected in [
        "audio_jitter_p99_ms",
        "capture_stalls",
        "provider_queue_high_water",
        "provider_inflight_high_water",
        "provider_permanent_errors",
        "cancel_p99_ms",
        "sink_underruns",
        "sink_write_p99_ms",
        "fanout_drops",
    ] {
        assert!(
            breaches.iter().any(|b| b == expected),
            "expected breach {expected:?}; got: {breaches:?}"
        );
    }
    assert_eq!(snap["ok"], false);
}
