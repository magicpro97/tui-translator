//! HC-03: Session recorder supervisor stub.
//!
//! Session recorder change classification has been moved to
//! [`crate::config::recorder_supervisor`] to keep the `session` module free
//! of `config` imports and to match the HC-02 pattern used by
//! [`crate::config::provider_supervisor`].
//!
//! This module intentionally contains no re-exports: several standalone test
//! binaries include `session` without `config`, so re-exporting config types
//! here would break those crates.
