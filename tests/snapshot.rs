//! Snapshot tests for the subtitle pane widget (issue #57).
//!
//! Covers three representative terminal sizes: 80×24, 120×40, 200×50.
//!
//! Run once to generate snapshots:
//!   INSTA_UPDATE=new cargo test --test snapshot
//!
//! Subsequent runs will compare against the stored `.snap` files in
//! `tests/snapshots/`.

#[path = "../src/tui/mod.rs"]
mod tui;

use ratatui::{backend::TestBackend, Terminal};
use tui::{SubtitlePair, SubtitlePane};

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

/// Convert a ratatui test buffer to a multi-line string.
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

// ── 80×24 ────────────────────────────────────────────────────────────────────

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

// ── 120×40 ───────────────────────────────────────────────────────────────────

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

// ── 200×50 ───────────────────────────────────────────────────────────────────

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
