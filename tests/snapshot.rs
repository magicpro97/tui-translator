//! Snapshot tests for the subtitle pane widget (issue #57) and the new
//! status/metrics widgets (issues #41, #51, #58-#66).
//!
//! Covers three representative terminal sizes: 80×24, 120×40, 200×50.
//! Also covers: narrow (60 col) and wide (130 col) adaptive layouts (issue #60),
//! new SttState variants (issue #41), always-shown hints bar (issue #65), and
//! narrow/short terminal fallback behavior (issues #185-#187).
//!
//! Run once to generate snapshots:
//!   INSTA_UPDATE=new cargo test --test snapshot
//!
//! Subsequent runs will compare against the stored `.snap` files in
//! `tests/snapshots/`.

#[path = "../src/metrics/mod.rs"]
mod metrics;

#[path = "../src/config/mod.rs"]
mod config;

#[path = "../src/tui/mod.rs"]
mod tui;

use metrics::SttState;
use ratatui::{backend::TestBackend, Terminal};
use tui::{
    draw_ui, expanded_metrics_height, render_auth_error_banner, render_help_overlay,
    render_language_prompt, truncate_device_name, AppState, ConfigEditorMode, ConfigEditorState,
    ControlHintsBar, StatusMetricsStrip, SubtitlePair, SubtitlePane,
};

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Render `pane` at the given terminal size and return a plain-text
/// representation suitable for snapshot comparison.
fn render_pane(pane: &SubtitlePane, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.size();
            frame.render_widget(pane, area);
        })
        .unwrap();
    buffer_to_string(terminal.backend().buffer())
}

/// Render a `StatusMetricsStrip` at the given size.
fn render_strip(strip: &StatusMetricsStrip<'_>, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            frame.render_widget(strip, frame.size());
        })
        .unwrap();
    buffer_to_string(terminal.backend().buffer())
}

/// Render a `ControlHintsBar` at the given size.
fn render_hints(bar: &ControlHintsBar, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            frame.render_widget(bar, frame.size());
        })
        .unwrap();
    buffer_to_string(terminal.backend().buffer())
}

/// Render the help overlay on a blank terminal of the given size.
fn render_help(width: u16, height: u16, scroll_offset: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.size();
            render_help_overlay(frame, area, scroll_offset);
        })
        .unwrap();
    buffer_to_string(terminal.backend().buffer())
}

/// Render the language prompt on a blank terminal of the given size.
fn render_lang_prompt(input: &str, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.size();
            render_language_prompt(frame, area, input);
        })
        .unwrap();
    buffer_to_string(terminal.backend().buffer())
}

/// Render the auth error banner on a blank terminal of the given size.
fn render_auth_banner(message: &str, restart: bool, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.size();
            // Simulate the y_offset as if this were anchored below a 6-row header.
            let subtitle_y_offset = 6u16.min(area.height.saturating_sub(5));
            render_auth_error_banner(frame, area, message, restart, subtitle_y_offset);
        })
        .unwrap();
    buffer_to_string(terminal.backend().buffer())
}

/// Render the full UI via `draw_ui` at the given terminal size.
fn render_full_ui(width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = AppState::new();
    terminal
        .draw(|frame| {
            draw_ui(frame, &state, 0.0, false, 0.0);
        })
        .unwrap();
    buffer_to_string(terminal.backend().buffer())
}

fn render_config_editor(editor: &ConfigEditorState, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            tui::render_config_editor(frame, frame.size(), editor);
        })
        .unwrap();
    buffer_to_string(terminal.backend().buffer())
}

fn render_overlay_then_ui(width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = AppState::new();
    {
        let mut pane = state.subtitle_pane.lock().unwrap();
        pane.push(SubtitlePair::new(
            "Overlay clear regression",
            "Overlay cleared",
        ));
    }
    let mut config_for_overlay = config::AppConfig::default();
    config_for_overlay.audio_source = "file".to_string();
    config_for_overlay.audio_file_path = Some(r"C:\capture\meeting.wav".to_string());
    state.open_config_editor(
        ConfigEditorMode::Settings,
        &config_for_overlay,
        std::path::Path::new(r"C:\Users\demo\.tui-translator\config.json"),
    );
    terminal
        .draw(|frame| {
            draw_ui(frame, &state, 0.0, false, 0.0);
        })
        .unwrap();
    state.close_config_editor();
    terminal
        .draw(|frame| {
            draw_ui(frame, &state, 0.0, false, 0.0);
        })
        .unwrap();
    buffer_to_string(terminal.backend().buffer())
}
///
/// Each row becomes one line; cells containing multi-column characters are
/// represented by their first Unicode scalar so every row has exactly `width`
/// characters.
fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
    let area = buf.area;
    let mut rows = Vec::with_capacity(area.height as usize);
    for y in 0..area.height {
        let row: String = (0..area.width)
            .map(|x| buf.get(x, y).symbol().chars().next().unwrap_or(' '))
            .collect();
        rows.push(row);
    }
    rows.join("\n")
}

