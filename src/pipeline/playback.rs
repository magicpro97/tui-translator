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
            let (tx, rx) = mpsc::channel::<PlaybackCmd>();
            let (startup_tx, startup_rx) = mpsc::channel::<Result<(), String>>();
            let enabled_flag = Arc::new(AtomicBool::new(enabled));
            let thread_enabled = Arc::clone(&enabled_flag);
            let output_device = output_device.map(str::to_owned);

            let thread = std::thread::Builder::new()
                .name("tts-playback".to_string())
                .spawn(move || {
                    // Device I/O happens inside the thread so rodio's OutputStream
                    // lifetime (and !Send bound) stay on this OS thread.
                    let sink = match RodioSink::try_new(output_device.as_deref()) {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = startup_tx.send(Err(e.clone()));
                            tracing::error!("tts-playback: {e}");
                            return;
                        }
                    };
                    let _ = startup_tx.send(Ok(()));
                    run_rodio_loop(rx, thread_enabled, sink);
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
            let (tx, rx) = mpsc::channel::<PlaybackCmd>();
            let enabled_flag = Arc::new(AtomicBool::new(enabled));
            let thread_enabled = Arc::clone(&enabled_flag);

            let thread = std::thread::Builder::new()
                .name("tts-playback".to_string())
                .spawn(move || {
                    run_loop_with(rx, thread_enabled, move |bytes| sink.play_bytes(bytes));
                })?;

            Ok(Self {
                tx,
                enabled: enabled_flag,
                thread: Some(thread),
            })
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

    fn run_rodio_loop(rx: mpsc::Receiver<PlaybackCmd>, enabled: Arc<AtomicBool>, sink: RodioSink) {
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
                    let outcome = sink.play_bytes_until_interrupted(bytes, || {
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
}

// ── Non-Windows stub ─────────────────────────────────────────────────────────

#[cfg(not(windows))]
mod imp {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    };

    use super::super::audio_sink::AudioSink;

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
    }

    impl PlaybackService {
        /// No-op constructor: audio bytes are silently dropped.
        pub fn new(enabled: bool, _output_device: Option<&str>) -> std::io::Result<Self> {
            Self::with_sink(enabled, Box::new(NoOpSink))
        }

        /// Start the playback service using a caller-supplied [`AudioSink`].
        ///
        /// Enables cross-platform testing: pass a [`MockAudioSink`] to record
        /// submitted chunks without any hardware dependency.
        ///
        /// [`MockAudioSink`]: super::super::audio_sink::MockAudioSink
        pub fn with_sink(enabled: bool, sink: Box<dyn AudioSink>) -> std::io::Result<Self> {
            let (tx, rx) = mpsc::channel::<PlaybackCmd>();
            let enabled_flag = Arc::new(AtomicBool::new(enabled));
            let thread_enabled = Arc::clone(&enabled_flag);

            let thread = std::thread::Builder::new()
                .name("tts-playback".to_string())
                .spawn(move || {
                    run_loop_with(rx, thread_enabled, move |bytes| sink.play_bytes(bytes));
                })?;

            Ok(Self {
                tx,
                enabled: enabled_flag,
                thread: Some(thread),
            })
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
mod tests {
    use std::time::{Duration, Instant};

    use super::PlaybackService;
    use crate::pipeline::audio_sink::MockAudioSink;

    fn wait_for_call_count(handle: &MockAudioSink, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if handle.call_count() == expected {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn disabled_playback_service_drops_audio() {
        let mock = MockAudioSink::new();
        let handle = mock.clone();

        let svc =
            PlaybackService::with_sink(false, Box::new(mock)).expect("with_sink should not fail");

        svc.play(vec![1, 2, 3]);
        svc.play(vec![4, 5, 6]);

        // The service is disabled: play() returns before enqueueing.
        assert_eq!(
            handle.call_count(),
            0,
            "disabled service must not route audio"
        );
    }

    #[test]
    fn playback_service_routes_to_mock_sink() {
        let mock = MockAudioSink::new();
        let handle = mock.clone();

        let svc =
            PlaybackService::with_sink(true, Box::new(mock)).expect("with_sink should not fail");

        svc.play(vec![10, 20, 30]);
        svc.play(vec![40, 50, 60]);
        wait_for_call_count(&handle, 2);

        assert_eq!(handle.call_count(), 2);
        let chunks = handle.received_chunks();
        assert_eq!(chunks[0], vec![10, 20, 30]);
        assert_eq!(chunks[1], vec![40, 50, 60]);
    }

    #[test]
    fn set_enabled_gates_subsequent_play() {
        let mock = MockAudioSink::new();
        let handle = mock.clone();

        let svc =
            PlaybackService::with_sink(true, Box::new(mock)).expect("with_sink should not fail");
        svc.play(vec![1]);
        wait_for_call_count(&handle, 1);
        svc.set_enabled(false);
        svc.play(vec![2]);

        // Only the first clip should have been routed.
        assert_eq!(handle.call_count(), 1);
    }
}
