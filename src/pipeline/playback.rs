//! TTS playback service.
//!
//! On Windows this module owns a dedicated playback thread backed by `RodioSink`.
//! On other platforms it provides a lightweight stub so Linux CI can build and
//! test the project without optional system audio libraries such as ALSA.
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

// ── Windows implementation ───────────────────────────────────────────────────

#[cfg(windows)]
mod imp {
    use std::collections::VecDeque;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError},
        Arc,
    };
    use std::time::Duration;

    use super::super::audio_sink::{AudioSink, RodioPlaybackOutcome, RodioSink};
    use super::{build_sinks_for_targets, play_to_audio_sinks, PlaybackRoutePlan};

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
        pub fn new_with_route(enabled: bool, route: PlaybackRoutePlan) -> std::io::Result<Self> {
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
            })
        }

        /// Human-readable route label used by startup/status messages.
        pub fn route_label(&self) -> &'static str {
            self.route_label
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

// ── Non-Windows stub ─────────────────────────────────────────────────────────

#[cfg(not(windows))]
mod imp {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    };

    use super::super::audio_sink::AudioSink;
    use super::{build_sinks_for_targets, play_to_audio_sinks, PlaybackRoutePlan};

    enum PlaybackCmd {
        Play(Vec<u8>),
        Shutdown,
    }

    /// Playback service stub for non-Windows builds.
    ///
    /// The product is Windows-native; Linux CI only needs this stub so the
    /// pipeline and contract layers can compile without system audio packages.
    ///
    /// `new()` silently drops all audio via a lightweight [`NoOpSink`].
    /// `with_sink()` spawns a background thread and routes audio through the
    /// provided sink — enabling cross-platform tests with [`MockAudioSink`].
    ///
    /// [`MockAudioSink`]: super::super::audio_sink::MockAudioSink
    pub struct PlaybackService {
        tx: mpsc::Sender<PlaybackCmd>,
        enabled: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
        route_label: &'static str,
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
            let sinks = build_sinks_for_targets(route.targets(), |_| {
                Ok::<Box<dyn AudioSink>, String>(Box::new(NoOpSink))
            })?;
            tracing::info!(
                route = route_label,
                sinks = sink_count,
                "tts-playback: started no-op route"
            );
            Self::with_sinks_and_label(enabled, sinks, route_label)
        }

        /// Start the playback service using a caller-supplied [`AudioSink`].
        ///
        /// Enables cross-platform testing: pass a [`MockAudioSink`] to record
        /// submitted chunks without any hardware dependency.
        ///
        /// [`MockAudioSink`]: super::super::audio_sink::MockAudioSink
        pub fn with_sink(enabled: bool, sink: Box<dyn AudioSink>) -> std::io::Result<Self> {
            Self::with_sinks(enabled, vec![sink])
        }

        /// Start the playback service with already-initialised sinks.
        pub fn with_sinks(enabled: bool, sinks: Vec<Box<dyn AudioSink>>) -> std::io::Result<Self> {
            Self::with_sinks_and_label(enabled, sinks, "custom")
        }

        fn with_sinks_and_label(
            enabled: bool,
            sinks: Vec<Box<dyn AudioSink>>,
            route_label: &'static str,
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
            })
        }

        /// Human-readable route label used by startup/status messages.
        pub fn route_label(&self) -> &'static str {
            self.route_label
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
}

// ── Re-export ────────────────────────────────────────────────────────────────

#[allow(unused_imports)]
pub use imp::PlaybackService;

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "playback_tests.rs"]
mod tests;
