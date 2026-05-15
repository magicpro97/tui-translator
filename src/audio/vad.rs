//! Voice Activity Detection (VAD) gate — EP-E.1 (issue #220).
//!
//! Provides a CPU-light VAD abstraction that distinguishes continuous speech
//! from silence and brief transient noise (clicks, pops) without requiring an
//! external ML model or C runtime.
//!
//! # Algorithm
//!
//! A five-state machine driven by per-frame RMS energy:
//!
//! ```text
//! ┌──────────┐  energy ≥ thr  ┌────────────┐  held ≥ min_speech_ms  ┌────────┐
//! │ Silence  │ ─────────────▶ │ Confirming │ ──────────────────────▶ │ Speech │
//! │(gate off)│                │ (gate off) │                         │(gate on│
//! └──────────┘                └────────────┘                         └────────┘
//!      ▲                           │ energy < thr                        │
//!      │ ◀───── transient ─────────┘ (suppressed)                       │ energy < thr
//!      │                                                                  ▼
//!      │                                                          ┌──────────────┐
//!      │                            energy ≥ thr                 │   PadOpen    │
//!      │ ◀─── pad expires ──────────── ... ──────────────────────│  (gate on,   │
//!      │                                                          │ speech_pad)  │
//!      │                                                          └──────────────┘
//!      │                                                                  │ pad expires
//!      │                                                                  ▼
//!      │                          energy ≥ thr                  ┌──────────────────┐
//!      └────────────────────────────── ... ──────────────────── │   PadClosed      │
//!       ◀──────── min_silence_ms elapses ───────────────────────│ (gate off, wait) │
//!                                                               └──────────────────┘
//! ```
//!
//! **Transient suppression**: the `Confirming` state acts as a confirmation
//! gate — a loud transient shorter than `min_speech_ms` never opens the gate.
//!
//! **Post-speech padding**: the `PadOpen` state keeps the gate open for
//! `speech_pad_ms` after energy drops, preventing clipping of word endings and
//! providing trailing context for the STT window.
//!
//! **End-of-speech confirmation**: the `PadClosed` state silently accumulates
//! silence for `min_silence_ms` before returning to `Silence`, allowing short
//! gaps mid-utterance to be bridged.
//!
//! # Why not WebRTC VAD / Silero ONNX?
//!
//! * WebRTC VAD requires a C build chain; on `x86_64-pc-windows-gnu` CI the
//!   required C dependencies may not be available in all build environments.
//! * Silero requires an ONNX or GGML runtime — a large binary footprint and
//!   inference cost that conflicts with the CPU-only performance requirement.
//!
//! The energy-threshold state machine is **O(n)** per chunk (one f64 RMS pass)
//! and is **deterministically testable** from synthetic PCM data.  All three
//! acceptance criteria in issue #220 (silence/speech/transient) are satisfied
//! without an ML model.  The abstraction is designed so that a `WebRtcVad` or
//! `SileroVad` backend can be swapped in behind the same `VadGate` interface.

use super::AudioChunk;

// ─── Default constants ────────────────────────────────────────────────────────

/// Default RMS energy threshold below which a frame is considered non-speech.
///
/// Roughly −40 dBFS; high enough to reject background hiss while remaining
/// below typical speech levels (−20 to −10 dBFS).
pub const DEFAULT_VAD_THRESHOLD: f32 = 0.01;

/// Default minimum consecutive speech milliseconds before the gate opens.
///
/// A 100 ms confirmation window suppresses transients (clicks, pops) without
/// introducing audible latency at the start of an utterance.
pub const DEFAULT_MIN_SPEECH_MS: u32 = 100;

/// Default post-speech padding: how long the gate stays open after the last
/// speech frame before entering the end-of-speech confirmation phase.
///
/// 300 ms of trailing context is forwarded to the STT window so that word
/// endings are not clipped.
pub const DEFAULT_SPEECH_PAD_MS: u32 = 300;

/// Default minimum silence milliseconds needed to confirm end of speech.
///
/// After the `speech_pad_ms` pad expires, the gate remains in a "closed but
/// monitoring" state for this duration before fully closing.  This bridges
/// short intra-utterance pauses (breaths, punctuation).
pub const DEFAULT_MIN_SILENCE_MS: u32 = 500;

// ─── VadConfig ────────────────────────────────────────────────────────────────

