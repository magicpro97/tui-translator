//! Unit tests for `segmentation` (extracted from `segmentation.rs` for the
//! STD-02 module-budget refactor; behavior unchanged).
//!
//! Compiled only under `#[cfg(test)]` via the parent module's `#[path]` include.

use super::*;

fn filter_and_commit(s: &mut SegmentStabilizer, transcript: String) -> Vec<String> {
    let items = s.filter_with_context(transcript, SegmentContext::default());
    for item in &items {
        if let Some(key) = item.dedup_key.as_deref() {
            s.record_committed_key(key);
        }
    }
    items.into_iter().map(|item| item.text).collect()
}

// ── normalize_for_dedup ───────────────────────────────────────────────────

#[test]
fn normalize_strips_trailing_ascii_punctuation() {
    assert_eq!(normalize_for_dedup("Hello world."), "hello world");
    assert_eq!(normalize_for_dedup("Hello world!"), "hello world");
    assert_eq!(normalize_for_dedup("Hello world?"), "hello world");
}

#[test]
fn normalize_strips_trailing_japanese_punctuation() {
    assert_eq!(normalize_for_dedup("こんにちは。"), "こんにちは");
    assert_eq!(normalize_for_dedup("ありがとう！"), "ありがとう");
    assert_eq!(normalize_for_dedup("本当に？"), "本当に");
}

#[test]
fn normalize_preserves_mid_sentence_punctuation() {
    assert_eq!(normalize_for_dedup("Hello, world"), "hello, world");
}

#[test]
fn normalize_lowercases_ascii() {
    assert_eq!(normalize_for_dedup("Hello World"), "hello world");
}

// ── split_at_boundary ─────────────────────────────────────────────────────

#[test]
fn split_no_split_needed_for_short_text() {
    let text = "短いテキスト";
    let (first, rest) = split_at_boundary(text, 40);
    assert_eq!(first, text);
    assert!(rest.is_none());
}

#[test]
fn split_at_last_japanese_punctuation_within_limit() {
    // 34 chars before the first 。; total well over 40 chars.
    let text =
        "これは長い日本語のテキストです。続きはこちらです。さらに追加の文章がここにあります。";
    let char_count = text.chars().count();
    assert!(char_count > 40, "pre-condition: text must be > 40 chars");

    let (first, rest) = split_at_boundary(text, 40);
    assert!(
        first.chars().count() <= 40,
        "first part must be ≤ 40 chars, got {}",
        first.chars().count()
    );
    // Split point must be at a punctuation character.
    let last_char = first.chars().last().unwrap();
    assert!(
        JP_SPLIT_CHARS.contains(&last_char),
        "first part must end at a safe boundary, got '{last_char}'"
    );
    assert!(rest.is_some(), "remainder must exist");
}

#[test]
fn split_falls_back_to_hard_limit_when_no_punctuation() {
    // 50 identical CJK characters with no punctuation.
    let text: String = "あ".repeat(50);
    let (first, rest) = split_at_boundary(&text, 40);
    assert_eq!(first.chars().count(), 40);
    assert_eq!(rest.unwrap().chars().count(), 10);
}

#[test]
fn split_all_repeats_until_every_chunk_is_within_limit() {
    let text = "あ".repeat((MAX_JAPANESE_CHARS * 2) + 7);
    let chunks = split_all_at_boundaries(&text, MAX_JAPANESE_CHARS);

    assert_eq!(chunks.len(), 3, "text over 2x limit must produce 3 chunks");
    assert!(
        chunks
            .iter()
            .all(|chunk| chunk.chars().count() <= MAX_JAPANESE_CHARS),
        "every split chunk must fit MAX_JAPANESE_CHARS"
    );
    assert_eq!(chunks.concat(), text);
}

/// T2 acceptance: Japanese text > 40 chars with punctuation splits at safe
/// boundary.
#[test]
fn split_t2_japanese_over_40_chars_splits_at_punctuation() {
    let text = "これは長い日本語のテキストです。さらに続きがあります。もっと長くしましょう。より多くの文字が必要です。";
    let char_count = text.chars().count();
    assert!(
        char_count > 40,
        "test text must be > 40 chars, got {char_count}"
    );

    let (first, rest) = split_at_boundary(text, 40);
    assert!(
        first.chars().count() <= 40,
        "first part must be ≤ 40 chars; got {}",
        first.chars().count()
    );
    assert!(rest.is_some(), "should have a remainder for T2");
}

// ── SegmentStabilizer ─────────────────────────────────────────────────────

