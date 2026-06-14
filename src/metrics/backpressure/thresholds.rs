//! Threshold envelope embedded in the snapshot so the QA8-05 runner
//! can flag breaches deterministically.

use serde_json::{json, Value};

/// Mapping from breach identifier to the threshold-key it derives from.
///
/// QA8-05 (issue #503) consumes the snapshot's `breaches` array; this
/// const table is exported in the snapshot under `breach_threshold_map`
/// so the runner has an unambiguous mapping when threshold keys and
/// breach identifiers diverge (e.g. `capture_stalls` vs
/// `max_capture_stalls`).
pub(crate) const BREACH_THRESHOLD_KEYS: &[(&str, &str)] = &[
    ("audio_jitter_p99_ms", "audio_jitter_p99_ms"),
    ("capture_stalls", "max_capture_stalls"),
    ("provider_queue_high_water", "provider_max_queue_depth"),
    ("provider_inflight_high_water", "provider_max_inflight"),
    ("provider_permanent_errors", "provider_max_permanent_errors"),
    ("cancel_p99_ms", "cancel_p99_ms"),
    ("sink_underruns", "max_sink_underruns"),
    ("sink_write_p99_ms", "sink_write_p99_ms"),
    ("fanout_drops", "max_fanout_drops"),
];

/// Threshold envelope embedded in the snapshot so the QA8-05 runner can
/// flag breaches without hard-coding limits in its own configuration.
///
/// Defaults in [`Self::PRODUCTION`] are **baseline / pre-calibration**
/// values aligned with QA8-02 SLO categories. The
/// `calibration_pending: true` marker in the snapshot signals that the
/// initial 30-minute soak + the 8-hour soak runner (QA8-05) will
/// tighten or relax these values based on observed P99/max
/// distributions. See PR #540 and the follow-up wiring task referenced
/// in the snapshot's `calibration_notes`.
#[derive(Debug, Clone, Copy)]
pub struct BackpressureThresholds {
    /// Maximum allowed p99 audio inter-chunk jitter, in milliseconds.
    pub audio_jitter_p99_ms: u64,
    /// Maximum tolerated audio capture stalls per soak run.
    pub max_capture_stalls: u64,
    /// Maximum allowed provider queue depth high-water mark.
    pub provider_max_queue_depth: u64,
    /// Maximum allowed in-flight high-water mark per provider.
    pub provider_max_inflight: u64,
    /// Maximum tolerated permanent provider errors.
    ///
    /// **Pending calibration**: 0 is the strict baseline; soak runs may
    /// raise this if low-rate transient cloud errors that exhaust
    /// retries are observed as benign.
    pub provider_max_permanent_errors: u64,
    /// Maximum allowed p99 cancellation latency, in milliseconds.
    pub cancel_p99_ms: u64,
    /// Maximum tolerated sink underruns per soak run.
    ///
    /// **Pending calibration**: 0 is the strict baseline. Real WASAPI
    /// loopback paths under contention may produce a small, bounded
    /// number of underruns; the 30-min calibration soak documented in
    /// `calibration_notes` will determine the production limit.
    pub max_sink_underruns: u64,
    /// Maximum allowed p99 sink write latency, in milliseconds.
    ///
    /// **Pending calibration**: 10 ms aligns with the audio-frame
    /// cadence target (#460); production soak may relax to 15–20 ms if
    /// kernel-side virtual-mic write jitter is observed.
    pub sink_write_p99_ms: u64,
    /// Maximum tolerated fanout drops per soak run under nominal dual
    /// mode.
    pub max_fanout_drops: u64,
}

impl BackpressureThresholds {
    /// Production-defaults aligned with QA8-02 SLO categories. Tightened
    /// or relaxed over time as soak evidence accumulates; the
    /// `calibration_pending` flag is emitted alongside this envelope so
    /// QA8-05 can distinguish enforced thresholds from advisory ones.
    pub const PRODUCTION: BackpressureThresholds = BackpressureThresholds {
        audio_jitter_p99_ms: 100,
        max_capture_stalls: 0,
        provider_max_queue_depth: 32,
        provider_max_inflight: 8,
        provider_max_permanent_errors: 0,
        cancel_p99_ms: 500,
        max_sink_underruns: 0,
        sink_write_p99_ms: 10,
        max_fanout_drops: 0,
    };

    pub(crate) fn to_json(self) -> Value {
        json!({
            "audio_jitter_p99_ms": self.audio_jitter_p99_ms,
            "max_capture_stalls": self.max_capture_stalls,
            "provider_max_queue_depth": self.provider_max_queue_depth,
            "provider_max_inflight": self.provider_max_inflight,
            "provider_max_permanent_errors": self.provider_max_permanent_errors,
            "cancel_p99_ms": self.cancel_p99_ms,
            "max_sink_underruns": self.max_sink_underruns,
            "sink_write_p99_ms": self.sink_write_p99_ms,
            "max_fanout_drops": self.max_fanout_drops,
        })
    }
}

impl Default for BackpressureThresholds {
    fn default() -> Self {
        Self::PRODUCTION
    }
}

/// Calibration notes embedded in the snapshot so a downstream reader
/// (QA8-05, evidence inspectors) understands which thresholds are
/// strict and which are advisory until soak evidence is gathered.
pub(crate) fn calibration_notes() -> Value {
    json!({
        "calibration_pending": true,
        "notes": {
            "max_sink_underruns": "Strict baseline 0; live WASAPI virtual-mic path may require a small bounded budget after 30-min soak.",
            "provider_max_permanent_errors": "Strict baseline 0; cloud STT/MT may produce rare benign exhaustions under throttling.",
            "sink_write_p99_ms": "10 ms matches audio frame cadence; relax to ≤20 ms only with evidence.",
        },
        "follow_up": "Live wiring into wasapi_capture/audio_sink/pipeline + QA8-05 runner consumption + 30-min calibration soak — see PR #540 follow-up plan.",
    })
}
#[cfg(test)]
#[path = "thresholds_tests.rs"]
mod tests;