// ── SubtitlePane — 80×24 ─────────────────────────────────────────────────────

#[test]
fn snapshot_80x24_empty() {
    let pane = SubtitlePane::new();
    insta::assert_snapshot!("80x24_empty", render_pane(&pane, 80, 24));
}

#[test]
fn snapshot_80x24_with_pairs() {
    let mut pane = SubtitlePane::new();
    pane.push(SubtitlePair::new(
        "Hello, how are you today?",
        "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}\u{3001}\u{4eca}\u{65e5}\u{306f}\u{304a}\u{5143}\u{6c17}\u{3067}\u{3059}\u{304b}\u{ff1f}",
    ));
    pane.push(SubtitlePair::new(
        "I am fine, thank you very much.",
        "\u{5143}\u{6c17}\u{3067}\u{3059}\u{3001}\u{3042}\u{308a}\u{304c}\u{3068}\u{3046}\u{3054}\u{3056}\u{3044}\u{307e}\u{3059}\u{3002}",
    ));
    insta::assert_snapshot!("80x24_with_pairs", render_pane(&pane, 80, 24));
}

#[test]
fn snapshot_80x24_long_line_wraps() {
    let mut pane = SubtitlePane::new();
    pane.push(SubtitlePair::new(
        "This is a very long source line that should definitely wrap at the terminal boundary here.",
        "These are the translated words that also need to wrap around the terminal width boundary.",
    ));
    insta::assert_snapshot!("80x24_long_line_wraps", render_pane(&pane, 80, 24));
}

// ── SubtitlePane — 120×40 ────────────────────────────────────────────────────

#[test]
fn snapshot_120x40_empty() {
    let pane = SubtitlePane::new();
    insta::assert_snapshot!("120x40_empty", render_pane(&pane, 120, 40));
}

#[test]
fn snapshot_120x40_with_pairs() {
    let mut pane = SubtitlePane::new();
    pane.push(SubtitlePair::new(
        "Welcome to the meeting. Let us begin with the agenda.",
        "\u{4f1a}\u{8b70}\u{3078}\u{3088}\u{3046}\u{3053}\u{305d}\u{3002}\u{30a2}\u{30b8}\u{30a7}\u{30f3}\u{30c0}\u{304b}\u{3089}\u{59cb}\u{3081}\u{307e}\u{3057}\u{3087}\u{3046}\u{3002}",
    ));
    pane.push(SubtitlePair::new(
        "First item: quarterly review.",
        "\u{6700}\u{521d}\u{306e}\u{8b70}\u{9898}\u{ff1a}\u{56db}\u{534a}\u{671f}\u{30ec}\u{30d3}\u{30e5}\u{30fc}\u{3002}",
    ));
    pane.push(SubtitlePair::new(
        "Second item: product roadmap for the next two years.",
        "\u{4e8c}\u{756a}\u{76ee}\u{306e}\u{8b70}\u{9898}\u{ff1a}\u{6b21}\u{306e}\u{4e8c}\u{5e74}\u{9593}\u{306e}\u{88fd}\u{54c1}\u{30ed}\u{30fc}\u{30c9}\u{30de}\u{30c3}\u{30d7}\u{3002}",
    ));
    insta::assert_snapshot!("120x40_with_pairs", render_pane(&pane, 120, 40));
}

// ── SubtitlePane — 200×50 ────────────────────────────────────────────────────

#[test]
fn snapshot_200x50_empty() {
    let pane = SubtitlePane::new();
    insta::assert_snapshot!("200x50_empty", render_pane(&pane, 200, 50));
}

