//! Linux stub for [`super::PlaybackService`].
//!
//! Included as `#[path = "playback_stub.rs"] mod imp;` inside
//! `playback.rs` under `#[cfg(not(any(windows, target_os = "macos")))]`.
//!
//! Linux CI only needs this stub so the pipeline and contract layers can
//! compile without system audio packages (ALSA/PulseAudio).  Windows and
//! macOS use the real rodio-backed implementation in `playback.rs`.
//!
//! `new()` silently drops all audio via a lightweight [`NoOpSink`].
//! `with_sink()` spawns a background thread and routes audio through the
//! provided sink — enabling cross-platform tests with [`MockAudioSink`].
//! `new_with_vmic_sink()` exercises the watchdog-fallback loop with an
//! injectable sink and health channel.
//!
//! [`MockAudioSink`]: crate::pipeline::audio_sink::MockAudioSink

use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

use super::super::audio_sink::AudioSink;
use super::{build_sinks_for_targets, play_to_audio_sinks, PlaybackRoutePlan, PlaybackSinkTarget};

enum PlaybackCmd {
    Play(Vec<u8>),
    Shutdown,
}

/// Playback service stub for Linux builds.
///
/// See module-level documentation for details.
pub struct PlaybackService {
    tx: mpsc::Sender<PlaybackCmd>,
    enabled: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    route_label: &'static str,
    vmic_device: Option<String>,
}

impl PlaybackService {
    /// No-op constructor: audio bytes are silently dropped.
    pub fn new(enabled: bool, _output_device: Option<&str>) -> std::io::Result<Self> {
        Self::new_with_route(enabled, PlaybackRoutePlan::speakers(None))
    }

    /// Start the playback service from config routing fields.
    pub fn new_for_config(
        enabled: bool,
        routing: crate::config::TtsRouting,
        tts_output_device: Option<&str>,
        virtual_mic_device: Option<&str>,
    ) -> std::io::Result<Self> {
        Self::new_with_route(
            enabled,
            PlaybackRoutePlan::from_config(routing, tts_output_device, virtual_mic_device)?,
        )
    }

    /// No-op route constructor for non-Windows builds.
    pub fn new_with_route(enabled: bool, route: PlaybackRoutePlan) -> std::io::Result<Self> {
        let route_label = route.label();
        let sink_count = route.targets().len();
        let vmic_device = route.targets().iter().find_map(|t| {
            if let PlaybackSinkTarget::VirtualMic { device } = t {
                Some(device.clone())
            } else {
                None
            }
        });
        let sinks = build_sinks_for_targets(route.targets(), |_| {
            Ok::<Box<dyn AudioSink>, String>(Box::new(NoOpSink))
        })?;
        tracing::info!(
            route = route_label,
            sinks = sink_count,
            "tts-playback: started no-op route"
        );
        Self::with_sinks_and_label(enabled, sinks, route_label, vmic_device)
    }

    /// Stub: routes audio through `vmic_sink` and monitors `health_rx`.
    ///
    /// On Linux CI this exercises the watchdog-fallback logic using a
    /// [`MockAudioSink`] without any audio hardware.  On device-loss the
    /// stub silences audio (no `RodioSink` available on Linux).
    ///
    /// [`MockAudioSink`]: crate::pipeline::audio_sink::MockAudioSink
    pub fn new_with_vmic_sink(
        enabled: bool,
        vmic_device: impl Into<String>,
        vmic_sink: Box<dyn AudioSink>,
        mut health_rx: tokio::sync::watch::Receiver<crate::audio::device_watchdog::SubsystemHealth>,
    ) -> std::io::Result<Self> {
        use crate::audio::device_watchdog::SubsystemHealth;
        use std::sync::mpsc::RecvTimeoutError;
        use std::time::Duration;

        let device = vmic_device.into();
        let (tx, rx) = mpsc::channel::<PlaybackCmd>();
        let enabled_flag = Arc::new(AtomicBool::new(enabled));
        let thread_enabled = Arc::clone(&enabled_flag);
        let device_for_log = device.clone();

        let thread = std::thread::Builder::new()
            .name("tts-playback".to_string())
            .spawn(move || {
                let mut active: Option<Box<dyn AudioSink>> = Some(vmic_sink);
                let mut warned_fallback = false;

                loop {
                    if health_rx.has_changed().unwrap_or(false) {
                        let health = health_rx.borrow_and_update().clone();
                        if matches!(health, SubsystemHealth::Failed { .. }) && !warned_fallback {
                            warned_fallback = true;
                            tracing::warn!(
                                device = %device_for_log,
                                "tts-playback: virtual-mic device lost; \
                                 falling back to system speaker"
                            );
                            // No RodioSink on Linux CI; audio is silenced.
                            active = None;
                        }
                    }

                    let cmd = match rx.recv_timeout(Duration::from_millis(20)) {
                        Ok(cmd) => cmd,
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => {
                            tracing::info!("tts-playback: stopping");
                            return;
                        }
                    };

                    match cmd {
                        PlaybackCmd::Play(bytes) => {
                            if !thread_enabled.load(Ordering::SeqCst) {
                                continue;
                            }
                            if let Some(ref s) = active {
                                s.play_bytes(bytes);
                            }
                        }
                        PlaybackCmd::Shutdown => {
                            tracing::info!("tts-playback: stopping");
                            return;
                        }
                    }
                }
            })?;

        Ok(Self {
            tx,
            enabled: enabled_flag,
            thread: Some(thread),
            route_label: "virtual_mic",
            vmic_device: Some(device),
        })
    }

