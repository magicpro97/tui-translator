//! Indirection layer between provider dispatch (`with_retry`) and the
//! QA8-07 backpressure telemetry registry (`metrics::backpressure::emit`).
//!
//! Production wires these hooks at startup; tests can install their own
//! delegates to observe lifecycle counters. With no installation every
//! helper is a cheap atomic load + early return.
//!
//! See issue #505 (QA8-07).

use std::sync::OnceLock;

static ENQUEUE: OnceLock<fn()> = OnceLock::new();
static DEQUEUE_START: OnceLock<fn()> = OnceLock::new();
static COMPLETE: OnceLock<fn()> = OnceLock::new();
static RECOVERED_ERROR: OnceLock<fn()> = OnceLock::new();
static PERMANENT_ERROR: OnceLock<fn()> = OnceLock::new();

pub fn install_enqueue(f: fn()) {
    let _ = ENQUEUE.set(f);
}
pub fn install_dequeue_start(f: fn()) {
    let _ = DEQUEUE_START.set(f);
}
pub fn install_complete(f: fn()) {
    let _ = COMPLETE.set(f);
}
pub fn install_recovered_error(f: fn()) {
    let _ = RECOVERED_ERROR.set(f);
}
pub fn install_permanent_error(f: fn()) {
    let _ = PERMANENT_ERROR.set(f);
}

#[inline]
pub(crate) fn enqueue() {
    if let Some(f) = ENQUEUE.get() {
        f();
    }
}
#[inline]
pub(crate) fn dequeue_start() {
    if let Some(f) = DEQUEUE_START.get() {
        f();
    }
}
#[inline]
pub(crate) fn complete() {
    if let Some(f) = COMPLETE.get() {
        f();
    }
}
#[inline]
pub(crate) fn recovered_error() {
    if let Some(f) = RECOVERED_ERROR.get() {
        f();
    }
}
#[inline]
pub(crate) fn permanent_error() {
    if let Some(f) = PERMANENT_ERROR.get() {
        f();
    }
}
