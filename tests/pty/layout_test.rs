//! PTY layout tests — issue #105.
//!
//! Tests spawn `tui-translator` in a ConPTY at three standard terminal sizes
//! (80×24, 120×40, 200×50), wait for the first complete TUI frame to appear,
//! and assert on the structural layout of that frame.
//!
//! ## Layout contract (compact-metrics mode, regardless of audio state)
//!
//! ```text
//! Row  0–2          : title bar  — "TUI Translator" centered, STT state
//! Row  3–5          : audio level gauge
//! Row  6–(H-5)      : subtitle pane (bordered, shows "Subtitles" in title)
//! Row  (H-4)–(H-2)  : metrics strip (compact, 3 rows)
//! Row  (H-1)        : control hints bar — "Space pause … Q quit"
//! ```
//!
//! All assertions target elements that are **always present** regardless of
//! whether WASAPI audio capture succeeds or not, so the tests remain
//! deterministic in CI environments without an audio render device.
//!
//! ## Two-keystroke quit
//!
//! After pressing `q`, the app draws a session-summary overlay and then waits
//! for *any* second keystroke before exiting.  Tests therefore send `b"qq"`:
//! the first `q` triggers the quit path, the second `q` dismisses the summary.

use super::harness::{PtySession, EXIT_TIMEOUT, STARTUP_TIMEOUT};
use std::time::{Duration, Instant};

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Wait for the initial TUI frame to appear; panic if it does not arrive within
/// `STARTUP_TIMEOUT`.
///
/// The presence of `"Q quit"` in the hints bar is the sentinel: it is always
/// rendered at widths ≥ 80 columns regardless of audio or API state.
fn assert_initial_frame(session: &PtySession, label: &str) {
    assert!(
        session.wait_for_text("Q quit", STARTUP_TIMEOUT),
        "{label}: timed out waiting for initial TUI frame — \
         hints bar text 'Q quit' never appeared in {} s",
        STARTUP_TIMEOUT.as_secs(),
    );
}

/// Assert the structural layout contract for a PTY session of `cols × rows`.
///
/// Checked invariants:
/// 1. Control hints bar occupies the very last row and contains "Space" and "Q
///    quit".
/// 2. "TUI Translator" appears in the title-bar area (rows 0–2).
/// 3. The subtitle region is present; in auth-error state the banner can cover
///    the border title, so the empty-pane text is accepted too.
/// 4. No `[SRC]` or `[TGT]` pairs are visible on an empty startup.
fn check_layout(session: &PtySession, cols: u16, rows: u16) {
    // ── Control hints bar — must be in the last row ───────────────────────────
    let last_row = session.row_text(rows - 1);
    assert!(
        last_row.contains("Q quit"),
        "control bar not in bottom row {} at {cols}×{rows}: {:?}",
        rows - 1,
        last_row,
    );
    assert!(
        last_row.contains("Space"),
        "hints bar missing 'Space' in bottom row at {cols}×{rows}: {:?}",
        last_row,
    );

    // ── Title bar — must be within the first three rows ───────────────────────
    let title_area: String = (0..3)
        .map(|r| session.row_text(r))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        title_area.contains("TUI Translator"),
        "title 'TUI Translator' not found in rows 0–2 at {cols}×{rows}: {:?}",
        title_area,
    );

    // ── Subtitle pane — bordered widget with "Subtitles" title ────────────────
    let all_rows = session.all_rows();
    let has_subtitle_region = all_rows
        .iter()
        .any(|r| r.contains("Subtitles") || r.contains("No subtitles yet."));
    assert!(
        has_subtitle_region,
        "subtitle region not found in {cols}×{rows} layout",
    );

    // ── Empty startup — no subtitle pairs yet ─────────────────────────────────
    let has_pair_text = all_rows
        .iter()
        .any(|r| r.contains("[SRC]") || r.contains("[TGT]"));
    assert!(
        !has_pair_text,
        "unexpected subtitle pair content on empty startup at {cols}×{rows}",
    );
}

