//! Crash and panic diagnostics for tui-translator (QA8-08, issue #506).
//!
//! This module owns the runtime side of the crash-evidence pipeline. The
//! kernel-level minidump is produced by Windows Error Reporting (WER)
//! LocalDumps — see `docs/13-crash-dump-symbolication.md` for the registry
//! configuration and symbolication workflow. The Rust code here adds two
//! complementary artefacts that WER cannot produce on its own:
//!
//! 1. A *panic sidecar* (`panic-<timestamp>-<pid>.json`) with a normalised
//!    schema so the `crash-root-cause` agent and downstream tooling can
//!    ingest panic evidence the same way they ingest minidump metadata.
//! 2. A plain-text `panic-log.txt` rolling append log so a human running the
//!    binary outside WER (e.g. during a soak) can still see panics.
//!
//! Both artefacts are written to a configurable dump directory. See
//! [`resolve_dump_dir`] for the override precedence.
//!
//! The panic hook is best-effort: any I/O failure is swallowed after a
//! `tracing::error!` because re-panicking inside a panic hook would abort
//! the process without surfacing the original cause.
//!
//! No telemetry is uploaded. Secrets discovered in the panic payload are
//! scrubbed before being written to disk so neither the sidecar nor the
//! plain-text log can leak a Google API key into a captured artefact.

pub mod panic;

pub use panic::{install_panic_hook, resolve_dump_dir};