#[test]
fn snapshot_200x50_with_pairs() {
    let mut pane = SubtitlePane::new();
    pane.push(SubtitlePair::new(
        "Good morning everyone. Today we will discuss the quarterly financial results in detail.",
        "\u{304a}\u{306f}\u{3088}\u{3046}\u{3054}\u{3056}\u{3044}\u{307e}\u{3059}\u{3002}\u{672c}\u{65e5}\u{306f}\u{56db}\u{534a}\u{671f}\u{306e}\u{8ca1}\u{52d9}\u{7d50}\u{679c}\u{306b}\u{3064}\u{3044}\u{3066}\u{8a73}\u{3057}\u{304f}\u{8a71}\u{3057}\u{5408}\u{3044}\u{307e}\u{3059}\u{3002}",
    ));
    pane.push(SubtitlePair::new(
        "Revenue was up fifteen percent compared to the same period last year.",
        "\u{58f2}\u{4e0a}\u{9ad8}\u{306f}\u{524d}\u{5e74}\u{540c}\u{671f}\u{6bd4}\u{3067}15\u{30d1}\u{30fc}\u{30bb}\u{30f3}\u{30c8}\u{5897}\u{52a0}\u{3057}\u{307e}\u{3057}\u{305f}\u{3002}",
    ));
    pane.push(SubtitlePair::new(
        "We expect continued growth in the coming months driven by new product launches.",
        "\u{65b0}\u{88fd}\u{54c1}\u{306e}\u{767a}\u{5c04}\u{306b}\u{3088}\u{308a}\u{3001}\u{4eca}\u{5f8c}\u{6570}\u{30f6}\u{6708}\u{3082}\u{7d99}\u{7d9a}\u{7684}\u{306a}\u{6210}\u{9577}\u{304c}\u{898b}\u{8fbc}\u{307e}\u{308c}\u{307e}\u{3059}\u{3002}",
    ));
    insta::assert_snapshot!("200x50_with_pairs", render_pane(&pane, 200, 50));
}

// ── StatusMetricsStrip — compact ─────────────────────────────────────────────

#[test]
fn snapshot_status_strip_compact_idle() {
    let stt = SttState::Idle;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: false,
        target_language: "vi".to_string(),
        pairs: 0,
        audio_secs: 0.0,
        cost_usd: 0.0,
        elapsed: "0:00".to_string(),
        show_restart: false,
        expanded: false,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    insta::assert_snapshot!("status_strip_compact_idle", render_strip(&strip, 120, 3));
}

#[test]
fn snapshot_status_strip_compact_listening_tts_on() {
    let stt = SttState::Listening;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: true,
        target_language: "vi".to_string(),
        pairs: 7,
        audio_secs: 42.0,
        cost_usd: 0.0168,
        elapsed: "1:23".to_string(),
        show_restart: false,
        expanded: false,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    insta::assert_snapshot!(
        "status_strip_compact_listening_tts_on",
        render_strip(&strip, 120, 3)
    );
}

#[test]
fn snapshot_status_strip_compact_restart_notice() {
    let stt = SttState::Idle;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: false,
        target_language: "vi".to_string(),
        pairs: 0,
        audio_secs: 0.0,
        cost_usd: 0.0,
        elapsed: "0:00".to_string(),
        show_restart: true,
        expanded: false,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    insta::assert_snapshot!(
        "status_strip_compact_restart_notice",
        render_strip(&strip, 120, 3)
    );
}

// ── StatusMetricsStrip — new STT states (issue #41) ──────────────────────────

#[test]
fn snapshot_status_strip_compact_sending() {
    let stt = SttState::Sending;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: false,
        target_language: "vi".to_string(),
        pairs: 3,
        audio_secs: 15.0,
        cost_usd: 0.006,
        elapsed: "0:30".to_string(),
        show_restart: false,
        expanded: false,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    insta::assert_snapshot!("status_strip_compact_sending", render_strip(&strip, 120, 3));
}

#[test]
fn snapshot_status_strip_compact_waiting() {
    let stt = SttState::Waiting;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: false,
        target_language: "vi".to_string(),
        pairs: 5,
        audio_secs: 25.0,
        cost_usd: 0.01,
        elapsed: "0:45".to_string(),
        show_restart: false,
        expanded: false,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    insta::assert_snapshot!("status_strip_compact_waiting", render_strip(&strip, 120, 3));
}

#[test]
fn snapshot_status_strip_compact_error() {
    let stt = SttState::Error("network timeout".to_string());
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: false,
        target_language: "vi".to_string(),
        pairs: 0,
        audio_secs: 0.0,
        cost_usd: 0.0,
        elapsed: "0:05".to_string(),
        show_restart: false,
        expanded: false,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    insta::assert_snapshot!("status_strip_compact_error", render_strip(&strip, 120, 3));
}

// ── StatusMetricsStrip — expanded ────────────────────────────────────────────

#[test]
fn snapshot_status_strip_expanded_idle() {
    let stt = SttState::Idle;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: false,
        target_language: "vi".to_string(),
        pairs: 0,
        audio_secs: 0.0,
        cost_usd: 0.0,
        elapsed: "0:00".to_string(),
        show_restart: false,
        expanded: true,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    insta::assert_snapshot!("status_strip_expanded_idle", render_strip(&strip, 80, 6));
}

#[test]
fn snapshot_status_strip_expanded_listening() {
    let stt = SttState::Listening;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: true,
        target_language: "vi".to_string(),
        pairs: 12,
        audio_secs: 180.0,
        cost_usd: 0.072,
        elapsed: "3:00".to_string(),
        show_restart: false,
        expanded: true,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    insta::assert_snapshot!(
        "status_strip_expanded_listening",
        render_strip(&strip, 80, 6)
    );
}