/// Configuration for the VAD gate.
///
/// Mirrors the parameters found in popular VAD implementations (Silero,
/// WebRTC) so the config file schema is forward-compatible if a model-based
/// backend is added later.
#[derive(Debug, Clone, PartialEq)]
pub struct VadConfig {
    /// RMS energy threshold below which a frame is considered non-speech
    /// (normalised 0.0–1.0, where 1.0 is full-scale i16).
    pub threshold: f32,

    /// Minimum consecutive above-threshold milliseconds before the gate opens.
    ///
    /// Used to suppress brief transient noise (clicks, applause peaks, etc.)
    /// that are shorter than a real syllable.
    pub min_speech_ms: u32,

    /// Milliseconds the gate stays **open** after the last speech frame.
    ///
    /// Trailing audio is forwarded so the STT engine receives full word
    /// endings.  When energy recovers within this window the gate stays open.
    pub speech_pad_ms: u32,

    /// Milliseconds of silence after `speech_pad_ms` before the gate closes.
    ///
    /// Short intra-utterance pauses (breaths, hesitations) stay below this
    /// threshold so consecutive words are merged into one STT window.
    pub min_silence_ms: u32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_VAD_THRESHOLD,
            min_speech_ms: DEFAULT_MIN_SPEECH_MS,
            speech_pad_ms: DEFAULT_SPEECH_PAD_MS,
            min_silence_ms: DEFAULT_MIN_SILENCE_MS,
        }
    }
}

// ─── VadDecision ─────────────────────────────────────────────────────────────

/// Result returned by [`VadGate::process`] for each audio chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadDecision {
    /// This chunk contains (or immediately follows) speech — forward to STT.
    Speech,
    /// This chunk is silence or suppressed noise — drop.
    Silence,
}

// ─── Internal state ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VadState {
    /// Quiet baseline; gate closed.
    Silence,
    /// Energy rose above threshold; accumulating confirmation.
    Confirming,
    /// Confirmed speech; gate open.
    Speech,
    /// Energy dropped; forwarding trailing context (`speech_pad_ms` window).
    PadOpen,
    /// Pad expired; gate closed, waiting for `min_silence_ms` to confirm end.
    PadClosed,
}

// ─── VadGate ─────────────────────────────────────────────────────────────────

/// Streaming VAD gate driven by per-frame RMS energy.
///
/// Feed every [`AudioChunk`] through [`VadGate::process`] before the STT
/// accumulation window.  The returned [`VadDecision`] tells the caller whether
/// to forward or drop the chunk.
///
/// # Example
///
/// ```rust,ignore
/// let mut gate = VadGate::new(VadConfig::default());
/// for chunk in audio_stream {
///     if gate.process(&chunk) == VadDecision::Speech {
///         speech_window.push(chunk);
///     }
/// }
/// ```
pub struct VadGate {
    config: VadConfig,
    state: VadState,
    /// Milliseconds accumulated in the current state (used for timed transitions).
    state_ms: u32,
}

impl VadGate {
    /// Create a new gate with the given configuration.
    pub fn new(config: VadConfig) -> Self {
        Self {
            config,
            state: VadState::Silence,
            state_ms: 0,
        }
    }

    /// Create a gate with default configuration.
    pub fn default_gate() -> Self {
        Self::new(VadConfig::default())
    }

