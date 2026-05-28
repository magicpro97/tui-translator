use super::*;

// ── DM-04: dual-pane TUI (issue #380) ────────────────────────────────────────

#[test]
fn focused_pane_toggles_a_to_b_and_back() {
    let state = AppState::new();
    // Single-slot: toggle is a no-op.
    assert_eq!(state.focused_pane_index(), 0);
    state.toggle_pane_focus();
    assert_eq!(
        state.focused_pane_index(),
        0,
        "single-slot toggle should be no-op"
    );

    // Wire slot B so toggle becomes active.
    let b_pane = Arc::new(Mutex::new(SubtitlePane::new()));
    state.wire_slot_b(b_pane, "en-US".to_string(), "google".to_string());
    assert_eq!(state.focused_pane_index(), 0);
    state.toggle_pane_focus();
    assert_eq!(state.focused_pane_index(), 1, "focus should move to B");
    state.toggle_pane_focus();
    assert_eq!(state.focused_pane_index(), 0, "focus should return to A");
}

#[test]
fn has_slot_b_reflects_wire_state() {
    let state = AppState::new();
    assert!(!state.has_slot_b());
    let b_pane = Arc::new(Mutex::new(SubtitlePane::new()));
    state.wire_slot_b(b_pane, "vi".to_string(), "google".to_string());
    assert!(state.has_slot_b());
}

#[test]
fn dual_pane_wide_renders_both_panes() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(160, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = AppState::new();

    {
        let mut pane = state.subtitle_pane.lock().unwrap();
        pane.push(SubtitlePair::new(
            "Hello from A".to_string(),
            "Xin chào từ A".to_string(),
        ));
    }

    let b_pane = Arc::new(Mutex::new(SubtitlePane::new()));
    {
        let mut bp = b_pane.lock().unwrap();
        bp.push(SubtitlePair::new(
            "Hello from B".to_string(),
            "Xin chào từ B".to_string(),
        ));
    }
    *state.slot_a_provider_name.lock().unwrap() = "google".to_string();
    state.wire_slot_b(
        Arc::clone(&b_pane),
        "en-US".to_string(),
        "local".to_string(),
    );

    terminal
        .draw(|frame| draw_ui(frame, &state, 0.0, false, 0.0))
        .unwrap();

    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();

    assert!(
        rendered.contains("[A]"),
        "wide dual-pane: pane A label should be visible"
    );
    assert!(
        rendered.contains("[B]"),
        "wide dual-pane: pane B label should be visible"
    );
}

#[test]
fn dual_pane_wide_renders_per_slot_error_status() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(160, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = AppState::new();
    let b_pane = Arc::new(Mutex::new(SubtitlePane::new()));

    *state.slot_a_provider_name.lock().unwrap() = "google".to_string();
    state.wire_slot_b(
        Arc::clone(&b_pane),
        "en-US".to_string(),
        "local".to_string(),
    );
    *state.slot_a_error_status_label.lock().unwrap() = "auth: bad key".to_string();
    *state.slot_b_error_status_label.lock().unwrap() = "error: timeout".to_string();

    terminal
        .draw(|frame| draw_ui(frame, &state, 0.0, false, 0.0))
        .unwrap();

    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();

    assert!(
        rendered.contains("auth: bad key"),
        "wide dual-pane: slot A error status should be visible"
    );
    assert!(
        rendered.contains("error: timeout"),
        "wide dual-pane: slot B error status should be visible"
    );
}

#[test]
fn dual_pane_narrow_renders_only_focused_pane() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = AppState::new();

    let b_pane = Arc::new(Mutex::new(SubtitlePane::new()));
    state.wire_slot_b(
        Arc::clone(&b_pane),
        "en-US".to_string(),
        "google".to_string(),
    );

    terminal
        .draw(|frame| draw_ui(frame, &state, 0.0, false, 0.0))
        .unwrap();

    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();

    assert!(
        rendered.contains("[A]"),
        "narrow dual-pane: focused A indicator should be visible"
    );
}

#[test]
fn dual_pane_narrow_focused_b_shows_b_pane() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = AppState::new();

    let b_pane = Arc::new(Mutex::new(SubtitlePane::new()));
    state.wire_slot_b(
        Arc::clone(&b_pane),
        "en-US".to_string(),
        "google".to_string(),
    );
    state.toggle_pane_focus();
    assert_eq!(state.focused_pane_index(), 1);

    terminal
        .draw(|frame| draw_ui(frame, &state, 0.0, false, 0.0))
        .unwrap();

    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();

    assert!(
        rendered.contains("[B]"),
        "narrow dual-pane focused B: B indicator should be visible"
    );
}

#[test]
fn single_slot_renders_default_subtitles_block() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(120, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = AppState::new();

    terminal
        .draw(|frame| draw_ui(frame, &state, 0.0, false, 0.0))
        .unwrap();

    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();

    assert!(
        rendered.contains("Subtitles"),
        "single-slot: default Subtitles block title should be present"
    );
}

#[test]
fn help_overlay_documents_tab_shortcut() {
    use ratatui::{backend::TestBackend, Terminal};
    let _i18n_guard = crate::i18n::lock_for_test();
    let backend = TestBackend::new(80, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| render_help_overlay(frame, ratatui::layout::Rect::new(0, 0, 80, 30), 0))
        .unwrap();

    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();

    assert!(
        rendered.contains("Tab"),
        "help overlay should document the Tab shortcut"
    );
}
