//! TTS playback service.
//!
//! [`PlaybackService`] owns a dedicated OS thread that receives MP3 audio
//! bytes over a channel and plays them back through the system audio output
//! via `rodio`. The service can be toggled on or off at runtime via
//! [`PlaybackService::set_enabled`]; when disabled, queued or currently playing
//! audio is stopped promptly instead of waiting for the sentence to finish.
//!
//! # Output device
//! The service uses the configured output device when one is provided; otherwise
//! it falls back to the system default output device.
//!
//! # Shutdown
//! Drop the [`PlaybackService`] or call [`PlaybackService::shutdown`] to
//! signal the playback thread to stop and join it cleanly.

use rodio::cpal::traits::{DeviceTrait, HostTrait};
use std::collections::VecDeque;
use std::io::Cursor;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, TryRecvError},
    Arc,
};
use std::time::Duration;

// ── Command channel ───────────────────────────────────────────────────────────

enum PlaybackCmd {
    Play(Vec<u8>),
    Shutdown,
}

// ── Service ───────────────────────────────────────────────────────────────────

/// A background TTS playback service.
///
/// Receives raw MP3 audio bytes and plays them sequentially on the system
/// default audio output device.  Audio clips are played one at a time; if a
/// new clip arrives while another is playing, it queues behind the current one.
///
/// # Thread safety
/// The service is [`Send`] and [`Sync`]; clone the inner [`mpsc::Sender`]
/// via [`PlaybackService::play`] or hold a reference to call it from multiple
/// async tasks.
pub struct PlaybackService {
    tx: mpsc::Sender<PlaybackCmd>,
    enabled: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PlaybackService {
    /// Start the playback service.
    ///
    /// `enabled` controls whether submitted audio is actually played; when
    /// `false`, calls to [`play`](Self::play) are no-ops.
    ///
    /// When `output_device` is `Some(name)`, the playback thread attempts to
    /// open that device by name; otherwise it uses the system default output.
    pub fn new(enabled: bool, output_device: Option<&str>) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel::<PlaybackCmd>();
        let enabled_flag = Arc::new(AtomicBool::new(enabled));
        let thread_enabled = Arc::clone(&enabled_flag);
        let output_device = output_device.map(str::to_owned);

        let thread = std::thread::Builder::new()
            .name("tts-playback".to_string())
            .spawn(move || run_playback_loop(rx, thread_enabled, output_device))?;

        Ok(Self {
            tx,
            enabled: enabled_flag,
            thread: Some(thread),
        })
    }

    /// Submit `audio_bytes` (MP3) for playback.
    ///
    /// Returns immediately.  The audio is discarded when the service is
    /// disabled ([`set_enabled(false)`](Self::set_enabled)) or when the
    /// internal channel is closed.
    pub fn play(&self, audio_bytes: Vec<u8>) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        let _ = self.tx.send(PlaybackCmd::Play(audio_bytes));
    }

    /// Enable or disable playback at runtime.
    ///
    /// When `false`, subsequent [`play`](Self::play) calls are no-ops and the
    /// playback thread stops the current clip plus any queued clips promptly.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    /// Signal the playback thread to stop and wait for it to exit.
    ///
    /// After this call the service is inert; calling [`play`](Self::play)
    /// will silently drop the audio.
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

// ── Playback loop (runs in dedicated OS thread) ───────────────────────────────

fn run_playback_loop(
    rx: mpsc::Receiver<PlaybackCmd>,
    enabled: Arc<AtomicBool>,
    output_device: Option<String>,
) {
    let stream_result = open_output_stream(output_device.as_deref());
    let (_stream, stream_handle) = match stream_result {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!("tts-playback: {e}");
            drain_channel(rx);
            return;
        }
    };

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
                                tracing::error!("tts-playback: failed to create audio sink: {e}");
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

                            match rx.try_recv() {
                                Ok(PlaybackCmd::Play(bytes)) => {
                                    pending.push_back(PlaybackCmd::Play(bytes));
                                }
                                Ok(PlaybackCmd::Shutdown) => {
                                    sink.stop();
                                    tracing::info!("tts-playback: stopping");
                                    return;
                                }
                                Err(TryRecvError::Empty) => {}
                                Err(TryRecvError::Disconnected) => {
                                    sink.stop();
                                    tracing::info!("tts-playback: channel disconnected");
                                    return;
                                }
                            }

                            std::thread::sleep(Duration::from_millis(20));
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
