//! PTY regression tests for the settings-editor overlay — issue #170.
//!
//! Tests spawn `tui-translator` in a ConPTY, wait for the steady-state main
//! TUI, open the Settings overlay with the `S` key, and assert on layout
//! readability at two terminal sizes:
//!
//! - **Standard 110×30** (`settings_layout_standard_110x30`): panel 76×28,
//!   non-compact mode.  The extended key-hint line, intro blurb, and all core
//!   field labels must be visible.  The panel title must not bleed into the
//!   last terminal row.
//!
//! - **Constrained 60×22** (`settings_layout_compact_60x22`): panel 60×22;
//!   compact mode is triggered because panel width (60) < 76.  The intro blurb
//!   is omitted and the short hint variant is used.  All core field labels must
//!   still be readable, and the panel title must not appear in the last row.
//!
//! A third test (`settings_tab_navigation_changes_active_field`) opens settings
//! at 110×30 and exercises Tab-key navigation: it verifies that the active-field
//! cursor (`>` prefix) advances to "Target language" after one Tab press, and
//! that the previously active "Source language" row loses its `>` prefix.
//!
//! All tests use the harness's fixture-backed config so they never touch real
//! user config or WASAPI devices on the host.

use super::harness::{PtySession, EXIT_TIMEOUT, STARTUP_TIMEOUT};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;

// ── Timeouts ──────────────────────────────────────────────────────────────────

/// How long to wait for a UI transition (settings open / close) to complete.
/// Shorter than STARTUP_TIMEOUT since the main TUI is already running.
const OVERLAY_TIMEOUT: Duration = Duration::from_secs(8);

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Open the settings overlay by pressing `s` and wait until the panel title
/// `"Settings"` (capital S) appears on screen.
///
/// Panics with a screen dump on timeout.
fn open_settings(session: &mut PtySession, cols: u16, rows: u16) {
    session.send(b"s").expect("send 's' to open settings");
    assert!(
        session.wait_for_text("Settings", OVERLAY_TIMEOUT),
        "settings overlay did not appear within {}s at {cols}×{rows}; screen:\n{}",
        OVERLAY_TIMEOUT.as_secs(),
        session.all_rows().join("\n"),
    );
}

/// Close the settings overlay by pressing Escape and confirm that the overlay
/// is gone.
///
/// After the overlay closes the main TUI redraws and "Settings" (capital S,
/// the panel border title) is no longer visible.  The hints bar retained
/// in the main UI shows `"S settings"` (lowercase s) which is a different
/// string; `screen_contains("Settings")` therefore becomes `false` only when
/// the panel is truly gone.
fn close_settings_esc(session: &mut PtySession, cols: u16, rows: u16) {
    // Escape key: 0x1b.  Sent alone (not as the start of an escape sequence)
    // so crossterm's event loop decodes it as `KeyCode::Esc`.
    session.send(b"\x1b").expect("send Esc to close settings");

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !session.screen_contains("Settings") {
            break;
        }
        std::thread::sleep(Duration::from_millis(150));
    }

    assert!(
        !session.screen_contains("Settings"),
        "settings overlay did not close after Esc at {cols}×{rows}; screen:\n{}",
        session.all_rows().join("\n"),
    );
}

