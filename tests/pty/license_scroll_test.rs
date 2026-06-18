// SPDX-License-Identifier: MIT
//
// End-to-end PTY coverage for the LicenseReview pager bindings
// (issue #883).  Drives the real onboarding wizard through
//   BranchSelection → HardwareSurvey → LicenseReview[0] (MIT, 21 lines)
//   → LicenseReview[1] (Apache-2.0, 184 lines)
// and asserts that PageDown / End / Home actually move the scroll
// offset, observed through the footer "line N/184" indicator that
// the renderer prints (onboarding_render.rs).
//
// Why the *second* model: only the opus-mt Apache-2.0 license
// (184 lines) is longer than the 26-line visible body, so it is
// the only license whose offset is observable.  whisper-tiny's
// MIT text is 21 lines and never scrolls — a test that asserted on
// it would be a no-op masquerading as coverage.
//
// The unit tests in `src/tui/onboarding_tests.rs` and
// `src/main.rs::tests::wizard_pager_keys_route_to_scroll_events`
// pin the handler arithmetic and the keymap routing
// deterministically; this PTY test is the integration proof that
// the keys survive the full crossterm → key_to_action → wizard
// path a real user exercises.

use super::harness::{PtySession, STARTUP_TIMEOUT};
use std::time::Duration;
use tempfile::TempDir;

/// The opus-mt Apache-2.0 license is the only bundled license longer
/// than the 26-line visible body, so it is the one the pager test
/// drives.  Compute its line count from the asset at compile time
/// rather than hard-coding a magic number: if the asset is ever
/// reformatted the test re-derives the expected `line N/<total>`
/// footer instead of silently breaking.  This must match the source
/// the binary embeds (src/providers/local/manifest.rs ->
/// OPUS_MT_APACHE_LICENSE = include_str!("../../../assets/licenses/opus-mt-apache.txt")).
const APACHE_LICENSE: &str = include_str!("../../assets/licenses/opus-mt-apache.txt");

/// Lines in the Apache license body, matching the renderer's
/// `text.lines().count()` (onboarding_render.rs).
fn apache_total_lines() -> usize {
    APACHE_LICENSE.lines().count()
}

/// First-screen footer position: the renderer shows the first
/// VISIBLE_BODY (26) lines, so `visible_end == min(26, total)`.
const VISIBLE_BODY: usize = 26;

/// Spawn the wizard in an isolated config dir so a developer's real
/// `~/.config` cannot suppress onboarding.  Mirrors the helper in
/// `onboarding_test.rs`.  Returns the session plus the TempDir
/// guard (kept alive for the lifetime of the session).
fn spawn_wizard(cols: u16, rows: u16) -> (PtySession, TempDir) {
    let fake_home = TempDir::new().expect("temp home for license-scroll test");
    let home_str = fake_home
        .path()
        .to_str()
        .expect("temp home path must be valid UTF-8")
        .to_string();
    let config_dir_str = fake_home
        .path()
        .join("tui-config")
        .to_str()
        .expect("config dir path must be valid UTF-8")
        .to_string();
    let session = PtySession::spawn(
        cols,
        rows,
        &[
            ("TUI_TRANSLATOR_SKIP_ONBOARDING", "0"),
            ("TUI_TRANSLATOR_CONFIG_DIR", config_dir_str.as_str()),
            ("USERPROFILE", home_str.as_str()),
        ],
    )
    .unwrap_or_else(|e| panic!("spawn license-scroll wizard {cols}×{rows}: {e}"));
    (session, fake_home)
}

