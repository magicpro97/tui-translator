//! Unit tests for `crate::tui::onboarding_render`.
//!
//! WP-25.05 (coverage-100% follow-up): the audit noted
//! `src/tui/onboarding_render.rs` had no test file.  Add
//! tests for the pure `render_wizard_lines` function.
//!
//! The function is a deterministic text renderer that maps
//! the `OnboardingWizardState` (a pure-data type) to a
//! `Vec<String>` of display lines.  The tests run without
//! a terminal, without a ratatui `Buffer`, and without any
//! I/O.

// The types used by the tests live in
// `src/tui/onboarding.rs`.  This test file is
// `#[path]`-included from `src/tui/onboarding_render.rs`,
// which itself is `#[path]`-included from two places:
//   (a) the main `tui-translator` bin (where `tui` is
//       declared as `pub mod` in `main.rs`), and
//   (b) the `onboarding_integration` integration-test
//       binary (where `tui` does NOT exist — instead, the
//       integration test does
//       `#[path = "../src/tui/onboarding.rs"] mod onboarding;`,
//       exposing the module as `crate::onboarding`).
//
// `onboarding_render.rs` has `use super::*;` at the top,
// which re-exports the types from the parent (whether
// the parent is `crate::tui` or `crate::onboarding`).
// Inheriting the same `use super::*;` here works in
// both contexts — the types come from the parent of this
// file, which is `onboarding_render`, which in turn
// inherits from its own parent (the bin's root or
// `crate::tui::onboarding`).
//
// Note: the local `#[path = "onboarding.rs"] mod onboarding;`
// approach (used in earlier revisions of this file) was
// rejected because it created a SECOND, distinct
// `OnboardingWizardState` type that mismatched the one
// `render_wizard_lines` expects, producing E0308
// "mismatched types" errors.
use super::*;

// ── Test helpers ────────────────────────────────────────────────────────────
//
// The `OnboardingBranch` / `OnboardingStep` / `OnboardingWizardState` /
// `LocalModelLicense` types live in `src/tui/onboarding.rs` and are imported
// above.  The `render_wizard_lines` function lives in `onboarding_render.rs`
// (this file's `super`) and is brought in via the `use super::*;` glob.
//
// `#![path]` is set in the integration-test binary (see
// `tests/onboarding_integration.rs`) so the `crate::tui::onboarding::`
// path resolves to the onboarding module there as well.  This means the
// same import works in both the binary and the integration-test crates.

fn empty_state() -> OnboardingWizardState {
    OnboardingWizardState::new(vec![], cable_probe_no_devices)
}

fn state_with_step(step: OnboardingStep) -> OnboardingWizardState {
    let mut s = OnboardingWizardState::new(vec![], cable_probe_no_devices);
    s.step = step;
    s
}

fn cable_probe_no_devices() -> Vec<String> {
    Vec::new()
}

fn sample_model(name: &str, license: &str) -> LocalModelLicense {
    LocalModelLicense {
        display_name: name.to_string(),
        license_text: license.to_string(),
    }
}

// ── Tests for the BranchSelection step ────────────────────────────────────────

#[test]
fn render_branch_selection_starts_with_header() {
    let lines = render_wizard_lines(&empty_state());
    assert!(!lines.is_empty());
    // The first non-empty line is the section header.
    let first = lines.iter().find(|l| !l.is_empty()).unwrap();
    assert!(
        first.contains("Setup Wizard"),
        "header must include 'Setup Wizard': {first}"
    );
}

#[test]
fn render_branch_selection_lists_all_three_branches() {
    let lines = render_wizard_lines(&empty_state());
    let text = lines.join("\n");
    assert!(text.contains("Local-only"));
    assert!(text.contains("Local + Google fallback"));
    assert!(text.contains("Google Cloud"));
}

#[test]
fn render_branch_selection_marks_default_branch() {
    // The default branch is LocalOnly; the marker should
    // appear next to it.
    let lines = render_wizard_lines(&empty_state());
    let text = lines.join("\n");
    // The marker is a Unicode triangle pointing to the
    // selected line.  Pin its presence, not its position,
    // since the renderer may reorder rows.
    assert!(
        text.contains("► Local-only"),
        "default branch must be marked with the triangle: {text}"
    );
}

