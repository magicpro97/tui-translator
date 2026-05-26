//! Unit tests for `vad` (extracted from `vad.rs` for the
//! STD-02 module-budget refactor; behavior unchanged).
//!
//! Compiled only under `#[cfg(test)]` via the parent module's `#[path]` include.

use super::*;

// ── Helpers ───────────────────────────────────────────────────────────────

/// Build a silent chunk of `duration_ms` milliseconds (energy = 0).
fn silent_chunk(duration_ms: u32) -> AudioChunk {
    let samples = (duration_ms as usize * 16_000) / 1_000;
    AudioChunk::new(vec![0i16; samples])
}

/// Build a speech-level chunk of `duration_ms` milliseconds.
///
/// Amplitude is set to 30 % of i16::MAX, well above the default VAD
/// threshold (0.01) while leaving headroom for future tests.
fn speech_chunk(duration_ms: u32) -> AudioChunk {
    let samples = (duration_ms as usize * 16_000) / 1_000;
    let amp = (i16::MAX as f32 * 0.3) as i16;
    AudioChunk::new(vec![amp; samples])
}

/// Build a transient spike of `duration_ms` milliseconds at full scale.
///
/// A real transient (click, pop, applause onset) has very high energy for
/// a very short duration — typically < 50 ms.
fn transient_chunk(duration_ms: u32) -> AudioChunk {
    let samples = (duration_ms as usize * 16_000) / 1_000;
    AudioChunk::new(vec![i16::MAX; samples])
}

/// Count how many chunks in `decisions` are `VadDecision::Speech`.
fn count_speech(decisions: &[VadDecision]) -> usize {
    decisions
        .iter()
        .filter(|&&d| d == VadDecision::Speech)
        .count()
}

// ── T1: Silence fixture → 0 speech decisions ──────────────────────────────

/// T1 — 30 s of synthesised silence produces zero Speech decisions.
///
/// Acceptance criterion from issue #220:
///   "Silence fixture emits no speech segments."
#[test]
fn t1_silence_fixture_emits_zero_speech_decisions() {
    let mut gate = VadGate::default_gate();

    // 30 s of silence in 100 ms chunks (300 chunks).
    let decisions: Vec<VadDecision> = (0..300).map(|_| gate.process(&silent_chunk(100))).collect();

    assert_eq!(
        count_speech(&decisions),
        0,
        "30 s of silence must produce zero Speech decisions; got {}",
        count_speech(&decisions)
    );
}

/// T1 variant — silence gate does not open even for a single full-scale
/// silent chunk.
#[test]
fn t1_single_silent_chunk_is_suppressed() {
    let mut gate = VadGate::default_gate();
    let decision = gate.process(&silent_chunk(500));
    assert_eq!(decision, VadDecision::Silence);
}

// ── T2: Speech fixture opens gate with padding ─────────────────────────────

/// T2 — sustained speech opens the gate promptly and the gate emits Speech
/// decisions.
///
/// Acceptance criterion from issue #220:
///   "Speech fixture opens gate promptly."
///
/// We synthesise 3 s of speech-level audio (matching the duration of the
/// `ja_speech_3s.wav` fixture).  The test also verifies that the gate
/// applies padding: after the speech ends, the gate remains open for
/// `speech_pad_ms` before closing.
#[test]
fn t2_speech_opens_gate_and_pads_trailing_silence() {
    let config = VadConfig {
        threshold: DEFAULT_VAD_THRESHOLD,
        min_speech_ms: 100,
        speech_pad_ms: 300,
        min_silence_ms: 500,
        pre_roll_ms: DEFAULT_PRE_ROLL_MS,
    };
    let mut gate = VadGate::new(config);

    let mut decisions = Vec::new();

    // 3 s of speech (30 × 100 ms chunks).
    for _ in 0..30 {
        decisions.push(gate.process(&speech_chunk(100)));
    }

    // 1 s of silence following speech — gate should stay open for speech_pad_ms.
    let mut pad_open_count = 0usize;
    for _ in 0..10 {
        let d = gate.process(&silent_chunk(100));
        if d == VadDecision::Speech {
            pad_open_count += 1;
        }
    }

    let total_speech = count_speech(&decisions);
    assert!(
        total_speech > 0,
        "3 s of speech-level audio must produce Speech decisions; got 0"
    );
    assert!(
        pad_open_count > 0,
        "gate must remain open for speech_pad_ms after speech ends; \
         expected at least one Speech decision in the trailing-silence window"
    );
}

