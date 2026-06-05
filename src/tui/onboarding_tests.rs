//! Unit tests for `onboarding` (extracted from `onboarding.rs` for the
//! STD-02 module-budget refactor; behavior unchanged).
//!
//! Compiled only under `#[cfg(test)]` via the parent module's `#[path]` include.

use super::*;

fn make_wizard() -> OnboardingWizardState {
    OnboardingWizardState::new(vec![])
}

fn make_wizard_with_models() -> OnboardingWizardState {
    OnboardingWizardState::new(vec![
        LocalModelLicense {
            display_name: "Whisper Tiny".to_owned(),
            license_text: "MIT License\n\nCopyright (c) OpenAI".to_owned(),
        },
        LocalModelLicense {
            display_name: "OPUS-MT EN-VI".to_owned(),
            license_text: "Apache License 2.0\n\nCopyright (c) Helsinki-NLP".to_owned(),
        },
    ])
}

// ── Default state ─────────────────────────────────────────────────────────

#[test]
fn default_branch_is_local_only() {
    assert_eq!(make_wizard().branch, OnboardingBranch::LocalOnly);
}

#[test]
fn default_step_is_branch_selection() {
    assert_eq!(make_wizard().step, OnboardingStep::BranchSelection);
}

// ── Direct key selection 1 / 2 / 3 ──────────────────────────────────────

#[test]
fn key1_selects_local_only() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch2);
    let outcome = w.handle(OnboardingEvent::SelectBranch1);
    assert!(
        outcome.is_none(),
        "branch selection must not terminate wizard"
    );
    assert_eq!(w.branch, OnboardingBranch::LocalOnly);
}

#[test]
fn key2_selects_local_google_fallback() {
    let mut w = make_wizard();
    let outcome = w.handle(OnboardingEvent::SelectBranch2);
    assert!(outcome.is_none());
    assert_eq!(w.branch, OnboardingBranch::LocalGoogleFallback);
}

#[test]
fn key3_selects_google_cloud() {
    let mut w = make_wizard();
    let outcome = w.handle(OnboardingEvent::SelectBranch3);
    assert!(outcome.is_none());
    assert_eq!(w.branch, OnboardingBranch::GoogleCloud);
}

// ── Arrow navigation with wrap ────────────────────────────────────────────

#[test]
fn arrow_down_cycles_through_branches() {
    let mut w = make_wizard();
    assert_eq!(w.branch, OnboardingBranch::LocalOnly);
    w.handle(OnboardingEvent::ArrowDown);
    assert_eq!(w.branch, OnboardingBranch::LocalGoogleFallback);
    w.handle(OnboardingEvent::ArrowDown);
    assert_eq!(w.branch, OnboardingBranch::GoogleCloud);
    // Wrap
    w.handle(OnboardingEvent::ArrowDown);
    assert_eq!(w.branch, OnboardingBranch::LocalOnly);
}

#[test]
fn arrow_up_cycles_through_branches_with_wrap() {
    let mut w = make_wizard();
    assert_eq!(w.branch, OnboardingBranch::LocalOnly);
    // Wrap upward from first branch
    w.handle(OnboardingEvent::ArrowUp);
    assert_eq!(w.branch, OnboardingBranch::GoogleCloud);
    w.handle(OnboardingEvent::ArrowUp);
    assert_eq!(w.branch, OnboardingBranch::LocalGoogleFallback);
    w.handle(OnboardingEvent::ArrowUp);
    assert_eq!(w.branch, OnboardingBranch::LocalOnly);
}

// ── LocalOnly skips key-entry step ────────────────────────────────────────

#[test]
fn local_only_no_models_goes_directly_to_confirmation() {
    let mut w = make_wizard();
    assert_eq!(w.branch, OnboardingBranch::LocalOnly);
    w.handle(OnboardingEvent::Enter); // BranchSelection → Confirmation
    assert_eq!(
        w.step,
        OnboardingStep::Confirmation,
        "LocalOnly with no models must skip to Confirmation"
    );
}

#[test]
fn local_only_with_models_reviews_licenses_then_goes_to_confirmation() {
    let mut w = make_wizard_with_models();
    // BranchSelection → LicenseReview[0]
    w.handle(OnboardingEvent::Enter);
    assert_eq!(w.step, OnboardingStep::LicenseReview { model_index: 0 });
    // LicenseReview[0] → LicenseReview[1]
    w.handle(OnboardingEvent::Enter);
    assert_eq!(w.step, OnboardingStep::LicenseReview { model_index: 1 });
    // LicenseReview[1] → Confirmation (no key step for LocalOnly)
    w.handle(OnboardingEvent::Enter);
    assert_eq!(
        w.step,
        OnboardingStep::Confirmation,
        "LocalOnly must skip GoogleKeyEntry after license review"
    );
}

