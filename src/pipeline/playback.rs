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

use crate::config::TtsRouting;

/// A concrete destination for synthesized TTS audio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackSinkTarget {
    /// Speaker output, optionally pinned to a configured playback device.
    Speakers { output_device: Option<String> },
    /// Virtual microphone render endpoint, addressed by exact device name.
    VirtualMic { device: String },
}

impl PlaybackSinkTarget {
    fn output_device(&self) -> Option<&str> {
        match self {
            Self::Speakers { output_device } => output_device.as_deref(),
            Self::VirtualMic { device } => Some(device.as_str()),
        }
    }

    fn description(&self) -> String {
        match self {
            Self::Speakers {
                output_device: Some(device),
            } => format!("speakers device '{device}'"),
            Self::Speakers {
                output_device: None,
            } => "speakers (system default)".to_string(),
            Self::VirtualMic { device } => format!("virtual mic device '{device}'"),
        }
    }
}

/// Resolved TTS playback route built from user configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackRoutePlan {
    label: &'static str,
    targets: Vec<PlaybackSinkTarget>,
}

impl PlaybackRoutePlan {
    /// Resolve a route from config fields.
    pub fn from_config(
        routing: TtsRouting,
        tts_output_device: Option<&str>,
        virtual_mic_device: Option<&str>,
    ) -> std::io::Result<Self> {
        match routing {
            TtsRouting::Speakers => Ok(Self::speakers(tts_output_device)),
            TtsRouting::VirtualMic => {
                let device = virtual_mic_device.ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "tts_routing \"virtual_mic\" requires virtual_mic_device",
                    )
                })?;
                Ok(Self::virtual_mic(device))
            }
            TtsRouting::Both => {
                let device = virtual_mic_device.ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "tts_routing \"both\" requires virtual_mic_device",
                    )
                })?;
                Ok(Self::both(tts_output_device, device))
            }
        }
    }

    /// Route to speakers only, preserving pre-VMIC behaviour.
    pub fn speakers(output_device: Option<&str>) -> Self {
        Self {
            label: "speakers",
            targets: vec![PlaybackSinkTarget::Speakers {
                output_device: output_device.map(str::to_owned),
            }],
        }
    }

    /// Route exclusively to a virtual microphone render endpoint.
    pub fn virtual_mic(device: &str) -> Self {
        Self {
            label: "virtual_mic",
            targets: vec![PlaybackSinkTarget::VirtualMic {
                device: device.to_string(),
            }],
        }
    }

    /// Route to both speakers and a virtual microphone render endpoint.
    pub fn both(output_device: Option<&str>, virtual_mic_device: &str) -> Self {
        Self {
            label: "both",
            targets: vec![
                PlaybackSinkTarget::Speakers {
                    output_device: output_device.map(str::to_owned),
                },
                PlaybackSinkTarget::VirtualMic {
                    device: virtual_mic_device.to_string(),
                },
            ],
        }
    }

    /// Human-readable route label used by startup/status logs.
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// Sink targets for this route.
    pub fn targets(&self) -> &[PlaybackSinkTarget] {
        &self.targets
    }
}

fn build_sinks_for_targets<T, F>(
    targets: &[PlaybackSinkTarget],
    mut make_sink: F,
) -> std::io::Result<Vec<T>>
where
    F: FnMut(&PlaybackSinkTarget) -> Result<T, String>,
{
    if targets.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "TTS playback route must contain at least one sink",
        ));
    }

    let mut sinks = Vec::with_capacity(targets.len());
    for target in targets {
        match make_sink(target) {
            Ok(sink) => sinks.push(sink),
            Err(err) => {
                return Err(std::io::Error::other(format!(
                    "failed to initialise TTS sink for {}: {err}",
                    target.description()
                )));
            }
        }
    }
    Ok(sinks)
}

fn play_to_audio_sinks(sinks: &[Box<dyn super::audio_sink::AudioSink>], audio_bytes: Vec<u8>) {
    if sinks.is_empty() {
        return;
    }

    for sink in sinks.iter().take(sinks.len() - 1) {
        sink.play_bytes(audio_bytes.clone());
    }
    if let Some(sink) = sinks.last() {
        sink.play_bytes(audio_bytes);
    }
}

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
mod tests {
    use std::time::{Duration, Instant};

    use super::{build_sinks_for_targets, PlaybackRoutePlan, PlaybackService, PlaybackSinkTarget};
    use crate::config::TtsRouting;
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
    fn playback_route_speakers_one_sink() {
        let plan = PlaybackRoutePlan::from_config(
            TtsRouting::Speakers,
            Some("Speakers (Realtek Audio)"),
            Some("CABLE Input (VB-Audio Virtual Cable)"),
        )
        .expect("speakers route should resolve");

        assert_eq!(plan.label(), "speakers");
        assert_eq!(
            plan.targets(),
            &[PlaybackSinkTarget::Speakers {
                output_device: Some("Speakers (Realtek Audio)".to_string())
            }]
        );
    }

    #[test]
    fn playback_route_virtual_mic_one_sink() {
        let plan = PlaybackRoutePlan::from_config(
            TtsRouting::VirtualMic,
            Some("Speakers (Realtek Audio)"),
            Some("CABLE Input (VB-Audio Virtual Cable)"),
        )
        .expect("virtual-mic route should resolve");

        assert_eq!(plan.label(), "virtual_mic");
        assert_eq!(
            plan.targets(),
            &[PlaybackSinkTarget::VirtualMic {
                device: "CABLE Input (VB-Audio Virtual Cable)".to_string()
            }]
        );
    }

    #[test]
    fn playback_route_both_fans_out() {
        let speakers = MockAudioSink::new();
        let virtual_mic = MockAudioSink::new();
        let speakers_handle = speakers.clone();
        let virtual_mic_handle = virtual_mic.clone();
        let payload = vec![7, 8, 9, 10];

        let svc =
            PlaybackService::with_sinks(true, vec![Box::new(speakers), Box::new(virtual_mic)])
                .expect("with_sinks should start");

        svc.play(payload.clone());
        wait_for_call_count(&speakers_handle, 1);
        wait_for_call_count(&virtual_mic_handle, 1);

        assert_eq!(speakers_handle.received_chunks(), vec![payload.clone()]);
        assert_eq!(virtual_mic_handle.received_chunks(), vec![payload]);
    }

    #[test]
    fn virtual_sink_failure_is_startup_error() {
        let plan = PlaybackRoutePlan::from_config(
            TtsRouting::VirtualMic,
            None,
            Some("CABLE Input (VB-Audio Virtual Cable)"),
        )
        .expect("virtual-mic route should resolve");

        let err = build_sinks_for_targets::<(), _>(plan.targets(), |target| match target {
            PlaybackSinkTarget::VirtualMic { device } => {
                Err(format!("cannot open endpoint '{device}'"))
            }
            PlaybackSinkTarget::Speakers { .. } => Ok(()),
        })
        .expect_err("virtual sink factory failure must propagate");
        let msg = err.to_string();

        assert!(
            msg.contains("virtual mic device 'CABLE Input (VB-Audio Virtual Cable)'"),
            "error should identify failed virtual mic device; got: {msg}"
        );
        assert!(
            msg.contains("cannot open endpoint"),
            "error should include root cause; got: {msg}"
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
