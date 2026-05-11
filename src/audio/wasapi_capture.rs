//! Windows WASAPI loopback capture implementation.
//!
//! This module is only compiled on Windows (`#[cfg(windows)]` in the parent).
//!
// Items are wired up in Phase 1; suppress dead-code lints in this skeleton.
#![allow(dead_code)]
//!
//! # Thread model
//!
//! WASAPI's event-driven capture API must run on a dedicated OS thread —
//! the Windows audio engine delivers notifications via `SetEvent`, and the
//! capture loop must call `WaitForSingleObject` (via [`Handle::wait_for_event`])
//! which blocks the current thread.  A regular `tokio::spawn` task would
//! starve the executor, so we use `std::thread::spawn` instead and communicate
//! back to async consumers via `tokio::sync::mpsc::Sender::blocking_send`.
//!
//! # Data flow
//!
//! ```text
//! WASAPI render endpoint (loopback)
//!   │  raw PCM bytes, native format (e.g. 48 kHz stereo f32)
//!   ▼
//! interleaved_to_mono_f32()
//!   │  mono f32, native rate
//!   ▼
//! rubato SincFixedIn resampler
//!   │  mono f32, 16 kHz
//!   ▼
//! f32 → i16 conversion
//!   │  i16 samples, 16 kHz mono
//!   ▼
//! SilenceDetector
//!   │  filtered AudioChunks
//!   ▼
//! tokio mpsc channel → downstream STT pipeline
//! ```

use std::collections::VecDeque;

use anyhow::{anyhow, Result};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use tokio::sync::mpsc;
use wasapi::{get_default_device, initialize_mta, Direction, ShareMode};

use super::{AudioChunk, SilenceDetector};

/// Target output sample rate (required by Google Speech-to-Text).
const TARGET_RATE: u32 = 16_000;

/// How many input frames to feed rubato per iteration (≈10 ms at 48 kHz).
const FRAMES_PER_CHUNK: usize = 480;

/// WASAPI event wait timeout in milliseconds.
const EVENT_TIMEOUT_MS: u32 = 1_000;

/// Spawn the WASAPI loopback capture thread.
///
/// Returns immediately; the thread runs until the channel receiver is dropped.
pub(super) fn spawn(tx: mpsc::Sender<AudioChunk>, silence_threshold: f32) -> Result<()> {
    std::thread::Builder::new()
        .name("wasapi-loopback".into())
        .spawn(move || {
            if let Err(e) = capture_loop(tx, silence_threshold) {
                tracing::error!("WASAPI capture thread failed: {e:#}");
            }
        })
        .map_err(|e| anyhow!("spawn wasapi-loopback thread: {e}"))?;
    Ok(())
}