/// Expanded mode with an active cost warning: the warning row must be visible
/// and the block must be 7 rows tall (2 borders + 4 standard rows + 1 warning).
#[test]
fn snapshot_status_strip_expanded_with_warning() {
    let stt = SttState::Listening;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: true,
        target_language: "vi".to_string(),
        pairs: 20,
        audio_secs: 300.0,
        cost_usd: 0.75,
        elapsed: "5:00".to_string(),
        show_restart: false,
        expanded: true,
        cost_warning_usd: 0.50,
        cpu_pct: 12.0,
        ram_bytes: 64 * 1024 * 1024,
        net_kbps_tx: 8.0,
        net_kbps_rx: 32.0,
        e2e_latency_ms: Some(420),
        loss_pct: 0.0,
    };
    let height = strip.expanded_height();
    assert_eq!(
        height, 7,
        "expanded_height() must be 7 when over_threshold; got {height}"
    );
    insta::assert_snapshot!(
        "status_strip_expanded_with_warning",
        render_strip(&strip, 80, height)
    );
}

// ── StatusMetricsStrip — adaptive layout (issue #60) ─────────────────────────

/// Narrow terminal (< 80 cols): abbreviated labels.
#[test]
fn snapshot_status_strip_narrow_abbreviated() {
    let stt = SttState::Listening;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: true,
        target_language: "en".to_string(),
        pairs: 4,
        audio_secs: 20.0,
        cost_usd: 0.008,
        elapsed: "0:20".to_string(),
        show_restart: false,
        expanded: false,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    insta::assert_snapshot!(
        "status_strip_narrow_abbreviated",
        render_strip(&strip, 60, 3)
    );
}

/// Wide terminal (>= 120 cols): full labels with audio seconds.
#[test]
fn snapshot_status_strip_wide_full_labels() {
    let stt = SttState::Listening;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: false,
        target_language: "fr".to_string(),
        pairs: 8,
        audio_secs: 60.0,
        cost_usd: 0.024,
        elapsed: "1:00".to_string(),
        show_restart: false,
        expanded: false,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    insta::assert_snapshot!(
        "status_strip_wide_full_labels",
        render_strip(&strip, 130, 3)
    );
}

// ── ControlHintsBar — always shown (issue #65) ───────────────────────────────

#[test]
fn snapshot_hints_bar_tts_off() {
    let bar = ControlHintsBar { tts_on: false };
    insta::assert_snapshot!("hints_bar_tts_off", render_hints(&bar, 80, 1));
}

#[test]
fn snapshot_hints_bar_tts_on() {
    let bar = ControlHintsBar { tts_on: true };
    insta::assert_snapshot!("hints_bar_tts_on", render_hints(&bar, 80, 1));
}

/// Narrow terminal: abbreviated hints bar.
#[test]
fn snapshot_hints_bar_narrow() {
    let bar = ControlHintsBar { tts_on: false };
    insta::assert_snapshot!("hints_bar_narrow", render_hints(&bar, 60, 1));
}

#[test]
fn snapshot_config_editor_onboarding() {
    let mut editor = ConfigEditorState::from_config(
        &config::AppConfig::default(),
        std::path::Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Onboarding,
    );
    editor.google_api_key = "demo-key".to_string();
    editor.audio_file_path = r"C:\capture\fixture.wav".to_string();
    insta::assert_snapshot!(
        "config_editor_onboarding",
        render_config_editor(&editor, 90, 20)
    );
}

#[test]
fn snapshot_config_editor_onboarding_narrow() {
    let editor = ConfigEditorState::from_config(
        &config::AppConfig::default(),
        std::path::Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Onboarding,
    );
    insta::assert_snapshot!(
        "config_editor_onboarding_narrow",
        render_config_editor(&editor, 60, 16)
    );
}