/// T2 variant — gate opens within `min_speech_ms` of sustained speech.
#[test]
fn t2_gate_opens_promptly_after_min_speech_ms() {
    // Use a short min_speech_ms so we can precisely count frames until opening.
    let config = VadConfig {
        threshold: DEFAULT_VAD_THRESHOLD,
        min_speech_ms: 100, // 100 ms = 1 × 100 ms chunk
        speech_pad_ms: 200,
        min_silence_ms: 300,
        pre_roll_ms: DEFAULT_PRE_ROLL_MS,
    };
    let mut gate = VadGate::new(config);

    // First speech chunk (100 ms) reaches exactly min_speech_ms → gate opens.
    let d = gate.process(&speech_chunk(100));
    assert_eq!(
        d,
        VadDecision::Speech,
        "gate must open on the first chunk that reaches min_speech_ms"
    );
}

// ── T3: Loud transient is suppressed ─────────────────────────────────────

/// T3 — a 50 ms full-scale transient is suppressed (not counted as speech).
///
/// Acceptance criterion from issue #220:
///   "Loud transient is suppressed or marked non-speech."
///
/// The default `min_speech_ms` (100 ms) exceeds the transient duration
/// (50 ms) so the gate never leaves `Confirming`.
#[test]
fn t3_short_transient_is_suppressed() {
    let mut gate = VadGate::default_gate();

    // 50 ms transient spike.
    let d_transient = gate.process(&transient_chunk(50));

    // 5 s of silence following the transient.
    let silence_decisions: Vec<VadDecision> =
        (0..50).map(|_| gate.process(&silent_chunk(100))).collect();

    assert_eq!(
        d_transient,
        VadDecision::Silence,
        "50 ms transient must be suppressed (duration < min_speech_ms)"
    );
    assert_eq!(
        count_speech(&silence_decisions),
        0,
        "silence after a transient must remain silence; got {} Speech",
        count_speech(&silence_decisions)
    );
}

/// T3 variant — a 200 ms burst at full scale IS confirmed as speech
/// (duration > min_speech_ms), demonstrating the threshold boundary.
#[test]
fn t3_sustained_burst_above_min_speech_ms_is_confirmed_as_speech() {
    let config = VadConfig {
        threshold: DEFAULT_VAD_THRESHOLD,
        min_speech_ms: 100,
        speech_pad_ms: 200,
        min_silence_ms: 300,
        pre_roll_ms: DEFAULT_PRE_ROLL_MS,
    };
    let mut gate = VadGate::new(config);

    // Feed 200 ms of full-scale audio in two 100 ms chunks.
    let d1 = gate.process(&transient_chunk(100));
    let d2 = gate.process(&transient_chunk(100));

    // At least one of the two chunks must be marked Speech once the gate opens.
    assert!(
        d1 == VadDecision::Speech || d2 == VadDecision::Speech,
        "200 ms burst above min_speech_ms must open the gate"
    );
}

// ── State machine transitions ─────────────────────────────────────────────

/// Verify that the gate starts in Silence state.
#[test]
fn initial_state_is_silence() {
    let gate = VadGate::default_gate();
    assert_eq!(gate.state_label(), "silence");
}