/// The capture loop — runs for the lifetime of the application on its own
/// OS thread.
fn capture_loop(tx: mpsc::Sender<AudioChunk>, silence_threshold: f32) -> Result<()> {
    // COM must be initialized on every thread that uses WASAPI.
    initialize_mta().map_err(|e| anyhow!("initialize COM MTA: {e}"))?;

    // Get the default *render* (speakers) endpoint.  WASAPI loopback capture
    // reads from a render endpoint with Direction::Capture + loopback=true.
    let device = get_default_device(&Direction::Render)
        .map_err(|e| anyhow!("get default render device: {e}"))?;
    let device_name = device
        .get_friendlyname()
        .unwrap_or_else(|_| "unknown".into());
    tracing::info!(device = %device_name, "WASAPI loopback opened");

    let mut audio_client = device
        .get_iaudioclient()
        .map_err(|e| anyhow!("get IAudioClient: {e}"))?;

    // Query the mix format (shared mode native format).
    let wave_fmt = audio_client
        .get_mixformat()
        .map_err(|e| anyhow!("get mix format: {e}"))?;
    let channels = wave_fmt.get_nchannels() as usize;
    let native_rate = wave_fmt.get_samplespersec();
    let bits = wave_fmt.get_bitspersample();
    let blockalign = wave_fmt.get_blockalign() as usize;

    tracing::info!(channels, native_rate, bits, "WASAPI capture format");
    if bits != 16 && bits != 32 {
        tracing::warn!(
            bits,
            "unsupported PCM bit depth; zero-filling captured audio"
        );
    }

    // Initialise in shared loopback mode.
    let (_def_period, min_period) = audio_client
        .get_periods()
        .map_err(|e| anyhow!("get device periods: {e}"))?;
    audio_client
        .initialize_client(
            &wave_fmt,
            min_period,
            &Direction::Capture,
            &ShareMode::Shared,
            true, // loopback = true
        )
        .map_err(|e| anyhow!("initialize audio client: {e}"))?;

    // Set up event-driven notification.
    let h_event = audio_client
        .set_get_eventhandle()
        .map_err(|e| anyhow!("set event handle: {e}"))?;

    let capture_client = audio_client
        .get_audiocaptureclient()
        .map_err(|e| anyhow!("get AudioCaptureClient: {e}"))?;

    audio_client
        .start_stream()
        .map_err(|e| anyhow!("start audio stream: {e}"))?;

    // Build the rubato resampler: native_rate → TARGET_RATE, 1 channel (mono).
    let resample_ratio = TARGET_RATE as f64 / native_rate as f64;
    let sinc_params = SincInterpolationParameters {
        sinc_len: 64,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 64,
        window: WindowFunction::BlackmanHarris2,
    };
    let mut resampler = SincFixedIn::<f32>::new(
        resample_ratio,
        2.0,
        sinc_params,
        FRAMES_PER_CHUNK,
        1, // mono output
    )
    .map_err(|e| anyhow!("create rubato resampler: {e}"))?;

    let mut silence_detector = SilenceDetector::new(silence_threshold, 500);

    // Carry buffer: unprocessed mono-f32 samples that didn't fill a full chunk.
    let mut carry: Vec<f32> = Vec::with_capacity(FRAMES_PER_CHUNK * 2);

    // Deque that WASAPI writes raw bytes into.
    let mut sample_queue: VecDeque<u8> = VecDeque::with_capacity(blockalign * FRAMES_PER_CHUNK * 4);

    loop {
        if tx.is_closed() {
            tracing::info!("WASAPI capture: channel closed, exiting thread");
            return Ok(());
        }

        // Drain any pending WASAPI packets into the deque.
        capture_client
            .read_from_device_to_deque(blockalign, &mut sample_queue)
            .map_err(|e| anyhow!("read from WASAPI device: {e}"))?;

        // Convert interleaved bytes → mono f32 and append to carry buffer.
        let mono_frames = raw_bytes_to_mono_f32(&sample_queue, channels, bits);
        sample_queue.clear();
        carry.extend_from_slice(&mono_frames);

        // Process complete FRAMES_PER_CHUNK-sized windows through the resampler.
        while carry.len() >= FRAMES_PER_CHUNK {
            let input: Vec<f32> = carry.drain(..FRAMES_PER_CHUNK).collect();
            let resampled = resampler
                .process(&[input], None)
                .map_err(|e| anyhow!("rubato resample: {e}"))?;

            let samples_i16: Vec<i16> = resampled[0]
                .iter()
                .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .collect();

            let chunk = AudioChunk::new(samples_i16);
            if silence_detector.process(&chunk) && tx.blocking_send(chunk).is_err() {
                // Receiver was dropped — the application is shutting down.
                tracing::info!("WASAPI capture: channel closed, exiting thread");
                return Ok(());
            }
        }

        // Wait for the next buffer event (or timeout after 1 s).
        if h_event.wait_for_event(EVENT_TIMEOUT_MS).is_err() {
            tracing::warn!("WASAPI event wait timeout — stream stalled?");
        }
    }
}

/// Convert a raw byte deque of interleaved PCM into a mono `Vec<f32>`.
///
/// Mixes down all channels by averaging.  Supports 16-bit integer and
/// 32-bit float sample formats; other bit depths are silently skipped.
fn raw_bytes_to_mono_f32(data: &VecDeque<u8>, channels: usize, bits: u16) -> Vec<f32> {
    if channels == 0 || data.is_empty() {
        return vec![];
    }
    let bytes_per_sample = (bits / 8) as usize;
    let frame_bytes = bytes_per_sample * channels;
    if frame_bytes == 0 || data.len() < frame_bytes {
        return vec![];
    }

    let data_slice: Vec<u8> = data.iter().copied().collect();
    let num_frames = data_slice.len() / frame_bytes;
    if bits != 16 && bits != 32 {
        return vec![0.0; num_frames];
    }

    let mut out = Vec::with_capacity(num_frames);

    for frame in data_slice.chunks_exact(frame_bytes) {
        let mono: f32 = if bits == 32 {
            let sum: f32 = frame
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .sum();
            sum / channels as f32
        } else {
            let sum: f32 = frame
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes(b.try_into().unwrap()) as f32 / i16::MAX as f32)
                .sum();
            sum / channels as f32
        };
        out.push(mono);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_pcm_depth_zero_fills_once_per_chunk() {
        let data = VecDeque::from(vec![0_u8; 12]);
        let mono = raw_bytes_to_mono_f32(&data, 2, 24);

        assert_eq!(mono, vec![0.0, 0.0]);
    }
}
