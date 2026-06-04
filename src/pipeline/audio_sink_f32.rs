//! WASAPI render writer for IEEE-float 32-bit PCM endpoints.
//!
//! Extracted from the [`super`] module (`audio_sink`) in WP-24 / US-08 (#733, closes BUG #724)
//! so the parent module stays under the 600 LOC engineering-standards gate.
//!
//! VB-CABLE — the canonical virtual cable on Windows — always advertises
//! `WAVEFORMATEXTENSIBLE` / `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT` (32-bit float,
//! stereo) via `IAudioClient::GetMixFormat()`. CPAL loses the SubFormat GUID,
//! so we use the wasapi crate directly to preserve the encoding end-to-end.

#![cfg(windows)]

use crate::audio::pcm_format::PcmFormat;
use crate::pipeline::audio_sink::{OemCablePcmWriter, OemCableWriteSummary, ProductionSinkError};

/// WASAPI render writer for IEEE-float 32-bit PCM endpoints (VB-CABLE shared mode).
///
/// Unlike `WindowsRenderPcmWriter` (rodio/CPAL), this writer uses the wasapi
/// crate directly so the F32 subformat GUID is preserved end-to-end.
pub(super) struct WasapiF32RenderPcmWriter;

impl OemCablePcmWriter for WasapiF32RenderPcmWriter {
    fn write_pcm(
        &self,
        device_name: &str,
        format: PcmFormat,
        samples: &[i16],
    ) -> Result<OemCableWriteSummary, ProductionSinkError> {
        // Convert i16 → f32 without changing channel count.
        // The caller (try_play_bytes) guarantees the input is already in the
        // correct channel layout; duplicating would produce wrong interleaving.
        let f32_samples: Vec<f32> = samples.iter().map(|&s| s as f32 / 32_768.0).collect();
        self.write_f32_pcm(device_name, format, &f32_samples)
    }

    fn write_f32_pcm(
        &self,
        device_name: &str,
        format: PcmFormat,
        samples: &[f32],
    ) -> Result<OemCableWriteSummary, ProductionSinkError> {
        use std::collections::VecDeque;
        use wasapi::{calculate_period_100ns, DeviceCollection, Direction, ShareMode, WaveFormat};
        // This method runs on the playback thread which may not have COM initialized.
        // Repeated init returns RPC_E_CHANGED_MODE which is harmless; ignore it.
        wasapi::initialize_mta().ok();
        let collection = DeviceCollection::new(&Direction::Render)
            .map_err(|e| ProductionSinkError::WriteFailed(e.to_string()))?;
        let device = collection
            .get_device_with_name(device_name)
            .map_err(|e| ProductionSinkError::WriteFailed(e.to_string()))?;
        let mut audio_client = device
            .get_iaudioclient()
            .map_err(|e| ProductionSinkError::WriteFailed(e.to_string()))?;
        let wave_fmt = WaveFormat::new(
            32,
            32,
            &wasapi::SampleType::Float,
            format.sample_rate_hz as usize,
            format.channels as usize,
            None,
        );
        let desired_period = calculate_period_100ns(480, format.sample_rate_hz as i64);
        audio_client
            .initialize_client(
                &wave_fmt,
                desired_period,
                &Direction::Render,
                &ShareMode::Shared,
                false,
            )
            .map_err(|e| ProductionSinkError::WriteFailed(e.to_string()))?;
        let render_client = audio_client
            .get_audiorenderclient()
            .map_err(|e| ProductionSinkError::WriteFailed(e.to_string()))?;
        audio_client
            .start_stream()
            .map_err(|e| ProductionSinkError::WriteFailed(e.to_string()))?;
        let bytes_per_frame = (format.channels as usize) * 4;
        let mut byte_queue: VecDeque<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let total_frames = samples.len() / format.channels as usize;
        let mut written = 0usize;
        while !byte_queue.is_empty() {
            let available = audio_client
                .get_available_space_in_frames()
                .map_err(|e| ProductionSinkError::WriteFailed(e.to_string()))?
                as usize;
            if available == 0 {
                std::thread::sleep(std::time::Duration::from_millis(1));
                continue;
            }
            let frames_to_write = available.min(byte_queue.len() / bytes_per_frame);
            if frames_to_write == 0 {
                break;
            }
            render_client
                .write_to_device_from_deque(frames_to_write, bytes_per_frame, &mut byte_queue, None)
                .map_err(|e| ProductionSinkError::WriteFailed(e.to_string()))?;
            written += frames_to_write;
        }
        // Drain: wait until the endpoint has played all buffered frames.
        loop {
            let padding = audio_client
                .get_current_padding()
                .map_err(|e| ProductionSinkError::WriteFailed(e.to_string()))?;
            if padding == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        audio_client
            .stop_stream()
            .map_err(|e| ProductionSinkError::WriteFailed(e.to_string()))?;
        Ok(OemCableWriteSummary {
            sample_count: written as u64 * format.channels as u64,
            dropped_frames: (total_frames.saturating_sub(written)) as u64,
        })
    }
}
