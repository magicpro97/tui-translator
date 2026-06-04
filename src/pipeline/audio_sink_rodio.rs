//! Rodio-based speaker output sink.
//!
//! Extracted from the [`super`] module (`audio_sink`) in WP-24 / US-08 (#733) so the parent
//! module stays under the 600 LOC engineering-standards gate after the F32
//! virtual-mic writer was added.
//!
//! `RodioSink` does not implement `AudioSink` because `rodio::OutputStream`
//! is `!Send` (CPAL marks `Stream` as non-Send on Windows due to WASAPI /
//! COM threading) and `AudioSink: Send + 'static`. Instead,
//! [`crate::pipeline::playback::PlaybackService::new`] constructs a `RodioSink` inside
//! the playback thread (pinning the `OutputStream` to one OS thread).

#![cfg(any(windows, target_os = "macos"))]

/// Plays MP3 audio via `rodio` (Windows WASAPI; macOS CoreAudio; Linux no-op).
///
/// **Must be constructed and used on the same OS thread** — `rodio::OutputStream`
/// is `!Send` and therefore `RodioSink` does not implement
/// [`super::AudioSink`]. [`crate::pipeline::playback::PlaybackService::new`]
/// handles this by constructing `RodioSink` inside the dedicated playback thread.
pub struct RodioSink {
    _stream: rodio::OutputStream,
    stream_handle: rodio::OutputStreamHandle,
}

/// Result of an interruptible `RodioSink` playback attempt.
pub(crate) enum RodioPlaybackOutcome {
    /// The clip reached its natural end.
    Completed,
    /// Playback was stopped early by the caller.
    Interrupted,
}

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
                // CTRL-01: apply real-time output volume.
                sink.set_volume(crate::audio::audio_gain::output_volume_linear());
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

/// Open a rodio output stream for the named device (or default when `None`).
///
/// `pub(super)` so the Windows render PCM writer in `audio_sink.rs` can reuse it.
pub(super) fn open_output_stream(
    output_device: Option<&str>,
) -> Result<(rodio::OutputStream, rodio::OutputStreamHandle), String> {
    match output_device {
        Some(device_name) => {
            let device = find_output_device_by_name(device_name)?;
            rodio::OutputStream::try_from_device(&device).map_err(|e| {
                format!("failed to open configured TTS output device '{device_name}': {e}")
            })
        }
        None => rodio::OutputStream::try_default()
            .map_err(|e| format!("failed to open default audio output device: {e}")),
    }
}

fn find_output_device_by_name(device_name: &str) -> Result<rodio::cpal::Device, String> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};

    let host = rodio::cpal::default_host();
    let devices = host
        .output_devices()
        .map_err(|e| format!("failed to enumerate audio output devices: {e}"))?;

    for device in devices {
        match device.name() {
            Ok(name) if name == device_name => return Ok(device),
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("tts-playback: failed to read output-device name: {e}");
            }
        }
    }

    Err(format!(
        "configured output render endpoint '{device_name}' was not found"
    ))
}
