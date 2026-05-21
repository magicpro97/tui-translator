//! HC-03: Session recorder supervisor stub.
//!
//! Session recorder change classification has been moved to
//! [`crate::config::recorder_supervisor`] to keep the `session` module free
//! of `config` imports and to match the HC-02 pattern used by
//! [`crate::config::provider_supervisor`].
//!
//! Re-export the types here for convenience when only `session` is in scope.