#[test]
fn render_branch_selection_marker_follows_selected_branch() {
    let mut state = empty_state();
    state.branch = OnboardingBranch::GoogleCloud;
    let lines = render_wizard_lines(&state);
    let text = lines.join("\n");
    assert!(
        text.contains("► Google Cloud"),
        "selected branch must be marked: {text}"
    );
    assert!(
        !text.contains("► Local-only"),
        "unselected branches must not be marked: {text}"
    );
}

#[test]
fn render_branch_selection_ends_with_navigation_hint() {
    let lines = render_wizard_lines(&empty_state());
    let last = lines.last().expect("non-empty");
    assert!(
        last.contains("Enter"),
        "last line must include Enter: {last}"
    );
    assert!(last.contains("Esc"), "last line must include Esc: {last}");
}

// ── Tests for the VirtualCableGate step ──────────────────────────────────────

#[test]
fn render_virtual_cable_gate_empty_mentions_vb_cable() {
    let lines = render_wizard_lines(&state_with_step(OnboardingStep::VirtualCableGate {
        available: vec![],
    }));
    let text = lines.join("\n");
    assert!(text.contains("VB-CABLE"));
    assert!(text.contains("Refresh"));
    assert!(text.contains("Skip"));
}

#[test]
fn render_virtual_cable_gate_with_device_mentions_detected_label() {
    let lines = render_wizard_lines(&state_with_step(OnboardingStep::VirtualCableGate {
        available: vec!["CABLE Input (VB-Audio Virtual Cable)".to_string()],
    }));
    let text = lines.join("\n");
    assert!(text.contains("Detected"));
    assert!(text.contains("CABLE Input"));
}

// ── Tests for the LicenseReview step ──────────────────────────────────────────

#[test]
fn render_license_review_renders_license_text_verbatim() {
    let mut state = empty_state();
    state.local_models = vec![sample_model(
        "whisper-tiny",
        "MIT License\n\nCopyright (c) OpenAI",
    )];
    let lines = render_wizard_lines(&state_with_license(0, &state));
    let text = lines.join("\n");
    assert!(text.contains("MIT License"));
    assert!(text.contains("Copyright (c) OpenAI"));
    // Each license line must be prefixed with the box-
    // drawing character used by the renderer.
    assert!(text.contains("│  MIT License"));
}

fn state_with_license(model_index: usize, state: &OnboardingWizardState) -> OnboardingWizardState {
    let mut s = state.clone();
    s.step = OnboardingStep::LicenseReview { model_index };
    s
}
#[test]
fn render_license_review_out_of_range_index_uses_unknown_label() {
    // When `model_index` is out of range, the renderer
    // falls back to the "(unknown)" name and an empty
    // license body.  This is a defensive branch: the
    // caller is supposed to keep `model_index` in range,
    // but if it doesn't, the renderer must not panic.
    let lines = render_wizard_lines(&state_with_license(99, &empty_state()));
    let text = lines.join("\n");
    assert!(text.contains("(unknown)"));
}

#[test]
fn render_license_review_uses_one_based_index_in_header() {
    let mut state = empty_state();
    state.local_models = vec![
        sample_model("model-a", "License A"),
        sample_model("model-b", "License B"),
        sample_model("model-c", "License C"),
    ];
    let lines = render_wizard_lines(&state_with_license(0, &state));
    let text = lines.join("\n");
    // The header shows "License (1/3) — model-a" (1-based
    // index for the user).
    assert!(
        text.contains("License (1/3)"),
        "header must show 1-based index: {text}"
    );
    assert!(text.contains("model-a"));

    let lines = render_wizard_lines(&state_with_license(2, &state));
    let text = lines.join("\n");
    assert!(
        text.contains("License (3/3)"),
        "header must show 1-based index: {text}"
    );
    assert!(text.contains("model-c"));
}

// ── Tests for the GoogleKeyEntry step ────────────────────────────────────────

#[test]
fn render_google_key_entry_empty_buffer_shows_blank_cursor() {
    let lines = render_wizard_lines(&state_with_step(OnboardingStep::GoogleKeyEntry));
    let text = lines.join("\n");
    assert!(text.contains("Key"));
    // The masked key area is empty; the cursor (▌) is
    // rendered immediately after.
    assert!(text.contains("▌"), "cursor must be rendered: {text}");
}

#[test]
fn render_google_key_entry_with_buffer_masks_input() {
    let mut state = empty_state();
    state.key_buffer = "abcdefgh".to_string();
    state.step = OnboardingStep::GoogleKeyEntry;
    let lines = render_wizard_lines(&state);
    let text = lines.join("\n");
    // The raw key must NOT appear in the output (security:
    // the operator's screen could be recorded).
    assert!(!text.contains("abcdefgh"));
    // The masked version is 8 asterisks.
    assert!(text.contains("********"));
}

