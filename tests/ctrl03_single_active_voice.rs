//! CTRL-03 — Single active voice invariant across single and dual modes
//! (issue #456).
//!
//! These tests are the property-style witness that the configured TTS gating
//! never permits more than one concurrent voice across **any** legal
//! combination of `(SlotMode, TtsSource, tts_enabled)`.
//!
//! Scope:
//!   * Cartesian-product invariant: `active_slot_count(is_dual) <= 1`.
//!   * Mutual exclusion between slot A and slot B in dual mode (the two
//!     `is_active_for_slot` calls cannot both return `true`).
//!   * Single-slot mode always has exactly one synthesising slot (slot B is
//!     never constructed; see `pipeline::SlotId` docs).
//!   * Effective synthesis count (`tts_enabled AND tts_active_for_slot`)
//!     remains `<= 1` for every combination, including `tts_enabled = false`.
//!   * `tts_source` mutations are classified `requires_restart = true`, so
//!     the "active slot" identity is captured once at orchestrator
//!     construction and cannot change mid-utterance — voice swaps therefore
//!     take effect on the next utterance, not the current one (the rest of
//!     the swap-on-next-utterance behaviour belongs to CTRL-02 / #455).
//!
//! Out of scope:
//!   * Voice catalog and hot-swap UX (issue #455).
//!   * Per-provider voice metadata (issue #494).
//!   * TTS provider implementation choices (issue #493).
//!
//! Note on the playback owner: the process holds a single
//! `Arc<Mutex<Option<PlaybackService>>>` shared between both orchestrator
//! slots (see `SharedPlaybackService` in `src/main.rs`).  Even if both slots
//! mistakenly attempted to synthesise, they would serialise through that
//! mutex and the active backend would mix into one output device.  The
//! gating tested here prevents that mistake from occurring in the first
//! place.
//!
//! Run with:
//!   cargo test --test ctrl03_single_active_voice

#[path = "../src/audio/mod.rs"]
mod audio;

#[path = "../src/config/mod.rs"]
mod config;

use config::{AppConfig, TtsSource};

/// Every `TtsSource` variant, used to drive Cartesian-product loops.
const ALL_SOURCES: &[TtsSource] = &[TtsSource::Off, TtsSource::A, TtsSource::B];

// ─── Invariant 1: active_slot_count is always <= 1 ───────────────────────────

#[test]
fn active_slot_count_never_exceeds_one() {
    for &src in ALL_SOURCES {
        for &is_dual in &[false, true] {
            let count = src.active_slot_count(is_dual);
            assert!(
                count <= 1,
                "CTRL-03 violated: src={src:?} is_dual={is_dual} \
                 produced active_slot_count={count}, expected <= 1"
            );
        }
    }
}

#[test]
fn active_slot_count_dual_off_is_zero() {
    assert_eq!(TtsSource::Off.active_slot_count(true), 0);
}

#[test]
fn active_slot_count_dual_a_or_b_is_one() {
    assert_eq!(TtsSource::A.active_slot_count(true), 1);
    assert_eq!(TtsSource::B.active_slot_count(true), 1);
}

#[test]
fn active_slot_count_single_mode_is_one_for_every_source() {
    for &src in ALL_SOURCES {
        assert_eq!(
            src.active_slot_count(false),
            1,
            "single-slot mode must always have exactly one synthesising slot (src={src:?})",
        );
    }
}

// ─── Invariant 2: per-slot gates agree with the typed count ──────────────────

/// The two `is_active_for_slot` calls in dual mode are mutually exclusive,
/// and their sum exactly equals `active_slot_count(true)`.  This is the
/// cross-check between the boolean gate consumed by the orchestrator
/// (`OrchestratorContext::tts_active_for_slot`) and the typed count used by
/// invariant tests.
#[test]
fn dual_mode_slot_gates_are_mutually_exclusive() {
    for &src in ALL_SOURCES {
        let slot_a_active = src.is_active_for_slot(true, true);
        let slot_b_active = src.is_active_for_slot(false, true);
        assert!(
            !(slot_a_active && slot_b_active),
            "CTRL-03 violated: src={src:?} marked BOTH slot A and slot B active in dual mode",
        );
        let counted = u8::from(slot_a_active) + u8::from(slot_b_active);
        assert_eq!(
            counted,
            src.active_slot_count(true),
            "is_active_for_slot disagrees with active_slot_count for src={src:?}",
        );
    }
}

#[test]
fn single_mode_always_reports_slot_active() {
    for &src in ALL_SOURCES {
        assert!(
            src.is_active_for_slot(true, false),
            "single-slot mode (slot A only) must be active regardless of tts_source (src={src:?})",
        );
    }
}

// ─── Invariant 3: effective synthesis = tts_enabled AND tts_active_for_slot ──

/// The orchestrator gates TTS provider calls with
/// `tts_enabled AND tts_active_for_slot` (see `src/pipeline/mod.rs`).  This
/// test reproduces the boolean expression at the test layer and proves that
/// the number of slots whose gate evaluates to `true` is always `<= 1`,
/// across the full Cartesian product including `tts_enabled = false`.
#[test]
fn effective_synthesising_slot_count_is_at_most_one() {
    for &src in ALL_SOURCES {
        for &is_dual in &[false, true] {
            for &tts_enabled in &[false, true] {
                let slot_a_gate = tts_enabled && src.is_active_for_slot(true, is_dual);
                let slot_b_gate = if is_dual {
                    tts_enabled && src.is_active_for_slot(false, is_dual)
                } else {
                    // Slot B is never constructed in single-slot mode (see
                    // `pipeline::SlotId` docs), so its effective gate is
                    // structurally false.
                    false
                };
                let active = u8::from(slot_a_gate) + u8::from(slot_b_gate);
                assert!(
                    active <= 1,
                    "CTRL-03 violated: src={src:?} is_dual={is_dual} \
                     tts_enabled={tts_enabled} produced {active} active slots",
                );
            }
        }
    }
}

// ─── Invariant 4: tts_source changes are restart-classified ──────────────────

/// The active-voice identity is captured once when the orchestrator starts
/// (`tts_active_for_slot` is a plain `bool` populated in `main.rs` and never
/// mutated thereafter).  For that capture to be safe, `tts_source` mutations
/// must force a restart so the captured value can be recomputed.
///
/// This test pins the classification so a future live-reload change cannot
/// silently break the invariant.
#[test]
fn tts_source_mutation_requires_restart() {
    let mut cfg_off = AppConfig::default();
    cfg_off.tts_source = TtsSource::Off;

    let mut cfg_a = AppConfig::default();
    cfg_a.tts_source = TtsSource::A;

    let mut cfg_b = AppConfig::default();
    cfg_b.tts_source = TtsSource::B;

    for (from, to) in [
        (&cfg_off, &cfg_a),
        (&cfg_off, &cfg_b),
        (&cfg_a, &cfg_b),
        (&cfg_b, &cfg_a),
        (&cfg_a, &cfg_off),
        (&cfg_b, &cfg_off),
    ] {
        assert!(
            from.requires_restart(to),
            "CTRL-03 violated: changing tts_source from {:?} to {:?} must require restart \
             so the captured active-slot identity cannot drift mid-session",
            from.tts_source,
            to.tts_source,
        );
    }
}
