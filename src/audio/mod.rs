//! Audio capture stub.
//!
//! Phase 1 will fill this module with real WASAPI loopback capture using the
//! `wasapi-rs` crate.  For now the module only exposes the types and traits
//! that the rest of the codebase will depend on so they can be compiled and
//! reasoned about before the audio code exists.

// These items are stubs — they will be used in Phase 1 onwards.
#![allow(dead_code)]

use anyhow::Result;

/// A single chunk of captured audio, ready to be sent to the STT pipeline.
///
/// Audio is always 16 kHz, mono, 16-bit signed PCM — the format required
/// by Google Speech-to-Text.  The resampling step (powered by `rubato` in
/// Phase 1) converts whatever the sound card produces into this format.
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// Raw PCM samples, little-endian i16, 16 kHz mono.
    pub samples: Vec<i16>,
    /// Duration of this chunk in milliseconds (derived from sample count).
    pub duration_ms: u32,
}

/// Trait that any audio source must implement.
///
/// The only source in v1 is the Windows WASAPI loopback device.  The trait
/// exists so that unit tests and CI can inject a mock source without touching
/// real audio hardware.
pub trait AudioSource: Send {
    /// Block until the next chunk is available, then return it.
    fn next_chunk(&mut self) -> Result<AudioChunk>;

    /// A human-readable name for the audio device (shown in the status bar).
    fn device_name(&self) -> &str;
}

/// Stub implementation used in tests and Phase 0.
///
/// Always returns silence so the rest of the pipeline can be exercised
/// without real audio hardware.
pub struct SilentSource;

impl AudioSource for SilentSource {
    fn next_chunk(&mut self) -> Result<AudioChunk> {
        // 500 ms of silence at 16 kHz = 8 000 samples
        Ok(AudioChunk {
            samples: vec![0i16; 8_000],
            duration_ms: 500,
        })
    }

    fn device_name(&self) -> &str {
        "silent (stub)"
    }
}