/// Advance from BranchSelection to the Apache-2.0 LicenseReview step.
///
/// Flow (macOS/Linux, no VirtualCableGate):
///   `1`  → Local-only branch, auto-advances to HardwareSurvey
///   `\r` → accept HardwareSurvey preset → LicenseReview[0] (MIT)
///   `\r` → accept MIT → LicenseReview[1] (Apache-2.0, multi-page)
///
/// On Windows the VirtualCableGate step sits between BranchSelection
/// and HardwareSurvey; the extra `\r` below is absorbed harmlessly
/// because the gate also advances on Enter.
fn drive_to_apache_license(session: &mut PtySession) {
    assert!(
        session.wait_for_text("Setup Wizard", STARTUP_TIMEOUT),
        "wizard heading should appear at launch"
    );
    // BranchSelection: '1' selects Local-only, '\r' advances off the
    // branch step (selection alone does not advance — see the
    // GoogleCloud flow in onboarding_test.rs which also needs the
    // explicit Enter).
    session.send(b"1").expect("select Local-only");
    std::thread::sleep(Duration::from_millis(150));
    session.send(b"\r").expect("advance from branch selection");

    // On Windows the VirtualCableGate step may sit here; skip it.
    // On macOS/Linux the gate is disabled (cfg!(windows)) so the
    // 500ms probe is a cheap miss.
    let survey_deadline =
        std::time::Instant::now() + Duration::from_secs(STARTUP_TIMEOUT.as_secs() * 2);
    loop {
        if session.wait_for_text("Virtual Cable Gate", Duration::from_millis(500)) {
            session.send(b"s").expect("skip virtual cable gate");
        }
        if session.wait_for_text("Hardware Survey", Duration::from_millis(500)) {
            break;
        }
        if std::time::Instant::now() >= survey_deadline {
            panic!(
                "Hardware Survey step never appeared after Local-only; screen:\n{}",
                session.all_rows().join("\n")
            );
        }
    }

    // HardwareSurvey → accept preset → LicenseReview[0] (MIT).
    session.send(b"\r").expect("accept hardware survey");
    assert!(
        session.wait_for_text("License", STARTUP_TIMEOUT),
        "first License step (MIT) should appear after the hardware survey; screen:\n{}",
        session.all_rows().join("\n")
    );

    // Accept licenses with Enter until the multi-page Apache footer
    // is visible.  whisper-tiny (MIT, 21 lines) is model 0; opus-mt
    // (Apache-2.0) is model 1, the only license longer than the
    // viewport and therefore the only scrollable one.  The footer
    // suffix "/<total>" identifies it.
    //
    // Overshoot guard: send ONE Enter, then poll specifically for
    // the Apache marker before sending the next.  A blind
    // send-then-sleep loop could fire a second Enter while the
    // Apache screen is still rendering on a slow box, accepting the
    // license and skipping past it into the main UI.  Gating each
    // Enter on a bounded wait makes the advance one-license-at-a-time.
    let apache_marker = format!("/{}", apache_total_lines());
    let apache_deadline =
        std::time::Instant::now() + Duration::from_secs(STARTUP_TIMEOUT.as_secs() * 2);
    while std::time::Instant::now() < apache_deadline {
        if session.screen_contains(&apache_marker) {
            return;
        }
        session.send(b"\r").expect("advance license step");
        // Bounded wait for the Apache footer; if it appears we exit
        // on the next loop check, if not we advance again.  Either
        // way only one Enter is in flight at a time.
        let _ = session.wait_for_text(&apache_marker, Duration::from_secs(2));
    }
    panic!(
        "Apache-2.0 ({}-line) license step never appeared; screen:\n{}",
        apache_total_lines(),
        session.all_rows().join("\n")
    );
}

#[test]
fn license_review_pager_keys_move_apache_offset() {
    // 120 columns keeps the footer
    //   "… [Home/End] top/bot    line N/<total>"
    // on a single row (panel inner width ≈94), so the "line N/<total>"
    // indicator is never split by word-wrap and `screen_contains`
    // can match it verbatim.
    let total = apache_total_lines();
    // Footer positions derived from the renderer's arithmetic
    // (onboarding_render.rs): visible_end = min(scroll + 26, total).
    //
    // Match only the "N/M" slash token, NOT the "line N/M" phrase:
    // the indicator is rendered inside the box-drawing TITLE row,
    // and Windows ConPTY collapses the space between "line" and the
    // number (observed: "line26/184").  The slash token is immune to
    // that and still uniquely identifies the scroll position.
    let top = format!("{}/{}", VISIBLE_BODY.min(total), total);
    let bottom = format!("{total}/{total}");
    let page_down = format!("{}/{}", (10 + VISIBLE_BODY).min(total), total);

    let (mut session, _home) = spawn_wizard(120, 40);
    drive_to_apache_license(&mut session);

    // Initial offset: the body shows the first 26 lines.
    assert!(
        session.wait_for_text(&top, STARTUP_TIMEOUT),
        "Apache license should open at the top ({top}); screen:\n{}",
        session.all_rows().join("\n")
    );

    // End → jump to the bottom: footer must report the full length.
    session.send(b"\x1b[F").expect("send End");
    assert!(
        session.wait_for_text(&bottom, STARTUP_TIMEOUT),
        "End must scroll the Apache license to the bottom ({bottom}); screen:\n{}",
        session.all_rows().join("\n")
    );

    // Home → jump back to the top.
    session.send(b"\x1b[H").expect("send Home");
    assert!(
        session.wait_for_text(&top, STARTUP_TIMEOUT),
        "Home must scroll the Apache license back to the top ({top}); screen:\n{}",
        session.all_rows().join("\n")
    );

    // PageDown advances by 10 lines: 26 → 36.
    session.send(b"\x1b[6~").expect("send PageDown");
    assert!(
        session.wait_for_text(&page_down, STARTUP_TIMEOUT),
        "PageDown must advance the Apache license by 10 lines ({page_down}); screen:\n{}",
        session.all_rows().join("\n")
    );

    // PageUp returns by 10 lines: 36 → 26.
    session.send(b"\x1b[5~").expect("send PageUp");
    assert!(
        session.wait_for_text(&top, STARTUP_TIMEOUT),
        "PageUp must rewind the Apache license by 10 lines ({top}); screen:\n{}",
        session.all_rows().join("\n")
    );

    session.kill();
}
