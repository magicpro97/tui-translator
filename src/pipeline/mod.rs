//! Translation pipeline orchestrator stub.
//!
//! The pipeline ties together:
//!   1. Audio capture (`audio` module)
//!   2. Speech-to-text (`providers::SttProvider`)
//!   3. Translation (`providers::TranslationProvider`)
//!   4. Optional text-to-speech (`providers::TtsProvider`)
//!   5. Display (`tui` module)
//!
//! Phase 1–4 will implement the real pipeline.  The types here let the
//! compiler verify module wiring before the logic exists.

// Stub types used in Phase 1–4.
#![allow(dead_code)]

pub mod playback;

/// Runtime state of the pipeline.  Shown in the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineState {
    /// Waiting to receive audio.
    Idle,
    /// Actively capturing and translating.
    Running,
    /// User paused translation with Space.
    Paused,
    /// A non-fatal error occurred; retrying.
    Retrying { attempt: u8 },
    /// A fatal error stopped the pipeline.
    Error(String),
}

impl std::fmt::Display for PipelineState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Running => write!(f, "running"),
            Self::Paused => write!(f, "paused"),
            Self::Retrying { attempt } => write!(f, "retrying ({attempt}/5)"),
            Self::Error(msg) => write!(f, "error: {msg}"),
        }
    }
}