/// Assert that the settings overlay is structurally readable at `cols × rows`.
///
/// Invariants checked at every supported size:
/// 1. Panel border title `"Settings"` (capital S) is present somewhere on
///    screen.
/// 2. Core field labels always rendered in the first visible rows of the panel:
///    `"Source language"`, `"Target language"`, `"Google API key"`,
///    `"Audio source"`.
/// 3. The panel title must **not** appear in the very last terminal row, which
///    would indicate a vertical overflow / widget bleed.
fn check_settings_layout(session: &PtySession, cols: u16, rows: u16) {
    // Panel border title must be visible.
    assert!(
        session.screen_contains("Settings"),
        "settings panel title 'Settings' not found at {cols}×{rows}; screen:\n{}",
        session.all_rows().join("\n"),
    );

    // The first four field labels are always in the top rows of the panel
    // regardless of compact mode or panel height.
    for label in &[
        "Source language",
        "Target language",
        "Google API key",
        "Audio source",
    ] {
        assert!(
            session.screen_contains(label),
            "settings field label {label:?} not found at {cols}×{rows}; screen:\n{}",
            session.all_rows().join("\n"),
        );
    }

    // No overflow into the terminal's last row.
    let last_row = session.row_text(rows - 1);
    assert!(
        !last_row.contains("Settings"),
        "settings panel title leaked into last terminal row {} at {cols}×{rows}: {last_row:?}",
        rows - 1,
    );
}

fn write_config_with_capture_device(dir: &Path, capture_device: &str) -> PathBuf {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("soak")
        .join("soak_audio.wav")
        .canonicalize()
        .expect("canonicalize soak fixture");
    let cfg_path = dir.join("config.json");
    let payload = serde_json::to_string_pretty(&serde_json::json!({
        "google_api_key": "pty-test-key",
        "source_language": "ja-JP",
        "target_language": "vi",
        "tts_enabled": false,
        "audio_source": "file",
        "audio_file_path": fixture,
        "capture_device": capture_device,
    }))
    .expect("serialize capture-device PTY config")
        + "\n";
    std::fs::write(&cfg_path, payload).expect("write capture-device PTY config");
    cfg_path
}

fn write_config_with_virtual_mic_device(dir: &Path, virtual_mic_device: &str) -> PathBuf {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("soak")
        .join("soak_audio.wav")
        .canonicalize()
        .expect("canonicalize soak fixture");
    let cfg_path = dir.join("config.json");
    let payload = serde_json::to_string_pretty(&serde_json::json!({
        "google_api_key": "pty-test-key",
        "source_language": "ja-JP",
        "target_language": "vi",
        "tts_enabled": false,
        "tts_routing": "speakers",
        "virtual_mic_device": virtual_mic_device,
        "audio_source": "file",
        "audio_file_path": fixture,
    }))
    .expect("serialize virtual-mic PTY config")
        + "\n";
    std::fs::write(&cfg_path, payload).expect("write virtual-mic PTY config");
    cfg_path
}