#[test]
fn snapshot_config_editor_settings() {
    let mut editor = ConfigEditorState::from_config(
        &config::AppConfig::default(),
        std::path::Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.selected_field = 3;
    editor.audio_source = "file".to_string();
    editor.audio_file_path = r"C:\capture\meeting.wav".to_string();
    insta::assert_snapshot!(
        "config_editor_settings",
        render_config_editor(&editor, 90, 20)
    );
}

#[test]
fn snapshot_config_editor_settings_narrow() {
    let mut editor = ConfigEditorState::from_config(
        &config::AppConfig::default(),
        std::path::Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.selected_field = 1;
    insta::assert_snapshot!(
        "config_editor_settings_narrow",
        render_config_editor(&editor, 60, 16)
    );
}

#[test]
fn closing_config_editor_redraw_clears_overlay_pixels() {
    let rendered = render_overlay_then_ui(90, 20);
    assert!(
        !rendered.contains("Audio file path"),
        "closed settings overlay should not leave stale field labels on screen:\n{rendered}"
    );
    assert!(
        !rendered.contains("Edit the saved config"),
        "closed settings overlay should not leave stale intro text on screen:\n{rendered}"
    );
    assert!(
        rendered.contains("[SRC] Overlay clear regression"),
        "base subtitle content should redraw after closing the overlay:\n{rendered}"
    );
}

// ── Behavioral assertions (non-snapshot) ─────────────────────────────────────

/// The hints bar text always includes the required controls (issue #64/#65).
#[test]
fn hints_bar_contains_required_controls() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(120, 1);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            frame.render_widget(&ControlHintsBar { tts_on: false }, frame.size());
        })
        .unwrap();
    let rendered = buffer_to_string(terminal.backend().buffer());
    assert!(rendered.contains("Space"), "hints bar must include Space");
    assert!(
        rendered.contains("lang") || rendered.contains('L'),
        "hints bar must include L/lang"
    );
    assert!(
        rendered.contains("reload") || rendered.contains('R'),
        "hints bar must include R/reload"
    );
    assert!(
        rendered.contains("help") || rendered.contains('?'),
        "hints bar must include ?/help"
    );
    assert!(
        rendered.contains("quit") || rendered.contains('q'),
        "hints bar must include q/quit"
    );
}

/// SttState::Error carries its message through to the label (issue #41).
#[test]
fn stt_error_state_label_contains_message() {
    let stt = SttState::Error("auth failed".to_string());
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: false,
        target_language: "vi".to_string(),
        pairs: 0,
        audio_secs: 0.0,
        cost_usd: 0.0,
        elapsed: "0:00".to_string(),
        show_restart: false,
        expanded: false,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    let rendered = render_strip(&strip, 120, 3);
    assert!(
        rendered.contains("auth failed"),
        "compact strip must show error message; got: {rendered:?}"
    );
}

/// Narrow strip uses abbreviated labels (issue #60).
#[test]
fn narrow_strip_uses_abbreviated_labels() {
    let stt = SttState::Listening;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: false,
        target_language: "vi".to_string(),
        pairs: 0,
        audio_secs: 0.0,
        cost_usd: 0.0,
        elapsed: "0:00".to_string(),
        show_restart: false,
        expanded: false,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    let narrow = render_strip(&strip, 60, 3);
    let wide = render_strip(&strip, 120, 3);
    // Narrow version should not contain the full "listening" word.
    assert!(
        !narrow.contains("listening"),
        "narrow strip must abbreviate; got: {narrow:?}"
    );
    // Wide version should contain the full label.
    assert!(
        wide.contains("listening"),
        "wide strip must use full label; got: {wide:?}"
    );
}

/// In expanded mode the cost-warning line is actually rendered (not clipped)
/// when `cost_usd` exceeds `cost_warning_usd` (issue #74).
///
/// Verifies:
/// 1. `expanded_height()` returns 7 (not 6) when over threshold.
/// 2. `expanded_metrics_height(true, true)` matches that value.
/// 3. The rendered text at 7 rows contains the warning.
/// 4. The same strip at 6 rows (old, wrong height) does NOT show the warning.
#[test]
fn expanded_warning_renders_when_over_threshold() {
    let stt = SttState::Idle;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: false,
        target_language: "vi".to_string(),
        pairs: 5,
        audio_secs: 60.0,
        cost_usd: 1.20,
        elapsed: "2:00".to_string(),
        show_restart: false,
        expanded: true,
        cost_warning_usd: 1.00,
        cpu_pct: 5.0,
        ram_bytes: 32 * 1024 * 1024,
        net_kbps_tx: 4.0,
        net_kbps_rx: 16.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };

    // Height accounting must be 7 when warning is active.
    assert_eq!(
        strip.expanded_height(),
        7,
        "expanded_height() must return 7 when cost exceeds threshold"
    );
    assert_eq!(
        expanded_metrics_height(true, true),
        7,
        "expanded_metrics_height(expanded=true, over_threshold=true) must be 7"
    );
    assert_eq!(
        expanded_metrics_height(true, false),
        6,
        "expanded_metrics_height(expanded=true, over_threshold=false) must be 6"
    );
    assert_eq!(
        expanded_metrics_height(false, true),
        3,
        "expanded_metrics_height(expanded=false, ...) must always be 3"
    );

    // At the correct height of 7 the warning IS visible.
    let rendered_7 = render_strip(&strip, 80, 7);
    assert!(
        rendered_7.contains("Cost warning"),
        "expanded strip at 7 rows must show cost warning; got:\n{rendered_7}"
    );
    assert!(
        rendered_7.contains("1.20"),
        "cost warning must include the current cost value; got:\n{rendered_7}"
    );

    // At the old (wrong) height of 6 the 5th line is clipped — regression guard.
    let rendered_6 = render_strip(&strip, 80, 6);
    assert!(
        !rendered_6.contains("Cost warning"),
        "at 6 rows the warning row IS clipped — confirms fix was needed; got:\n{rendered_6}"
    );
}

