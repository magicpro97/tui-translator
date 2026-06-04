//! US-11 (issue #736) — Hardware self-hosted runner smoke test for the VB-CABLE F32 path.
//!
//! Gated on `#[cfg(all(target_os = "windows", feature = "hw-smoke"))]` so it
//! never runs in the default `cargo test --all` path and never compiles on
//! non-Windows hosts.
//!
//! Prerequisite: a self-hosted Windows runner with VB-CABLE installed and
//! the render endpoint named "CABLE Input" present and active.
//!
//! Run manually with:
//!   cargo test --features hw-smoke --test hw_smoke_vbcable -- --nocapture

// ── Module stubs: provide the `crate::audio` and `crate::pipeline` paths ──────
//
// `audio_sink.rs` resolves `crate::audio::*` and `crate::pipeline::backpressure_hook`.
// We replicate the necessary sub-tree here using `#[path]` so the integration
// test compiles without a `[lib]` target.  Paths are relative to this file
// (`tests/hw_smoke_vbcable.rs`), i.e. `../src/` is the crate source root.
//
// Transitive `crate::` references from sub-modules:
//   audio_gain        — none (only std)
//   pcm_format        — none (only std + serde)
//   vbcable_ci        — none (only std + serde)
//   backpressure_hook — none (only std)
//   audio_sink        — crate::audio::{audio_gain, pcm_format, vbcable_ci}
//                       crate::pipeline::backpressure_hook
//                       (all provided below)

// rustfmt cannot follow #[path] inside inline modules (known limitation).
// The #[rustfmt::skip] attributes below prevent fmt from trying to resolve
// these sub-module files; the Rust compiler has no such restriction.
//
// Path note: inside an inline `mod audio {}` the virtual module directory is
// `tests/audio/`, so paths to `src/` need `../../` to reach the crate root.
#[cfg(all(target_os = "windows", feature = "hw-smoke"))]
#[rustfmt::skip]
mod audio {
    // Suppress dead_code for constants/structs imported but not exercised here.
    #![allow(dead_code)]
    #[path = "../../src/audio/audio_gain.rs"] pub mod audio_gain;
    #[path = "../../src/audio/pcm_format.rs"] pub mod pcm_format;
    #[path = "../../src/audio/vbcable_ci.rs"] pub mod vbcable_ci;
}

#[cfg(all(target_os = "windows", feature = "hw-smoke"))]
#[rustfmt::skip]
mod pipeline {
    // Suppress dead_code for constants/structs imported but not exercised here.
    #![allow(dead_code)]
    #[path = "../../src/pipeline/backpressure_hook.rs"] pub mod backpressure_hook;
    #[path = "../../src/pipeline/audio_sink.rs"]        pub mod audio_sink;
}

// ── Smoke tests ────────────────────────────────────────────────────────────────

#[cfg(all(target_os = "windows", feature = "hw-smoke"))]
mod hw_smoke {
    use super::pipeline::audio_sink::OemCableSink;

    /// Standard VB-CABLE render endpoint name (the "Input" side that apps write to).
    const VBCABLE_DEVICE: &str = "CABLE Input";
    /// Source sample rate for the synthetic tone.
    const SAMPLE_RATE_HZ: u32 = 16_000;
    /// Frequency of the synthetic sine tone.
    const FREQUENCY_HZ: f64 = 440.0;
    /// Duration of the synthetic tone in milliseconds.
    const DURATION_MS: u64 = 1_000;

