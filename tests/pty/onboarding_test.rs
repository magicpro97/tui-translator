use super::harness::{PtySession, EXIT_TIMEOUT, STARTUP_TIMEOUT};
use std::time::Duration;
use tempfile::TempDir;

// ── Onboarding layout helpers ─────────────────────────────────────────────────

/// Spawn a PTY session that will trigger the first-run onboarding overlay.
///
/// Uses an isolated temporary directory as `USERPROFILE` so no pre-existing
/// config is found and the binary opens the "First-Run Setup" overlay.
fn spawn_onboarding_session(cols: u16, rows: u16) -> (PtySession, TempDir) {
    let fake_home = TempDir::new().expect("temp home for onboarding layout test");
    let home_str = fake_home
        .path()
        .to_str()
        .expect("temp home path must be valid UTF-8")
        .to_string();
    let session = PtySession::spawn(
        cols,
        rows,
        &[
            ("TUI_TRANSLATOR_SKIP_ONBOARDING", "0"),
            ("USERPROFILE", home_str.as_str()),
        ],
    )
    .unwrap_or_else(|e| panic!("spawn onboarding session {cols}×{rows}: {e}"));
    (session, fake_home)
}

/// Assert that the onboarding overlay is visible and structurally readable.
///
/// Checks that:
/// 1. The "First-Run Setup" border title is present.
/// 2. Key field labels visible at every supported size are rendered.
/// 3. No obvious row overflow: the overlay title must not bleed into the
///    very last terminal row (the TUI background row).
///
/// Keyboard-hint assertions are size-dependent (the hint line may be clipped
/// at small panel heights) and are performed in the individual test functions.
fn check_onboarding_layout(session: &PtySession, cols: u16, rows: u16) {
    // Title must be present (border title of the overlay panel).
    assert!(
        session.screen_contains("First-Run Setup"),
        "onboarding panel title 'First-Run Setup' not found at {cols}×{rows}; rows:\n{}",
        session.all_rows().join("\n"),
    );

    // Core field labels always visible (Source language is the first field,
    // always active/highlighted on startup, and present at all sizes).
    assert!(
        session.screen_contains("Source language"),
        "'Source language' field label not found at {cols}×{rows}; rows:\n{}",
        session.all_rows().join("\n"),
    );
    assert!(
        session.screen_contains("Google API key"),
        "'Google API key' field label not found at {cols}×{rows}; rows:\n{}",
        session.all_rows().join("\n"),
    );

    // The panel title must not appear in the very last row of the terminal
    // (that would indicate overflow / widget bleeding).
    let last_row = session.row_text(rows - 1);
    assert!(
        !last_row.contains("First-Run Setup"),
        "onboarding panel title leaked into last terminal row {}: {last_row:?}",
        rows - 1,
    );
}

/// Quit cleanly from the onboarding overlay via two Ctrl+C presses.
fn quit_onboarding(session: &mut PtySession) {
    session.send(&[0x03]).expect("Ctrl+C to quit onboarding");
    std::thread::sleep(Duration::from_millis(300));
    session.send(&[0x03]).expect("Ctrl+C to dismiss summary");
}

