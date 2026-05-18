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
use std::sync::mpsc::{sync_channel, RecvTimeoutError, SyncSender};
use std::time::Duration;

use anyhow::{anyhow, Result};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use tokio::sync::mpsc;
use wasapi::{
    get_default_device, initialize_mta, AudioCaptureClient, Device, DeviceCollection, DeviceState,
    Direction, ShareMode,
};

use super::{AudioChunk, CaptureDeviceInfo, CaptureInfo, SilenceDetector, DEFAULT_SILENCE_GATE_MS};

/// Target output sample rate (required by Google Speech-to-Text).
const TARGET_RATE: u32 = 16_000;

/// How many input frames to feed rubato per iteration (≈10 ms at 48 kHz).
const FRAMES_PER_CHUNK: usize = 480;

/// WASAPI event wait timeout in milliseconds.
const EVENT_TIMEOUT_MS: u32 = 1_000;

/// Spawn the WASAPI loopback capture thread.
///
/// Returns immediately; the thread runs until the channel receiver is dropped.
pub(super) fn spawn(
    tx: mpsc::Sender<AudioChunk>,
    capture_device: Option<String>,
    silence_threshold: f32,
) -> Result<CaptureInfo> {
    let (init_tx, init_rx) = sync_channel(1);
    let error_tx = init_tx.clone();

    std::thread::Builder::new()
        .name("wasapi-loopback".into())
        .spawn(move || {
            if let Err(e) = capture_loop(tx, capture_device, silence_threshold, init_tx) {
                let _ = error_tx.send(Err(format!("{e:#}")));
                tracing::error!("WASAPI capture thread failed: {e:#}");
            }
        })
        .map_err(|e| anyhow!("spawn wasapi-loopback thread: {e}"))?;

    match init_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(info)) => Ok(info),
        Ok(Err(message)) => Err(anyhow!("WASAPI capture initialization failed: {message}")),
        Err(RecvTimeoutError::Timeout) => Err(anyhow!(
            "timed out waiting for WASAPI capture initialization"
        )),
        Err(RecvTimeoutError::Disconnected) => Err(anyhow!(
            "WASAPI capture thread exited before reporting device information"
        )),
    }
}