/// Zero-state narrow strip snapshot — regression guard for zero cost at 60 cols.
#[test]
fn snapshot_status_strip_zero_state_narrow() {
    let stt = SttState::Idle;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: false,
        target_language: "vi".to_string(),
        pairs: 0,
        audio_secs: 0.0,
        cost_usd: 0.0,
        elapsed: "0:00".to_string(),
        show_restart: false,
        expanded: false,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    insta::assert_snapshot!(
        "status_strip_zero_state_narrow",
        render_strip(&strip, 60, 3)
    );
}

/// Zero-state expanded snapshot — regression guard.
#[test]
fn snapshot_status_strip_zero_state_expanded() {
    let stt = SttState::Idle;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: false,
        target_language: "vi".to_string(),
        pairs: 0,
        audio_secs: 0.0,
        cost_usd: 0.0,
        elapsed: "0:00".to_string(),
        show_restart: false,
        expanded: true,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    insta::assert_snapshot!(
        "status_strip_zero_state_expanded",
        render_strip(&strip, 80, 6)
    );
}

/// Short device names pass through unchanged.
#[test]
fn truncate_device_name_short_unchanged() {
    assert_eq!(truncate_device_name("Speakers", 32), "Speakers");
    assert_eq!(truncate_device_name("", 32), "");
    assert_eq!(truncate_device_name("A", 1), "A");
}

/// Names longer than max_cols are truncated with an ellipsis (U+2026).
#[test]
fn truncate_device_name_long_gets_ellipsis() {
    let long = "Speakers (Realtek High Definition Audio) [Loopback]";
    let result = truncate_device_name(long, 16);
    assert_eq!(
        unicode_width::UnicodeWidthStr::width(result.as_str()),
        16,
        "truncated result must be exactly max_cols columns wide; got {result:?}"
    );
    assert!(
        result.ends_with('\u{2026}'),
        "truncated result must end with ellipsis; got {result:?}"
    );
}

/// Wide glyphs must also be truncated by terminal display width, not char count.
#[test]
fn truncate_device_name_wide_chars_respects_display_width() {
    let result = truncate_device_name("日本語Audio", 5);
    assert_eq!(result, "日本…");
}

/// Truncation to 0 produces an empty string (no panic).
#[test]
fn truncate_device_name_zero_max_is_empty() {
    assert_eq!(truncate_device_name("anything", 0), "");
}

/// Truncation to 1 produces just the ellipsis for long input.
#[test]
fn truncate_device_name_one_max_is_just_ellipsis() {
    assert_eq!(truncate_device_name("long name", 1), "\u{2026}");
}

// ── Regression tests for issues #185-#187 ────────────────────────────────────

/// Narrow compact strip now uses "Lang:" instead of "L:" (#186).
#[test]
fn narrow_compact_strip_uses_lang_label() {
    let stt = SttState::Listening;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: true,
        target_language: "vi".to_string(),
        pairs: 3,
        audio_secs: 10.0,
        cost_usd: 0.004,
        elapsed: "0:10".to_string(),
        show_restart: false,
        expanded: false,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    let rendered = render_strip(&strip, 60, 3);
    assert!(
        rendered.contains("Lang:"),
        "narrow strip must spell out 'Lang:' not 'L:'; got: {rendered:?}"
    );
    assert!(
        !rendered.contains("| L:"),
        "narrow strip must not use bare 'L:' abbreviation; got: {rendered:?}"
    );
}

/// Narrow compact strip now uses "TTS:" instead of "T:" (#186).
#[test]
fn narrow_compact_strip_uses_tts_label() {
    let stt = SttState::Idle;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: false,
        target_language: "en".to_string(),
        pairs: 0,
        audio_secs: 0.0,
        cost_usd: 0.0,
        elapsed: "0:00".to_string(),
        show_restart: false,
        expanded: false,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    let rendered = render_strip(&strip, 60, 3);
    assert!(
        rendered.contains("TTS:"),
        "narrow strip must spell out 'TTS:' not 'T:'; got: {rendered:?}"
    );
    assert!(
        !rendered.contains("| T:"),
        "narrow strip must not use bare 'T:' abbreviation; got: {rendered:?}"
    );
}