#[test]
fn first_run_setup_creates_home_config_and_stays_gone_after_restart() {
    let fake_home = TempDir::new().expect("temp home");
    let home_str = fake_home
        .path()
        .to_str()
        .expect("temp home path must be valid UTF-8")
        .to_string();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("soak")
        .join("soak_audio.wav");
    let fixture_str = fixture
        .to_str()
        .expect("fixture path must be valid UTF-8")
        .to_string();
    let config_path = fake_home.path().join(".tui-translator").join("config.json");

    {
        let mut session = PtySession::spawn(
            110,
            30,
            &[
                ("TUI_TRANSLATOR_SKIP_ONBOARDING", "0"),
                ("USERPROFILE", home_str.as_str()),
            ],
        )
        .expect("spawn onboarding session");
        assert!(
            session.wait_for_text("First-Run Setup", STARTUP_TIMEOUT),
            "first launch should show the onboarding overlay"
        );

        let setup_input = format!(
            "\t\tdemo-key\t\x08\x08\x08\x08\x08\x08file\t\t{fixture}\r",
            fixture = fixture_str
        );
        session
            .send(setup_input.as_bytes())
            .expect("save onboarding config");
        std::thread::sleep(Duration::from_millis(600));

        assert!(
            config_path.exists(),
            "saving onboarding must create a home config"
        );
        session.quit_cleanly().expect("quit first session");
        let exit = session.wait_exit(EXIT_TIMEOUT).expect("first session exit");
        assert_eq!(exit, 0, "first onboarding session should exit cleanly");
    }

    let written = std::fs::read_to_string(&config_path).expect("read saved config");
    assert!(
        written.contains("\"google_api_key\": \"demo-key\""),
        "saved config should contain the typed API key; got:\n{written}"
    );
    assert!(
        written.contains("\"audio_source\": \"file\""),
        "saved config should switch relaunch proof to file audio; got:\n{written}"
    );
    assert!(
        written.contains("\"source_language\": \"ja-JP\""),
        "saved config should keep the default source language"
    );
    assert!(
        written.contains("\"target_language\": \"vi\""),
        "saved config should keep the default target language"
    );

    {
        let mut session = PtySession::spawn(
            110,
            30,
            &[
                ("TUI_TRANSLATOR_SKIP_ONBOARDING", "0"),
                ("USERPROFILE", home_str.as_str()),
            ],
        )
        .expect("spawn relaunch session");
        assert!(
            session.wait_for_text("TUI Translator", STARTUP_TIMEOUT),
            "relaunch should reach the normal UI"
        );
        assert!(
            !session.screen_contains("First-Run Setup"),
            "relaunch should use the saved home config instead of reopening onboarding"
        );
        session.quit_cleanly().expect("quit relaunch session");
        let exit = session.wait_exit(EXIT_TIMEOUT).expect("relaunch exit");
        assert_eq!(exit, 0, "relaunch session should exit cleanly");
    }
}

#[test]
fn settings_save_defaults_blank_file_audio_path_and_closes_overlay() {
    let temp = TempDir::new().expect("temp config dir");
    let config_dir = temp.path().join("custom-config");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    let config_path = config_dir.join("config.json");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("soak")
        .join("soak_audio.wav");
    let fixture_str = fixture
        .to_str()
        .expect("fixture path must be valid UTF-8")
        .replace('\\', "\\\\");
    std::fs::write(
        &config_path,
        format!(
            concat!(
                "{{\n",
                "  \"google_api_key\": \"pty-test-key\",\n",
                "  \"source_language\": \"ja-JP\",\n",
                "  \"target_language\": \"vi\",\n",
                "  \"tts_enabled\": false,\n",
                "  \"audio_source\": \"file\",\n",
                "  \"audio_file_path\": \"{}\"\n",
                "}}\n"
            ),
            fixture_str
        ),
    )
    .expect("write initial config");

    let config_path_str = config_path
        .to_str()
        .expect("config path must be valid UTF-8")
        .to_string();

    let mut session = PtySession::spawn(
        110,
        30,
        &[("TUI_TRANSLATOR_CONFIG", config_path_str.as_str())],
    )
    .expect("spawn settings session");
    assert!(
        session.wait_for_text("TUI Translator", STARTUP_TIMEOUT),
        "session should reach the normal UI"
    );

    session.send(b"s").expect("open settings");
    assert!(
        session.wait_for_text("Settings", STARTUP_TIMEOUT),
        "settings overlay should open"
    );

    let clear_path = "\t\t\t\t\t".to_string()
        + &"\x08".repeat(
            fixture
                .to_str()
                .expect("fixture path must be valid UTF-8")
                .len(),
        )
        + "\r";
    session
        .send(clear_path.as_bytes())
        .expect("clear audio path and save");

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if !session.screen_contains("Settings") {
            break;
        }
        std::thread::sleep(Duration::from_millis(150));
    }

    assert!(
        !session.screen_contains("Settings"),
        "settings overlay should close after save"
    );
    assert!(session.is_running(), "app should keep running after save");

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let expected_path = config_dir.join("audio-input.wav");
    let expected_path_str = expected_path.to_string_lossy().into_owned();
    let mut persisted_path = None;
    while std::time::Instant::now() < deadline {
        let raw = std::fs::read_to_string(&config_path).expect("read saved config");
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse saved config");
        persisted_path = parsed["audio_file_path"].as_str().map(ToOwned::to_owned);
        if persisted_path.as_deref() == Some(expected_path_str.as_str()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(150));
    }

    assert_eq!(
        persisted_path.as_deref(),
        Some(expected_path_str.as_str()),
        "blank file-audio path should default next to the active config file"
    );

    session.quit_cleanly().expect("quit session");
    let exit = session
        .wait_exit(EXIT_TIMEOUT)
        .expect("settings session exit");
    assert_eq!(exit, 0, "settings session should exit cleanly");
}

