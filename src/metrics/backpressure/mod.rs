//! Backpressure telemetry for the 8-hour-stability QA roadmap (QA8-07, issue #505).
//!
//! This module ships the observability primitives a soak run needs to
//! detect audio-capture jitter / stalls, provider queue / inflight /
//! error recovery, cancellation latency, and audio-sink / virtual-mic
//! underruns + write latency.
//!
//! The submodules expose the production-side traits and counters:
//!
//! * [`audio::AudioCaptureBackpressure`] — chunk inter-arrival jitter +
//!   stall counter (single-producer recording path).
//! * [`provider::ProviderBackpressure`] — queue / inflight gauges + CAS
//!   saturated decrement, recovered + permanent error counters.
//! * [`cancellation::CancellationLatency`] — issued / observed counters
//!   and exit-latency histogram.
//! * [`sink::SinkBackpressure`] — write/underrun/fanout-drop counters
//!   and a write-latency histogram.
//!
//! [`BackpressureTelemetry`] aggregates these and exposes
//! [`BackpressureTelemetry::snapshot_json`] which conforms to
//! `verification-evidence/qa8/QA8-07-backpressure-telemetry.schema.json`.
//!
//! ## Scope and follow-ups
//!
//! This module is intentionally **infrastructure-only**. Wiring into
//! `src/audio/wasapi_capture.rs`, `src/audio/fanout.rs`,
//! `src/pipeline/audio_sink.rs`, and the provider dispatch in
//! `src/pipeline/mod.rs`, plus QA8-05 (issue #503) runner consumption
//! and a 30-minute calibration soak, are explicit follow-ups tracked
//! in PR #540 and reflected in the snapshot's `calibration_notes` /
//! `calibration_pending` fields. The PR therefore references #505 but
//! does not auto-close it.

pub mod audio;
pub mod cancellation;
pub mod clock;
pub mod histogram;
pub mod provider;
pub mod sink;
pub mod thresholds;

#[allow(unused_imports)]
pub use audio::AudioCaptureBackpressure;
#[allow(unused_imports)]
pub use cancellation::CancellationLatency;
#[allow(unused_imports)]
pub use clock::FakeNanoClock;
#[allow(unused_imports)]
pub use histogram::HistogramUs;
#[allow(unused_imports)]
pub use provider::ProviderBackpressure;
#[allow(unused_imports)]
pub use sink::SinkBackpressure;
#[allow(unused_imports)]
pub use thresholds::BackpressureThresholds;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use thresholds::{calibration_notes, BREACH_THRESHOLD_KEYS};

/// Schema identifier embedded in every snapshot so QA8-05 can pin the
/// reader's expected schema version.
pub const SCHEMA_VERSION: &str = "qa8-07.v1";

/// Issues this telemetry is evidence for. The array is the canonical
/// form so QA8-05 (issue #503) evidence can reuse the same telemetry
/// without a single-issue claim.
pub const RELATED_ISSUES: &[&str] = &["#505"];

/// Aggregate backpressure telemetry: audio capture, provider, cancel,
/// sink. One instance is shared across the pipeline via `Arc`.
#[derive(Debug, Default)]
pub struct BackpressureTelemetry {
    /// Audio inter-chunk jitter + stall counters.
    pub audio_capture: AudioCaptureBackpressure,
    /// STT/MT/TTS provider backpressure counters.
    pub provider: ProviderBackpressure,
    /// Cancellation latency distribution.
    pub cancellation: CancellationLatency,
    /// Audio sink / virtual-mic underrun + write-latency counters.
    pub sink: SinkBackpressure,
    /// Monotonically-increasing snapshot index, used by soak readers
    /// to order samples without relying on wall-clock timestamps.
    sample_index: AtomicU64,
}

impl BackpressureTelemetry {
    /// Construct an empty telemetry aggregate.
    pub fn new() -> Self {
        Self::default()
    }

