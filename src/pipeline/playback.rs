//! TTS playback service.
//!
//! On Windows this module owns a dedicated playback thread backed by `rodio`.
//! On other platforms it provides a no-op stub so Linux CI can build and test
//! the project without optional system audio libraries such as ALSA.

#[cfg(windows)]
mod imp {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    use std::collections::VecDeque;
    use std::io::Cursor;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError},
        Arc,
    };
    use std::time::Duration;

    enum PlaybackCmd {
        Play(Vec<u8>),
        Shutdown,
    }

    /// A background TTS playback service.
    ///
    /// Receives raw MP3 audio bytes and plays them sequentially on the system
    /// default audio output device. Audio clips are played one at a time; if a
    /// new clip arrives while another is playing, it queues behind the current
    /// one.
    pub struct PlaybackService {
        tx: mpsc::Sender<PlaybackCmd>,
        enabled: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl PlaybackService {
        /// Start the playback service.
        ///
        /// When `output_device` is `Some(name)`, the playback thread attempts to
        /// open that device by name; otherwise it uses the system default output.
        pub fn new(enabled: bool, output_device: Option<&str>) -> std::io::Result<Self> {
            let (tx, rx) = mpsc::channel::<PlaybackCmd>();
            let (startup_tx, startup_rx) = mpsc::channel::<Result<(), String>>();
            let enabled_flag = Arc::new(AtomicBool::new(enabled));
            let thread_enabled = Arc::clone(&enabled_flag);
            let output_device = output_device.map(str::to_owned);

            let thread = std::thread::Builder::new()
                .name("tts-playback".to_string())
                .spawn(move || run_playback_loop(rx, thread_enabled, output_device, startup_tx))?;

            let thread = thread;

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

        /// Submit `audio_bytes` (MP3) for playback.
        pub fn play(&self, audio_bytes: Vec<u8>) {
            if !self.enabled.load(Ordering::Relaxed) {
                return;
            }
            let _ = self.tx.send(PlaybackCmd::Play(audio_bytes));
        }

        /// Enable or disable playback at runtime.
        pub fn set_enabled(&self, enabled: bool) {
            self.enabled.store(enabled, Ordering::Relaxed);
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

    fn run_playback_loop(
        rx: mpsc::Receiver<PlaybackCmd>,
        enabled: Arc<AtomicBool>,
        output_device: Option<String>,
        startup_tx: mpsc::Sender<Result<(), String>>,
    ) {
        let stream_result = open_output_stream(output_device.as_deref());
        let (_stream, stream_handle) = match stream_result {
            Ok(pair) => pair,
            Err(e) => {
                let _ = startup_tx.send(Err(e.clone()));
                tracing::error!("tts-playback: {e}");
                return;
            }
        };

        if let Err(e) = rodio::Sink::try_new(&stream_handle) {
            let message = format!("failed to create audio sink: {e}");
            let _ = startup_tx.send(Err(message.clone()));
            tracing::error!("tts-playback: {message}");
            return;
        }

        let _ = startup_tx.send(Ok(()));

        let mut pending = VecDeque::new();

        loop {
            let command = match pending.pop_front() {
                Some(command) => Ok(command),
                None => rx.recv(),
            };

            match command {
                Ok(PlaybackCmd::Play(bytes)) => {
                    if !enabled.load(Ordering::Relaxed) {
                        continue;
                    }
                    let cursor = Cursor::new(bytes);
                    match rodio::Decoder::new(cursor) {
                        Ok(source) => {
                            let sink = match rodio::Sink::try_new(&stream_handle) {
                                Ok(s) => s,
                                Err(e) => {
                                    tracing::error!(
                                        "tts-playback: failed to create audio sink: {e}"
                                    );
                                    drain_channel(rx);
                                    return;
                                }
                            };
                            sink.append(source);
                            loop {
                                if !enabled.load(Ordering::Relaxed) {
                                    sink.stop();
                                    break;
                                }

                                if sink.empty() {
                                    break;
                                }

                                match rx.recv_timeout(Duration::from_millis(20)) {
                                    Ok(PlaybackCmd::Play(bytes)) => {
                                        pending.push_back(PlaybackCmd::Play(bytes));
                                    }
                                    Ok(PlaybackCmd::Shutdown) => {
                                        sink.stop();
                                        tracing::info!("tts-playback: stopping");
                                        return;
                                    }
                                    Err(RecvTimeoutError::Timeout) => {}
                                    Err(RecvTimeoutError::Disconnected) => {
                                        sink.stop();
                                        tracing::info!("tts-playback: channel disconnected");
                                        return;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("tts-playback: failed to decode audio: {e}");
                        }
                    }
                }
                Ok(PlaybackCmd::Shutdown) | Err(_) => {
                    tracing::info!("tts-playback: stopping");
                    break;
                }
            }
        }
    }

    fn open_output_stream(
        output_device: Option<&str>,
    ) -> Result<(rodio::OutputStream, rodio::OutputStreamHandle), String> {
        match output_device {
            Some(device_name) => {
                let host = rodio::cpal::default_host();
                let devices = host
                    .output_devices()
                    .map_err(|e| format!("failed to enumerate audio output devices: {e}"))?;
                let mut matching_device = None;

                for device in devices {
                    match device.name() {
                        Ok(name) if name == device_name => {
                            matching_device = Some(device);
                            break;
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!("tts-playback: failed to read output-device name: {e}");
                        }
                    }
                }

                let device = matching_device.ok_or_else(|| {
                    format!("configured TTS output device '{device_name}' was not found")
                })?;

                rodio::OutputStream::try_from_device(&device).map_err(|e| {
                    format!("failed to open configured TTS output device '{device_name}': {e}")
                })
            }
            None => rodio::OutputStream::try_default()
                .map_err(|e| format!("failed to open default audio output device: {e}")),
        }
    }

    fn drain_channel(rx: mpsc::Receiver<PlaybackCmd>) {
        loop {
            match rx.recv() {
                Ok(PlaybackCmd::Shutdown) | Err(_) => break,
                Ok(PlaybackCmd::Play(_)) => {}
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    /// No-op playback service for non-Windows builds.
    ///
    /// The product is Windows-native; Linux CI only needs this stub so the
    /// provider and contract layers can compile without system audio packages.
    pub struct PlaybackService {
        enabled: Arc<AtomicBool>,
    }

    impl PlaybackService {
        pub fn new(enabled: bool, _output_device: Option<&str>) -> std::io::Result<Self> {
            Ok(Self {
                enabled: Arc::new(AtomicBool::new(enabled)),
            })
        }

        pub fn play(&self, _audio_bytes: Vec<u8>) {
            let _ = self.enabled.load(Ordering::Relaxed);
        }

        pub fn set_enabled(&self, enabled: bool) {
            self.enabled.store(enabled, Ordering::Relaxed);
        }

        pub fn shutdown(&mut self) {}
    }
}

#[allow(unused_imports)]
pub use imp::PlaybackService;