/// Compact restart notice now says "restart required" (#186).
#[test]
fn compact_restart_notice_is_spelled_out() {
    let stt = SttState::Idle;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: false,
        target_language: "vi".to_string(),
        pairs: 0,
        audio_secs: 0.0,
        cost_usd: 0.0,
        elapsed: "0:00".to_string(),
        show_restart: true,
        expanded: false,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    let rendered = render_strip(&strip, 120, 3);
    assert!(
        rendered.contains("restart required"),
        "restart notice must say 'restart required'; got: {rendered:?}"
    );
    assert!(
        !rendered.contains("req'd"),
        "restart notice must not use 'req'd' abbreviation; got: {rendered:?}"
    );
}

/// Narrow snapshot at 30 cols: renders without panic and shows something (#185).
#[test]
fn snapshot_status_strip_very_narrow_30cols() {
    let stt = SttState::Listening;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: true,
        target_language: "ja".to_string(),
        pairs: 1,
        audio_secs: 5.0,
        cost_usd: 0.001,
        elapsed: "0:05".to_string(),
        show_restart: false,
        expanded: false,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    let rendered = render_strip(&strip, 30, 3);
    // Must not be empty and must render borders at minimum.
    assert!(
        !rendered.is_empty(),
        "very narrow strip must render something at 30 cols"
    );
    insta::assert_snapshot!("status_strip_very_narrow_30cols", rendered);
}

/// Help overlay at small size: fits without overflowing the terminal (#185).
#[test]
fn snapshot_help_overlay_narrow() {
    let rendered = render_help(40, 15, 0);
    assert!(!rendered.is_empty(), "help overlay must render at 40×15");
    insta::assert_snapshot!("help_overlay_narrow_40x15", rendered);
}

/// Language prompt at small size: renders with correct structure (#185).
#[test]
fn snapshot_lang_prompt_narrow() {
    let rendered = render_lang_prompt("ja", 40, 8);
    let rows: Vec<&str> = rendered.split('\n').collect();
    assert!(!rendered.is_empty(), "lang prompt must render at 40×8");
    assert!(
        rows[0].trim().is_empty() && rows[6].trim().is_empty() && rows[7].trim().is_empty(),
        "lang prompt at 40×8 must keep the top row and bottom padding rows blank; got: {rows:?}"
    );
    assert!(
        rows[1].contains("Change Language") && rows[5].contains("┘"),
        "lang prompt at 40×8 must stay vertically centered inside its 5-row panel; got: {rows:?}"
    );
    // The prompt instruction must still be visible.
    assert!(
        rendered.contains("Enter") || rendered.contains("Esc") || rendered.contains("language"),
        "lang prompt at 40×8 must include instruction text; got: {rendered:?}"
    );
    insta::assert_snapshot!("lang_prompt_narrow_40x8", rendered);
}

/// Auth error banner at a short terminal: stays within bounds (#185).
#[test]
fn auth_banner_clamped_to_terminal_bounds() {
    // Very short terminal — banner must not extend past the bottom.
    let height: u16 = 10;
    let rendered = render_auth_banner("invalid API key", false, 60, height);
    assert!(
        !rendered.is_empty(),
        "auth banner must render at 60×{height}"
    );
    // Count rendered rows — must equal height (no overflow).
    let row_count = rendered.lines().count() as u16;
    assert_eq!(
        row_count, height,
        "rendered output must have exactly {height} rows; got {row_count}"
    );
}

/// Snapshot for auth banner at narrow width (#185).
#[test]
fn snapshot_auth_banner_narrow() {
    let rendered = render_auth_banner("invalid API key", false, 60, 10);
    insta::assert_snapshot!("auth_banner_narrow_60x10", rendered);
}

/// Full UI at minimum usable size does not panic (#185).
///
/// MIN_USABLE_ROWS = 10 (title 3 + audio 3 + metrics-compact 3 + hints 1),
/// MIN_USABLE_COLS = 20 — rendering exactly at the threshold must not panic.
#[test]
fn full_ui_at_minimum_size_does_not_panic() {
    // Exactly at the boundary: full UI is rendered (no fallback).
    let rendered = render_full_ui(20, 10);
    assert!(
        !rendered.is_empty(),
        "draw_ui must render something at 20×10"
    );
    // One row below: fallback must appear instead of a broken layout.
    let rendered_below = render_full_ui(20, 9);
    assert!(
        rendered_below.contains("Resize") || rendered_below.contains("resize"),
        "draw_ui at 20×9 (below MIN_USABLE_ROWS=10) must show resize message; got: {rendered_below:?}"
    );
}

/// Full UI below minimum size shows fallback message instead of crashing (#185).
#[test]
fn full_ui_below_minimum_shows_fallback() {
    // Both dimensions below the minimum thresholds.
    let rendered = render_full_ui(10, 4);
    assert!(
        rendered.contains("Resize") || rendered.contains("resize"),
        "terminal below minimum must show resize message; got: {rendered:?}"
    );
}

