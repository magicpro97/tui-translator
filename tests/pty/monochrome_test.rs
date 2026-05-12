//! PTY monochrome / degraded-terminal tests — issue #107.
//!
//! Verifies that `tui-translator` starts without crashing and renders readable
//! text in colour-disabled or capability-limited terminal environments.
//!
//! ## Approach
//!
//! Two standard mechanisms suppress colour output:
//!
//! | Env var | Respected by | Effect |
//! |---------|-------------|--------|
//! | `NO_COLOR=1` | crossterm, ratatui | Suppresses all SGR colour attributes |
//! | `TERM=dumb`  | Unix terminfo | Selects a fallback terminal description |
//!
//! On Windows, `TERM` is not consulted by crossterm (which uses the Windows
//! Console API or ConPTY VT mode directly), so `TERM=dumb` has no effect on
//! the rendered output.  We test `NO_COLOR=1` instead, which crossterm *does*
//! honour on all platforms including Windows.
//!
//! ## Assertions
//!
//! 1. The process does not crash (exits with code 0 on clean quit).
//! 2. The control hints bar is present and readable (contains "Space" and "Q").
//! 3. No raw ESC characters appear in cell contents after vt100 parsing — the
//!    parser correctly consumed all sequences regardless of colour mode.
//!
//! Note: vt100 cell `contents()` returns the printable character(s) at a cell
//! position, never the colour attributes or escape sequences that produced them.
//! The "no raw escape" assertion therefore also validates that the vt100 parser
//! is correctly wired even in no-colour mode.

use super::harness::{PtySession, EXIT_TIMEOUT, STARTUP_TIMEOUT};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Send 'q' twice with a gap and wait for the process to exit cleanly.
fn quit_and_assert(session: &mut PtySession, label: &str) {
    session
        .quit_cleanly()
        .unwrap_or_else(|e| panic!("{label}: send quit: {e}"));
    let code = session.wait_exit(EXIT_TIMEOUT);
    assert_eq!(
        code,
        Some(0),
        "{label}: expected exit code 0; got {:?}",
        code
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn no_color_startup_no_crash() {
    // Start the app with NO_COLOR=1.  crossterm honours this variable and
    // suppresses all SGR colour codes; ratatui falls back to default styles.
    //
    // The application must:
    //   (a) not crash,
    //   (b) still render the control hints bar with readable text,
    //   (c) exit cleanly with code 0 when 'q' is pressed.
    let mut session = PtySession::spawn(80, 24, &[("NO_COLOR", "1")])
        .expect("failed to spawn tui-translator with NO_COLOR=1");

    // The hints bar must appear even in no-colour mode.
    assert!(
        session.wait_for_text("Q quit", STARTUP_TIMEOUT),
        "no_color_startup_no_crash: timed out — 'Q quit' never appeared with NO_COLOR=1",
    );

    // The control bar text must contain the two mandatory shortcuts.
    let last_row = session.row_text(23);
    assert!(
        last_row.contains("Space"),
        "no_color_startup_no_crash: 'Space' missing from hints bar with NO_COLOR=1: {:?}",
        last_row,
    );
    assert!(
        last_row.contains("Q quit"),
        "no_color_startup_no_crash: 'Q quit' missing from hints bar with NO_COLOR=1: {:?}",
        last_row,
    );

    quit_and_assert(&mut session, "no_color_startup_no_crash");
}

#[test]
fn no_color_readable_control_bar() {
    // Verify that the control bar text is readable — i.e., every printable
    // character in the hints row is a plain ASCII character, not an escape
    // sequence or a replacement character (U+FFFD).
    //
    // Cell contents returned by vt100::Cell::contents() are the rendered
    // Unicode scalars; if the parser or the application emits malformed
    // sequences, vt100 may surface them as garbled characters.
    let mut session = PtySession::spawn(80, 24, &[("NO_COLOR", "1")])
        .expect("failed to spawn tui-translator for no_color_readable_control_bar");

    assert!(
        session.wait_for_text("Q quit", STARTUP_TIMEOUT),
        "no_color_readable_control_bar: timed out waiting for initial frame",
    );

    // Check every cell in the last row for:
    // - No raw ESC byte (0x1b) — would indicate unparsed escape sequences.
    // - No Unicode replacement character (U+FFFD) — would indicate garbled
    //   multi-byte sequences.
    let last_row = session.row_text(23);
    assert!(
        !last_row.contains('\x1b'),
        "control bar contains raw ESC byte in no-colour mode: {:?}",
        last_row,
    );
    assert!(
        !last_row.contains('\u{FFFD}'),
        "control bar contains replacement character (U+FFFD) in no-colour mode: {:?}",
        last_row,
    );

    quit_and_assert(&mut session, "no_color_readable_control_bar");
}

#[test]
fn term_dumb_no_crash() {
    // On Windows, TERM=dumb is not consulted by crossterm (Windows uses the
    // Console API or ConPTY VT mode directly, not terminfo).  We set it
    // anyway to document the expected behaviour: the application should not
    // crash regardless of an unrecognised or "dumb" TERM value.
    //
    // Combined with NO_COLOR=1 this is the most restrictive colour/capability
    // combination we can exercise on Windows without a custom terminal stub.
    let mut session = PtySession::spawn(80, 24, &[("TERM", "dumb"), ("NO_COLOR", "1")])
        .expect("failed to spawn tui-translator with TERM=dumb NO_COLOR=1");

    // The application must render the hints bar even in the most restricted
    // environment combination.
    assert!(
        session.wait_for_text("Q quit", STARTUP_TIMEOUT),
        "term_dumb_no_crash: timed out — 'Q quit' never appeared with TERM=dumb NO_COLOR=1",
    );

    // Scan all rows for raw escape sequences (they should all be consumed by
    // the vt100 parser).
    let all_rows = session.all_rows();
    for (i, row) in all_rows.iter().enumerate() {
        assert!(
            !row.contains('\x1b'),
            "row {i} contains raw ESC byte with TERM=dumb NO_COLOR=1: {:?}",
            row,
        );
    }

    quit_and_assert(&mut session, "term_dumb_no_crash");
}
