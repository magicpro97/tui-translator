//! Real-time input gain and output volume controls (CTRL-01, issue #454).
//!
//! Provides two process-wide gain stages controlled by hotkeys and persisted
//! to `config.json`:
//!
//! * **Input gain** — a software multiplier applied to captured PCM samples
//!   on the WASAPI capture path, after resampling to 16 kHz mono and before
//!   `f32 → i16` quantisation.  Range: `INPUT_GAIN_MIN_DB`..`INPUT_GAIN_MAX_DB`
//!   in 1 dB steps.
//! * **Output volume** — a multiplier applied to the rodio TTS playback sink
//!   via `rodio::Sink::set_volume`.  Range: `OUTPUT_VOLUME_MIN_DB`..
//!   `OUTPUT_VOLUME_MAX_DB` in 1 dB steps.
//!
//! Values are clamped on every write; persistent storage uses dB (linear and
//! human-friendly).  Internal reads convert to a linear factor cached in an
//! `AtomicU32` (storing `f32::to_bits`) so the audio capture thread does not
//! take a lock.
//!
//! Hot-key changes write to the atomic immediately and are observed by the
//! capture loop on the next FRAMES_PER_CHUNK (30 ms at 16 kHz / 480 samples)
//! — well under the 50 ms latency budget required by AC.
//!
//! A short per-block linear ramp between the previous and current gain is
//! applied by [`InputGainRamp`] to avoid zipper noise on step changes.

use std::sync::atomic::{AtomicU32, Ordering};

/// Lower clamp for input gain, in dB.  Below this the result is effectively
/// silence and STT will time out, so we refuse it.
pub const INPUT_GAIN_MIN_DB: f32 = -24.0;
/// Upper clamp for input gain, in dB.  +24 dB lets users recover quiet talkers
/// without saturating headroom past the i16 limiter at `start_sink`.
pub const INPUT_GAIN_MAX_DB: f32 = 24.0;
/// Lower clamp for output volume, in dB.  -60 dB is the rodio mute floor.
pub const OUTPUT_VOLUME_MIN_DB: f32 = -60.0;
/// Upper clamp for output volume, in dB.  Capped at +6 dB to keep rodio's
/// software gain inside its safe operating range.
pub const OUTPUT_VOLUME_MAX_DB: f32 = 6.0;
/// Default dB step for `[`/`]` and `{`/`}` hotkeys.
pub const DB_STEP: f32 = 1.0;

/// Convert a dB value to a linear amplitude factor.
///
/// `db_to_linear(0.0) == 1.0`; `db_to_linear(-6.0) ≈ 0.5012`.
#[inline]
pub fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Clamp a dB value to `[min_db, max_db]` after rejecting NaN.
///
/// NaN inputs return `0.0` (unity gain) so a corrupt config never silences
/// the user or drives the limiter into clipping.
pub fn clamp_db(db: f32, min_db: f32, max_db: f32) -> f32 {
    if db.is_nan() {
        return 0.0;
    }
    db.clamp(min_db, max_db)
}

// ── Process-wide controllers ─────────────────────────────────────────────────

static INPUT_GAIN_DB_BITS: AtomicU32 = AtomicU32::new(0u32); // f32::to_bits(0.0)
static INPUT_GAIN_LINEAR_BITS: AtomicU32 = AtomicU32::new(0x3f80_0000); // f32::to_bits(1.0)

static OUTPUT_VOLUME_DB_BITS: AtomicU32 = AtomicU32::new(0u32);
static OUTPUT_VOLUME_LINEAR_BITS: AtomicU32 = AtomicU32::new(0x3f80_0000);

/// Current input gain, in dB.  Reads the last value written by
/// [`set_input_gain_db`].
pub fn input_gain_db() -> f32 {
    f32::from_bits(INPUT_GAIN_DB_BITS.load(Ordering::Relaxed))
}

/// Current input-gain linear amplitude factor — equal to
/// `db_to_linear(input_gain_db())` at the time of the last write.
pub fn input_gain_linear() -> f32 {
    f32::from_bits(INPUT_GAIN_LINEAR_BITS.load(Ordering::Relaxed))
}

/// Clamp and store a new input gain value, in dB.  Returns the clamped value
/// actually stored (so callers can mirror it into `AppConfig`).
pub fn set_input_gain_db(db: f32) -> f32 {
    let clamped = clamp_db(db, INPUT_GAIN_MIN_DB, INPUT_GAIN_MAX_DB);
    INPUT_GAIN_DB_BITS.store(clamped.to_bits(), Ordering::Relaxed);
    INPUT_GAIN_LINEAR_BITS.store(db_to_linear(clamped).to_bits(), Ordering::Relaxed);
    clamped
}