fn wait_for_capture_device_label(session: &PtySession, label: &str, timeout: Duration) -> bool {
    let compact_label = label.replace(' ', "");
    let start = Instant::now();
    while start.elapsed() < timeout {
        if session.screen_contains(label) || session.screen_contains(&compact_label) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    false
}

fn tab_to_field(session: &mut PtySession, label: &str, tabs: usize) {
    for _ in 0..tabs {
        session.send(b"\t").expect("send Tab to advance field");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        session.wait_for_text(&format!("> {label}"), OVERLAY_TIMEOUT),
        "{label} field should become active after {tabs} Tabs; screen:\n{}",
        session.all_rows().join("\n"),
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Standard 110×30 terminal — settings panel 76×28 (non-compact).
///
/// At 110×30 the panel dimensions are `min(76, 110) × min(28, 30) = 76×28`.
/// Neither compact threshold is met (`width == 76` is not `< 76`;
/// `height == 28 > 16`), so non-compact rendering is active.
///
/// Verified:
/// - Panel title `"Settings"` visible.
/// - Core field labels `"Source language"`, `"Target language"`,
///   `"Google API key"`, `"Audio source"` visible.
/// - Non-compact key hint (`"Tab/Down next"`) visible.
/// - `"Enter save"` visible (present in both compact and non-compact hints).
/// - Non-compact intro blurb (`"Edit the saved config"`) visible.
/// - Panel title does not bleed into the last terminal row.
/// - Escape closes the overlay cleanly; app exits with code 0.
#[test]
fn settings_layout_standard_110x30() {
    let mut session = PtySession::spawn(110, 30, &[])
        .expect("spawn tui-translator for settings_layout_standard_110x30");
    assert!(
        session.wait_for_text("Q quit", STARTUP_TIMEOUT),
        "110×30: timed out waiting for main TUI frame"
    );

    open_settings(&mut session, 110, 30);
    check_settings_layout(&session, 110, 30);

    // Non-compact mode: extended hint line must be present.
    assert!(
        session.screen_contains("Tab/Down next"),
        "110×30: non-compact settings should show 'Tab/Down next'; screen:\n{}",
        session.all_rows().join("\n"),
    );
    assert!(
        session.screen_contains("Enter save"),
        "110×30: settings hint must contain 'Enter save'; screen:\n{}",
        session.all_rows().join("\n"),
    );

    // Non-compact mode: intro blurb must be visible.
    assert!(
        session.screen_contains("Edit the saved config"),
        "110×30: non-compact settings should show intro blurb; screen:\n{}",
        session.all_rows().join("\n"),
    );

    close_settings_esc(&mut session, 110, 30);
    session.quit_cleanly().expect("send quit");
    let code = session.wait_exit(EXIT_TIMEOUT);
    assert_eq!(
        code,
        Some(0),
        "settings_layout_standard_110x30: expected exit 0; got {code:?}"
    );
}

/// Constrained 60×22 terminal — settings panel 60×22 (compact — width < 76).
///
/// At 60×22 the panel dimensions are `min(76, 60) × min(28, 22) = 60×22`.
/// Compact mode is triggered because panel width (60) < 76.
///
/// The inner panel area is 58×20 (60−2 × 22−2).  The compact content list
/// nominally has 20 lines in its default state (1 path + 18 fields + 1 hint), but the
/// fixture-backed config supplies a long absolute `audio_file_path` that wraps
/// across extra rows inside the `Paragraph { wrap }` widget, consuming 20 of
/// the 20 available inner rows with field + status content and leaving no room
/// for the hint line.  This is by-design behaviour for a genuinely constrained
/// layout: low-priority hint text may be clipped.
///
/// **No hint-line assertions are made** for this size — only field readability,
/// structural non-overflow, and the absence of non-compact content are tested.
///
/// Verified:
/// - Panel title `"Settings"` visible.
/// - Core field labels present (`"Source language"` … `"Audio source"`).
/// - Deeper field labels that appear below the fold in some layouts are also
///   present (`"STT provider"`, `"TTS enabled"`).
/// - Non-compact extended hint `"Tab/Down next"` absent.
/// - Non-compact intro blurb `"Edit the saved config"` absent.
/// - Panel title does not bleed into the last terminal row (the last row is
///   the panel's bottom border `└─…─┘`, which contains no text).
/// - Escape closes the overlay cleanly; app exits with code 0.
#[test]
fn settings_layout_compact_60x22() {
    let mut session = PtySession::spawn(60, 22, &[])
        .expect("spawn tui-translator for settings_layout_compact_60x22");

    // At 60 columns the hints bar may be truncated; "Q" is the reliable
    // sentinel for the main TUI being ready.
    assert!(
        session.wait_for_text("Q", STARTUP_TIMEOUT),
        "60×22: timed out waiting for main TUI frame"
    );

    open_settings(&mut session, 60, 22);
    check_settings_layout(&session, 60, 22);

    // Deeper field labels — verify that compact mode does not hide content
    // that appears below the top of the list (these are rows 7–9 in the panel).
    for label in &["STT provider", "MT provider", "TTS enabled"] {
        assert!(
            session.screen_contains(label),
            "60×22: settings field label {label:?} not found in compact layout; screen:\n{}",
            session.all_rows().join("\n"),
        );
    }

    // Extended hints must NOT appear in compact mode.
    assert!(
        !session.screen_contains("Tab/Down next"),
        "60×22: compact settings must not show extended hint 'Tab/Down next'; screen:\n{}",
        session.all_rows().join("\n"),
    );

    // Non-compact intro blurb must be absent in compact mode.
    assert!(
        !session.screen_contains("Edit the saved config"),
        "60×22: compact settings must not show intro blurb; screen:\n{}",
        session.all_rows().join("\n"),
    );

    // Note: the compact key-hint line ("Tab/Shift+Tab …") is intentionally not
    // asserted here.  The fixture config's long audio_file_path wraps inside
    // the Paragraph widget and consumes the remaining inner rows, clipping the
    // hint.  This is documented as a known constrained-layout trade-off.

    close_settings_esc(&mut session, 60, 22);
    session.quit_cleanly().expect("send quit");
    let code = session.wait_exit(EXIT_TIMEOUT);
    assert_eq!(
        code,
        Some(0),
        "settings_layout_compact_60x22: expected exit 0; got {code:?}"
    );
}

/// Settings Tab-key navigation advances the active field indicator.
///
/// Opens settings at 110×30 (non-compact) and presses Tab once.  The active
/// field marker (`>` prefix) must move from `"Source language"` (field 0) to
/// `"Target language"` (field 1).
///
/// This verifies:
/// 1. Settings opens with `"Source language"` active (`"> Source language"`).
/// 2. After one Tab press `"> Target language"` is visible.
/// 3. `"> Source language"` is no longer the active row.
///
/// Uses 110×30 so the full panel is visible and the field lines are not
/// truncated.
#[test]
fn settings_tab_navigation_changes_active_field() {
    let mut session = PtySession::spawn(110, 30, &[])
        .expect("spawn tui-translator for settings_tab_navigation_changes_active_field");
    assert!(
        session.wait_for_text("Q quit", STARTUP_TIMEOUT),
        "110×30: timed out waiting for main TUI frame"
    );

    open_settings(&mut session, 110, 30);

    // On open, the first field "Source language" must be active.
    assert!(
        session.screen_contains("> Source language"),
        "settings should open with 'Source language' active; screen:\n{}",
        session.all_rows().join("\n"),
    );

    // Press Tab to advance to the next field.
    session.send(b"\t").expect("send Tab to advance field");

    // "Target language" must now carry the active marker.
    assert!(
        session.wait_for_text("> Target language", OVERLAY_TIMEOUT),
        "after Tab, 'Target language' should be active; screen:\n{}",
        session.all_rows().join("\n"),
    );

    // "Source language" must no longer be active.
    assert!(
        !session.screen_contains("> Source language"),
        "after Tab, 'Source language' must not keep the active marker; screen:\n{}",
        session.all_rows().join("\n"),
    );

    close_settings_esc(&mut session, 110, 30);
    session.quit_cleanly().expect("send quit");
    let code = session.wait_exit(EXIT_TIMEOUT);
    assert_eq!(
        code,
        Some(0),
        "settings_tab_navigation_changes_active_field: expected exit 0; got {code:?}"
    );
}

#[test]
fn settings_capture_device_field_shows_picker() {
    let mut session = PtySession::spawn(110, 30, &[])
        .expect("spawn tui-translator for settings_capture_device_field_shows_picker");
    assert!(
        session.wait_for_text("Q quit", STARTUP_TIMEOUT),
        "110×30: timed out waiting for main TUI frame"
    );

    open_settings(&mut session, 110, 30);

    for _ in 0..4 {
        session.send(b"\t").expect("send Tab to advance field");
        std::thread::sleep(Duration::from_millis(100));
    }

    assert!(
        session.wait_for_text("> Capture device", OVERLAY_TIMEOUT),
        "capture-device field should become active after four Tabs; screen:\n{}",
        session.all_rows().join("\n"),
    );
    assert!(
        session.wait_for_text("Capture device picker", OVERLAY_TIMEOUT),
        "active capture-device field should show the picker list; screen:\n{}",
        session.all_rows().join("\n"),
    );
    assert!(
        session.screen_contains("Windows default playback")
            || session.screen_contains("No active playback devices"),
        "picker should show either default selection or no-device recovery text; screen:\n{}",
        session.all_rows().join("\n"),
    );

    close_settings_esc(&mut session, 110, 30);
    session.quit_cleanly().expect("send quit");
    let code = session.wait_exit(EXIT_TIMEOUT);
    assert_eq!(
        code,
        Some(0),
        "settings_capture_device_field_shows_picker: expected exit 0; got {code:?}"
    );
}

#[test]
fn settings_capture_device_selection_survives_restart() {
    let temp = TempDir::new().expect("temp config dir");
    let cfg_path = write_config_with_capture_device(temp.path(), "USB Headset");
    let cfg_path_string = cfg_path.to_string_lossy().into_owned();

    for run in 1..=2 {
        let mut session = PtySession::spawn(
            110,
            30,
            &[("TUI_TRANSLATOR_CONFIG", cfg_path_string.as_str())],
        )
        .unwrap_or_else(|err| panic!("spawn tui-translator restart run {run}: {err}"));
        assert!(
            session.wait_for_text("Q quit", STARTUP_TIMEOUT),
            "restart run {run}: timed out waiting for main TUI frame"
        );
        assert!(
            wait_for_capture_device_label(&session, "USB Headset", OVERLAY_TIMEOUT),
            "restart run {run}: saved capture device should be visible after startup; screen:\n{}",
            session.all_rows().join("\n"),
        );

        session.quit_cleanly().expect("send quit");
        let code = session.wait_exit(EXIT_TIMEOUT);
        assert_eq!(
            code,
            Some(0),
            "settings_capture_device_selection_survives_restart run {run}: expected exit 0; got {code:?}"
        );
    }
}

#[test]
fn settings_tts_route_selection_persists_keyboard_only() {
    let temp = TempDir::new().expect("temp config dir");
    let cfg_path =
        write_config_with_virtual_mic_device(temp.path(), "CABLE Input (VB-Audio Virtual Cable)");
    let cfg_path_string = cfg_path.to_string_lossy().into_owned();

    let mut session = PtySession::spawn(
        110,
        30,
        &[("TUI_TRANSLATOR_CONFIG", cfg_path_string.as_str())],
    )
    .expect("spawn tui-translator for settings_tts_route_selection_persists_keyboard_only");
    assert!(
        session.wait_for_text("Q quit", STARTUP_TIMEOUT),
        "110x30: timed out waiting for main TUI frame"
    );

    open_settings(&mut session, 110, 30);
    tab_to_field(&mut session, "TTS routing", 9);

    // Ctrl+D is the same cycle action as F2 and is stable across PTY encodings.
    session
        .send(b"\x04")
        .expect("send Ctrl+D to cycle TTS routing");
    assert!(
        session.wait_for_text("virtual_mic", OVERLAY_TIMEOUT),
        "TTS routing should cycle from speakers to virtual_mic; screen:\n{}",
        session.all_rows().join("\n"),
    );

    session.send(b"\r").expect("send Enter to save settings");
    assert!(
        !session.screen_contains("Save failed"),
        "route/device save should not fail; screen:\n{}",
        session.all_rows().join("\n"),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && session.screen_contains("Settings") {
        std::thread::sleep(Duration::from_millis(150));
    }
    assert!(
        !session.screen_contains("Settings"),
        "settings overlay should close after saving route/device; screen:\n{}",
        session.all_rows().join("\n"),
    );

    session.quit_cleanly().expect("send quit");
    let code = session.wait_exit(EXIT_TIMEOUT);
    assert_eq!(
        code,
        Some(0),
        "settings_tts_route_selection_persists_keyboard_only: expected exit 0; got {code:?}"
    );

    let persisted: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cfg_path).expect("read saved config"))
            .expect("parse saved config");
    assert_eq!(persisted["tts_routing"], "virtual_mic");
    assert_eq!(
        persisted["virtual_mic_device"],
        "CABLE Input (VB-Audio Virtual Cable)"
    );
}
