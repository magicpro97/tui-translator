//! Indirection layer between audio production code and the QA8-07
//! backpressure telemetry registry (`metrics::backpressure::emit`).
//!
//! The audio module (`fanout`, `wasapi_capture`, …) calls the no-arg
//! helpers in this file. `main.rs` installs delegates that forward to
//! `crate::metrics::backpressure::emit::*`. When no delegate has been
//! installed every helper is a single atomic load and an early return,
//! so the audio module stays decoupled from `crate::metrics`. This
//! decoupling is what lets integration tests `#[path]`-include
//! `src/audio/mod.rs` without also pulling in the metrics tree.
//!
//! See issue #505 (QA8-07) for the broader telemetry plan.

use std::sync::OnceLock;

static FANOUT_DROP: OnceLock<fn(usize)> = OnceLock::new();
static AUDIO_CHUNK_AT: OnceLock<fn(u64)> = OnceLock::new();
static AUDIO_STALL: OnceLock<fn()> = OnceLock::new();
static MONOTONIC_NOW_NS: OnceLock<fn() -> u64> = OnceLock::new();

/// Install the production fanout-drop emitter. Called once from `main`.
pub fn install_fanout_drop(f: fn(usize)) {
    let _ = FANOUT_DROP.set(f);
}

/// Install the production audio-chunk timestamp emitter.
pub fn install_audio_chunk_at(f: fn(u64)) {
    let _ = AUDIO_CHUNK_AT.set(f);
}

/// Install the production capture-stall emitter.
pub fn install_audio_stall(f: fn()) {
    let _ = AUDIO_STALL.set(f);
}

/// Install the production monotonic clock source.
pub fn install_monotonic_now_ns(f: fn() -> u64) {
    let _ = MONOTONIC_NOW_NS.set(f);
}

#[inline]
pub(crate) fn fanout_drop(slot: usize) {
    if let Some(f) = FANOUT_DROP.get() {
        f(slot);
    }
}

#[inline]
pub(crate) fn audio_chunk_at(now_ns: u64) {
    if let Some(f) = AUDIO_CHUNK_AT.get() {
        f(now_ns);
    }
}

#[inline]
pub(crate) fn audio_capture_stall() {
    if let Some(f) = AUDIO_STALL.get() {
        f();
    }
}

#[inline]
pub(crate) fn monotonic_now_ns() -> u64 {
    if let Some(f) = MONOTONIC_NOW_NS.get() {
        f()
    } else {
        0
    }
}