/// Snapshot: full UI at minimum usable size (#185).
#[test]
fn snapshot_full_ui_too_small_fallback() {
    // Below minimum — should show fallback, not panic.
    let rendered = render_full_ui(15, 5);
    let rows: Vec<&str> = rendered.split('\n').collect();
    assert!(
        rows[0].contains("Resize terminal"),
        "full UI fallback at 15×5 must render the resize message in the visible frame; got: {rows:?}"
    );
    assert!(
        rows[1..].iter().all(|row| row.trim().is_empty()),
        "full UI fallback at 15×5 must leave the remaining rows blank after the message; got: {rows:?}"
    );
    insta::assert_snapshot!("full_ui_too_small_15x5", rendered);
}

/// Expanded metrics strip narrow mode uses "Lang:" label (#186).
#[test]
fn expanded_metrics_narrow_uses_lang_label() {
    let stt = SttState::Listening;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: false,
        target_language: "fr".to_string(),
        pairs: 5,
        audio_secs: 30.0,
        cost_usd: 0.01,
        elapsed: "0:30".to_string(),
        show_restart: false,
        expanded: true,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
    };
    let rendered = render_strip(&strip, 60, 6);
    assert!(
        rendered.contains("Lang:"),
        "expanded narrow strip must use 'Lang:' label; got: {rendered:?}"
    );
    assert!(
        !rendered.contains("L:fr"),
        "expanded narrow strip must not use bare 'L:' prefix; got: {rendered:?}"
    );
}

// ── Help overlay — issue #191 ─────────────────────────────────────────────────

/// At 80×24 the full content fits; no scroll indicator should appear.
#[test]
fn help_overlay_80x24_no_scroll_indicator() {
    let rendered = render_help(80, 24, 0);
    // The "?" title implies the static, non-scrolling title format.
    assert!(
        rendered.contains("press ? or Esc to close"),
        "80×24: expected static title with no scroll indicator; got:\n{rendered}"
    );
    // All shortcuts must be visible.
    assert!(
        rendered.contains("Space"),
        "80×24: 'Space' shortcut not visible; got:\n{rendered}"
    );
    assert!(
        rendered.contains("Ctrl+C"),
        "80×24: 'Ctrl+C' shortcut not visible; got:\n{rendered}"
    );
}

/// Snapshot: help overlay at 80×24, no scroll.
#[test]
fn snapshot_help_overlay_80x24() {
    insta::assert_snapshot!("help_overlay_80x24", render_help(80, 24, 0));
}

/// At 80×8 the panel is shorter than 13 content lines; a scroll indicator
/// must appear in the title.
#[test]
fn help_overlay_constrained_shows_scroll_indicator() {
    let rendered = render_help(80, 8, 0);
    // Scroll indicator contains the position format "[N/M]".
    assert!(
        rendered.contains('['),
        "constrained 80×8: expected scroll indicator '[N/M]' in title; got:\n{rendered}"
    );
}

/// At 80×8 with a non-zero scroll offset the indicator must reflect the
/// clamped position, not the raw (potentially out-of-bounds) value.
#[test]
fn help_overlay_constrained_scroll_clamped() {
    // A very large scroll_offset must not panic and must be clamped.
    let rendered_far = render_help(80, 8, 9999);
    // The rendered output must still contain the block border without crashing.
    assert!(
        rendered_far.contains("Help"),
        "constrained 80×8 with large offset: expected 'Help' in title; got:\n{rendered_far}"
    );
    // The last shortcut entry must be visible after scrolling to the bottom.
    assert!(
        rendered_far.contains("Esc"),
        "constrained 80×8 scrolled to bottom: the final shortcut entry must be visible; got:\n{rendered_far}"
    );
}

/// Snapshot: help overlay at 80×8 (constrained height), scroll at 0.
#[test]
fn snapshot_help_overlay_80x8_top() {
    insta::assert_snapshot!("help_overlay_80x8_top", render_help(80, 8, 0));
}

/// Snapshot: help overlay at 80×8, scrolled to bottom.
#[test]
fn snapshot_help_overlay_80x8_bottom() {
    insta::assert_snapshot!("help_overlay_80x8_bottom", render_help(80, 8, 9999));
}

/// At a very narrow terminal (30 cols) the panel still renders without panic.
#[test]
fn help_overlay_very_narrow_no_panic() {
    // Just verify no panic.
    let _ = render_help(30, 20, 0);
}

/// At the absolute minimum (panel clamped to 4 rows) the block renders without
/// crashing.
#[test]
fn help_overlay_minimum_height_no_panic() {
    // height=4 → MIN_H=4, inner_h=2 → max_scroll=11 → must not panic.
    let _ = render_help(80, 4, 0);
}
