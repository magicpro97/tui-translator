//! Scripted audio feeder for the deterministic simulation harness.
//!
//! The feeder produces a sequence of [`PcmChunk`]s (the type accepted by
//! [`SttProvider::transcribe`](crate::providers::SttProvider::transcribe))
//! from in-memory specifications. It never touches WASAPI, never reads
//! from disk, and always returns chunks in a deterministic order with
//! monotonically increasing sequence numbers.
//!
//! For tests that need the existing on-disk fixture coverage (the L1
//! [`WavFileSource`](crate::audio::WavFileSource) replayer), see
//! `tests/test_01_file_source_replay.rs`. The feeder here covers L2+
//! scenarios where the source content is composed inside the test.

use std::collections::VecDeque;
use std::f32::consts::TAU;

use crate::providers::PcmChunk;

/// Canonical sample rate the entire pipeline operates at.
pub const SAMPLE_RATE_HZ: u32 = 16_000;

/// One unit of scripted audio to emit.
///
/// Variants are intentionally minimal — anything more elaborate can be
/// built up from raw `Samples`.
#[derive(Debug, Clone)]
pub enum AudioScript {
    /// `samples` consecutive zero-valued samples.
    Silence { samples: usize },
    /// A pure sine tone at `frequency_hz` for `samples` samples,
    /// quantised to i16 at `amplitude` (full-scale = 1.0).
    Tone {
        frequency_hz: f32,
        amplitude: f32,
        samples: usize,
    },
    /// Caller-supplied PCM samples. The feeder takes ownership.
    Samples(Vec<i16>),
}

/// Emits scripted PCM chunks one at a time.
///
/// Each call to [`ScriptedAudioFeeder::next_chunk`] pops one
/// [`AudioScript`] entry and returns the corresponding [`PcmChunk`]
/// with the next sequence number. Returns [`None`] when the script is
/// exhausted.
#[derive(Debug, Default)]
pub struct ScriptedAudioFeeder {
    script: VecDeque<AudioScript>,
    next_seq: u64,
}

impl ScriptedAudioFeeder {
    /// Construct a feeder from an ordered script.
    pub fn new<I: IntoIterator<Item = AudioScript>>(script: I) -> Self {
        Self {
            script: script.into_iter().collect(),
            next_seq: 0,
        }
    }

    /// Number of script entries still pending.
    pub fn remaining(&self) -> usize {
        self.script.len()
    }

    /// Sequence number that will be assigned to the next emitted chunk.
    pub fn next_sequence_number(&self) -> u64 {
        self.next_seq
    }

    /// Push an additional entry onto the end of the script.
    pub fn enqueue(&mut self, item: AudioScript) {
        self.script.push_back(item);
    }

    /// Pop the next entry and return it as a [`PcmChunk`].
    ///
    /// Returns `None` once the script is exhausted; callers can use this
    /// as a terminating condition for a driver loop.
    pub fn next_chunk(&mut self) -> Option<PcmChunk> {
        let item = self.script.pop_front()?;
        let samples = realise(item);
        let chunk = PcmChunk {
            samples,
            sequence_number: self.next_seq,
        };
        self.next_seq += 1;
        Some(chunk)
    }
}

fn realise(item: AudioScript) -> Vec<i16> {
    match item {
        AudioScript::Silence { samples } => vec![0i16; samples],
        AudioScript::Tone {
            frequency_hz,
            amplitude,
            samples,
        } => synth_tone(frequency_hz, amplitude, samples),
        AudioScript::Samples(buf) => buf,
    }
}

fn synth_tone(freq_hz: f32, amplitude: f32, samples: usize) -> Vec<i16> {
    let clamped = amplitude.clamp(0.0, 1.0);
    let peak = (clamped * i16::MAX as f32).round() as i16;
    let step = TAU * freq_hz / SAMPLE_RATE_HZ as f32;
    (0..samples)
        .map(|n| {
            let v = (n as f32 * step).sin();
            // Round, then clamp to i16 range so amplitude == 1.0 still
            // saturates correctly without overflow.
            (v * peak as f32)
                .round()
                .clamp(i16::MIN as f32, i16::MAX as f32) as i16
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_script_yields_none() {
        let mut feeder = ScriptedAudioFeeder::new(std::iter::empty());
        assert!(feeder.next_chunk().is_none());
    }

    #[test]
    fn silence_chunk_has_correct_length_and_value() {
        let mut feeder = ScriptedAudioFeeder::new([AudioScript::Silence { samples: 320 }]);
        let chunk = feeder.next_chunk().expect("one chunk");
        assert_eq!(chunk.sequence_number, 0);
        assert_eq!(chunk.samples.len(), 320);
        assert!(chunk.samples.iter().all(|&s| s == 0));
        assert!(feeder.next_chunk().is_none());
    }

    #[test]
    fn sequence_numbers_increase_monotonically() {
        let mut feeder = ScriptedAudioFeeder::new([
            AudioScript::Silence { samples: 16 },
            AudioScript::Silence { samples: 16 },
            AudioScript::Silence { samples: 16 },
        ]);
        let seqs: Vec<u64> = std::iter::from_fn(|| feeder.next_chunk())
            .map(|c| c.sequence_number)
            .collect();
        assert_eq!(seqs, vec![0, 1, 2]);
    }

    #[test]
    fn tone_is_deterministic_across_invocations() {
        let make = || {
            let mut feeder = ScriptedAudioFeeder::new([AudioScript::Tone {
                frequency_hz: 440.0,
                amplitude: 0.5,
                samples: 64,
            }]);
            feeder.next_chunk().expect("one chunk").samples
        };
        assert_eq!(make(), make());
    }

    #[test]
    fn raw_samples_pass_through_unchanged() {
        let payload: Vec<i16> = (0..8).map(|i| i as i16 * 100).collect();
        let mut feeder = ScriptedAudioFeeder::new([AudioScript::Samples(payload.clone())]);
        let chunk = feeder.next_chunk().expect("one chunk");
        assert_eq!(chunk.samples, payload);
    }
}