/// T1: "Hello world" then "Hello world!" — second result dropped as
/// near-duplicate.
#[test]
fn t1_near_duplicate_with_terminal_punctuation_is_dropped() {
    let mut s = SegmentStabilizer::new();
    let first = filter_and_commit(&mut s, "Hello world".into());
    assert_eq!(first.len(), 1, "first occurrence must be committed");

    let second = filter_and_commit(&mut s, "Hello world!".into());
    assert!(
        second.is_empty(),
        "near-duplicate differing only in terminal punctuation must be dropped"
    );
}

#[test]
fn exact_duplicate_is_dropped() {
    let mut s = SegmentStabilizer::new();
    filter_and_commit(&mut s, "Hello world".into());
    let result = filter_and_commit(&mut s, "Hello world".into());
    assert!(result.is_empty(), "exact duplicate must be dropped");
}

#[test]
fn different_text_is_not_dropped() {
    let mut s = SegmentStabilizer::new();
    filter_and_commit(&mut s, "Hello world".into());
    let result = filter_and_commit(&mut s, "Goodbye world".into());
    assert_eq!(result.len(), 1, "distinct text must be committed");
}

#[test]
fn duplicate_is_dropped_within_window() {
    let mut s = SegmentStabilizer::new();
    filter_and_commit(&mut s, "alpha".into());
    filter_and_commit(&mut s, "beta".into());
    let result = filter_and_commit(&mut s, "alpha".into());
    assert!(result.is_empty(), "duplicate within window must be dropped");
}

#[test]
fn entry_is_accepted_after_window_eviction() {
    let mut s = SegmentStabilizer::new();
    // Push DEDUP_WINDOW_SIZE + 1 distinct items to evict the first one.
    for i in 0..=DEDUP_WINDOW_SIZE {
        filter_and_commit(&mut s, format!("sentence-{i} long enough text"));
    }
    // "sentence-0 long enough text" has now been evicted.
    let result = filter_and_commit(&mut s, "sentence-0 long enough text".into());
    assert_eq!(result.len(), 1, "evicted entry must be re-accepted");
}

#[test]
fn short_segment_is_buffered_not_committed() {
    let mut s = SegmentStabilizer::new();
    let result = filter_and_commit(&mut s, "hi".into());
    assert!(
        result.is_empty(),
        "transcript shorter than MIN_CHARS_FOR_COMMIT must be buffered"
    );
    assert!(
        s.pending_short.is_some(),
        "buffered text must be stored in pending_short"
    );
}

#[test]
fn short_segment_merges_with_next() {
    let mut s = SegmentStabilizer::new();
    // Buffer a short transcript.
    filter_and_commit(&mut s, "hi".into());
    // Next segment should receive the prepended buffered text.
    let result = filter_and_commit(&mut s, "how are you".into());
    assert_eq!(result.len(), 1, "merged segment must be committed");
    let transcript = &result[0];
    assert!(
        transcript.contains("hi"),
        "merged transcript must contain the buffered text: got '{transcript}'"
    );
    assert!(
        transcript.contains("how are you"),
        "merged transcript must contain the new text: got '{transcript}'"
    );
}

#[test]
fn flush_pending_returns_buffered_text_on_shutdown() {
    let mut s = SegmentStabilizer::new();
    filter_and_commit(&mut s, "ok".into()); // buffered (< MIN_CHARS_FOR_COMMIT)
    let flushed = s.flush_pending();
    assert!(
        flushed.is_some(),
        "flush_pending must return the buffered text"
    );
    assert_eq!(flushed.unwrap(), "ok");
}

#[test]
fn flush_pending_returns_none_when_empty() {
    let mut s = SegmentStabilizer::new();
    assert!(
        s.flush_pending().is_none(),
        "flush_pending on an empty buffer must return None"
    );
}

#[test]
fn long_japanese_transcript_is_split() {
    let text = "これは長い日本語のテキストです。さらに続きがあります。もっと長くしましょう。より多くの文字が必要です。";
    assert!(text.chars().count() > 40);

    let mut s = SegmentStabilizer::new();
    let result = filter_and_commit(&mut s, text.into());

    assert!(
        result.len() >= 2,
        "long Japanese text must produce multiple chunks"
    );
    for transcript in &result {
        assert!(
            transcript.chars().count() <= MAX_JAPANESE_CHARS,
            "split transcript must be ≤ {MAX_JAPANESE_CHARS} chars; got {}",
            transcript.chars().count()
        );
    }
}

#[test]
fn clear_resets_dedup_window_and_pending() {
    let mut s = SegmentStabilizer::new();
    filter_and_commit(&mut s, "alpha long enough text".into());
    filter_and_commit(&mut s, "hi".into()); // buffered
    s.clear();
    // After clear, "alpha long enough text" is not in the window.
    let result = filter_and_commit(&mut s, "alpha long enough text".into());
    assert_eq!(result.len(), 1, "cleared stabilizer must accept old text");
    assert!(
        s.pending_short.is_none(),
        "clear must discard the pending buffer"
    );
}