#[test]
fn render_google_key_entry_long_buffer_still_masked() {
    let mut state = empty_state();
    state.key_buffer = "a".repeat(50);
    state.step = OnboardingStep::GoogleKeyEntry;
    let lines = render_wizard_lines(&state);
    let text = lines.join("\n");
    // 50 chars must be 50 asterisks.
    let asterisks = text.matches('*').count();
    assert!(asterisks >= 50, "at least 50 asterisks: {asterisks}");
    // The raw 50-char string must not appear.
    assert!(!text.contains(&"a".repeat(50)));
}

// ── Tests for the Confirmation step ───────────────────────────────────────────

#[test]
fn render_confirmation_local_only_says_no_key_required() {
    let mut state = empty_state();
    state.branch = OnboardingBranch::LocalOnly;
    state.step = OnboardingStep::Confirmation;
    let lines = render_wizard_lines(&state);
    let text = lines.join("\n");
    assert!(text.contains("not required"));
}

#[test]
fn render_confirmation_google_branch_with_empty_key_says_prompt_at_startup() {
    let mut state = empty_state();
    state.branch = OnboardingBranch::GoogleCloud;
    state.key_buffer = String::new();
    state.step = OnboardingStep::Confirmation;
    let lines = render_wizard_lines(&state);
    let text = lines.join("\n");
    assert!(text.contains("none"));
    assert!(text.contains("prompt at startup"));
}

#[test]
fn render_confirmation_google_branch_with_key_shows_preview() {
    let mut state = empty_state();
    state.branch = OnboardingBranch::GoogleCloud;
    state.key_buffer = "AIzaSyABCDEFGHIJKLMNOPQRSTUVWXYZ".to_string();
    state.step = OnboardingStep::Confirmation;
    let lines = render_wizard_lines(&state);
    let text = lines.join("\n");
    // The renderer shows the first 4 chars + ellipsis.
    assert!(text.contains("AIza"));
    assert!(text.contains("…"));
    // The full key must NOT appear in the output.
    assert!(!text.contains("AIzaSyABCDEFGHIJKLMNOPQRSTUVWXYZ"));
}

#[test]
fn render_confirmation_short_key_still_masked() {
    // Keys shorter than 4 chars still get the preview
    // treatment (the renderer takes the first 4 chars
    // via `chars().take(4)`, which is at most 4 chars
    // even if the key is shorter).
    let mut state = empty_state();
    state.branch = OnboardingBranch::GoogleCloud;
    state.key_buffer = "ab".to_string();
    state.step = OnboardingStep::Confirmation;
    let lines = render_wizard_lines(&state);
    let text = lines.join("\n");
    // "ab" is 2 chars; the renderer shows "ab…".
    assert!(text.contains("ab…"));
}

// ── Tests for the PlatformParityNotice step ──────────────────────────────────

#[test]
fn render_platform_parity_notice_mentions_virtual_mic() {
    let lines = render_wizard_lines(&state_with_step(OnboardingStep::PlatformParityNotice));
    let text = lines.join("\n");
    assert!(text.contains("Virtual-mic") || text.contains("virtual-mic"));
    assert!(text.contains("macOS") || text.contains("Linux"));
}

#[test]
fn render_platform_parity_notice_mentions_speaker_only_fallback() {
    let lines = render_wizard_lines(&state_with_step(OnboardingStep::PlatformParityNotice));
    let text = lines.join("\n");
    assert!(text.contains("speaker-only"));
}

// ── Tests for the full coverage of all 6 steps ──────────────────────────────

#[test]
fn all_six_steps_render_non_empty_output() {
    let steps = [
        OnboardingStep::BranchSelection,
        OnboardingStep::VirtualCableGate { available: vec![] },
        OnboardingStep::LicenseReview { model_index: 0 },
        OnboardingStep::GoogleKeyEntry,
        OnboardingStep::Confirmation,
        OnboardingStep::PlatformParityNotice,
    ];
    for step in steps {
        let lines = render_wizard_lines(&state_with_step(step.clone()));
        assert!(
            !lines.is_empty(),
            "step {step:?} must produce non-empty output"
        );
        assert!(
            !lines.iter().all(|l| l.is_empty()),
            "step {step:?} must produce at least one non-empty line"
        );
    }
}