/// Current output volume, in dB.
pub fn output_volume_db() -> f32 {
    f32::from_bits(OUTPUT_VOLUME_DB_BITS.load(Ordering::Relaxed))
}

/// Current output-volume linear amplitude factor.
pub fn output_volume_linear() -> f32 {
    f32::from_bits(OUTPUT_VOLUME_LINEAR_BITS.load(Ordering::Relaxed))
}

/// Clamp and store a new output volume value, in dB.  Returns the clamped
/// value actually stored.
pub fn set_output_volume_db(db: f32) -> f32 {
    let clamped = clamp_db(db, OUTPUT_VOLUME_MIN_DB, OUTPUT_VOLUME_MAX_DB);
    OUTPUT_VOLUME_DB_BITS.store(clamped.to_bits(), Ordering::Relaxed);
    OUTPUT_VOLUME_LINEAR_BITS.store(db_to_linear(clamped).to_bits(), Ordering::Relaxed);
    clamped
}

/// Reset both controllers to 0 dB (unity).
pub fn reset_to_unity() {
    set_input_gain_db(0.0);
    set_output_volume_db(0.0);
}

// ── Smoothed application ─────────────────────────────────────────────────────

/// Per-thread ramp state for [`InputGainRamp::apply_in_place`].  Keeps the last
/// applied linear gain so step changes interpolate across one chunk instead
/// of producing a click.
#[derive(Debug, Clone, Copy)]
pub struct InputGainRamp {
    last_linear: f32,
}

impl Default for InputGainRamp {
    fn default() -> Self {
        Self { last_linear: 1.0 }
    }
}

