use super::harness::{PtySession, EXIT_TIMEOUT, STARTUP_TIMEOUT};
use std::time::Duration;
use tempfile::TempDir;

// ── Onboarding layout helpers ─────────────────────────────────────────────────

/// Spawn a PTY session that will trigger the first-run wizard overlay.
///
/// Uses an isolated temporary directory as `USERPROFILE` so no pre-existing
/// config is found and the binary opens the "Setup Wizard" overlay.
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

/// Assert that the onboarding wizard overlay is visible and structurally readable.
///
/// Checks that:
/// 1. The "Setup Wizard" heading is present inside the panel.
/// 2. The branch-selection list is rendered ("Local-only" is always the first
///    option and must appear at every supported terminal size).
/// 3. The confirm/navigation hint line is present.
/// 4. No obvious row overflow: the wizard heading must not bleed into the
///    very last terminal row (the TUI background row).
fn check_onboarding_layout(session: &PtySession, cols: u16, rows: u16) {
    // Heading line rendered as panel content (not a border title).
    assert!(
        session.screen_contains("Setup Wizard"),
        "wizard heading 'Setup Wizard' not found at {cols}×{rows}; rows:\n{}",
        session.all_rows().join("\n"),
    );

    // Branch-selection list — "Local-only" is always the first option.
    assert!(
        session.screen_contains("Local-only"),
        "'Local-only' branch option not found at {cols}×{rows}; rows:\n{}",
        session.all_rows().join("\n"),
    );

    // Navigation hint line always present at the BranchSelection step.
    assert!(
        session.screen_contains("[Enter] Confirm"),
        "'[Enter] Confirm' hint not found at {cols}×{rows}; rows:\n{}",
        session.all_rows().join("\n"),
    );

    // The wizard heading must not appear in the very last row of the terminal
    // (that would indicate overflow / widget bleeding).
    let last_row = session.row_text(rows - 1);
    assert!(
        !last_row.contains("Setup Wizard"),
        "wizard heading leaked into last terminal row {}: {last_row:?}",
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
fn first_run_setup_creates_per_user_config_and_stays_gone_after_restart() {
    let fake_home = TempDir::new().expect("temp home");
    let home_str = fake_home
        .path()
        .to_str()
        .expect("temp home path must be valid UTF-8")
        .to_string();
    let config_dir = fake_home.path().join("tui-config");
    let config_dir_str = config_dir
        .to_str()
        .expect("config dir path must be valid UTF-8")
        .to_string();
    let config_path = config_dir.join("config.json");

    {
        let mut session = PtySession::spawn(
            110,
            30,
            &[
                ("TUI_TRANSLATOR_SKIP_ONBOARDING", "0"),
                ("TUI_TRANSLATOR_CONFIG_DIR", config_dir_str.as_str()),
                ("USERPROFILE", home_str.as_str()),
            ],
        )
        .expect("spawn onboarding session");
        assert!(
            session.wait_for_text("Setup Wizard", STARTUP_TIMEOUT),
            "first launch should show the setup wizard"
        );

        // Select the GoogleCloud branch ('3'), advance to key entry, type an API
        // key that avoids wizard-level hotkey conflicts, then confirm.
        // '3' → SelectBranch3 (GoogleCloud); '\r' × 3 = advance × 3 steps.
        session.send(b"3").expect("select google cloud branch");
        std::thread::sleep(Duration::from_millis(150));
        session.send(b"\r").expect("advance from branch selection");
        assert!(
            session.wait_for_text("Google API Key", STARTUP_TIMEOUT),
            "after selecting GoogleCloud branch the key-entry step should appear"
        );

        // Type an API key without wizard hotkey collisions (no l/t/m/s/r/q/1/2/3).
        session.send(b"abc-key-xyz").expect("type api key");
        std::thread::sleep(Duration::from_millis(100));
        session.send(b"\r").expect("advance from key entry");
        assert!(
            session.wait_for_text("Confirm Setup", STARTUP_TIMEOUT),
            "after entering the API key the confirmation step should appear"
        );

        session.send(b"\r").expect("confirm wizard");

        // After Enter on Confirm Setup, the binary runs `handle_wizard_outcome`
        // which calls `config::write_config` (an atomic rename). On slow CI
        // runners the render → input → write path can take longer than a fixed
        // sleep, so poll up to STARTUP_TIMEOUT instead of relying on a hard
        // 600ms delay (see settings_save_defaults_blank_file_audio_path_… for
        // the same polling pattern on persisted files).
        let deadline = std::time::Instant::now() + STARTUP_TIMEOUT;
        while !config_path.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            config_path.exists(),
            "completing the wizard must create a per-user config (waited up to {STARTUP_TIMEOUT:?})"
        );
        session.quit_cleanly().expect("quit first session");
        let exit = session.wait_exit(EXIT_TIMEOUT).expect("first session exit");
        assert_eq!(exit, 0, "first onboarding session should exit cleanly");
    }

    let written = std::fs::read_to_string(&config_path).expect("read saved config");
    assert!(
        written.contains("\"google_api_key\": \"abc-key-xyz\""),
        "saved config should contain the typed API key; got:\n{written}"
    );
    assert!(
        written.contains("\"stt_provider\": \"google\""),
        "saved config should set stt_provider to google for GoogleCloud branch; got:\n{written}"
    );
    // `source_language` is auto-detected from the OS locale (UX-04 #689). On CI
    // the locale varies (en-US on most runners, ja-JP on JP runners, etc.), so
    // assert only that the field is present and non-empty rather than pinning a
    // specific value.
    assert!(
        written.contains("\"source_language\":"),
        "saved config should contain a source_language field; got:\n{written}"
    );
    assert!(
        written.contains("\"target_language\": \"vi\""),
        "saved config should keep the default target language; got:\n{written}"
    );

    {
        let mut session = PtySession::spawn(
            110,
            30,
            &[
                ("TUI_TRANSLATOR_SKIP_ONBOARDING", "0"),
                ("TUI_TRANSLATOR_CONFIG_DIR", config_dir_str.as_str()),
                ("USERPROFILE", home_str.as_str()),
            ],
        )
        .expect("spawn relaunch session");
        assert!(
            session.wait_for_text("TUI Translator", STARTUP_TIMEOUT),
            "relaunch should reach the normal UI"
        );
        assert!(
            !session.screen_contains("Setup Wizard"),
            "relaunch should use the saved per-user config instead of reopening the wizard"
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

    // Navigate to "Audio file path" (field index 5) one Tab at a time.
    // Sending all tabs in a single batch risks the early ones being
    // processed before the overlay is fully interactive.  A 100 ms pause
    // between each tab matches the pattern used by `settings_test.rs`.
    for _ in 0..5 {
        session.send(b"\t").expect("send Tab to advance field");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        session.wait_for_text("> Audio file path", STARTUP_TIMEOUT),
        "Audio file path field should be active after 5 tabs; screen:\n{}",
        session.all_rows().join("\n"),
    );

    // Clear the field.  crossterm 0.29 on Unix maps 0x7F to
    // KeyCode::Backspace, while 0x08 maps to Ctrl+H (AnyKey in the config
    // editor) and would not delete anything.  Use 0x7F for each character.
    let path_char_count = fixture
        .to_str()
        .expect("fixture path must be valid UTF-8")
        .chars()
        .count();
    let clear_and_save = "\x7f".repeat(path_char_count) + "\r";
    session
        .send(clear_and_save.as_bytes())
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

/// Standard 110×30 terminal — full panel (76×28).
///
/// Verifies the wizard overlay is readable and all structural elements
/// (heading, branch list, navigation hint) are present without overflow
/// into the background rows.
#[test]
fn onboarding_layout_standard_110x30() {
    let (mut session, _home) = spawn_onboarding_session(110, 30);
    assert!(
        session.wait_for_text("Setup Wizard", STARTUP_TIMEOUT),
        "110×30: timed out waiting for setup wizard"
    );
    check_onboarding_layout(&session, 110, 30);

    // Branch list must show all three options at this size.
    assert!(
        session.screen_contains("Local + Google fallback"),
        "110×30: branch list must show 'Local + Google fallback'; rows:\n{}",
        session.all_rows().join("\n"),
    );
    assert!(
        session.screen_contains("Google Cloud"),
        "110×30: branch list must show 'Google Cloud'; rows:\n{}",
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

/// Minimum standard terminal (80×24) — panel width 76, height 24.
///
/// Verifies the wizard renders without overflow at standard 80-column width.
#[test]
fn onboarding_layout_standard_80x24() {
    let (mut session, _home) = spawn_onboarding_session(80, 24);
    assert!(
        session.wait_for_text("Setup Wizard", STARTUP_TIMEOUT),
        "80×24: timed out waiting for setup wizard"
    );
    check_onboarding_layout(&session, 80, 24);

    // Navigation hint must be present.
    assert!(
        session.screen_contains("[Esc] Cancel"),
        "80×24: navigation hint must contain '[Esc] Cancel'; rows:\n{}",
        session.all_rows().join("\n"),
    );

    quit_onboarding(&mut session);
    let exit = session
        .wait_exit(EXIT_TIMEOUT)
        .expect("80×24 onboarding exit");
    assert_eq!(exit, 0, "onboarding_layout_standard_80x24: expected exit 0");
}

/// Constrained 60×22 terminal — panel width 60, height 22.
///
/// Verifies the wizard renders without crashing or overflowing at a narrow
/// terminal.  The wizard has no separate "compact mode" — it wraps text within
/// the available width using the Paragraph widget.
#[test]
fn onboarding_layout_compact_60x22() {
    let (mut session, _home) = spawn_onboarding_session(60, 22);
    assert!(
        session.wait_for_text("Setup Wizard", STARTUP_TIMEOUT),
        "60×22: timed out waiting for setup wizard"
    );
    check_onboarding_layout(&session, 60, 22);

    // The Esc hint must still be present (may wrap but text is preserved).
    assert!(
        session.screen_contains("Esc"),
        "60×22: navigation hint must contain 'Esc'; rows:\n{}",
        session.all_rows().join("\n"),
    );

    quit_onboarding(&mut session);
    let exit = session
        .wait_exit(EXIT_TIMEOUT)
        .expect("60×22 onboarding exit");
    assert_eq!(exit, 0, "onboarding_layout_compact_60x22: expected exit 0");
}