// ── LocalGoogleFallback reaches key-entry step ────────────────────────────

#[test]
fn local_google_fallback_no_models_reaches_key_entry() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch2);
    w.handle(OnboardingEvent::Enter);
    assert_eq!(w.step, OnboardingStep::GoogleKeyEntry);
}

#[test]
fn local_google_fallback_with_models_reaches_key_entry_after_licenses() {
    let mut w = make_wizard_with_models();
    w.handle(OnboardingEvent::SelectBranch2);
    w.handle(OnboardingEvent::Enter); // → LicenseReview[0]
    w.handle(OnboardingEvent::Enter); // → LicenseReview[1]
    w.handle(OnboardingEvent::Enter); // → GoogleKeyEntry
    assert_eq!(w.step, OnboardingStep::GoogleKeyEntry);
}

// ── GoogleCloud reaches key-entry step ────────────────────────────────────

#[test]
fn google_cloud_reaches_key_entry_step() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch3);
    w.handle(OnboardingEvent::Enter);
    assert_eq!(w.step, OnboardingStep::GoogleKeyEntry);
}

#[test]
fn google_cloud_with_local_models_skips_license_review() {
    let mut w = make_wizard_with_models();
    w.handle(OnboardingEvent::SelectBranch3);
    w.handle(OnboardingEvent::Enter);
    assert_eq!(
        w.step,
        OnboardingStep::GoogleKeyEntry,
        "GoogleCloud must skip LicenseReview even when local model list is non-empty"
    );
}

// ── Key entry accumulation ────────────────────────────────────────────────

#[test]
fn key_entry_accumulates_characters() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch3);
    w.handle(OnboardingEvent::Enter);
    w.handle(OnboardingEvent::Char('A'));
    w.handle(OnboardingEvent::Char('B'));
    w.handle(OnboardingEvent::Char('C'));
    assert_eq!(w.key_buffer, "ABC");
}

#[test]
fn backspace_removes_last_character() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch3);
    w.handle(OnboardingEvent::Enter);
    w.handle(OnboardingEvent::Char('X'));
    w.handle(OnboardingEvent::Char('Y'));
    w.handle(OnboardingEvent::Backspace);
    assert_eq!(w.key_buffer, "X");
}

#[test]
fn backspace_on_empty_buffer_is_noop() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch3);
    w.handle(OnboardingEvent::Enter);
    let outcome = w.handle(OnboardingEvent::Backspace);
    assert!(outcome.is_none());
    assert_eq!(w.key_buffer, "");
}

// ── Completion outcomes ───────────────────────────────────────────────────

#[test]
fn local_only_completion_produces_correct_patch() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::Enter); // → Confirmation
    let outcome = w.handle(OnboardingEvent::Enter); // → Done
    assert_eq!(
        outcome,
        Some(OnboardingOutcome::Done(OnboardingConfigPatch {
            branch: OnboardingBranch::LocalOnly,
            google_api_key: None,
        }))
    );
}

#[test]
fn google_cloud_completion_with_key_produces_correct_patch() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch3);
    w.handle(OnboardingEvent::Enter); // → GoogleKeyEntry
    w.handle(OnboardingEvent::Char('k'));
    w.handle(OnboardingEvent::Char('e'));
    w.handle(OnboardingEvent::Char('y'));
    w.handle(OnboardingEvent::Enter); // → Confirmation
    let outcome = w.handle(OnboardingEvent::Enter); // → Done
    assert_eq!(
        outcome,
        Some(OnboardingOutcome::Done(OnboardingConfigPatch {
            branch: OnboardingBranch::GoogleCloud,
            google_api_key: Some("key".to_owned()),
        }))
    );
}

#[test]
fn google_cloud_completion_empty_key_stays_on_key_entry() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch3);
    w.handle(OnboardingEvent::Enter); // → GoogleKeyEntry (empty key)
    w.handle(OnboardingEvent::Enter); // → Confirmation
    let outcome = w.handle(OnboardingEvent::Enter); // → Done
    assert_eq!(outcome, None);
    assert_eq!(w.step, OnboardingStep::GoogleKeyEntry);
}

// ── Cancellation ──────────────────────────────────────────────────────────

