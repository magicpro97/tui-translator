//! Layer-3 terminal behaviour tests driven by a PTY harness (issues #104–#107).
//!
//! Run with:
//!   cargo test --test pty -- --nocapture
//!
//! These tests spawn the compiled `tui-translator` binary inside a Windows
//! ConPTY (via `portable-pty`), parse PTY output through a `vt100` terminal
//! emulator, and assert on screen layout, clean exit, and graceful degradation
//! in monochrome environments.
//!
//! **Environment notes:**
//! - Each test starts a fresh process; WASAPI audio initialisation may take up
//!   to 5 s before the TUI appears, hence the generous `STARTUP_TIMEOUT`.
//! - `RUST_LOG=off` is always injected so `tracing` log lines do not pollute
//!   the raw PTY byte stream.
//! - Tests work whether or not a real audio render device is present: the TUI
//!   renders its full layout even when audio capture returns an error state.

#[path = "pty/harness.rs"]
mod harness;

#[path = "pty/layout_test.rs"]
mod layout_test;

#[path = "pty/exit_test.rs"]
mod exit_test;

#[path = "pty/monochrome_test.rs"]
mod monochrome_test;
