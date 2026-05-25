//! Deterministic simulation harness for TEST-01 (issue #460).
//!
//! This module is the L2–L4 successor to the L1 `WavFileSource` replayer
//! shipped in wave 1. It is intentionally test-only (lives under `tests/`)
//! so it can be `#[path]`-included by any integration test that needs a
//! reproducible, hardware-free simulation of the live-meeting pipeline:
//!
//! * [`clock::FakeClock`] — virtual monotonic clock with explicit `advance`
//!   and `sleep` semantics. Tests can build minute-long scenarios that run
//!   in microseconds of wall time.
//! * [`feeder::ScriptedAudioFeeder`] — emits a scripted sequence of
//!   `PcmChunk`s. Supports silence, sine-tone, and arbitrary sample
//!   vectors so audio fixtures can be assembled in-memory.
//! * [`fakes::FakeSttProvider`] / [`fakes::FakeMtProvider`] /
//!   [`fakes::FakeTtsProvider`] — implement the production provider
//!   traits with configurable per-call latency and error injection
//!   (transient 429/503 for retry/backoff coverage; permanent
//!   auth/invalid-input errors for fast-fail coverage).
//! * [`recorder::FrameRecorder`] — in-memory `vt100`-backed recorder
//!   that accepts raw PTY byte streams or `ratatui::buffer::Buffer`
//!   snapshots and stores stable per-frame screen strings. Tests use
//!   this for golden-frame assertions without spawning a real OS PTY.
//! * [`evidence::EvidenceBuilder`] — builds a JSON document that
//!   conforms to `verification-evidence/test/TEST-01-evidence-schema.json`
//!   so every harness level emits the same machine-checkable artifact.
//!
//! The API surface is intentionally small and reusable. Successor
//! issues (#474, #507, #503) are expected to consume this harness
//! without modifying it.
//!
//! ## Determinism guarantees
//!
//! * No wall-clock reads leak into harness state. Virtual time is owned
//!   by [`clock::FakeClock`] and advanced explicitly.
//! * No background threads, no `tokio::time::sleep`, no real network or
//!   audio I/O. Errors and latency are deterministic functions of the
//!   call index plus the script the test installed.
//! * No `unwrap`/`expect` outside test bodies; harness public APIs
//!   return `Result` where they can fail and panic with explicit
//!   messages only when invariants the caller controls are violated.

#![allow(dead_code)]

pub mod clock;
pub mod evidence;
pub mod fakes;
pub mod feeder;
pub mod recorder;