    /// Produce a JSON snapshot conforming to
    /// `verification-evidence/qa8/QA8-07-backpressure-telemetry.schema.json`.
    ///
    /// Each call increments `sample_index` so QA8-05 / soak readers can
    /// order samples without depending on `sample_unix_ms` (which uses
    /// the wall clock for human-readable timestamps).
    pub fn snapshot_json(&self, thresholds: BackpressureThresholds) -> Value {
        let mut breaches: Vec<&'static str> = Vec::new();
        let jitter_p99_ms = self.audio_capture.jitter().percentile_us(99.0) / 1_000;
        if jitter_p99_ms > thresholds.audio_jitter_p99_ms {
            breaches.push("audio_jitter_p99_ms");
        }
        if self.audio_capture.stall_count() > thresholds.max_capture_stalls {
            breaches.push("capture_stalls");
        }
        if self.provider.queue_high_water() > thresholds.provider_max_queue_depth {
            breaches.push("provider_queue_high_water");
        }
        if self.provider.inflight_high_water() > thresholds.provider_max_inflight {
            breaches.push("provider_inflight_high_water");
        }
        if self.provider.permanent_errors() > thresholds.provider_max_permanent_errors {
            breaches.push("provider_permanent_errors");
        }
        let cancel_p99_ms = self.cancellation.histogram().percentile_us(99.0) / 1_000;
        if cancel_p99_ms > thresholds.cancel_p99_ms {
            breaches.push("cancel_p99_ms");
        }
        if self.sink.underruns() > thresholds.max_sink_underruns {
            breaches.push("sink_underruns");
        }
        let sink_p99_ms = self.sink.write_latency().percentile_us(99.0) / 1_000;
        if sink_p99_ms > thresholds.sink_write_p99_ms {
            breaches.push("sink_write_p99_ms");
        }
        if self.sink.fanout_drops() > thresholds.max_fanout_drops {
            breaches.push("fanout_drops");
        }

        let breach_threshold_map: Value = breaches
            .iter()
            .map(|b| {
                let key = BREACH_THRESHOLD_KEYS
                    .iter()
                    .find(|(id, _)| id == b)
                    .map(|(_, k)| *k)
                    .unwrap_or("");
                (b.to_string(), Value::String(key.to_string()))
            })
            .collect::<serde_json::Map<_, _>>()
            .into();

        let sample_index = self.sample_index.fetch_add(1, Ordering::Relaxed);
        let sample_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        json!({
            "schema_version": SCHEMA_VERSION,
            "related_issues": RELATED_ISSUES,
            "sample_index": sample_index,
            "sample_unix_ms": sample_unix_ms,
            "audio_capture": self.audio_capture.to_json(),
            "provider": self.provider.to_json(),
            "cancellation": self.cancellation.to_json(),
            "sink": self.sink.to_json(),
            "thresholds": thresholds.to_json(),
            "breaches": breaches,
            "breach_threshold_map": breach_threshold_map,
            "calibration": calibration_notes(),
            "ok": breaches.is_empty(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_clean_telemetry_is_ok() {
        let t = BackpressureTelemetry::new();
        let snap = t.snapshot_json(BackpressureThresholds::PRODUCTION);
        assert_eq!(snap["ok"], true);
        assert_eq!(snap["schema_version"], SCHEMA_VERSION);
        assert_eq!(snap["related_issues"][0], "#505");
        assert!(snap["breaches"].as_array().is_some_and(|a| a.is_empty()));
        assert_eq!(snap["calibration"]["calibration_pending"], true);
    }

    #[test]
    fn sample_index_monotonically_increases() {
        let t = BackpressureTelemetry::new();
        let a = t.snapshot_json(BackpressureThresholds::PRODUCTION);
        let b = t.snapshot_json(BackpressureThresholds::PRODUCTION);
        assert_eq!(a["sample_index"].as_u64(), Some(0));
        assert_eq!(b["sample_index"].as_u64(), Some(1));
    }

    #[test]
    fn breach_threshold_map_aligns_every_breach_to_threshold_key() {
        let t = BackpressureTelemetry::new();
        // Force every breach.
        t.audio_capture.record_chunk_at(0);
        t.audio_capture.record_chunk_at(2_000_000_000);
        for _ in 0..100 {
            t.provider.on_enqueue();
        }
        for _ in 0..50 {
            t.provider.on_dequeue_start();
        }
        t.provider.record_permanent_error();
        t.cancellation.record_issue();
        t.cancellation.record_exit(2_000_000_000);
        t.sink.record_underrun();
        t.sink.record_write(1, 2_000_000_000);
        t.sink.record_fanout_drop();

        let snap = t.snapshot_json(BackpressureThresholds::PRODUCTION);
        let map = snap["breach_threshold_map"]
            .as_object()
            .expect("map present");
        for b in snap["breaches"].as_array().expect("array") {
            let id = b.as_str().expect("string");
            let key = map.get(id).and_then(|v| v.as_str()).unwrap_or("");
            assert!(
                !key.is_empty(),
                "breach {id} must map to a threshold key; map: {map:?}"
            );
            // The mapped key must exist in the thresholds object.
            assert!(
                snap["thresholds"].get(key).is_some(),
                "threshold key {key} (for breach {id}) missing in thresholds: {}",
                snap["thresholds"]
            );
        }
    }
}
