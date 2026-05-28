use super::*;

fn overflowing_pane() -> SubtitlePane {
    let mut pane = SubtitlePane::new();
    for idx in 0..6 {
        pane.push(SubtitlePair::new(
            format!("Source line {idx} with enough words to wrap around the viewport"),
            format!("Target line {idx} with enough translated words to wrap as well"),
        ));
    }
    pane
}

#[test]
fn subtitle_pair_stores_source_and_target() {
    let pair = SubtitlePair::new("Hello", "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}");
    assert_eq!(pair.source, "Hello");
    assert_eq!(pair.target, "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}");
}

#[test]
fn new_pane_is_empty_and_pinned() {
    let pane = SubtitlePane::new();
    assert!(pane.pairs.is_empty());
    assert!(pane.is_pinned());
    assert_eq!(pane.unread, 0);
}

#[test]
fn push_while_pinned_does_not_increment_unread() {
    let mut pane = SubtitlePane::new();
    pane.push(SubtitlePair::new("a", "b"));
    assert_eq!(pane.unread, 0);
    assert_eq!(pane.pairs.len(), 1);
}

#[test]
fn push_caps_committed_history_to_subtitle_history_cap() {
    let mut pane = SubtitlePane::new();

    for idx in 0..(SUBTITLE_HISTORY_CAP + 25) {
        pane.push(SubtitlePair::new(
            format!("source-{idx}"),
            format!("target-{idx}"),
        ));
    }

    assert_eq!(pane.pair_count(), SUBTITLE_HISTORY_CAP);
    assert_eq!(
        pane.committed_pair_at(0).map(|pair| pair.source.as_str()),
        Some("source-25")
    );
    let expected_target = format!("target-{}", SUBTITLE_HISTORY_CAP + 24);
    assert_eq!(
        pane.committed_pair_at(SUBTITLE_HISTORY_CAP - 1)
            .map(|pair| pair.target.as_str()),
        Some(expected_target.as_str())
    );
}

#[test]
fn push_while_scrolled_increments_unread() {
    let mut pane = overflowing_pane();
    pane.clamp_scroll(30, 6);
    pane.scroll_up(30, 6);
    let before_scroll = pane.scroll;
    pane.push(SubtitlePair::new("a", "b"));
    assert_eq!(pane.unread, 1);
    assert!(pane.scroll >= before_scroll);
}

#[test]
fn first_unread_push_keeps_viewport_anchored_when_badge_appears() {
    let mut pane = overflowing_pane();
    pane.clamp_scroll(30, 6);
    pane.scroll_up(30, 6);
    let before_scroll = pane.scroll;
    let pair = SubtitlePair::new(
        "New source line with enough words to wrap around once",
        "New target line with enough translated words to wrap around once",
    );
    let expected_delta = pane.visual_lines_for_pair(&pair, 30, true) as u16 + 1;

    pane.push(pair);

    assert_eq!(pane.unread, 1);
    assert_eq!(pane.scroll, before_scroll.saturating_add(expected_delta));
}

#[test]
fn scroll_to_bottom_clears_unread_and_pins() {
    let mut pane = SubtitlePane::new();
    pane.scroll = 3;
    pane.push(SubtitlePair::new("a", "b"));
    pane.scroll_to_bottom();
    assert!(pane.is_pinned());
    assert_eq!(pane.unread, 0);
}

#[test]
fn scroll_down_to_zero_clears_unread() {
    let mut pane = SubtitlePane::new();
    pane.scroll = 3;
    pane.push(SubtitlePair::new("a", "b"));
    pane.scroll_down(40, 6);
    assert!(pane.is_pinned());
    assert_eq!(pane.unread, 0);
}

#[test]
fn scroll_up_clamps_at_real_max_scroll() {
    let mut pane = overflowing_pane();
    let max_scroll = pane.max_scroll(30, 6);

    for _ in 0..100 {
        pane.scroll_up(30, 6);
    }

    assert_eq!(pane.scroll, max_scroll);
}

