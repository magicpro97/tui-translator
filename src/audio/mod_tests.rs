//! Unit tests for `mod` (extracted from `mod.rs` for the
//! STD-02 module-budget refactor; behavior unchanged).
//!
//! Compiled only under `#[cfg(test)]` via the parent module's `#[path]` include.

use super::*;

// ── AudioChunk helpers ──────────────────────────────────────────────────

#[test]
fn chunk_duration_empty() {
    let c = AudioChunk::new(vec![]);
    assert_eq!(c.duration_ms, 0);
}

#[test]
fn chunk_duration_8000_samples() {
    // 8 000 samples at 16 kHz = 500 ms
    let c = AudioChunk::new(vec![0i16; 8_000]);
    assert_eq!(c.duration_ms, 500);
}

#[test]
fn chunk_rms_silence() {
    let c = AudioChunk::new(vec![0i16; 8_000]);
    assert_eq!(c.rms_energy(), 0.0);
}

#[test]
fn chunk_rms_full_scale_positive() {
    // All samples at i16::MAX → RMS ≈ 1.0
    let c = AudioChunk::new(vec![i16::MAX; 1_000]);
    let rms = c.rms_energy();
    assert!((rms - 1.0).abs() < 1e-4, "rms={rms}");
}

#[test]
fn chunk_rms_full_scale_negative() {
    // All samples at i16::MIN → RMS ≈ 1.0 (normalised by i16::MAX)
    let c = AudioChunk::new(vec![i16::MIN; 1_000]);
    let rms = c.rms_energy();
    // i16::MIN / i16::MAX ≈ -1.000_03, clamped via squaring → ~1.000_06
    assert!(rms > 0.999, "rms={rms}");
}

#[test]
fn channel_capacity_covers_five_seconds_of_wasapi_backpressure() {
    const WASAPI_CHUNK_MS: usize = 10;
    let capacity = std::hint::black_box(CHANNEL_CAPACITY);

    assert!(
        capacity * WASAPI_CHUNK_MS >= 5_000,
        "audio channel must buffer at least five seconds of 10 ms WASAPI chunks"
    );
}

// ── SilenceDetector ─────────────────────────────────────────────────────

fn silence_chunk(ms: u32) -> AudioChunk {
    let samples = (ms as usize) * 16; // 16 samples per ms at 16 kHz
    AudioChunk::new(vec![0i16; samples])
}

fn loud_chunk(ms: u32) -> AudioChunk {
    let samples = (ms as usize) * 16;
    AudioChunk::new(vec![i16::MAX / 2; samples])
}

#[test]
fn detector_passes_loud_chunk() {
    let mut det = SilenceDetector::new(DEFAULT_SILENCE_THRESHOLD, 500);
    assert!(det.process(&loud_chunk(100)));
}

#[test]
fn detector_passes_initial_silence_below_gate() {
    // First 400 ms of silence should still be forwarded (< 500 ms gate).
    let mut det = SilenceDetector::new(DEFAULT_SILENCE_THRESHOLD, 500);
    assert!(det.process(&silence_chunk(400)));
}

#[test]
fn detector_suppresses_after_gate_exceeded() {
    // 600 ms of silence (> 500 ms gate) should be suppressed.
    let mut det = SilenceDetector::new(DEFAULT_SILENCE_THRESHOLD, 500);
    // Feed 500 ms first (still passes due to ">" not ">=")
    det.process(&silence_chunk(500));
    // Next silent chunk pushes us over the gate.
    let suppressed = !det.process(&silence_chunk(100));
    assert!(suppressed, "expected suppression after 600 ms of silence");
}

#[test]
fn detector_resets_on_loud_chunk() {
    let mut det = SilenceDetector::new(DEFAULT_SILENCE_THRESHOLD, 500);
    // Accumulate 600 ms silence to trigger suppression.
    det.process(&silence_chunk(500));
    det.process(&silence_chunk(100)); // now suppressing
                                      // A loud chunk should reopen the gate.
    assert!(
        det.process(&loud_chunk(100)),
        "loud chunk should pass after reset"
    );
    // And the next silence chunk should also pass (accumulator was reset).
    assert!(
        det.process(&silence_chunk(100)),
        "short silence should pass after loud reset"
    );
}

#[test]
fn detector_reset_method_clears_accumulator() {
    let mut det = SilenceDetector::new(DEFAULT_SILENCE_THRESHOLD, 500);
    det.process(&silence_chunk(600)); // trigger suppression
    det.reset();
    // After manual reset, short silence should pass again.
    assert!(det.process(&silence_chunk(100)));
}

// ── SilentSource ────────────────────────────────────────────────────────

#[test]
fn silent_source_chunk_is_silence() {
    let mut src = SilentSource;
    let chunk = src.next_chunk().unwrap();
    assert_eq!(chunk.duration_ms, 500);
    assert!(chunk.rms_energy() < DEFAULT_SILENCE_THRESHOLD);
}

#[test]
fn silent_source_device_name() {
    let src = SilentSource;
    assert_eq!(src.device_name(), "silent (stub)");
}