    /// Build a minimal single-channel 16-bit PCM WAV buffer containing a
    /// `freq_hz` Hz sine tone at `sample_rate` Hz for `duration_ms` milliseconds.
    ///
    /// The returned bytes are accepted by `rodio::Decoder` and thus by
    /// `OemCableSink::new_windows` which uses `RodioTtsPcmDecoder` internally.
    fn sine_as_wav(sample_rate: u32, freq_hz: f64, duration_ms: u64) -> Vec<u8> {
        let num_samples = (sample_rate as u64 * duration_ms / 1_000) as usize;
        let samples: Vec<i16> = (0..num_samples)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                let s = (std::f64::consts::TAU * freq_hz * t).sin() * 0.5;
                (s * i16::MAX as f64) as i16
            })
            .collect();

        let data_size = (num_samples * 2) as u32;
        let mut buf = Vec::with_capacity(44 + num_samples * 2);

        // RIFF / WAVE header
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36 + data_size).to_le_bytes());
        buf.extend_from_slice(b"WAVE");

        // fmt sub-chunk — PCM, mono, 16-bit
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes()); // sub-chunk size
        buf.extend_from_slice(&1u16.to_le_bytes()); // AudioFormat = PCM
        buf.extend_from_slice(&1u16.to_le_bytes()); // NumChannels  = mono
        buf.extend_from_slice(&sample_rate.to_le_bytes()); // SampleRate
        buf.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // ByteRate
        buf.extend_from_slice(&2u16.to_le_bytes()); // BlockAlign
        buf.extend_from_slice(&16u16.to_le_bytes()); // BitsPerSample

        // data sub-chunk
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());
        for s in samples {
            buf.extend_from_slice(&s.to_le_bytes());
        }

        buf
    }

    /// US-11 (#736): VB-CABLE F32 path end-to-end smoke.
    ///
    /// Opens the VB-CABLE Input render endpoint via `OemCableSink::new_windows`,
    /// plays a 1-second 440 Hz mono sine, and asserts:
    ///   (a) the sink opens successfully and `try_play_bytes` returns `Ok`, and
    ///   (b) the WASAPI client reports a non-zero written sample count, proving
    ///       the F32 VB-CABLE negotiation path (fixed in US-08) is exercised.
    ///
    /// If the VB-CABLE device is not found on this runner the test emits a
    /// diagnostic and returns without failing; on a properly configured
    /// self-hosted runner with the `vb-cable` label this path should not be taken.
    #[test]
    fn vbcable_f32_smoke_oem_cable_sink_routes_sine() {
        let sink = match OemCableSink::new_windows(VBCABLE_DEVICE) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "[hw-smoke] SKIP — could not open '{VBCABLE_DEVICE}': {e}\n\
                     Ensure VB-CABLE is installed and the runner has the `vb-cable` label."
                );
                return;
            }
        };

        let wav = sine_as_wav(SAMPLE_RATE_HZ, FREQUENCY_HZ, DURATION_MS);
        // Expected number of i16 samples rodio will decode from the WAV above.
        let expected_decoded: u64 = SAMPLE_RATE_HZ as u64 * DURATION_MS / 1_000;

        let evidence = sink
            .try_play_bytes(wav)
            .expect("OemCableSink::try_play_bytes must succeed on a VB-CABLE hardware runner");

        assert_eq!(
            evidence.decoded_sample_count, expected_decoded,
            "decoded_sample_count must equal the source sine ({expected_decoded} samples)"
        );
        assert!(
            evidence.written_sample_count > 0,
            "WASAPI writer must report at least one written sample; \
             got 0 — the F32 VB-CABLE path may have silently dropped audio"
        );
        assert_eq!(
            evidence.dropped_frames, 0,
            "VB-CABLE smoke: no frames should be dropped on a dedicated hardware runner"
        );

        eprintln!(
            "[hw-smoke] PASS — device={VBCABLE_DEVICE:?}, \
             decoded={decoded}, converted={converted}, written={written}, \
             dropped={dropped}, rms={rms:.4}, latency_ms={lat:.2}",
            decoded = evidence.decoded_sample_count,
            converted = evidence.converted_sample_count,
            written = evidence.written_sample_count,
            dropped = evidence.dropped_frames,
            rms = evidence.rms,
            lat = evidence.latency_ms,
        );
    }
}
