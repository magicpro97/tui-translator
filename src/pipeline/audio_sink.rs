//! `AudioSink` abstraction for TTS audio output.
//!
//! Decouples [`super::playback::PlaybackService`] from the concrete rodio
//! implementation so alternative sinks (virtual mic, file recorder, test
//! double) can be plugged in without changing the pipeline.
//!
//! # Implementations
//!
//! | Type | Platform | Purpose |
//! |------|----------|---------|
//! | `RodioSink` | Windows only | Default speaker output via rodio |
//! | [`MockAudioSink`] | All | Test double; records submitted chunks |
//!
//! ## Why `RodioSink` does not implement `AudioSink`
//!
//! `rodio::OutputStream` is `!Send` (CPAL marks `Stream` as non-Send on
//! Windows due to WASAPI / COM threading constraints).  Because
//! `AudioSink: Send + 'static` is required to move sinks into background
//! threads, `RodioSink` cannot formally implement the trait.
//!
//! Instead, [`PlaybackService::new`] constructs a `RodioSink` *inside* the
//! playback thread (so the OutputStream is always pinned to one OS thread),
//! and calls `RodioSink::play_bytes` directly.
//!
//! [`PlaybackService::new`]: super::playback::PlaybackService::new

use std::sync::{Arc, Mutex};

// ── Trait ────────────────────────────────────────────────────────────────────

/// Receives raw MP3 audio bytes and plays or records them.
///
/// Implementors are called from the `PlaybackService` background thread.
/// Implementations **may block** until playback or recording is complete.
pub trait AudioSink: Send + 'static {
    /// Play or record the given MP3 `audio_bytes`.
    fn play_bytes(&self, audio_bytes: Vec<u8>);
}

// ── Windows: RodioSink ───────────────────────────────────────────────────────

/// Plays MP3 audio through the system speaker via `rodio`.
///
/// **Must be constructed and used on the same OS thread** — `rodio::OutputStream`
/// is `!Send` and therefore `RodioSink` does not implement [`AudioSink`].
/// [`PlaybackService::new`] handles this by constructing `RodioSink` inside
/// the dedicated playback thread.
///
/// [`PlaybackService::new`]: super::playback::PlaybackService::new
#[cfg(windows)]
pub struct RodioSink {
    _stream: rodio::OutputStream,
    stream_handle: rodio::OutputStreamHandle,
}

/// Result of an interruptible `RodioSink` playback attempt.
#[cfg(windows)]
pub(crate) enum RodioPlaybackOutcome {
    /// The clip reached its natural end.
    Completed,
    /// Playback was stopped early by the caller.
    Interrupted,
}

#[cfg(windows)]
impl RodioSink {
    /// Open an output stream for `output_device` (or the system default when
    /// `None`) and verify that a rodio `Sink` can be created on it.
    ///
    /// Returns `Err(String)` on failure so the caller can propagate the
    /// message back through a startup channel.
    pub fn try_new(output_device: Option<&str>) -> Result<Self, String> {
        let (_stream, stream_handle) = open_output_stream(output_device)?;

        // Validate that a sink can be created on this device before reporting
        // a successful startup to the caller.
        rodio::Sink::try_new(&stream_handle)
            .map_err(|e| format!("failed to create audio sink: {e}"))?;

        Ok(Self {
            _stream,
            stream_handle,
        })
    }

    /// Decode and play `audio_bytes` (MP3), blocking until playback completes.
    pub fn play_bytes(&self, audio_bytes: Vec<u8>) {
        if let Some(sink) = self.start_sink(audio_bytes) {
            sink.sleep_until_end();
        }
    }

    /// Decode and play `audio_bytes` while allowing the caller to interrupt playback.
    ///
    /// `should_interrupt` is called until the clip ends. It may block briefly
    /// while polling for control messages.
    pub(crate) fn play_bytes_until_interrupted<F>(
        &self,
        audio_bytes: Vec<u8>,
        mut should_interrupt: F,
    ) -> RodioPlaybackOutcome
    where
        F: FnMut() -> bool,
    {
        let Some(sink) = self.start_sink(audio_bytes) else {
            return RodioPlaybackOutcome::Completed;
        };

        loop {
            if sink.empty() {
                return RodioPlaybackOutcome::Completed;
            }
            if should_interrupt() {
                sink.stop();
                return RodioPlaybackOutcome::Interrupted;
            }
        }
    }

    /// Decode `audio_bytes` and start a rodio sink without waiting for it to finish.
    pub(crate) fn start_sink(&self, audio_bytes: Vec<u8>) -> Option<rodio::Sink> {
        use std::io::Cursor;

        let cursor = Cursor::new(audio_bytes);
        let source = match rodio::Decoder::new(cursor) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("RodioSink: failed to decode audio: {e}");
                return None;
            }
        };
        match rodio::Sink::try_new(&self.stream_handle) {
            Ok(sink) => {
                sink.append(source);
                Some(sink)
            }
            Err(e) => {
                tracing::error!("RodioSink: failed to create rodio sink: {e}");
                None
            }
        }
    }
}

#[cfg(windows)]
fn open_output_stream(
    output_device: Option<&str>,
) -> Result<(rodio::OutputStream, rodio::OutputStreamHandle), String> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};

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

// ── Cross-platform: MockAudioSink ────────────────────────────────────────────

/// Test double that records every audio chunk submitted via [`AudioSink::play_bytes`].
///
/// Cheaply [`Clone`]able: all clones share the same recording buffer so you
/// can pass ownership into a [`super::playback::PlaybackService`] while keeping
/// a handle for assertions.
///
/// # Example
///
/// ```rust,ignore
/// use crate::pipeline::audio_sink::{AudioSink, MockAudioSink};
///
/// let sink = MockAudioSink::new();
/// sink.play_bytes(vec![1, 2, 3]);
/// assert_eq!(sink.call_count(), 1);
/// assert_eq!(sink.received_chunks(), vec![vec![1, 2, 3]]);
/// ```
#[derive(Clone, Default)]
pub struct MockAudioSink {
    received: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl MockAudioSink {
    /// Create an empty `MockAudioSink`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a snapshot of all audio chunks that have been submitted so far.
    pub fn received_chunks(&self) -> Vec<Vec<u8>> {
        self.received
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Number of [`play_bytes`](AudioSink::play_bytes) calls received so far.
    pub fn call_count(&self) -> usize {
        self.received
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }
}

impl AudioSink for MockAudioSink {
    fn play_bytes(&self, audio_bytes: Vec<u8>) {
        self.received
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(audio_bytes);
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_sink_records_play_calls() {
        let sink = MockAudioSink::new();
        sink.play_bytes(vec![1, 2, 3]);
        sink.play_bytes(vec![4, 5, 6]);

        assert_eq!(sink.call_count(), 2);
        assert_eq!(sink.received_chunks(), vec![vec![1, 2, 3], vec![4, 5, 6]]);
    }

    #[test]
    fn mock_sink_clone_shares_buffer() {
        let original = MockAudioSink::new();
        let clone = original.clone();

        original.play_bytes(vec![10, 20]);
        clone.play_bytes(vec![30, 40]);

        assert_eq!(original.call_count(), 2);
        assert_eq!(clone.call_count(), 2);
    }

    #[test]
    fn mock_sink_empty_by_default() {
        let sink = MockAudioSink::new();
        assert_eq!(sink.call_count(), 0);
        assert!(sink.received_chunks().is_empty());
    }
}