#[test]
fn invalid_startup_config_opens_repair_settings_instead_of_exiting() {
    let temp = TempDir::new().expect("temp config dir");
    let config_path = temp.path().join("config.json");
    std::fs::write(
        &config_path,
        concat!(
            "{\n",
            "  \"google_api_key\": \"pty-test-key\",\n",
            "  \"source_language\": \"ja-JPdas\",\n",
            "  \"target_language\": \"vi\",\n",
            "  \"tts_enabled\": false\n",
            "}\n"
        ),
    )
    .expect("write invalid config");

    let config_path_str = config_path
        .to_str()
        .expect("config path must be valid UTF-8")
        .to_string();

    let mut session = PtySession::spawn(
        110,
        30,
        &[
            ("TUI_TRANSLATOR_CONFIG", config_path_str.as_str()),
            ("TUI_TRANSLATOR_SKIP_ONBOARDING", "0"),
        ],
    )
    .expect("spawn invalid-config session");
    assert!(
        session.wait_for_text("Settings", STARTUP_TIMEOUT),
        "invalid startup config should open the settings repair overlay instead of exiting"
    );
    assert!(
        session.wait_for_text("Config needs repair", STARTUP_TIMEOUT),
        "repair overlay should explain that config needs repair"
    );
    assert!(
        session.screen_contains("ja-JPdas"),
        "repair overlay should preserve the invalid value so it can be edited"
    );

    // In the config editor, plain `q` edits the active field. Ctrl+C is the
    // global quit path and must remain available from repair mode.
    session.send(&[0x03]).expect("quit invalid-config session");
    std::thread::sleep(Duration::from_millis(300));
    session.send(&[0x03]).expect("dismiss session summary");
    let exit = session
        .wait_exit(EXIT_TIMEOUT)
        .expect("invalid-config session exit");
    assert_eq!(exit, 0, "invalid-config session should exit cleanly");
}

// ── Onboarding layout tests (issue #165) ─────────────────────────────────────