/// Verify that `reset` returns the gate to Silence from any state.
#[test]
fn reset_returns_gate_to_silence() {
    let mut gate = VadGate::default_gate();

    // Drive to Speech.
    gate.process(&speech_chunk(200));
    assert_ne!(gate.state_label(), "silence");

    gate.reset();
    assert_eq!(gate.state_label(), "silence");

    // After reset, silence chunks must return Silence.
    let d = gate.process(&silent_chunk(100));
    assert_eq!(d, VadDecision::Silence);
}

/// Speech followed by silence followed by resumed speech reopens the gate.
#[test]
fn gate_reopens_on_speech_resumption_within_pad_window() {
    let config = VadConfig {
        threshold: DEFAULT_VAD_THRESHOLD,
        min_speech_ms: 100,
        speech_pad_ms: 300,
        min_silence_ms: 500,
        pre_roll_ms: DEFAULT_PRE_ROLL_MS,
    };
    let mut gate = VadGate::new(config);

    // Open gate with confirmed speech.
    gate.process(&speech_chunk(200));
    assert_eq!(gate.state_label(), "speech");

    // 100 ms of silence — still within speech_pad_ms.
    gate.process(&silent_chunk(100));
    assert_eq!(gate.state_label(), "pad-open");

    // Speech resumes — gate should go back to "speech".
    let d = gate.process(&speech_chunk(100));
    assert_eq!(d, VadDecision::Speech);
    assert_eq!(gate.state_label(), "speech");
}

/// Gate closes fully after speech_pad_ms + min_silence_ms of silence.
#[test]
fn gate_closes_after_full_silence_window() {
    let config = VadConfig {
        threshold: DEFAULT_VAD_THRESHOLD,
        min_speech_ms: 100,
        speech_pad_ms: 300,
        min_silence_ms: 500,
        pre_roll_ms: DEFAULT_PRE_ROLL_MS,
    };
    let mut gate = VadGate::new(config);

    // Open gate.
    gate.process(&speech_chunk(200));

    // 400 ms silence: first 300 ms is PadOpen, last 100 ms enters PadClosed.
    for _ in 0..4 {
        gate.process(&silent_chunk(100));
    }
    // Now in PadClosed — need min_silence_ms (500 ms) more.
    for _ in 0..5 {
        gate.process(&silent_chunk(100));
    }
    assert_eq!(
        gate.state_label(),
        "silence",
        "gate must be fully closed after speech_pad_ms + min_silence_ms of silence"
    );
}

// ── EndOfUtterance signal (issue #264) ───────────────────────────────────

/// `EndOfUtterance` is emitted exactly once at the `PadClosed → Silence`
/// transition, and the gate is in `Silence` state afterward.
#[test]
fn end_of_utterance_fires_at_pad_closed_expiry() {
    let config = VadConfig {
        threshold: DEFAULT_VAD_THRESHOLD,
        min_speech_ms: 100,
        speech_pad_ms: 300,
        min_silence_ms: 500,
        pre_roll_ms: DEFAULT_PRE_ROLL_MS,
    };
    let mut gate = VadGate::new(config);

    // Open the gate with 200 ms of speech.
    gate.process(&speech_chunk(200));
    assert_eq!(gate.state_label(), "speech");

    // Drain through PadOpen (300 ms) then into PadClosed (100 ms).
    for _ in 0..4 {
        gate.process(&silent_chunk(100));
    }
    assert_eq!(
        gate.state_label(),
        "pad-closed",
        "gate must be in pad-closed after speech_pad_ms of silence"
    );

    // Feed silence until min_silence_ms - 1 (400 ms total): must stay Silence, no EoU.
    // After the first loop, state_ms = 100 ms in PadClosed; 3 more × 100 ms → 400 ms.
    for _ in 0..3 {
        let d = gate.process(&silent_chunk(100));
        assert_eq!(
            d,
            VadDecision::Silence,
            "gate must return Silence while accumulating pad-closed silence"
        );
    }
    assert_eq!(gate.state_label(), "pad-closed");

    // The next 100 ms chunk crosses min_silence_ms (500 ms) → EndOfUtterance.
    let eou = gate.process(&silent_chunk(100));
    assert_eq!(
        eou,
        VadDecision::EndOfUtterance,
        "gate must return EndOfUtterance when min_silence_ms is reached"
    );

    // Gate must be fully closed now.
    assert_eq!(gate.state_label(), "silence");

    // Subsequent silent chunks return plain Silence (not another EoU).
    let d = gate.process(&silent_chunk(100));
    assert_eq!(
        d,
        VadDecision::Silence,
        "only one EndOfUtterance per utterance; subsequent chunks return Silence"
    );
}

