//! PTY regression for log-routing fix — issues #183 and #184.
//!
//! Verifies that when `tui-translator` is launched with logging enabled,
//! `tracing-subscriber` output is routed to the log file and does **not**
//! appear in the PTY terminal stream.
//!
//! ## Background
//!
//! Prior to issue #183, `tracing_subscriber::fmt()` used its default writer
//! (stderr).  On a ConPTY slave, stderr shares the same file descriptor as
//! stdout, so structured log lines were injected directly into the byte stream
//! that `vt100::Parser` was trying to interpret as ratatui escape sequences.
//! The previous workaround was to force `RUST_LOG=off` in every PTY test so
//! that `tracing` emitted nothing at all.
//!
//! The fix (issue #183) routes tracing output to a file via
//! `with_writer(Mutex::new(file))`, keeping the ConPTY byte stream clean even
//! when logging is enabled.
//!
//! ## What this test proves
//!
//! - The child process is spawned with `RUST_LOG=tui_translator=info` so that
//!   at least one `INFO` line (`"tui-translator starting"`) is emitted at
//!   startup.
//! - After the TUI frame appears, none of the known log-output patterns are
//!   present in the accumulated raw PTY bytes.
//! - If the routing fix is reverted the test fails because log lines like
//!   `"2026-... INFO tui_translator: tui-translator starting"` appear in the
//!   raw byte stream before ratatui can clear the screen.

use super::harness::{PtySession, EXIT_TIMEOUT};

/// Substrings that would appear in the PTY byte stream if the `tracing`
/// output leaked into it.  These match the default `tracing-subscriber` fmt
/// format:
/// ```text
/// 2026-05-13T12:00:00.000000Z  INFO tui_translator: tui-translator starting
/// ```
const LOG_LEAK_PATTERNS: &[&str] = &[
    " INFO ",
    " WARN ",
    " ERROR ",
    " DEBUG ",
    "tui_translator:",
    "tui-translator starting",
];

#[test]
fn log_output_not_in_pty_stream() {
    // Spawn with logging explicitly enabled — unlike all other PTY tests,
    // this one does NOT suppress tracing so the fix is genuinely exercised.
    let mut session = PtySession::spawn(80, 24, &[("RUST_LOG", "tui_translator=info")])
        .expect("failed to spawn tui-translator for log_output_not_in_pty_stream");

    // Wait for the process to emit at least its first batch of PTY bytes.
    // We do NOT require a complete TUI frame here — we only need enough time
    // for the startup log line (`tracing::info!("tui-translator starting")`)
    // to have been written.  That call happens synchronously inside main()
    // before any PTY output is generated, so any PTY output arriving here
    // implies the log line has already been dispatched to the writer.
    let start = std::time::Instant::now();
    let wait_limit = std::time::Duration::from_secs(5);
    while start.elapsed() < wait_limit && session.bytes_received() == 0 {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    // One extra beat for the log writer to flush.
    std::thread::sleep(std::time::Duration::from_millis(500));

    assert!(
        session.bytes_received() > 0,
        "log_output_not_in_pty_stream: no PTY bytes received in {:.0}s; \
         child process may have failed to start",
        wait_limit.as_secs_f32(),
    );

    // Capture the full raw PTY byte stream accumulated since spawn.
    let raw = session.raw_bytes_captured();
    let raw_text = String::from_utf8_lossy(&raw);

    // None of the log-leak patterns must appear in the raw PTY output.
    //
    // Before the fix (issue #183), tracing-subscriber wrote to stderr which
    // on a ConPTY slave shares fd 1, so the log line would appear verbatim in
    // this buffer.  After the fix it goes to a log file.
    for pattern in LOG_LEAK_PATTERNS {
        assert!(
            !raw_text.contains(pattern),
            "log output leaked into PTY stream — found {:?} in raw bytes.\n\
             This means tracing-subscriber is writing to stderr/stdout instead \
             of the log file.  Check init_tracing() in src/main.rs.\n\
             Raw PTY snippet (first 512 chars):\n{:?}",
            pattern,
            &raw_text[..raw_text.len().min(512)],
        );
    }

    // Clean up — best effort; the child may have already exited.
    let _ = session.quit_cleanly();
    session.wait_exit(EXIT_TIMEOUT);
}
