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

/// Default pre-roll duration in milliseconds.
///
/// Audio captured while VAD is in the `Confirming` state is buffered and
/// prepended to the STT window when speech is confirmed.  This ensures that
/// leading consonants are not lost during the confirmation window.
/// `0` disables pre-roll entirely.
pub const DEFAULT_PRE_ROLL_MS: u32 = 200;

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

    /// Audio buffered during the `Confirming` state that is prepended to the
    /// STT window when the gate opens.
    ///
    /// Prevents leading consonants from being lost while VAD waits to confirm
    /// speech onset.  The orchestrator trims the confirming-chunk buffer to
    /// the smallest recent chunk suffix that covers this duration.
    /// `0` disables pre-roll entirely.
    pub pre_roll_ms: u32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_VAD_THRESHOLD,
            min_speech_ms: DEFAULT_MIN_SPEECH_MS,
            speech_pad_ms: DEFAULT_SPEECH_PAD_MS,
            min_silence_ms: DEFAULT_MIN_SILENCE_MS,
            pre_roll_ms: DEFAULT_PRE_ROLL_MS,
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
    /// VAD has confirmed end-of-utterance: the gate fully closed after
    /// `speech_pad_ms` + `min_silence_ms` of post-speech silence.
    ///
    /// The chunk itself is silence and must **not** be forwarded to the STT
    /// window.  The caller should flush its speech accumulation window
    /// immediately so the completed utterance is processed without waiting for
    /// the safety-cap duration to expire.
    ///
    /// This signal is emitted at most once per utterance, on the chunk that
    /// causes the `PadClosed → Silence` transition.  After this point the gate
    /// is in `Silence` state and subsequent silent chunks return `Silence` as
    /// usual.
    EndOfUtterance,
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
    /// | State      | Energy / time condition               | Decision       | Transition              |
    /// |-----------|----------------------------------------|----------------|-------------------------|
    /// | Silence    | < threshold                            | Silence        | —                       |
    /// | Silence    | ≥ threshold                            | Silence        | → Confirming            |
    /// | Confirming | ≥ threshold                            | Silence        | Stay; Speech on confirm |
    /// | Confirming | < threshold                            | Silence        | → Silence (transient)   |
    /// | Speech     | ≥ threshold                            | Speech         | —                       |
    /// | Speech     | < threshold                            | Speech         | → PadOpen               |
    /// | PadOpen    | ≥ threshold                            | Speech         | → Speech                |
    /// | PadOpen    | < threshold                            | Speech         | Stay; PadClosed on exp. |
    /// | PadClosed  | ≥ threshold                            | Speech         | → Speech                |
    /// | PadClosed  | < threshold, silence < min_silence_ms  | Silence        | Stay                    |
    /// | PadClosed  | < threshold, silence ≥ min_silence_ms  | EndOfUtterance | → Silence               |
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
                        // Enough silence confirmed — fully close gate and signal
                        // end-of-utterance so the caller can flush immediately.
                        tracing::debug!(
                            silence_ms = self.state_ms,
                            "VAD: silence confirmed — gate closed, end-of-utterance"
                        );
                        self.state = VadState::Silence;
                        self.state_ms = 0;
                        VadDecision::EndOfUtterance
                    } else {
                        VadDecision::Silence
                    }
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
#[path = "vad_tests.rs"]
mod tests;
