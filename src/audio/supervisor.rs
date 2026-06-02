//! HC-03: Capture stream supervisor (lifecycle + gap metrics).
//!
//! Manages the lifecycle of a [`CaptureStream`] and records device-switch
//! metrics.  Config-change *classification* is handled by
//! `crate::config::capture_supervisor` so that audio types are not pulled
//! into the config module.
//!
//! # BLOCKED / SPLIT_REQUIRED — orchestrator wiring
//!
//! Wiring the supervisor into the live orchestrator requires `run_orchestrator`
//! to support swapping its owned `mpsc::Receiver<AudioChunk>` while running.
//! That change is deferred to a follow-up PR.  See
//! `crate::config::capture_supervisor` for the full BLOCKED documentation.

#![allow(dead_code)]

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Instant;

use anyhow::Result;

use super::{start_capture_with_device, CaptureStream};

// -- CaptureMetrics

/// Shared atomic counters for capture stream lifecycle diagnostics.
#[derive(Debug, Default)]
pub struct CaptureMetrics {
    /// Number of successful device switches since application start.
    device_switch_count: AtomicU64,
    /// Total gap time in milliseconds accumulated across all device switches.
    total_capture_gap_ms: AtomicU64,
}

impl CaptureMetrics {
    /// Create a zeroed metrics set wrapped in an `Arc`.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Increment the device-switch counter and add `gap_ms` to the total gap.
    pub fn record_switch(&self, gap_ms: u64) {
        self.device_switch_count.fetch_add(1, Ordering::Relaxed);
        self.total_capture_gap_ms
            .fetch_add(gap_ms, Ordering::Relaxed);
    }

    /// Number of device switches recorded since application start.
    pub fn device_switch_count(&self) -> u64 {
        self.device_switch_count.load(Ordering::Relaxed)
    }

    /// Total gap time in milliseconds accumulated across all switches.
    pub fn total_capture_gap_ms(&self) -> u64 {
        self.total_capture_gap_ms.load(Ordering::Relaxed)
    }
}

// -- CaptureStreamSupervisor

/// Manages the [`CaptureStream`] lifecycle and records device-switch metrics.
pub struct CaptureStreamSupervisor {
    metrics: Arc<CaptureMetrics>,
    silence_threshold: f32,
}

impl CaptureStreamSupervisor {
    /// Create a supervisor with the given silence threshold and shared metrics.
    pub fn new(silence_threshold: f32, metrics: Arc<CaptureMetrics>) -> Self {
        Self {
            metrics,
            silence_threshold,
        }
    }

    /// Start a fresh [`CaptureStream`] for `device_name`.
    pub async fn start(&self, device_name: Option<&str>) -> Result<CaptureStream> {
        let audio_source = super::platform_default_audio_source();
        start_capture_with_device(device_name, audio_source, self.silence_threshold).await
    }

    /// Stop `old_stream` and start a new one for `new_device`, recording gap metrics.
    ///
    /// Returns `(new_stream, gap_ms)`.  The caller is responsible for wiring
    /// `new_stream.receiver` into the consumer; see the BLOCKED note in module docs.
    pub async fn restart(
        &self,
        old_stream: CaptureStream,
        new_device: Option<&str>,
    ) -> Result<(CaptureStream, u64)> {
        let old_device = old_stream.info.device_name.clone();
        let stop_start = Instant::now();
        drop(old_stream);
        let audio_source = super::platform_default_audio_source();
        let new_stream =
            start_capture_with_device(new_device, audio_source, self.silence_threshold).await?;
        let gap_ms = stop_start.elapsed().as_millis() as u64;
        tracing::info!(
            old_device = %old_device,
            new_device = ?new_device,
            gap_ms,
            "capture stream restarted"
        );
        self.metrics.record_switch(gap_ms);
        Ok((new_stream, gap_ms))
    }

    /// Shared access to the capture lifecycle metrics.
    pub fn metrics(&self) -> &Arc<CaptureMetrics> {
        &self.metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_start_at_zero() {
        let m = CaptureMetrics::new();
        assert_eq!(m.device_switch_count(), 0);
        assert_eq!(m.total_capture_gap_ms(), 0);
    }

    #[test]
    fn metrics_record_switch_accumulates() {
        let m = CaptureMetrics::new();
        m.record_switch(100);
        m.record_switch(250);
        assert_eq!(m.device_switch_count(), 2);
        assert_eq!(m.total_capture_gap_ms(), 350);
    }

    #[test]
    fn metrics_independent_arcs() {
        let m = CaptureMetrics::new();
        let m2 = Arc::clone(&m);
        m.record_switch(10);
        assert_eq!(m2.device_switch_count(), 1);
        assert_eq!(m2.total_capture_gap_ms(), 10);
    }

    // macOS now uses real CoreAudio/BlackHole capture (MACOS-07, #638) which
    // fails on CI without BlackHole.  These stub-silence tests only apply to
    // Linux and other non-Windows/non-macOS platforms.
    #[cfg(not(any(windows, target_os = "macos")))]
    #[tokio::test]
    async fn supervisor_start_non_wasapi() {
        let metrics = CaptureMetrics::new();
        let supervisor = CaptureStreamSupervisor::new(0.001, Arc::clone(&metrics));
        let stream = supervisor.start(None).await.expect("start");
        assert!(
            stream.info.device_name.contains("silent") || stream.info.device_name.contains("stub")
        );
        drop(stream);
        assert_eq!(metrics.device_switch_count(), 0);
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[tokio::test]
    async fn supervisor_restart_records_gap_metrics() {
        let metrics = CaptureMetrics::new();
        let supervisor = CaptureStreamSupervisor::new(0.001, Arc::clone(&metrics));
        let old = supervisor.start(None).await.expect("start");
        let (_new_stream, gap_ms) = supervisor.restart(old, None).await.expect("restart");
        assert!(gap_ms < 60_000, "gap_ms={gap_ms} too large");
        assert_eq!(metrics.device_switch_count(), 1);
        assert_eq!(metrics.total_capture_gap_ms(), gap_ms);
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[tokio::test]
    async fn supervisor_multiple_restarts_accumulate() {
        let metrics = CaptureMetrics::new();
        let supervisor = CaptureStreamSupervisor::new(0.001, Arc::clone(&metrics));
        let s1 = supervisor.start(None).await.expect("start");
        let (s2, g1) = supervisor.restart(s1, None).await.expect("restart 1");
        let (_s3, g2) = supervisor.restart(s2, None).await.expect("restart 2");
        assert_eq!(metrics.device_switch_count(), 2);
        assert_eq!(metrics.total_capture_gap_ms(), g1 + g2);
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[tokio::test]
    async fn supervisor_new_stream_is_functional_after_restart() {
        use tokio::time::{timeout, Duration};
        let metrics = CaptureMetrics::new();
        let supervisor = CaptureStreamSupervisor::new(0.001, Arc::clone(&metrics));
        let old = supervisor.start(None).await.expect("start");
        let (mut new_stream, _gap) = supervisor.restart(old, None).await.expect("restart");
        let chunk = timeout(Duration::from_secs(2), new_stream.receiver.recv())
            .await
            .expect("should receive within 2 s")
            .expect("channel should be open");
        assert!(!chunk.samples.is_empty());
    }
}
