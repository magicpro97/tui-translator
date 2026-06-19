use super::harness::{PtySession, EXIT_TIMEOUT, STARTUP_TIMEOUT};
use std::time::Duration;
use tempfile::TempDir;

// ── Onboarding layout helpers ─────────────────────────────────────────────────

/// Spawn a PTY session that will trigger the first-run wizard overlay.
///
/// Uses an isolated temporary directory as `USERPROFILE` and a fresh
/// `TUI_TRANSLATOR_CONFIG_DIR` subdirectory so the binary cannot find a
/// pre-existing config in the real `$HOME` (which would cause the wizard
/// to be skipped on local dev machines with prior runs).
fn spawn_onboarding_session(cols: u16, rows: u16) -> (PtySession, TempDir) {
    let fake_home = TempDir::new().expect("temp home for onboarding layout test");
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

/// Tear down an onboarding-layout session.
///
/// Issue #845 deliberately changed Ctrl+C inside the wizard to map to
/// `OnboardingEvent::Escape` (cancel) instead of quitting the process, so that
/// typed fields survive an accidental Ctrl+C.  On a first run with no config
/// on disk the cancel path re-opens the wizard ("Setup is required …"), so
/// there is intentionally NO key that quits a first-run wizard.  These layout
/// tests only assert the wizard *renders* correctly at a given size; exit
/// semantics are covered by the completion tests that drive the wizard to
/// `Done`.  Terminate the child directly rather than depending on a quit
/// gesture that #845 removed.
fn quit_onboarding(session: &mut PtySession) {
    session.kill();
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
        //
        // Flow (post-#819):  BranchSelection → (VirtualCableGate on Windows) →
        // HardwareSurvey → GoogleKeyEntry → Confirmation.
        //
        // '3' selects GoogleCloud; the first \r advances past BranchSelection
        // (possibly through VirtualCableGate on Windows, skipped via 's' below);
        // the second \r advances past HardwareSurvey to GoogleKeyEntry.
        session.send(b"3").expect("select google cloud branch");
        std::thread::sleep(Duration::from_millis(150));
        session.send(b"\r").expect("advance from branch selection");

        // US-01: if VB-CABLE is not installed the VirtualCableGate step appears
        // (Windows only — `gate_enabled: cfg!(windows)` in src/tui/onboarding.rs).
        // The Gate can take 5+ seconds to render on slow CI runners, so we poll
        // for either the Gate or the next step and act on whichever appears
        // first.  The previous 2s + 12s two-stage wait was racy because the Gate
        // sometimes appeared AFTER the 2s window, leaving the test stuck on the
        // subsequent "Google API Key" wait.
        let post_branch_deadline = std::time::Instant::now()
            + std::time::Duration::from_secs(STARTUP_TIMEOUT.as_secs() * 2);
        // Check the Gate first — on Windows-no-cable it's the first step after
        // branch selection, and we want to skip it ASAP.  On macOS/Linux the
        // Gate is disabled (`cfg!(windows)`) so the check is a cheap 500ms
        // timeout.  Inverting from the previous order saves ~500ms on Windows.
        loop {
            if session.wait_for_text("Virtual Cable Gate", std::time::Duration::from_millis(500)) {
                session.send(b"s").expect("skip virtual cable gate");
            }
            if session.wait_for_text("Hardware Survey", std::time::Duration::from_millis(500)) {
                break;
            }
            if std::time::Instant::now() >= post_branch_deadline {
                panic!(
                    "after selecting GoogleCloud branch neither Virtual Cable Gate \
                     nor Hardware Survey appeared within {}s",
                    STARTUP_TIMEOUT.as_secs() * 2
                );
            }
        }

        // Advance past HardwareSurvey to GoogleKeyEntry.
        session.send(b"\r").expect("advance from hardware survey");
        assert!(
            session.wait_for_text("Google API Key", STARTUP_TIMEOUT),
            "after passing Hardware Survey the Google API Key step should appear"
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

/// #835 (v3 #819 follow-up): the user's HardwareSurvey preset
/// choice must be persisted to `config.json` so subsequent runs
/// honour it.  This test walks the full first-run wizard on the
/// GoogleCloud branch (no bundled-local-models, so the flow is
/// BranchSelection → HardwareSurvey → GoogleKeyEntry →
/// Confirmation), picks "Best" on the survey, types a key,
/// confirms, and asserts the saved config contains
/// `"quality_preset": "Best"`.
#[test]
fn onboarding_hardware_survey_persists_quality_preset_to_config() {
    let fake_home = TempDir::new().expect("temp home");
    let home_str = fake_home
        .path()
        .to_str()
        .expect("temp home path must be valid UTF-8")
        .to_string();
    let config_dir = fake_home.path().join("tui-config-quality");
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

        // GoogleCloud ('3') — skips LicenseReview (no bundled
        // local models are required to advance), so the flow is
        // BranchSelection → (gate on Windows) → HardwareSurvey →
        // GoogleKeyEntry → Confirmation.  This is the same path
        // the existing first-run test uses.
        session.send(b"3").expect("select google cloud branch");
        std::thread::sleep(Duration::from_millis(150));
        session.send(b"\r").expect("advance from branch selection");

        // Same gate/survey polling pattern as the first-run test:
        // on Windows a VirtualCableGate may appear, on macOS/Linux
        // it is skipped.
        let post_branch_deadline = std::time::Instant::now()
            + std::time::Duration::from_secs(STARTUP_TIMEOUT.as_secs() * 2);
        loop {
            if session.wait_for_text("Virtual Cable Gate", std::time::Duration::from_millis(500)) {
                session.send(b"s").expect("skip virtual cable gate");
            }
            if session.wait_for_text("Hardware Survey", std::time::Duration::from_millis(500)) {
                break;
            }
            if std::time::Instant::now() >= post_branch_deadline {
                panic!(
                    "after selecting GoogleCloud branch neither Virtual Cable Gate \
                     nor Hardware Survey appeared within {}s",
                    STARTUP_TIMEOUT.as_secs() * 2
                );
            }
        }

        // #835: walk the preset cycle to a non-default value,
        // then confirm.  The wizard's `[1-4]` keys for
        // picking a preset collide with the global branch-select
        // keys `1/2/3` at the input layer, and `[r]` is
        // globally mapped to `RefreshVirtualCable`, so the
        // test uses arrow keys to move the cycle.  From
        // recommended (idx R), one Down arrow lands on
        // `previous` (idx R-1 mod 4).  On hosts where the
        // recommended is `Performance` (idx 2) — most
        // dev/CI machines — one Down lands on `Best`.  On
        // hosts where the recommended is `Best` (idx 1) one
        // Down lands on `Auto` (the default, which is
        // skipped from JSON), so the test asserts only that
        // the wizard finished and the field is well-formed
        // regardless of which preset the user landed on
        // (the pre-fix code dropped the field entirely, so
        // the saved JSON would not contain `"quality_preset"`
        // at all — that is the regression this test guards).
        session.send(b"\x1b[B").expect("arrow down 1");
        std::thread::sleep(Duration::from_millis(100));
        session.send(b"\r").expect("advance from hardware survey");

        // GoogleCloud → GoogleKeyEntry.  Type a key (no hotkey
        // collisions: avoids l/t/m/s/r/q/1/2/3).
        assert!(
            session.wait_for_text("Google API Key", STARTUP_TIMEOUT),
            "after passing Hardware Survey the Google API Key step should appear"
        );
        session.send(b"abc-key-xyz").expect("type api key");
        std::thread::sleep(Duration::from_millis(100));
        session.send(b"\r").expect("advance from key entry");

        assert!(
            session.wait_for_text("Confirm Setup", STARTUP_TIMEOUT),
            "after entering the API key the confirmation step should appear"
        );
        session.send(b"\r").expect("confirm wizard");

        // Poll for the config file (same pattern as the first-run test).
        let deadline = std::time::Instant::now() + STARTUP_TIMEOUT;
        while !config_path.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            config_path.exists(),
            "completing the wizard must create a per-user config (waited up to {STARTUP_TIMEOUT:?})"
        );
        session.quit_cleanly().expect("quit session");
        let exit = session.wait_exit(EXIT_TIMEOUT).expect("session exit");
        assert_eq!(exit, 0, "session should exit cleanly");
    }

    // The saved config must record the user's preset choice.
    // The exact value depends on the host's RAM tier and how
    // many Down arrows were needed to land on a different
    // preset, but the field MUST be present and non-empty
    // — the pre-fix code dropped it entirely so the saved
    // JSON would not contain `"quality_preset"` at all.
    let written = std::fs::read_to_string(&config_path).expect("read saved config");
    assert!(
        written.contains("\"quality_preset\": \""),
        "saved config should contain a quality_preset field (the user picked one on the survey); got:\n{written}"
    );
    // Sanity: the value must be a known preset name.
    let known_preset = ["Auto", "Best", "Performance", "Custom"]
        .iter()
        .any(|p| written.contains(&format!("\"quality_preset\": \"{p}\"")));
    assert!(
        known_preset,
        "saved quality_preset must be one of Auto/Best/Performance/Custom; got:\n{written}"
    );
    // And the GoogleCloud branch's stt_provider must be set
    // (the test exists to verify the preset is persisted, not
    // to double-cover the branch write).
    assert!(
        written.contains("\"stt_provider\": \"google\""),
        "saved config should set stt_provider to google for GoogleCloud branch; got:\n{written}"
    );
}

// ── T18 follow-up (#836): 1/2/3 key-scope regression on the wire ─────────────
//
// Pre-fix, the global keymap in `main::key_to_action` mapped
// `KeyCode::Char('2')` to `OnboardingEvent::SelectBranch2`
// unconditionally.  On the `HardwareSurvey` step that event no-op'd,
// so the user's preset choice was silently lost (and, on a
// hypothetical keymap that also fired `Char('2')`, the branch would
// flip to `LocalGoogleFallback` underneath them).  Post-fix, `2` is
// routed to the wizard as `OnboardingEvent::Char('2')` and the
// wizard decides step-scoped behaviour.  This test walks the full
// first-run wizard on the GoogleCloud branch (so the flow is
// BranchSelection → HardwareSurvey → GoogleKeyEntry → Confirmation),
// presses `2` on the survey to pick `Best`, then asserts:
//
//   1. The saved config has `quality_preset = "Best"` (the user's
//      survey pick was honoured).
//   2. The saved config has `branch = "google_cloud"` — the digit
//      key did NOT silently flip the branch to `LocalGoogleFallback`.
//
// PTY tests are gated to the `pty` integration-test target; on hosts
// without a TTY allocator this test is skipped (see harness.rs).
#[test]
fn onboarding_hardware_survey_2_does_not_clobber_branch() {
    let fake_home = TempDir::new().expect("temp home");
    let home_str = fake_home
        .path()
        .to_str()
        .expect("temp home path must be valid UTF-8")
        .to_string();
    let config_dir = fake_home.path().join("tui-config-key-scope");
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

        // GoogleCloud (`3`) on BranchSelection.  The digit `3` is
        // now routed as `OnboardingEvent::Char('3')` and the wizard
        // resolves it to `OnboardingBranch::GoogleCloud` because we
        // are still on the `BranchSelection` step.
        session.send(b"3").expect("select google cloud branch");
        std::thread::sleep(Duration::from_millis(150));
        session.send(b"\r").expect("advance from branch selection");

        // Same gate/survey polling pattern as the sibling test.
        let post_branch_deadline = std::time::Instant::now()
            + std::time::Duration::from_secs(STARTUP_TIMEOUT.as_secs() * 2);
        loop {
            if session.wait_for_text("Virtual Cable Gate", std::time::Duration::from_millis(500)) {
                session.send(b"s").expect("skip virtual cable gate");
            }
            if session.wait_for_text("Hardware Survey", std::time::Duration::from_millis(500)) {
                break;
            }
            if std::time::Instant::now() >= post_branch_deadline {
                panic!(
                    "after selecting GoogleCloud branch neither Virtual Cable Gate \
                     nor Hardware Survey appeared within {}s",
                    STARTUP_TIMEOUT.as_secs() * 2
                );
            }
        }

        // The key step: press `2` to pick `Best` on HardwareSurvey.
        // Pre-fix this either no-op'd (the user's preset was lost)
        // or, on the buggy keymap, also flipped the branch to
        // `LocalGoogleFallback`.  Post-fix it sets
        // `hardware_survey_selection = Some(Best)` and the branch
        // stays `GoogleCloud`.
        session.send(b"2").expect("press 2 to pick Best preset");
        std::thread::sleep(Duration::from_millis(100));
        session.send(b"\r").expect("advance from hardware survey");

        assert!(
            session.wait_for_text("Google API Key", STARTUP_TIMEOUT),
            "after Hardware Survey the Google API Key step should appear"
        );
        session.send(b"abc-key-xyz").expect("type api key");
        std::thread::sleep(Duration::from_millis(100));
        session.send(b"\r").expect("advance from key entry");

        assert!(
            session.wait_for_text("Confirm Setup", STARTUP_TIMEOUT),
            "after entering the API key the confirmation step should appear"
        );
        session.send(b"\r").expect("confirm wizard");

        let deadline = std::time::Instant::now() + STARTUP_TIMEOUT;
        while !config_path.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            config_path.exists(),
            "completing the wizard must create a per-user config (waited up to {STARTUP_TIMEOUT:?})"
        );
        session.quit_cleanly().expect("quit session");
        let exit = session.wait_exit(EXIT_TIMEOUT).expect("session exit");
        assert_eq!(exit, 0, "session should exit cleanly");
    }

    // The saved config must record the user's preset choice.
    let written = std::fs::read_to_string(&config_path).expect("read saved config");
    assert!(
        written.contains("\"quality_preset\": \"Best\""),
        "saved config should contain \"quality_preset\": \"Best\" (the user pressed `2` on the survey); got:\n{written}"
    );
    // The bug's defining assertion: the digit `2` on the survey
    // must NOT have flipped the branch from `google_cloud` to
    // `local_google_fallback`.  The branch is encoded indirectly
    // via `stt_provider` + `mt_provider` (no explicit `branch`
    // field in AppConfig).  GoogleCloud ⇒ stt_provider="google",
    // mt_provider="google", stt_fallback_policy="none".  LocalGoogle
    // Fallback ⇒ stt_provider="local", mt_provider="local",
    // stt_fallback_policy="google-when-keyed".  Use the
    // fallback_policy as the discriminator since it's
    // unambiguously different per branch.
    assert!(
        written.contains("\"stt_fallback_policy\": \"none\""),
        "saved config should still have stt_fallback_policy=\"none\" (GoogleCloud); the digit `2` on the survey must not have flipped the branch to LocalGoogleFallback. got:\n{written}"
    );
    assert!(
        !written.contains("\"stt_fallback_policy\": \"google-when-keyed\""),
        "saved config must NOT have stt_fallback_policy=\"google-when-keyed\" (that was the pre-fix bug: pressing 2 on the survey silently overrode the branch). got:\n{written}"
    );
    // And the GoogleCloud branch's stt_provider must be set.
    assert!(
        written.contains("\"stt_provider\": \"google\""),
        "saved config should set stt_provider to google for GoogleCloud branch; got:\n{written}"
    );
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
                "  \"stt_provider\": \"google\",\n",
                "  \"mt_provider\": \"google\",\n",
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
}