    /// Start the playback service using a caller-supplied [`AudioSink`].
    ///
    /// Enables cross-platform testing: pass a [`MockAudioSink`] to record
    /// submitted chunks without any hardware dependency.
    ///
    /// [`MockAudioSink`]: crate::pipeline::audio_sink::MockAudioSink
    pub fn with_sink(enabled: bool, sink: Box<dyn AudioSink>) -> std::io::Result<Self> {
        Self::with_sinks(enabled, vec![sink])
    }

    /// Start the playback service with already-initialised sinks.
    pub fn with_sinks(enabled: bool, sinks: Vec<Box<dyn AudioSink>>) -> std::io::Result<Self> {
        Self::with_sinks_and_label(enabled, sinks, "custom", None)
    }

    fn with_sinks_and_label(
        enabled: bool,
        sinks: Vec<Box<dyn AudioSink>>,
        route_label: &'static str,
        vmic_device: Option<String>,
    ) -> std::io::Result<Self> {
        if sinks.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "TTS playback route must contain at least one sink",
            ));
        }
        let (tx, rx) = mpsc::channel::<PlaybackCmd>();
        let enabled_flag = Arc::new(AtomicBool::new(enabled));
        let thread_enabled = Arc::clone(&enabled_flag);

        let thread = std::thread::Builder::new()
            .name("tts-playback".to_string())
            .spawn(move || {
                run_loop_with(rx, thread_enabled, move |bytes| {
                    play_to_audio_sinks(&sinks, bytes)
                });
            })?;

        Ok(Self {
            tx,
            enabled: enabled_flag,
            thread: Some(thread),
            route_label,
            vmic_device,
        })
    }

    /// Human-readable route label used by startup/status messages.
    pub fn route_label(&self) -> &'static str {
        self.route_label
    }

    /// Virtual-mic device name when this service was started with a vmic route.
    ///
    /// Returns `None` for speaker-only routes.
    pub fn vmic_device_name(&self) -> Option<&str> {
        self.vmic_device.as_deref()
    }

    /// Submit `audio_bytes` (MP3) for playback.
    pub fn play(&self, audio_bytes: Vec<u8>) {
        if !self.enabled.load(Ordering::SeqCst) {
            return;
        }
        let _ = self.tx.send(PlaybackCmd::Play(audio_bytes));
    }

    /// Enable or disable playback at runtime.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    /// Signal the playback thread to stop and wait for it to exit.
    pub fn shutdown(&mut self) {
        let _ = self.tx.send(PlaybackCmd::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for PlaybackService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Discards all audio; used by the no-op `new()` path on non-Windows builds.
struct NoOpSink;

impl AudioSink for NoOpSink {
    fn play_bytes(&self, _audio_bytes: Vec<u8>) {}
}

fn run_loop_with<F>(rx: mpsc::Receiver<PlaybackCmd>, enabled: Arc<AtomicBool>, mut play_fn: F)
where
    F: FnMut(Vec<u8>),
{
    loop {
        match rx.recv() {
            Ok(PlaybackCmd::Play(bytes)) => {
                if !enabled.load(Ordering::SeqCst) {
                    continue;
                }
                play_fn(bytes);
            }
            Ok(PlaybackCmd::Shutdown) | Err(_) => {
                tracing::info!("tts-playback: stopping");
                break;
            }
        }
    }
}
