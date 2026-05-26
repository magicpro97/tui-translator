//! Indirection layer between pipeline sink code (`audio_sink`) and the
//! QA8-07 backpressure telemetry registry (`metrics::backpressure::emit`).
//!
//! See issue #505 (QA8-07).

use std::sync::OnceLock;

static SINK_WRITE: OnceLock<fn(u64, u64)> = OnceLock::new();
static SINK_UNDERRUN: OnceLock<fn()> = OnceLock::new();

pub fn install_sink_write(f: fn(u64, u64)) {
    let _ = SINK_WRITE.set(f);
}

pub fn install_sink_underrun(f: fn()) {
    let _ = SINK_UNDERRUN.set(f);
}

#[inline]
pub(crate) fn sink_write(bytes: u64, latency_ns: u64) {
    if let Some(f) = SINK_WRITE.get() {
        f(bytes, latency_ns);
    }
}

#[inline]
pub(crate) fn sink_underrun() {
    if let Some(f) = SINK_UNDERRUN.get() {
        f();
    }
}