#[test]
fn esc_at_branch_selection_cancels_wizard() {
    let mut w = make_wizard();
    let outcome = w.handle(OnboardingEvent::Escape);
    assert_eq!(outcome, Some(OnboardingOutcome::Cancelled));
}

#[test]
fn esc_at_confirmation_navigates_back_not_cancel() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::Enter); // → Confirmation
    w.handle(OnboardingEvent::Escape); // → BranchSelection
    assert_eq!(w.step, OnboardingStep::BranchSelection);
}

// ── Forbidden runtime shortcut keys are no-ops ────────────────────────────

#[test]
fn ignored_events_are_noop_in_branch_selection() {
    let mut w = make_wizard();
    let initial_branch = w.branch;
    let initial_step = w.step.clone();
    for _ in 0..5 {
        let outcome = w.handle(OnboardingEvent::Ignored);
        assert!(outcome.is_none(), "Ignored must never terminate the wizard");
    }
    assert_eq!(w.branch, initial_branch, "Ignored must not change branch");
    assert_eq!(w.step, initial_step, "Ignored must not change step");
}

#[test]
fn ignored_events_are_noop_in_key_entry() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch3);
    w.handle(OnboardingEvent::Enter);
    let outcome = w.handle(OnboardingEvent::Ignored);
    assert!(outcome.is_none());
    assert_eq!(w.step, OnboardingStep::GoogleKeyEntry);
    assert_eq!(w.key_buffer, "");
}

#[test]
fn ignored_events_are_noop_in_license_review() {
    let mut w = make_wizard_with_models();
    w.handle(OnboardingEvent::Enter); // → LicenseReview[0]
    let outcome = w.handle(OnboardingEvent::Ignored);
    assert!(outcome.is_none());
    assert_eq!(w.step, OnboardingStep::LicenseReview { model_index: 0 });
}

#[test]
fn ignored_events_are_noop_in_confirmation() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::Enter); // → Confirmation
    let outcome = w.handle(OnboardingEvent::Ignored);
    assert!(outcome.is_none());
    assert_eq!(w.step, OnboardingStep::Confirmation);
}

// ── License text verbatim preservation ───────────────────────────────────

#[test]
fn license_text_stored_verbatim_in_state() {
    let long_text = "A".repeat(2000) + "\n" + &"B".repeat(2000);
    let w = OnboardingWizardState::new(vec![LocalModelLicense {
        display_name: "Test Model".to_owned(),
        license_text: long_text.clone(),
    }]);
    assert_eq!(
        w.local_models[0].license_text, long_text,
        "license_text must be stored exactly as given"
    );
}

#[test]
fn current_license_text_returns_verbatim() {
    let text = "MIT License\n\nCopyright (c) 2024";
    let mut w = OnboardingWizardState::new(vec![LocalModelLicense {
        display_name: "M".to_owned(),
        license_text: text.to_owned(),
    }]);
    w.handle(OnboardingEvent::Enter); // → LicenseReview[0]
    assert_eq!(w.current_license_text(), Some(text));
}

#[test]
fn render_license_review_preserves_all_lines() {
    let source_text = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5";
    let mut w = OnboardingWizardState::new(vec![LocalModelLicense {
        display_name: "Test Model".to_owned(),
        license_text: source_text.to_owned(),
    }]);
    w.handle(OnboardingEvent::Enter); // → LicenseReview[0]
    let rendered = render_wizard_lines(&w);
    for expected in source_text.lines() {
        assert!(
            rendered.iter().any(|l| l.contains(expected)),
            "render_wizard_lines must preserve every license line; missing: {expected:?}"
        );
    }
}

// ── Render helpers – confirmation shows branch label ──────────────────────

#[test]
fn render_confirmation_includes_branch_label() {
    let mut w = make_wizard(); // LocalOnly
    w.handle(OnboardingEvent::Enter); // → Confirmation
    let combined = render_wizard_lines(&w).join("\n");
    assert!(
        combined.contains("Local-only"),
        "Confirmation screen must include branch label; got:\n{combined}"
    );
}

#[test]
fn render_confirmation_shows_no_key_required_for_local_only() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::Enter);
    let combined = render_wizard_lines(&w).join("\n");
    assert!(
        combined.contains("not required"),
        "LocalOnly confirmation must say key is not required; got:\n{combined}"
    );
}