/// Standard 110×30 terminal — full panel (76×28, non-compact).
///
/// Verifies the onboarding overlay is readable and all structural elements
/// (border title, core field labels, keyboard hints) are present without
/// overflow into the background rows.
///
/// The full panel (76×28, inner area 74×26) fits all content lines including
/// the keyboard hint row, so extended hint text ("Tab/Down next") is assertable.
#[test]
fn onboarding_layout_standard_110x30() {
    let (mut session, _home) = spawn_onboarding_session(110, 30);
    assert!(
        session.wait_for_text("First-Run Setup", STARTUP_TIMEOUT),
        "110×30: timed out waiting for onboarding overlay"
    );
    check_onboarding_layout(&session, 110, 30);

    // Full panel (76×28) is not compact: extended hint variant must be visible.
    assert!(
        session.screen_contains("Tab/Down next"),
        "110×30: full panel should show extended key hints with 'Tab/Down next'; rows:\n{}",
        session.all_rows().join("\n"),
    );
    assert!(
        session.screen_contains("Enter save"),
        "110×30: full panel keyboard hint must contain 'Enter save'; rows:\n{}",
        session.all_rows().join("\n"),
    );

    quit_onboarding(&mut session);
    let exit = session
        .wait_exit(EXIT_TIMEOUT)
        .expect("110×30 onboarding exit");
    assert_eq!(
        exit, 0,
        "onboarding_layout_standard_110x30: expected exit 0"
    );
}

/// Minimum standard terminal (80×24) — panel 76×24, non-compact.
///
/// At 80 columns the panel width equals the compact threshold (76, not < 76)
/// so non-compact rendering is used.  The panel height (24 rows, inner 22)
/// is too small to fit every content line, but the critical structural elements
/// — border title and field labels — are always in the first visible rows.
#[test]
fn onboarding_layout_standard_80x24() {
    let (mut session, _home) = spawn_onboarding_session(80, 24);
    assert!(
        session.wait_for_text("First-Run Setup", STARTUP_TIMEOUT),
        "80×24: timed out waiting for onboarding overlay"
    );
    check_onboarding_layout(&session, 80, 24);

    // At 80×24 the panel is non-compact (width == 76, height == 24 > 16).
    // The intro blurb appears in non-compact mode and is always in the first
    // visible content row, so we assert it as a compact/non-compact discriminator.
    assert!(
        session.screen_contains("Save your initial config"),
        "80×24: non-compact panel should show intro blurb; rows:\n{}",
        session.all_rows().join("\n"),
    );

    quit_onboarding(&mut session);
    let exit = session
        .wait_exit(EXIT_TIMEOUT)
        .expect("80×24 onboarding exit");
    assert_eq!(exit, 0, "onboarding_layout_standard_80x24: expected exit 0");
}

/// Constrained 60×22 terminal — panel 60×22, compact mode.
///
/// Panel width 60 < 76 triggers compact rendering: the intro blurb and extra
/// blank lines are omitted, and the shorter key-hint line is used.  The panel
/// inner area (58×20) is just large enough to fit all 19 compact content lines,
/// so "Enter save" and "Tab/Shift+Tab" are assertable.
#[test]
fn onboarding_layout_compact_60x22() {
    let (mut session, _home) = spawn_onboarding_session(60, 22);
    assert!(
        session.wait_for_text("First-Run Setup", STARTUP_TIMEOUT),
        "60×22: timed out waiting for onboarding overlay"
    );
    check_onboarding_layout(&session, 60, 22);

    // Compact mode uses the shorter hint variant ("Tab/Shift+Tab move …").
    assert!(
        session.screen_contains("Tab/Shift+Tab"),
        "60×22: compact panel should show short key hints with 'Tab/Shift+Tab'; rows:\n{}",
        session.all_rows().join("\n"),
    );
    assert!(
        session.screen_contains("Enter save"),
        "60×22: compact panel keyboard hint must contain 'Enter save'; rows:\n{}",
        session.all_rows().join("\n"),
    );
    // Full-panel hints must NOT appear in compact mode.
    assert!(
        !session.screen_contains("Tab/Down next"),
        "60×22: compact panel must not show extended hint 'Tab/Down next'",
    );
    // Non-compact intro blurb must be absent in compact mode.
    assert!(
        !session.screen_contains("Save your initial config"),
        "60×22: compact panel must not show the intro blurb",
    );

    quit_onboarding(&mut session);
    let exit = session
        .wait_exit(EXIT_TIMEOUT)
        .expect("60×22 onboarding exit");
    assert_eq!(exit, 0, "onboarding_layout_compact_60x22: expected exit 0");
}