fn wait_for_layout(session: &PtySession, cols: u16, rows: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        let last_row = session.row_text(rows - 1);
        let title_area: String = (0..3)
            .map(|r| session.row_text(r))
            .collect::<Vec<_>>()
            .join(" ");
        let all_rows = session.all_rows();
        if last_row.contains("Q quit")
            && last_row.contains("Space")
            && title_area.contains("TUI Translator")
            && all_rows
                .iter()
                .any(|r| r.contains("Subtitles") || r.contains("No subtitles yet."))
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    eprintln!("timed out waiting for complete {cols}×{rows} layout after resize");
    for (i, row) in session.all_rows().iter().enumerate() {
        eprintln!("  row {i:02}: {row:?}");
    }
    false
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn layout_80x24() {
    let mut session =
        PtySession::spawn(80, 24, &[]).expect("failed to spawn tui-translator for layout_80x24");
    assert_initial_frame(&session, "80×24");
    check_layout(&session, 80, 24);
    session.quit_cleanly().expect("send quit");
    let code = session.wait_exit(EXIT_TIMEOUT);
    assert_eq!(
        code,
        Some(0),
        "layout_80x24: expected clean exit (code 0); got {code:?}"
    );
}

#[test]
fn layout_120x40() {
    let mut session =
        PtySession::spawn(120, 40, &[]).expect("failed to spawn tui-translator for layout_120x40");
    assert_initial_frame(&session, "120×40");
    check_layout(&session, 120, 40);
    session.quit_cleanly().expect("send quit");
    let code = session.wait_exit(EXIT_TIMEOUT);
    assert_eq!(
        code,
        Some(0),
        "layout_120x40: expected clean exit (code 0); got {code:?}"
    );
}

#[test]
fn layout_200x50() {
    let mut session =
        PtySession::spawn(200, 50, &[]).expect("failed to spawn tui-translator for layout_200x50");
    assert_initial_frame(&session, "200×50");
    check_layout(&session, 200, 50);
    session.quit_cleanly().expect("send quit");
    let code = session.wait_exit(EXIT_TIMEOUT);
    assert_eq!(
        code,
        Some(0),
        "layout_200x50: expected clean exit (code 0); got {code:?}"
    );
}

#[test]
fn layout_40x10_zero_state_readable() {
    let mut session = PtySession::spawn(40, 10, &[])
        .expect("failed to spawn tui-translator for layout_40x10_zero_state_readable");
    assert!(
        session.wait_for_text("Lang:vi", STARTUP_TIMEOUT),
        "40x10: timed out waiting for compact zero-state status"
    );

    let status_rows = session.all_rows()[6..9].join("\n");
    assert!(
        status_rows.contains("Lang:vi") && status_rows.contains("0p"),
        "40x10 status rows should keep language and pair count readable; got:\n{status_rows}"
    );
    assert!(
        !status_rows.contains('$'),
        "40x10 zero-state status should not show a dollar amount; got:\n{status_rows}"
    );

    let last_row = session.row_text(9);
    assert!(
        last_row.contains('Q') && !last_row.contains("TUI Translator"),
        "40x10 hints row should remain separate from the title bar; got: {last_row:?}"
    );

    session.quit_cleanly().expect("send quit");
    let code = session.wait_exit(EXIT_TIMEOUT);
    assert_eq!(
        code,
        Some(0),
        "layout_40x10_zero_state_readable: expected clean exit (code 0); got {code:?}"
    );
}

#[test]
fn layout_60x10_zero_state_cost_wording() {
    let mut session = PtySession::spawn(60, 10, &[])
        .expect("failed to spawn tui-translator for layout_60x10_zero_state_cost_wording");
    assert!(
        session.wait_for_text("no charges", STARTUP_TIMEOUT),
        "60x10: timed out waiting for zero-state cost wording"
    );

    let status_rows = session.all_rows()[6..9].join("\n");
    assert!(
        status_rows.contains("Lang:vi") && status_rows.contains("no charges"),
        "60x10 status rows should keep language and zero-cost wording readable; got:\n{status_rows}"
    );
    assert!(
        !status_rows.contains('$'),
        "60x10 zero-state status should not show a dollar amount; got:\n{status_rows}"
    );

    let last_row = session.row_text(9);
    assert!(
        last_row.contains('Q') && !last_row.contains("TUI Translator"),
        "60x10 hints row should remain separate from the title bar; got: {last_row:?}"
    );

    session.quit_cleanly().expect("send quit");
    let code = session.wait_exit(EXIT_TIMEOUT);
    assert_eq!(
        code,
        Some(0),
        "layout_60x10_zero_state_cost_wording: expected clean exit (code 0); got {code:?}"
    );
}

#[test]
fn layout_resize_no_crash() {
    // Start at 120×40 then shrink to 80×24.
    //
    // The resize sends a Windows ConPTY resize event; ratatui receives a
    // `crossterm::event::Event::Resize` and redraws the full layout at the
    // new size.  This test verifies:
    //   1. No crash (process still running after resize).
    //   2. The resized frame satisfies the full layout contract at 80×24.
    //   3. No obvious garbling: title-bar content and hints-bar content each
    //      appear in their correct regions (no overlap).
    let mut session = PtySession::spawn(120, 40, &[])
        .expect("failed to spawn tui-translator for layout_resize_no_crash");
    assert_initial_frame(&session, "120×40 (pre-resize)");

    // Shrink the PTY window.  resize() now also updates session.rows/cols
    // and the vt100 parser dimensions so post-resize assertions are valid.
    session
        .resize(80, 24)
        .expect("PTY resize 120×40 → 80×24 failed");

    // Wait for the app to redraw at the new size. The pre-resize frame also
    // contains "Q quit", so wait for the full structural contract instead of
    // a single sentinel that can still be present while ConPTY is repainting.
    assert!(
        wait_for_layout(&session, 80, 24, Duration::from_secs(5)),
        "tui-translator did not redraw after PTY resize from 120×40 to 80×24",
    );

    // The process must still be running — no crash on resize.
    assert!(
        session.is_running(),
        "tui-translator crashed after PTY resize from 120×40 to 80×24",
    );

    // ── Layout contract at the new size ──────────────────────────────────────
    // Delegates to the same `check_layout` used by the static-size tests,
    // proving that the resized frame is not garbled or missing widgets.
    check_layout(&session, 80, 24);

    // ── No overlap / garbling check ──────────────────────────────────────────
    // Title-bar content must NOT bleed into the bottom row, and hints-bar
    // content must NOT bleed into the title-bar area.
    let last_row = session.row_text(23);
    assert!(
        !last_row.contains("TUI Translator"),
        "title-bar text leaked into the hints bar at 80×24 after resize: {:?}",
        last_row,
    );
    let title_area: String = (0..3)
        .map(|r| session.row_text(r))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        !title_area.contains("Q quit"),
        "hints-bar text leaked into the title area at 80×24 after resize: {:?}",
        title_area,
    );

    // Trigger clean exit and confirm it succeeds.
    session.quit_cleanly().expect("send quit after resize");
    let code = session.wait_exit(EXIT_TIMEOUT);
    assert_eq!(
        code,
        Some(0),
        "expected exit code 0 after resize; got {:?}",
        code
    );
}