#[test]
fn render_confirmation_handles_multibyte_key_preview() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch3);
    w.handle(OnboardingEvent::Enter);
    for c in "aé🙂z".chars() {
        w.handle(OnboardingEvent::Char(c));
    }
    w.handle(OnboardingEvent::Enter);

    let combined = render_wizard_lines(&w).join("\n");
    assert!(
        combined.contains("aé🙂z…"),
        "Confirmation screen must preview multibyte keys without slicing UTF-8; got:\n{combined}"
    );
}

// ── Branch predicate helpers ──────────────────────────────────────────────

#[test]
fn local_only_does_not_require_google_key() {
    assert!(!OnboardingBranch::LocalOnly.requires_google_key());
}

#[test]
fn local_google_fallback_requires_google_key() {
    assert!(OnboardingBranch::LocalGoogleFallback.requires_google_key());
}

#[test]
fn google_cloud_requires_google_key() {
    assert!(OnboardingBranch::GoogleCloud.requires_google_key());
}

#[test]
fn local_only_uses_local_models() {
    assert!(OnboardingBranch::LocalOnly.uses_local_models());
}

#[test]
fn google_cloud_does_not_use_local_models() {
    assert!(!OnboardingBranch::GoogleCloud.uses_local_models());
}

// ── PlatformParityNotice — US-07 (#732) ───────────────────────────────────

/// Helper: construct a wizard starting at PlatformParityNotice.
fn make_platform_parity_wizard() -> OnboardingWizardState {
    OnboardingWizardState::new_platform_parity_notice()
}

#[test]
fn platform_parity_notice_needed_false_on_windows() {
    // Synthetic is_windows = true → never show, regardless of seen_at.
    let seen_at: Option<u8> = None;
    assert!(
        !OnboardingWizardState::platform_parity_notice_needed(&seen_at, true),
        "notice must NOT be needed on Windows even when seen_at is None"
    );
}

#[test]
fn platform_parity_notice_needed_true_on_non_windows_when_not_seen() {
    let seen_at: Option<u8> = None;
    assert!(
        OnboardingWizardState::platform_parity_notice_needed(&seen_at, false),
        "notice must be needed on non-Windows when seen_at is None"
    );
}

#[test]
fn platform_parity_notice_needed_false_on_non_windows_when_already_seen() {
    let seen_at: Option<u8> = Some(42);
    assert!(
        !OnboardingWizardState::platform_parity_notice_needed(&seen_at, false),
        "notice must NOT be needed when seen_at is Some(_)"
    );
}

#[test]
fn platform_parity_notice_constructor_starts_at_correct_step() {
    let w = make_platform_parity_wizard();
    assert_eq!(
        w.step,
        OnboardingStep::PlatformParityNotice,
        "new_platform_parity_notice must start at PlatformParityNotice step"
    );
}

#[test]
fn platform_parity_notice_enter_produces_dismissed_outcome() {
    let mut w = make_platform_parity_wizard();
    let outcome = w.handle(OnboardingEvent::Enter);
    assert_eq!(
        outcome,
        Some(OnboardingOutcome::PlatformParityNoticeDismissed),
        "Enter on PlatformParityNotice must produce PlatformParityNoticeDismissed"
    );
}

#[test]
fn platform_parity_notice_escape_produces_dismissed_outcome() {
    let mut w = make_platform_parity_wizard();
    let outcome = w.handle(OnboardingEvent::Escape);
    assert_eq!(
        outcome,
        Some(OnboardingOutcome::PlatformParityNoticeDismissed),
        "Escape on PlatformParityNotice must produce PlatformParityNoticeDismissed"
    );
}

#[test]
fn platform_parity_notice_ignored_event_is_noop() {
    let mut w = make_platform_parity_wizard();
    let outcome = w.handle(OnboardingEvent::Ignored);
    assert!(
        outcome.is_none(),
        "Ignored event must be a no-op on PlatformParityNotice"
    );
    assert_eq!(w.step, OnboardingStep::PlatformParityNotice);
}

#[test]
fn platform_parity_notice_render_contains_issue_reference() {
    let w = make_platform_parity_wizard();
    let combined = render_wizard_lines(&w).join("\n");
    assert!(
        combined.contains("#734"),
        "PlatformParityNotice render must mention tracking issue #734; got:\n{combined}"
    );
}

#[test]
fn platform_parity_notice_render_mentions_windows_only() {
    let w = make_platform_parity_wizard();
    let combined = render_wizard_lines(&w).join("\n");
    assert!(
        combined.contains("Windows-only"),
        "PlatformParityNotice render must mention Windows-only; got:\n{combined}"
    );
}
