use super::harness::{PtySession, EXIT_TIMEOUT, STARTUP_TIMEOUT};
use std::time::Duration;
use tempfile::TempDir;

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

        let setup_input = format!("\t\tdemo-key\t\x08\x08\x08\x08\x08\x08file\t{fixture}\r", fixture = fixture_str);
        session.send(setup_input.as_bytes()).expect("save onboarding config");
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
