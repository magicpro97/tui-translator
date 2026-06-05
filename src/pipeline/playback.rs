//! TTS playback service.
//!
//! On Windows and macOS this module owns a dedicated playback thread backed by
//! `RodioSink` (WASAPI on Windows, CoreAudio on macOS). On Linux it provides a
//! lightweight stub so CI can build and test the project without optional system
//! audio libraries such as ALSA.
//!
//! The concrete audio backend is abstracted by the [`AudioSink`] trait
//! (see [`super::audio_sink`]).  A custom sink — including [`MockAudioSink`]
//! for deterministic tests — can be injected via [`PlaybackService::with_sink`]
//! on every platform.
//!
//! [`AudioSink`]: super::audio_sink::AudioSink
//! [`MockAudioSink`]: super::audio_sink::MockAudioSink

#[path = "playback_routing.rs"]
mod playback_routing;
use playback_routing::{build_sinks_for_targets, play_to_audio_sinks};
#[allow(unused_imports)]
pub use playback_routing::{PlaybackRoutePlan, PlaybackSinkTarget};

// ── Windows and macOS implementation ─────────────────────────────────────────

#[cfg(any(windows, target_os = "macos"))]
mod imp {
    use std::collections::VecDeque;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError},
        Arc,
    };
    use std::time::Duration;

    use super::super::audio_sink::{AudioSink, RodioPlaybackOutcome, RodioSink};
    use super::{
        build_sinks_for_targets, play_to_audio_sinks, PlaybackRoutePlan, PlaybackSinkTarget,
    };
    use crate::audio::device_watchdog::SubsystemHealth;

    enum PlaybackCmd {
        Play(Vec<u8>),
        Shutdown,
    }

    /// A background TTS playback service.
    ///
    /// Receives raw MP3 audio bytes and plays them sequentially through the
    /// configured audio backend.  Audio clips are played one at a time; a new
    /// clip arriving while another is playing queues behind the current one.
    pub struct PlaybackService {
        tx: mpsc::Sender<PlaybackCmd>,
        enabled: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
        route_label: &'static str,
        /// Virtual-mic device name tracked for hot-reload idempotency.
        vmic_device: Option<String>,
    }

    impl PlaybackService {
        /// Start the playback service using the default [`RodioSink`].
        ///
        /// When `output_device` is `Some(name)`, the playback thread attempts
        /// to open that device by name; otherwise it uses the system default.
        ///
        /// A startup channel propagates any device-open failure back to the
        /// caller as an `Err`.
        pub fn new(enabled: bool, output_device: Option<&str>) -> std::io::Result<Self> {
            Self::new_with_route(enabled, PlaybackRoutePlan::speakers(output_device))
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

        /// Start the playback service using a resolved route plan.
        ///
        /// When the route contains a [`PlaybackSinkTarget::VirtualMic`] target,
        /// `OemCableSink::new_windows` is used inside the spawned thread and the
        /// device-loss watchdog is subscribed automatically.  For speaker-only
        /// routes the existing [`RodioSink`] path is used unchanged.
        pub fn new_with_route(enabled: bool, route: PlaybackRoutePlan) -> std::io::Result<Self> {
            let vmic_device = route.targets().iter().find_map(|t| {
                if let PlaybackSinkTarget::VirtualMic { device } = t {
                    Some(device.clone())
                } else {
                    None
                }
            });

            if let Some(ref device) = vmic_device {
                // VirtualMic route: start watchdog and use OemCableSink.
                let device_owned = device.clone();
                let route_label = route.label();

                // Start the watchdog before the thread so the health receiver
                // is available as soon as audio starts flowing.  On non-Windows
                // builds `start_watching` returns a no-op healthy watchdog.
                let health_rx = crate::audio::device_watchdog::start_watching(&device_owned)
                    .map(|wd| wd.subscribe())
                    .unwrap_or_else(|e| {
                        tracing::warn!(
                            device = %device_owned,
                            error = %e,
                            "tts-playback: could not start vmic watchdog; \
                             device-loss detection disabled"
                        );
                        let (tx, rx) = tokio::sync::watch::channel(SubsystemHealth::Healthy);
                        // Keep the sender alive for the service lifetime.
                        std::mem::forget(tx);
                        rx
                    });

                return Self::new_with_vmic_sink_internal(
                    enabled,
                    device_owned.clone(),
                    route_label,
                    move || {
                        #[cfg(windows)]
                        {
                            crate::pipeline::audio_sink::OemCableSink::new_windows(
                                device_owned.as_str(),
                            )
                            .map(|s| Box::new(s) as Box<dyn AudioSink>)
                            .map_err(|e| e.to_string())
                        }
                        #[cfg(not(windows))]
                        {
                            let _ = device_owned;
                            Err("OemCableSink requires Windows WASAPI".to_string())
                        }
                    },
                    health_rx,
                );
            }

            // Speaker-only route: existing RodioSink path.
            let (tx, rx) = mpsc::channel::<PlaybackCmd>();
            let (startup_tx, startup_rx) = mpsc::channel::<Result<(), String>>();
            let enabled_flag = Arc::new(AtomicBool::new(enabled));
            let thread_enabled = Arc::clone(&enabled_flag);
            let route_label = route.label();
            let targets = route.targets().to_vec();

            let thread = std::thread::Builder::new()
                .name("tts-playback".to_string())
                .spawn(move || {
                    // Device I/O happens inside the thread so rodio's OutputStream
                    // lifetime (and !Send bound) stay on this OS thread.
                    let sinks = match build_sinks_for_targets(&targets, |target| {
                        RodioSink::try_new(target.output_device())
                    }) {
                        Ok(sinks) => sinks,
                        Err(e) => {
                            let msg = e.to_string();
                            let _ = startup_tx.send(Err(msg.clone()));
                            tracing::error!("tts-playback: {msg}");
                            return;
                        }
                    };
                    let _ = startup_tx.send(Ok(()));
                    tracing::info!(
                        route = route_label,
                        sinks = sinks.len(),
                        "tts-playback: started"
                    );
                    run_rodio_loop(rx, thread_enabled, sinks);
                })?;

            match startup_rx.recv() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    let _ = thread.join();
                    return Err(std::io::Error::other(error));
                }
                Err(_) => {
                    let _ = thread.join();
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "tts-playback failed to report startup status",
                    ));
                }
            }

            Ok(Self {
                tx,
                enabled: enabled_flag,
                thread: Some(thread),
                route_label,
                vmic_device: None,
            })
        }

        /// Start a virtual-mic playback service with an injectable sink and health channel.
        ///
        /// `vmic_sink` receives all TTS audio while the device is healthy.
        /// When `health_rx` delivers [`SubsystemHealth::Failed`], the service
        /// falls back to the system-default speaker output and emits a single
        /// [`tracing::warn!`] (subsequent `Failed` events are silently ignored —
        /// idempotent).
        ///
        /// This constructor is used by [`PlaybackService::new_with_route`] (production
        /// path) and is the primary test entry-point: pass a [`MockAudioSink`] to
        /// exercise the fallback logic without real hardware.
        ///
        /// [`MockAudioSink`]: crate::pipeline::audio_sink::MockAudioSink
        pub fn new_with_vmic_sink(
            enabled: bool,
            vmic_device: impl Into<String>,
            vmic_sink: Box<dyn AudioSink>,
            health_rx: tokio::sync::watch::Receiver<SubsystemHealth>,
        ) -> std::io::Result<Self> {
            let device = vmic_device.into();
            let sink_cell = std::sync::Mutex::new(Some(vmic_sink));
            Self::new_with_vmic_sink_internal(
                enabled,
                device,
                "virtual_mic",
                move || {
                    sink_cell
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .take()
                        .ok_or_else(|| "vmic sink already consumed".to_string())
                },
                health_rx,
            )
        }

        /// Internal constructor shared by the production and test paths.
        fn new_with_vmic_sink_internal(
            enabled: bool,
            vmic_device: String,
            route_label: &'static str,
            make_sink: impl FnOnce() -> Result<Box<dyn AudioSink>, String> + Send + 'static,
            health_rx: tokio::sync::watch::Receiver<SubsystemHealth>,
        ) -> std::io::Result<Self> {
            let (tx, rx) = mpsc::channel::<PlaybackCmd>();
            let (startup_tx, startup_rx) = mpsc::channel::<Result<(), String>>();
            let enabled_flag = Arc::new(AtomicBool::new(enabled));
            let thread_enabled = Arc::clone(&enabled_flag);
            let device_for_log = vmic_device.clone();

            let thread = std::thread::Builder::new()
                .name("tts-playback".to_string())
                .spawn(move || {
                    let primary_sink = match make_sink() {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(
                                device = %device_for_log,
                                error = %e,
                                "tts-playback: vmic sink unavailable; falling back to speaker"
                            );
                            let _ = startup_tx.send(Ok(()));
                            run_vmic_loop(rx, thread_enabled, None, health_rx, &device_for_log);
                            return;
                        }
                    };
                    let _ = startup_tx.send(Ok(()));
                    tracing::info!(
                        route = route_label,
                        device = %device_for_log,
                        "tts-playback: virtual-mic route started"
                    );
                    run_vmic_loop(
                        rx,
                        thread_enabled,
                        Some(primary_sink),
                        health_rx,
                        &device_for_log,
                    );
                })?;

            match startup_rx.recv() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    let _ = thread.join();
                    return Err(std::io::Error::other(error));
                }
                Err(_) => {
                    let _ = thread.join();
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "tts-playback failed to report startup status",
                    ));
                }
            }

            Ok(Self {
                tx,
                enabled: enabled_flag,
                thread: Some(thread),
                route_label,
                vmic_device: Some(vmic_device),
            })
        }

        /// Start the playback service using a caller-supplied [`AudioSink`].
        ///
        /// No startup notification channel is used; if the sink requires
        /// fallible initialisation, the caller should perform it before
        /// constructing the sink and passing it here.
        ///
        /// This constructor is primarily intended for tests (with
        /// [`MockAudioSink`]) and for future virtual-mic sinks.
        ///
        /// [`MockAudioSink`]: super::super::audio_sink::MockAudioSink
        pub fn with_sink(enabled: bool, sink: Box<dyn AudioSink>) -> std::io::Result<Self> {
            Self::with_sinks(enabled, vec![sink])
        }

        /// Start the playback service with already-initialised sinks.
        pub fn with_sinks(enabled: bool, sinks: Vec<Box<dyn AudioSink>>) -> std::io::Result<Self> {
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
                route_label: "custom",
                vmic_device: None,
            })
        }

        /// Human-readable route label used by startup/status messages.
        pub fn route_label(&self) -> &'static str {
            self.route_label
        }

        /// Virtual-mic device name when this service was started with a vmic route.
        ///
        /// Returns `None` for speaker-only routes.  Used by the hot-reload path
        /// to detect when the device name has not changed (idempotent re-configure).
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

    /// Core event loop used by [`PlaybackService::with_sink`].
    ///
    /// Receives commands from `rx` and calls `play_fn` for each audio chunk
    /// while `enabled` is true.  Exits on [`PlaybackCmd::Shutdown`] or when
    /// the channel is disconnected.
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

    // ``run_vmic_loop`` is included textually via ``include!`` rather than
    // ``mod`` so we avoid Rust's inline-mod ``#[path]`` traversal through a
    // non-existent ``playback/imp/`` directory (which works on Windows but
    // fails on macOS file APIs that do not normalise ``..`` through phantom
    // dirs).  The file is generated by ``cargo`` as a sibling of ``playback.rs``.
    include!("playback_vmic.rs");

    fn run_rodio_loop(
        rx: mpsc::Receiver<PlaybackCmd>,
        enabled: Arc<AtomicBool>,
        sinks: Vec<RodioSink>,
    ) {
        let mut pending = VecDeque::new();

        loop {
            let command = match pending.pop_front() {
                Some(command) => Ok(command),
                None => rx.recv(),
            };

            match command {
                Ok(PlaybackCmd::Play(bytes)) => {
                    if !enabled.load(Ordering::SeqCst) {
                        continue;
                    }

                    let mut stopping = false;
                    let outcome = play_rodio_sinks_until_interrupted(&sinks, bytes, || {
                        if !enabled.load(Ordering::SeqCst) {
                            return true;
                        }

                        match rx.recv_timeout(Duration::from_millis(20)) {
                            Ok(PlaybackCmd::Play(bytes)) => {
                                pending.push_back(PlaybackCmd::Play(bytes));
                                false
                            }
                            Ok(PlaybackCmd::Shutdown) => {
                                stopping = true;
                                true
                            }
                            Err(RecvTimeoutError::Timeout) => false,
                            Err(RecvTimeoutError::Disconnected) => {
                                stopping = true;
                                true
                            }
                        }
                    });

                    if stopping {
                        tracing::info!("tts-playback: stopping");
                        return;
                    }

                    if matches!(outcome, RodioPlaybackOutcome::Interrupted) {
                        continue;
                    }
                }
                Ok(PlaybackCmd::Shutdown) | Err(_) => {
                    tracing::info!("tts-playback: stopping");
                    break;
                }
            }
        }
    }

    fn play_rodio_sinks_until_interrupted<F>(
        sinks: &[RodioSink],
        audio_bytes: Vec<u8>,
        mut should_interrupt: F,
    ) -> RodioPlaybackOutcome
    where
        F: FnMut() -> bool,
    {
        if sinks.len() == 1 {
            return sinks[0].play_bytes_until_interrupted(audio_bytes, should_interrupt);
        }

        let mut active = Vec::new();
        for sink in sinks.iter().take(sinks.len().saturating_sub(1)) {
            if let Some(sink) = sink.start_sink(audio_bytes.clone()) {
                active.push(sink);
            }
        }
        if let Some(last) = sinks.last().and_then(|sink| sink.start_sink(audio_bytes)) {
            active.push(last);
        }

        loop {
            active.retain(|sink| !sink.empty());
            if active.is_empty() {
                return RodioPlaybackOutcome::Completed;
            }
            if should_interrupt() {
                for sink in active {
                    sink.stop();
                }
                return RodioPlaybackOutcome::Interrupted;
            }
        }
    }
}

// ── Linux stub (no system audio required) ────────────────────────────────────
//
// Extracted to `playback_stub.rs` to keep this file under the 600 LOC
// engineering-standards gate.  The module is re-exported identically to
// the Windows / macOS `imp` above.

#[cfg(not(any(windows, target_os = "macos")))]
#[path = "playback_stub.rs"]
mod imp;

// ── Re-export ────────────────────────────────────────────────────────────────

#[allow(unused_imports)]
pub use imp::PlaybackService;

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "playback_tests.rs"]
mod tests;