#[test]
fn scroll_to_top_uses_real_max_scroll_so_down_recovers_immediately() {
    let mut pane = overflowing_pane();
    let max_scroll = pane.max_scroll(30, 6);

    pane.scroll_to_top(30, 6);
    assert_eq!(pane.scroll, max_scroll);

    pane.scroll_down(30, 6);
    assert_eq!(pane.scroll, max_scroll.saturating_sub(3));
}

#[test]
fn wrap_to_lines_short_text_fits_on_one_line() {
    let lines = wrap_to_lines("[SRC] ", "Hello", 40, Color::Cyan);
    assert_eq!(lines.len(), 1);
}

#[test]
fn wrap_to_lines_long_text_produces_multiple_lines() {
    let text = "a".repeat(100);
    let lines = wrap_to_lines("[SRC] ", &text, 20, Color::Cyan);
    assert!(lines.len() > 1, "100 chars should not fit in 14 columns");
}

#[test]
fn wrap_to_lines_empty_text_returns_one_line() {
    let lines = wrap_to_lines("[SRC] ", "", 40, Color::Cyan);
    assert_eq!(lines.len(), 1);
}

#[test]
fn set_partial_first_call_is_never_a_flicker() {
    let mut pane = SubtitlePane::new();
    let is_flicker = pane.set_partial(SubtitlePair::new("Hello", ""));
    assert!(
        !is_flicker,
        "first set_partial with no previous partial must not report flicker"
    );
}

#[test]
fn set_partial_monotonic_growth_is_not_a_flicker() {
    let mut pane = SubtitlePane::new();
    pane.set_partial(SubtitlePair::new("Hello", ""));
    let is_flicker = pane.set_partial(SubtitlePair::new("Hello World", ""));
    assert!(
        !is_flicker,
        "extending 'Hello' -> 'Hello World' is monotonic growth, not a flicker"
    );
}

#[test]
fn set_partial_shrinking_text_is_a_flicker() {
    let mut pane = SubtitlePane::new();
    pane.set_partial(SubtitlePair::new("Hello World", ""));
    let is_flicker = pane.set_partial(SubtitlePair::new("Hello", ""));
    assert!(
        is_flicker,
        "'Hello World' -> 'Hello' is a regression (shrink) and must report flicker"
    );
}

#[test]
fn set_partial_text_replacement_is_a_flicker() {
    let mut pane = SubtitlePane::new();
    pane.set_partial(SubtitlePair::new("Hello", ""));
    let is_flicker = pane.set_partial(SubtitlePair::new("World", ""));
    assert!(
        is_flicker,
        "'Hello' -> 'World' is a non-monotonic replacement and must report flicker"
    );
}

#[test]
fn set_partial_same_text_is_not_a_flicker() {
    let mut pane = SubtitlePane::new();
    pane.set_partial(SubtitlePair::new("Hello", ""));
    let is_flicker = pane.set_partial(SubtitlePair::new("Hello", ""));
    assert!(
        !is_flicker,
        "identical text ('Hello' -> 'Hello') starts-with itself so is not a flicker"
    );
}

#[test]
fn set_partial_empty_previous_is_not_a_flicker() {
    let mut pane = SubtitlePane::new();
    pane.set_partial(SubtitlePair::new("", ""));
    let is_flicker = pane.set_partial(SubtitlePair::new("Hello", ""));
    assert!(
        !is_flicker,
        "empty previous partial must never trigger flicker (no content to regress from)"
    );
}

#[test]
fn set_partial_does_not_scroll_and_returns_flicker_state() {
    let mut pane = SubtitlePane::new();
    for i in 0..5 {
        pane.push(SubtitlePair::new(
            format!("Source {i}"),
            format!("Target {i}"),
        ));
    }
    pane.clamp_scroll(80, 10);
    pane.scroll_up(80, 10);
    let scroll_before = pane.scroll_value_for_test();

    pane.set_partial(SubtitlePair::new("Long text here", ""));
    let is_flicker = pane.set_partial(SubtitlePair::new("Short", ""));
    assert!(is_flicker, "shrink must be detected as flicker");
    assert_eq!(
        pane.scroll_value_for_test(),
        scroll_before,
        "set_partial must not change scroll position even when flicker is detected"
    );
}
