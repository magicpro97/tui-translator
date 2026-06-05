//! US-07 PlatformParityNotice tests, extracted from onboarding_tests.rs to keep
//! it under the 600 LOC engineering-standards gate.

use super::*;

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
