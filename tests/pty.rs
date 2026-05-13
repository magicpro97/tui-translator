//! Layer-3 terminal behaviour tests driven by a PTY harness (issues #104–#108).
//!
//! Run with:
//!   cargo test --test pty -- --nocapture
//!
//! These tests spawn the compiled `tui-translator` binary inside a Windows
//! ConPTY (via `portable-pty`), parse PTY output through a `vt100` terminal
//! emulator, and assert on screen layout, clean exit, and graceful degradation
//! in monochrome environments.
//!
//! **CI job (issue #108):** `.github/workflows/ci.yml` contains a dedicated
//! `pty-test` job that runs this binary on a `windows-latest` GitHub-hosted
//! runner on every push and pull request.  The job is separate from the
//! general `test` job so PTY behaviour is always exercised explicitly in CI.
//!
//! **Environment notes:**
//! - Each test starts a fresh process; WASAPI audio initialisation may take up
//!   to 5 s before the TUI appears, hence the generous `STARTUP_TIMEOUT`.
//! - Since issue #183, tracing output is routed to a log file in the OS temp
//!   directory (`tui-translator.log`), so log lines never pollute the raw PTY
//!   byte stream.  `RUST_LOG=off` is no longer injected by the harness.
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

#[path = "pty/log_routing_test.rs"]
mod log_routing_test;
