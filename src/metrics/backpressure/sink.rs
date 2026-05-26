//! Audio sink / virtual-mic underrun, write-latency, and fanout-drop telemetry.

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};

use super::histogram::HistogramUs;

/// Audio sink (virtual mic) underrun, write latency, and fanout drop
/// telemetry.
#[derive(Debug, Default)]
pub struct SinkBackpressure {
    writes: AtomicU64,
    bytes_written: AtomicU64,
    underruns: AtomicU64,
    fanout_drops: AtomicU64,
    write_latency: HistogramUs,
}

impl SinkBackpressure {
    /// Construct an empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// One successful sink write of `bytes` taking `latency_ns`.
    pub fn record_write(&self, bytes: u64, latency_ns: u64) {
        self.writes.fetch_add(1, Ordering::Relaxed);
        self.bytes_written.fetch_add(bytes, Ordering::Relaxed);
        self.write_latency.record_us(latency_ns / 1_000);
    }

    /// One underrun (sink ran out of data before the next write
    /// arrived).
    pub fn record_underrun(&self) {
        self.underruns.fetch_add(1, Ordering::Relaxed);
    }

    /// One fanout drop observed (e.g. slot-A or slot-B `try_send`
    /// failure). This mirrors the fanout `FanoutDropCounters` so QA8-05
    /// can read a single object.
    pub fn record_fanout_drop(&self) {
        self.fanout_drops.fetch_add(1, Ordering::Relaxed);
    }

    /// Total successful writes.
    pub fn writes(&self) -> u64 {
        self.writes.load(Ordering::Relaxed)
    }

    /// Total bytes written.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written.load(Ordering::Relaxed)
    }

    /// Total underruns.
    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }

    /// Total fanout drops.
    pub fn fanout_drops(&self) -> u64 {
        self.fanout_drops.load(Ordering::Relaxed)
    }

    /// Direct access to the write-latency distribution.
    pub fn write_latency(&self) -> &HistogramUs {
        &self.write_latency
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "writes": self.writes(),
            "bytes_written": self.bytes_written(),
            "underruns": self.underruns(),
            "fanout_drops": self.fanout_drops(),
            "write_latency": self.write_latency.to_json(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn underrun_independent_of_writes() {
        let s = SinkBackpressure::new();
        s.record_write(1024, 1_000_000);
        s.record_underrun();
        s.record_write(1024, 1_000_000);
        s.record_underrun();
        assert_eq!(s.writes(), 2);
        assert_eq!(s.underruns(), 2);
        assert_eq!(s.bytes_written(), 2048);
    }

    #[test]
    fn fanout_drop_independent_counter() {
        let s = SinkBackpressure::new();
        s.record_fanout_drop();
        s.record_fanout_drop();
        s.record_fanout_drop();
        assert_eq!(s.fanout_drops(), 3);
    }
}