impl InputGainRamp {
    /// Create a fresh ramp seeded at unity gain.
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply the current input gain to `samples` (in -1..=1 f32 PCM) in place.
    ///
    /// The gain is read once per call from the shared controller, then linearly
    /// interpolated between `self.last_linear` and the new target across the
    /// block.  This keeps gain steps from introducing zipper noise.
    ///
    /// Samples are clamped to `[-1.0, 1.0]` after gain to avoid feeding the
    /// `f32 → i16` quantiser values that would wrap.
    pub fn apply_in_place(&mut self, samples: &mut [f32]) {
        let target = input_gain_linear();
        if samples.is_empty() {
            self.last_linear = target;
            return;
        }
        let start = self.last_linear;
        if (start - target).abs() < f32::EPSILON {
            if (target - 1.0).abs() < f32::EPSILON {
                // Fast path: unity gain, nothing to do.
                self.last_linear = target;
                return;
            }
            for s in samples.iter_mut() {
                *s = (*s * target).clamp(-1.0, 1.0);
            }
            self.last_linear = target;
            return;
        }
        let n = samples.len() as f32;
        let inv_n = 1.0 / n;
        for (i, s) in samples.iter_mut().enumerate() {
            let t = (i as f32) * inv_n;
            let g = start + (target - start) * t;
            *s = (*s * g).clamp(-1.0, 1.0);
        }
        self.last_linear = target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialise every test in this module: the controllers are process-wide
    /// atomics, so when cargo runs the tests in parallel (default), one test
    /// can race the gain set by another and observe a perturbed value.  A
    /// single mutex around every test body restores determinism without
    /// needing the `serial_test` dependency.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Take the test mutex; on a poisoned lock (a prior panic in another test)
    /// recover the inner guard so the current test can still run.
    fn lock_tests() -> std::sync::MutexGuard<'static, ()> {
        match TEST_LOCK.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    /// Each test resets to a known state to avoid order-dependency on the
    /// process-wide controllers.
    fn reset() {
        set_input_gain_db(0.0);
        set_output_volume_db(0.0);
    }

    #[test]
    fn clamp_db_rejects_nan_and_clamps() {
        let _g = lock_tests();
        assert_eq!(clamp_db(f32::NAN, -24.0, 24.0), 0.0);
        assert_eq!(clamp_db(-100.0, -24.0, 24.0), -24.0);
        assert_eq!(clamp_db(100.0, -24.0, 24.0), 24.0);
        assert_eq!(clamp_db(5.0, -24.0, 24.0), 5.0);
    }

    #[test]
    fn db_to_linear_known_values() {
        let _g = lock_tests();
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
        // -6.0206 dB is exactly 0.5; 1 dB tolerance gives wider band.
        assert!((db_to_linear(-6.0206) - 0.5).abs() < 1e-3);
        assert!((db_to_linear(6.0206) - 2.0).abs() < 1e-3);
    }

    #[test]
    fn set_input_gain_clamps_to_limits() {
        let _g = lock_tests();
        reset();
        assert_eq!(set_input_gain_db(1000.0), INPUT_GAIN_MAX_DB);
        assert_eq!(input_gain_db(), INPUT_GAIN_MAX_DB);
        assert_eq!(set_input_gain_db(-1000.0), INPUT_GAIN_MIN_DB);
        assert_eq!(set_input_gain_db(f32::NAN), 0.0);
        assert_eq!(input_gain_db(), 0.0);
        assert!((input_gain_linear() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn set_output_volume_clamps_to_limits() {
        let _g = lock_tests();
        reset();
        assert_eq!(set_output_volume_db(1000.0), OUTPUT_VOLUME_MAX_DB);
        assert_eq!(set_output_volume_db(-1000.0), OUTPUT_VOLUME_MIN_DB);
        // 0 dB linear == 1.0.
        set_output_volume_db(0.0);
        assert!((output_volume_linear() - 1.0).abs() < 1e-6);
    }

    /// AC: "0.5 gain produces RMS approximately 0.5x input RMS within ±2%".
    ///
    /// We feed a steady-state sine and measure RMS before/after the ramp
    /// settles to a steady -6.0206 dB linear gain.
    #[test]
    fn half_gain_halves_rms_within_two_percent() {
        let _g = lock_tests();
        reset();
        // Generate one-second of 1 kHz at 16 kHz, amplitude 0.5 to leave
        // headroom for clamp.
        let n = 16_000usize;
        let input: Vec<f32> = (0..n)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 1_000.0 * i as f32 / 16_000.0).sin())
            .collect();

        let input_rms = rms(&input);

        // Set gain to exactly 0.5 linear (= -6.0206 dB).  set_input_gain_db
        // expects dB so use the known constant.
        set_input_gain_db(-6.0206);
        let mut ramp = InputGainRamp::new();
        // Seed ramp at the new target so we measure steady-state RMS — the
        // smoothing ramp is exercised separately below.
        ramp.last_linear = input_gain_linear();
        let mut buffer = input.clone();
        ramp.apply_in_place(&mut buffer);

        let output_rms = rms(&buffer);
        let ratio = output_rms / input_rms;
        assert!(
            (ratio - 0.5).abs() < 0.01,
            "expected ~0.5 RMS ratio, got {ratio}"
        );
        reset();
    }

    /// Smoothing ramp must not clip and must transition monotonically when
    /// raising the gain from unity to 2x.
    #[test]
    fn ramp_interpolates_without_clipping() {
        let _g = lock_tests();
        reset();
        set_input_gain_db(6.0206); // ~2x
        let mut ramp = InputGainRamp { last_linear: 1.0 };
        let mut block = vec![0.1f32; 480];
        ramp.apply_in_place(&mut block);
        // First sample is gained by ~1.0, last sample by ~2.0; they must
        // increase monotonically and remain within ±1.
        assert!(block[0] <= block[block.len() - 1]);
        assert!(block.iter().all(|s| s.abs() <= 1.0));
        // Steady-state linear is now ~2.0.
        assert!((ramp.last_linear - 2.0).abs() < 0.01);
        reset();
    }

    #[test]
    fn clamp_prevents_post_gain_overflow() {
        let _g = lock_tests();
        reset();
        set_input_gain_db(INPUT_GAIN_MAX_DB);
        let mut ramp = InputGainRamp::new();
        ramp.last_linear = input_gain_linear();
        let mut block = vec![0.9f32; 256];
        ramp.apply_in_place(&mut block);
        assert!(block.iter().all(|s| (-1.0..=1.0).contains(s)));
        reset();
    }

    #[test]
    fn reset_to_unity_restores_zero_db() {
        let _g = lock_tests();
        set_input_gain_db(10.0);
        set_output_volume_db(-12.0);
        reset_to_unity();
        assert_eq!(input_gain_db(), 0.0);
        assert_eq!(output_volume_db(), 0.0);
    }

    fn rms(samples: &[f32]) -> f32 {
        let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
        (sum_sq / samples.len() as f32).sqrt()
    }
}