// macOS now uses a real CoreAudio/BlackHole capture path (MACOS-07, issue #638)
// that fails on CI runners without BlackHole installed.  This stub-silence test
// only applies to Linux (linux-stub) and the generic non-Windows/non-macOS path.
#[cfg(not(any(windows, target_os = "macos")))]
#[tokio::test]
async fn non_windows_capture_emits_multiple_chunks() {
    let mut capture = start_capture(DEFAULT_SILENCE_THRESHOLD).await.unwrap();
    // Linux routing returns "silent (linux-stub)",
    // and the generic fallback returns "silent (stub)".
    assert!(
        capture.info.device_name.starts_with("silent"),
        "unexpected device name: {}",
        capture.info.device_name
    );
    assert_eq!(capture.info.native_sample_rate, 16_000);

    let first = tokio::time::timeout(std::time::Duration::from_secs(1), capture.receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let second = tokio::time::timeout(std::time::Duration::from_secs(1), capture.receiver.recv())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(first.samples.len(), 8_000);
    assert_eq!(second.samples.len(), 8_000);
}

// ── start_file_capture — silence gate and timing (issue #110 fixes) ──────
//
// These tests exercise the WavFileSource + SilenceDetector components
// directly (no async spawning) so they are fast, deterministic, and free
// of spawn_blocking timing concerns while still covering the exact logic
// that start_file_capture uses.

const SOAK_FIXTURE: &str = "tests/soak/soak_audio.wav";
/// Chunk count and sample count of the soak fixture.
const FIXTURE_SAMPLES: usize = 480_000;

/// The silence gate suppresses chunks once accumulated silent duration
/// exceeds DEFAULT_SILENCE_GATE_MS.
///
/// With `f32::MAX` threshold every chunk's RMS is below the threshold, so
/// the gate fires after the first 256-ms chunk (accumulated = 512 ms > 500 ms).
#[test]
fn file_source_silence_gate_suppresses_after_first_chunk() {
    if !std::path::Path::new(SOAK_FIXTURE).exists() {
        return; // skip when fixture is absent (sparse CI checkout)
    }
    let mut source = WavFileSource::open(SOAK_FIXTURE).unwrap();
    let mut detector = SilenceDetector::new(f32::MAX, DEFAULT_SILENCE_GATE_MS);

    // Chunk 1 (256 ms): accumulated silence = 256 ms ≤ gate (500 ms) → pass.
    let chunk1 = source.next_chunk().unwrap();
    assert_eq!(chunk1.duration_ms, 256, "fixture chunk should be 256 ms");
    assert!(
        detector.process(&chunk1),
        "first chunk should pass: accumulated silence below gate"
    );

    // Chunk 2 (256 ms): accumulated silence = 512 ms > gate (500 ms) → suppress.
    let chunk2 = source.next_chunk().unwrap();
    assert!(
        !detector.process(&chunk2),
        "second chunk should be suppressed: accumulated silence exceeds gate"
    );
}

/// With threshold `0.0` every chunk is treated as non-silent so nothing is
/// ever suppressed, regardless of its RMS energy.
#[test]
fn file_source_zero_threshold_never_suppresses() {
    if !std::path::Path::new(SOAK_FIXTURE).exists() {
        return;
    }
    let mut source = WavFileSource::open(SOAK_FIXTURE).unwrap();
    let mut detector = SilenceDetector::new(0.0, DEFAULT_SILENCE_GATE_MS);

    for i in 0..5 {
        let chunk = source.next_chunk().unwrap();
        assert!(
            detector.process(&chunk),
            "chunk {i} should always pass with threshold 0.0"
        );
    }
}

/// Tail chunks at the loop boundary are shorter than DEFAULT_CHUNK_SAMPLES.
/// Their `duration_ms` must reflect the actual sample count so that
/// `start_file_capture` paces them correctly and avoids drift.
#[test]
fn file_source_tail_chunk_has_shorter_duration_than_full_chunk() {
    if !std::path::Path::new(SOAK_FIXTURE).exists() {
        return;
    }
    let full_chunk_ms = file_source::DEFAULT_CHUNK_SAMPLES as u32 * 1000 / 16_000; // 256 ms
    let tail_samples = FIXTURE_SAMPLES % file_source::DEFAULT_CHUNK_SAMPLES; // 768
    let expected_tail_ms = tail_samples as u32 * 1000 / 16_000; // 48 ms

    let mut source = WavFileSource::open(SOAK_FIXTURE).unwrap();
    let full_chunks_before_tail = FIXTURE_SAMPLES / file_source::DEFAULT_CHUNK_SAMPLES;
    for _ in 0..full_chunks_before_tail {
        source.next_chunk().unwrap();
    }
    let tail = source.next_chunk().unwrap();

    assert_eq!(
        tail.duration_ms, expected_tail_ms,
        "tail chunk duration must match actual sample count, not DEFAULT_CHUNK_SAMPLES"
    );
    assert!(
        tail.duration_ms < full_chunk_ms,
        "tail chunk ({} ms) must be shorter than a full chunk ({full_chunk_ms} ms)",
        tail.duration_ms
    );
}

// ─── Platform label helper tests ─────────────────────────────────────────────

#[cfg(target_os = "macos")]
#[test]
fn audio_source_choices_macos_contains_coreaudio_not_wasapi() {
    let choices = super::audio_source_choices_for_os();
    assert!(
        choices.contains(&"coreaudio"),
        "macOS choices must include coreaudio; got: {choices:?}"
    );
    assert!(
        !choices.contains(&"wasapi"),
        "macOS choices must not include wasapi; got: {choices:?}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn capture_device_default_label_macos_not_windows() {
    let label = super::capture_device_default_label();
    assert_ne!(
        label, "Windows default playback",
        "macOS must not return the Windows label"
    );
    assert!(
        label.contains("macOS"),
        "macOS label should mention macOS; got: {label:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn audio_source_choices_linux_contains_pipewire() {
    let choices = super::audio_source_choices_for_os();
    assert!(
        choices.contains(&"pipewire"),
        "Linux choices must include pipewire; got: {choices:?}"
    );
}

#[cfg(windows)]
#[test]
fn audio_source_choices_windows_contains_wasapi() {
    let choices = super::audio_source_choices_for_os();
    assert!(
        choices.contains(&"wasapi"),
        "Windows choices must include wasapi; got: {choices:?}"
    );
}