    /// Returns the current internal state label for diagnostics.
    pub fn state_label(&self) -> &'static str {
        match self.state {
            VadState::Silence => "silence",
            VadState::Confirming => "confirming",
            VadState::Speech => "speech",
            VadState::PadOpen => "pad-open",
            VadState::PadClosed => "pad-closed",
        }
    }

    /// Feed a single audio chunk and decide whether to forward it.
    ///
    /// This is an O(n) operation (one RMS pass over the samples) and is
    /// designed to be called synchronously in the audio-receive hot path.
    ///
    /// # Decision rules
    ///
    /// | State      | Energy      | Decision | Transition             |
    /// |-----------|-------------|----------|------------------------|
    /// | Silence    | < threshold | Silence  | —                      |
    /// | Silence    | ≥ threshold | Silence  | → Confirming           |
    /// | Confirming | ≥ threshold | Silence  | Stay; Speech on confirm|
    /// | Confirming | < threshold | Silence  | → Silence (transient)  |
    /// | Speech     | ≥ threshold | Speech   | —                      |
    /// | Speech     | < threshold | Speech   | → PadOpen              |
    /// | PadOpen    | ≥ threshold | Speech   | → Speech               |
    /// | PadOpen    | < threshold | Speech   | Stay; PadClosed on exp.|
    /// | PadClosed  | ≥ threshold | Speech   | → Speech               |
    /// | PadClosed  | < threshold | Silence  | Stay; Silence on exp.  |
    #[tracing::instrument(skip_all, level = "trace")]
    pub fn process(&mut self, chunk: &AudioChunk) -> VadDecision {
        let is_above = chunk.rms_energy() >= self.config.threshold;
        let dur = chunk.duration_ms;

        match self.state {
            // ── Silence ───────────────────────────────────────────────────────
            VadState::Silence => {
                if is_above {
                    if dur >= self.config.min_speech_ms {
                        // First chunk already satisfies confirmation window —
                        // open the gate immediately without a Confirming detour.
                        tracing::debug!(
                            duration_ms = dur,
                            threshold = self.config.threshold,
                            "VAD: speech confirmed in single chunk — gate open"
                        );
                        self.state = VadState::Speech;
                        self.state_ms = 0;
                        VadDecision::Speech
                    } else {
                        self.state = VadState::Confirming;
                        self.state_ms = dur;
                        VadDecision::Silence
                    }
                } else {
                    VadDecision::Silence
                }
            }

            // ── Confirming ───────────────────────────────────────────────────
            VadState::Confirming => {
                if is_above {
                    self.state_ms = self.state_ms.saturating_add(dur);
                    if self.state_ms >= self.config.min_speech_ms {
                        tracing::debug!(
                            confirmed_ms = self.state_ms,
                            threshold = self.config.threshold,
                            "VAD: speech confirmed — gate open"
                        );
                        self.state = VadState::Speech;
                        self.state_ms = 0;
                        VadDecision::Speech
                    } else {
                        // Not yet confirmed; suppress.
                        VadDecision::Silence
                    }
                } else {
                    // Energy dropped before confirmation — transient suppressed.
                    tracing::trace!(
                        confirmed_ms = self.state_ms,
                        "VAD: transient suppressed (did not reach min_speech_ms)"
                    );
                    self.state = VadState::Silence;
                    self.state_ms = 0;
                    VadDecision::Silence
                }
            }

            // ── Speech ───────────────────────────────────────────────────────
            VadState::Speech => {
                if is_above {
                    self.state_ms = 0;
                    VadDecision::Speech
                } else {
                    // Energy dropped — enter post-speech pad window.
                    self.state = VadState::PadOpen;
                    self.state_ms = dur;
                    VadDecision::Speech
                }
            }

            // ── PadOpen (gate still open, forwarding trailing context) ───────
            VadState::PadOpen => {
                if is_above {
                    // Speech resumed within pad window.
                    tracing::debug!("VAD: speech resumed within pad window");
                    self.state = VadState::Speech;
                    self.state_ms = 0;
                    VadDecision::Speech
                } else {
                    self.state_ms = self.state_ms.saturating_add(dur);
                    if self.state_ms >= self.config.speech_pad_ms {
                        // Pad expired — move to end-of-speech confirmation.
                        tracing::debug!(
                            pad_ms = self.state_ms,
                            "VAD: pad expired — entering end-of-speech confirmation"
                        );
                        self.state = VadState::PadClosed;
                        self.state_ms = 0;
                        VadDecision::Silence
                    } else {
                        // Still within pad; forward.
                        VadDecision::Speech
                    }
                }
            }

            // ── PadClosed (gate closed, waiting for min_silence_ms) ──────────
            VadState::PadClosed => {
                if is_above {
                    // Speech resumed after pad — reopen gate.
                    tracing::debug!("VAD: speech resumed after pad — gate reopened");
                    self.state = VadState::Speech;
                    self.state_ms = 0;
                    VadDecision::Speech
                } else {
                    self.state_ms = self.state_ms.saturating_add(dur);
                    if self.state_ms >= self.config.min_silence_ms {
                        // Enough silence confirmed — fully close gate.
                        tracing::debug!(
                            silence_ms = self.state_ms,
                            "VAD: silence confirmed — gate closed"
                        );
                        self.state = VadState::Silence;
                        self.state_ms = 0;
                    }
                    VadDecision::Silence
                }
            }
        }
    }

    /// Reset the gate to the initial `Silence` state.
    ///
    /// Call when the pipeline is paused or the audio stream is restarted to
    /// avoid stale state carrying over into the next session.
    pub fn reset(&mut self) {
        self.state = VadState::Silence;
        self.state_ms = 0;
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
        let decisions: Vec<VadDecision> =
            (0..300).map(|_| gate.process(&silent_chunk(100))).collect();

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

    // ── Fixture-based T2: ja_speech_3s.wav ───────────────────────────────────

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
}