/// The capture loop — runs for the lifetime of the application on its own
/// OS thread.
fn capture_loop(
    tx: mpsc::Sender<AudioChunk>,
    capture_device: Option<String>,
    silence_threshold: f32,
    init_tx: SyncSender<std::result::Result<CaptureInfo, String>>,
) -> Result<()> {
    // COM must be initialized on every thread that uses WASAPI.
    initialize_mta().map_err(|e| anyhow!("initialize COM MTA: {e}"))?;

    // WASAPI loopback reads from a render endpoint with Direction::Capture +
    // loopback=true. Blank selection means Windows default playback endpoint.
    let (device, device_name) = select_render_device(capture_device.as_deref())?;
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

    let _ = init_tx.send(Ok(CaptureInfo {
        device_name: device_name.clone(),
        native_sample_rate: native_rate,
    }));

    let mut silence_detector = SilenceDetector::new(silence_threshold, DEFAULT_SILENCE_GATE_MS);

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
        read_available_packets(&capture_client, blockalign, &mut sample_queue)?;

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

pub(super) fn list_loopback_devices() -> Result<Vec<CaptureDeviceInfo>> {
    initialize_mta().map_err(|e| anyhow!("initialize COM MTA: {e}"))?;
    let (default_id, default_name) = default_render_identity();
    active_render_devices(default_id.as_deref(), default_name.as_deref())
}

fn active_render_devices(
    default_id: Option<&str>,
    default_name: Option<&str>,
) -> Result<Vec<CaptureDeviceInfo>> {
    let collection = DeviceCollection::new(&Direction::Render)
        .map_err(|e| anyhow!("enumerate active render devices: {e}"))?;
    let count = collection
        .get_nbr_devices()
        .map_err(|e| anyhow!("count active render devices: {e}"))?;
    let mut devices = Vec::with_capacity(count as usize);

    for index in 0..count {
        let device = match collection.get_device_at_index(index) {
            Ok(device) => device,
            Err(err) => {
                tracing::warn!(index, error = %err, "skipping unreadable render device");
                continue;
            }
        };
        match capture_device_info(&device, index, default_id, default_name) {
            Ok(info) => devices.push(info),
            Err(err) => {
                tracing::warn!(index, error = %err, "skipping unusable render device");
            }
        }
    }

    Ok(devices)
}

fn capture_device_info(
    device: &Device,
    index: u32,
    default_id: Option<&str>,
    default_name: Option<&str>,
) -> Result<CaptureDeviceInfo> {
    let state = device
        .get_state()
        .map_err(|e| anyhow!("read render device {index} state: {e}"))?;
    if state != DeviceState::Active {
        return Err(anyhow!(
            "render device {index} is not active (state: {state:?})"
        ));
    }

    let id = device
        .get_id()
        .map_err(|e| anyhow!("read render device {index} stable id: {e}"))?;
    let name = device
        .get_friendlyname()
        .map_err(|e| anyhow!("read render device {index} name: {e}"))?;
    device
        .get_iaudioclient()
        .and_then(|client| client.get_mixformat().map(|_| ()))
        .map_err(|e| anyhow!("query render device {index} mix format: {e}"))?;

    let is_default = default_id == Some(id.as_str())
        || (default_id.is_none() && default_name == Some(name.as_str()));

    Ok(CaptureDeviceInfo {
        id,
        name,
        is_default,
    })
}

fn default_render_identity() -> (Option<String>, Option<String>) {
    let default_device = get_default_device(&Direction::Render).ok();
    let default_id = default_device
        .as_ref()
        .and_then(|device| device.get_id().ok());
    let default_name = default_device
        .as_ref()
        .and_then(|device| device.get_friendlyname().ok());
    (default_id, default_name)
}

/// Normalise the operator-supplied `capture_device` config value.
///
/// Returns `Some(trimmed_name)` when a non-blank device name is provided,
/// or `None` when the caller should fall back to the Windows default
/// render endpoint (i.e. the value was absent, `""`, or all whitespace).
///
/// This function is pure (no I/O) so it can be unit-tested without WASAPI.
fn resolve_capture_device_name(requested: Option<&str>) -> Option<&str> {
    requested.map(str::trim).filter(|s| !s.is_empty())
}

fn select_render_device(requested: Option<&str>) -> Result<(Device, String)> {
    match resolve_capture_device_name(requested) {
        Some(name) => find_render_device_by_name(name),
        None => {
            let device = get_default_device(&Direction::Render)
                .map_err(|e| no_default_render_device_error(&e.to_string()))?;
            let device_name = device
                .get_friendlyname()
                .unwrap_or_else(|_| "unknown".into());
            Ok((device, device_name))
        }
    }
}

/// Build an operator-actionable error for the case where Windows reports no
/// default audio render device.
///
/// The message includes the raw WASAPI diagnostic string, instructions to
/// check Windows Sound Settings, and a hint to use `--list-capture-devices`
/// so the operator can select an explicit device via `capture_device` in
/// `config.json`.
fn no_default_render_device_error(wasapi_error: &str) -> anyhow::Error {
    anyhow!(
        "no default audio render device: {wasapi_error}. \
         Ensure a playback device is active in Windows Sound Settings \
         (right-click the speaker icon → Sound settings → Output), \
         or run `tui-translator --list-capture-devices` and set \
         `capture_device` in config.json to an explicit device name."
    )
}

fn find_render_device_by_name(name: &str) -> Result<(Device, String)> {
    let collection = DeviceCollection::new(&Direction::Render)
        .map_err(|e| anyhow!("enumerate active render devices: {e}"))?;
    let count = collection
        .get_nbr_devices()
        .map_err(|e| anyhow!("count active render devices: {e}"))?;
    let (default_id, default_name) = default_render_identity();
    let mut names = Vec::with_capacity(count as usize);

    for index in 0..count {
        let device = match collection.get_device_at_index(index) {
            Ok(device) => device,
            Err(err) => {
                tracing::warn!(index, error = %err, "skipping unreadable render device");
                continue;
            }
        };
        let info = match capture_device_info(
            &device,
            index,
            default_id.as_deref(),
            default_name.as_deref(),
        ) {
            Ok(info) => info,
            Err(err) => {
                tracing::warn!(index, error = %err, "skipping unusable render device");
                continue;
            }
        };
        if info.name == name {
            return Ok((device, info.name));
        }
        names.push(info.name);
    }

    Err(anyhow!(
        "render device {name:?} was not found. Open Settings and press F2 on \
         Capture device to choose one of: {}",
        format_device_names(&names)
    ))
}

fn read_available_packets(
    capture_client: &AudioCaptureClient,
    bytes_per_frame: usize,
    data: &mut VecDeque<u8>,
) -> Result<()> {
    if bytes_per_frame == 0 {
        return Err(anyhow!("WASAPI device reported zero bytes per frame"));
    }

    loop {
        let next_frames = capture_client
            .get_next_nbr_frames()
            .map_err(|e| anyhow!("query WASAPI packet size: {e}"))?
            .unwrap_or(0);
        if next_frames == 0 {
            return Ok(());
        }

        let packet_len = (next_frames as usize)
            .checked_mul(bytes_per_frame)
            .ok_or_else(|| anyhow!("WASAPI packet size overflow"))?;
        let mut packet = vec![0u8; packet_len];
        let (read_frames, _flags) = capture_client
            .read_from_device(bytes_per_frame, &mut packet)
            .map_err(|e| anyhow!("read from WASAPI device: {e}"))?;
        if read_frames == 0 {
            return Ok(());
        }
        let bytes_read = (read_frames as usize)
            .checked_mul(bytes_per_frame)
            .ok_or_else(|| anyhow!("WASAPI read size overflow"))?
            .min(packet.len());
        data.extend(packet.into_iter().take(bytes_read));
    }
}

fn format_device_names(names: &[String]) -> String {
    if names.is_empty() {
        "no active playback devices reported by Windows".to_string()
    } else {
        names.join(", ")
    }
}

/// Convert a raw byte deque of interleaved PCM into a mono `Vec<f32>`.
///
/// Mixes down all channels by averaging. Supports 16-bit integer and
/// 32-bit float sample formats; other bit depths are zero-filled.
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
                .map(|b| match b {
                    [a, b, c, d] => f32::from_le_bytes([*a, *b, *c, *d]),
                    _ => 0.0,
                })
                .sum();
            sum / channels as f32
        } else {
            let sum: f32 = frame
                .chunks_exact(2)
                .map(|b| match b {
                    [a, b] => i16::from_le_bytes([*a, *b]) as f32 / i16::MAX as f32,
                    _ => 0.0,
                })
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

    // ── resolve_capture_device_name — selection / fallback logic ─────────────

    /// `None` input (no `capture_device` key in config) must map to `None` so
    /// the caller opens the Windows default render endpoint.
    #[test]
    fn resolve_device_name_none_uses_default() {
        assert_eq!(resolve_capture_device_name(None), None);
    }

    /// A blank string (user cleared the field) must also map to `None` so the
    /// blank-means-default contract is upheld.
    #[test]
    fn resolve_device_name_empty_string_uses_default() {
        assert_eq!(resolve_capture_device_name(Some("")), None);
    }

    /// A whitespace-only string (e.g. accidental space in config.json) must
    /// also be treated as absent and fall back to the default device.
    #[test]
    fn resolve_device_name_whitespace_uses_default() {
        assert_eq!(resolve_capture_device_name(Some("   ")), None);
    }

    /// A name surrounded by whitespace must be trimmed and returned so the
    /// downstream lookup matches the exact Windows device name.
    #[test]
    fn resolve_device_name_trims_surrounding_whitespace() {
        assert_eq!(
            resolve_capture_device_name(Some("  Speakers (HDA Audio)  ")),
            Some("Speakers (HDA Audio)"),
        );
    }

    /// A name with no surrounding whitespace is returned verbatim.
    #[test]
    fn resolve_device_name_exact_name_is_returned_unchanged() {
        assert_eq!(
            resolve_capture_device_name(Some("Headphones (USB Audio)")),
            Some("Headphones (USB Audio)"),
        );
    }

    // ── format_device_names — label formatting ────────────────────────────────

    /// A single device name is returned verbatim (no trailing comma or separator).
    #[test]
    fn format_device_names_single_entry() {
        assert_eq!(
            format_device_names(&["Speakers (Realtek High Definition Audio)".into()]),
            "Speakers (Realtek High Definition Audio)",
        );
    }

    /// Multiple device names are joined by `", "`.
    #[test]
    fn format_device_names_multiple_entries_are_comma_separated() {
        let names = vec![
            "Speakers (Realtek HD Audio)".to_string(),
            "Headphones (USB Audio Device)".to_string(),
            "CABLE Input (VB-Audio Virtual Cable)".to_string(),
        ];
        assert_eq!(
            format_device_names(&names),
            "Speakers (Realtek HD Audio), Headphones (USB Audio Device), CABLE Input (VB-Audio Virtual Cable)",
        );
    }

    #[test]
    fn unsupported_pcm_depth_zero_fills_once_per_chunk() {
        let data = VecDeque::from(vec![0_u8; 12]);
        let mono = raw_bytes_to_mono_f32(&data, 2, 24);

        assert_eq!(mono, vec![0.0, 0.0]);
    }

    #[test]
    fn format_device_names_reports_empty_device_list() {
        assert_eq!(
            format_device_names(&[]),
            "no active playback devices reported by Windows"
        );
    }

    // ── Issue #196 regression tests ───────────────────────────────────────────

    /// `no_default_render_device_error` must produce an operator-actionable
    /// message that includes both a Windows UI hint and the CLI escape hatch.
    #[test]
    fn no_default_render_device_error_is_operator_actionable() {
        let err = no_default_render_device_error("HRESULT 0x80070490 (ERROR_NOT_FOUND)");
        let msg = err.to_string();
        assert!(
            msg.contains("no default audio render device"),
            "must state that no default device was found; got: {msg}"
        );
        assert!(
            msg.contains("Windows Sound Settings"),
            "must mention Windows Sound Settings for GUI recovery; got: {msg}"
        );
        assert!(
            msg.contains("--list-capture-devices"),
            "must suggest the CLI discovery flag; got: {msg}"
        );
        assert!(
            msg.contains("capture_device"),
            "must mention the config key so the operator knows where to set it; got: {msg}"
        );
    }

    /// The raw WASAPI diagnostic string must be preserved verbatim inside the
    /// error so it appears in logs and is useful for support.
    #[test]
    fn no_default_render_device_error_preserves_wasapi_diagnostic() {
        let raw = "HRESULT 0xDEADBEEF some-windows-error";
        let err = no_default_render_device_error(raw);
        assert!(
            err.to_string().contains(raw),
            "raw WASAPI error must be embedded for diagnostics"
        );
    }

    /// `find_render_device_by_name` must return `Err` (not panic) when the
    /// requested device does not exist.  This exercises the WASAPI COM path;
    /// the test skips gracefully if COM cannot be initialised (headless CI).
    #[test]
    fn find_render_device_by_name_unknown_returns_err_not_panic() {
        // Best-effort COM initialisation — skip rather than fail if unavailable.
        if initialize_mta().is_err() {
            return;
        }
        let result = find_render_device_by_name("____nonexistent_device_issue_196____");
        assert!(
            result.is_err(),
            "an unknown device name must produce Err, not panic"
        );
        // `wasapi::Device` does not implement Debug so we cannot use unwrap_err().
        let msg = match result {
            Err(e) => e.to_string(),
            Ok(_) => unreachable!(),
        };
        assert!(
            msg.contains("was not found"),
            "error should report the device was not found; got: {msg}"
        );
    }
}