/// A second utterance after `EndOfUtterance` produces a second signal.
#[test]
fn end_of_utterance_fires_again_for_second_utterance() {
    let config = VadConfig {
        threshold: DEFAULT_VAD_THRESHOLD,
        min_speech_ms: 100,
        speech_pad_ms: 100,
        min_silence_ms: 200,
        pre_roll_ms: DEFAULT_PRE_ROLL_MS,
    };
    let mut gate = VadGate::new(config);

    let drain = |gate: &mut VadGate| {
        // Four 100 ms silent chunks are required: the first enters PadOpen,
        // the second expires the 100 ms pad and enters PadClosed, and the
        // third/fourth accumulate the 200 ms min_silence_ms confirmation.
        gate.process(&silent_chunk(100)); // Speech -> PadOpen
        gate.process(&silent_chunk(100)); // PadOpen -> PadClosed
        gate.process(&silent_chunk(100)); // still PadClosed
        gate.process(&silent_chunk(100)) // PadClosed→Silence → EndOfUtterance
    };

    // First utterance.
    gate.process(&speech_chunk(200));
    let eou1 = drain(&mut gate);
    assert_eq!(
        eou1,
        VadDecision::EndOfUtterance,
        "first utterance must produce EoU"
    );

    // Second utterance.
    gate.process(&speech_chunk(200));
    let eou2 = drain(&mut gate);
    assert_eq!(
        eou2,
        VadDecision::EndOfUtterance,
        "second utterance must produce EoU"
    );
}

/// T2 fixture — feed `ja_speech_3s.wav` through the VAD gate and verify
/// that Speech decisions are emitted.
///
/// Uses [`super::super::WavFileSource`] to read the fixture without
/// requiring a real audio device.  The test is skipped (not failed) if the
/// fixture file is not present so CI can still run the remaining tests.
#[test]
fn t2_ja_speech_fixture_emits_speech_decisions() {
    use super::super::file_source::WavFileSource;
    use super::super::AudioSource;

    let fixture_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ja_speech_3s.wav"
    );

    // Skip gracefully if fixture is missing (e.g., partial checkout).
    if !std::path::Path::new(fixture_path).exists() {
        eprintln!("SKIP: fixture not found at {fixture_path}");
        return;
    }

    let mut source = WavFileSource::open(fixture_path)
        .expect("ja_speech_3s.wav must be a valid 16 kHz mono WAV");

    let mut gate = VadGate::default_gate();
    let mut speech_count = 0usize;
    let mut total_chunks = 0usize;

    // Read the whole file (3 s at 256 ms per chunk ≈ 12 chunks), plus a
    // small number of leading silence chunks to let the confirmation window
    // settle.  We stop after 60 chunks to guard against infinite loops if
    // WavFileSource loops.
    for _ in 0..60 {
        let chunk = source.next_chunk().expect("fixture read must not fail");
        let d = gate.process(&chunk);
        if d == VadDecision::Speech {
            speech_count += 1;
        }
        total_chunks += 1;
        // Stop once we have processed at least 3 s worth of audio.
        if total_chunks * 256 >= 3_000 {
            break;
        }
    }

    assert!(
        speech_count > 0,
        "ja_speech_3s.wav must trigger at least one Speech decision; \
         processed {total_chunks} chunks and got 0 Speech. \
         Check that the fixture contains audible speech and the VAD \
         threshold ({}) is appropriate.",
        DEFAULT_VAD_THRESHOLD
    );
}
