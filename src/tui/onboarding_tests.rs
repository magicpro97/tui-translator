//! Unit tests for `onboarding` (extracted from `onboarding.rs` for the
//! STD-02 module-budget refactor; behavior unchanged).
//!
//! Compiled only under `#[cfg(test)]` via the parent module's `#[path]` include.

use super::*;

fn make_wizard() -> OnboardingWizardState {
    let mut w = OnboardingWizardState::new(vec![], noop_probe);
    w.gate_enabled = false;
    w
}

fn make_wizard_with_models() -> OnboardingWizardState {
    let mut w = OnboardingWizardState::new(
        vec![
            LocalModelLicense {
                display_name: "Whisper Tiny".to_owned(),
                license_text: "MIT License\n\nCopyright (c) OpenAI".to_owned(),
            },
            LocalModelLicense {
                display_name: "OPUS-MT EN-VI".to_owned(),
                license_text: "Apache License 2.0\n\nCopyright (c) Helsinki-NLP".to_owned(),
            },
        ],
        noop_probe,
    );
    w.gate_enabled = false;
    w
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
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → Confirmation
    assert_eq!(
        w.step,
        OnboardingStep::Confirmation,
        "LocalOnly with no models must skip to Confirmation"
    );
}

#[test]
fn local_only_with_models_reviews_licenses_then_goes_to_confirmation() {
    let mut w = make_wizard_with_models();
    // BranchSelection → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter);
    assert!(matches!(w.step, OnboardingStep::HardwareSurvey { .. }));
    // HardwareSurvey → LicenseReview[0]
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
        "LocalOnly with models must reach Confirmation after licenses"
    );
}

// ── LocalGoogleFallback reaches key-entry step ────────────────────────────

#[test]
fn local_google_fallback_no_models_reaches_key_entry() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch2);
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter);
    assert_eq!(w.step, OnboardingStep::GoogleKeyEntry);
}

#[test]
fn local_google_fallback_with_models_reaches_key_entry_after_licenses() {
    let mut w = make_wizard_with_models();
    w.handle(OnboardingEvent::SelectBranch2);
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → LicenseReview[0]
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → LicenseReview[1]
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → GoogleKeyEntry
    assert_eq!(w.step, OnboardingStep::GoogleKeyEntry);
}

// ── GoogleCloud reaches key-entry step ────────────────────────────────────

#[test]
fn google_cloud_reaches_key_entry_step() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch3);
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter);
    assert_eq!(w.step, OnboardingStep::GoogleKeyEntry);
}

#[test]
fn google_cloud_with_local_models_skips_license_review() {
    let mut w = make_wizard_with_models();
    w.handle(OnboardingEvent::SelectBranch3);
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
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
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
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
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
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
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter);
    let outcome = w.handle(OnboardingEvent::Backspace);
    assert!(outcome.is_none());
    assert_eq!(w.key_buffer, "");
}

// ── Completion outcomes ───────────────────────────────────────────────────

#[test]
fn local_only_completion_produces_correct_patch() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → Confirmation
    let outcome = w.handle(OnboardingEvent::Enter); // → Done
                                                    // #835: the recommended preset (Auto resolved against the
                                                    // default 0-byte SysCaps → Performance) is carried into the
                                                    // patch.
    assert_eq!(
        outcome,
        Some(OnboardingOutcome::Done(OnboardingConfigPatch {
            branch: OnboardingBranch::LocalOnly,
            google_api_key: None,
            virtual_mic_device: None,
            virtual_mic_skipped: false,
            quality_preset: Some(crate::quality_preset::QualityPreset::Performance),
        }))
    );
}

#[test]
fn google_cloud_completion_with_key_produces_correct_patch() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch3);
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → GoogleKeyEntry
    w.handle(OnboardingEvent::Char('k'));
    w.handle(OnboardingEvent::Char('e'));
    w.handle(OnboardingEvent::Char('y'));
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → Confirmation
    let outcome = w.handle(OnboardingEvent::Enter); // → Done
                                                    // #835: same — recommended preset reaches the patch.
    assert_eq!(
        outcome,
        Some(OnboardingOutcome::Done(OnboardingConfigPatch {
            branch: OnboardingBranch::GoogleCloud,
            google_api_key: Some("key".to_owned()),
            virtual_mic_device: None,
            virtual_mic_skipped: false,
            quality_preset: Some(crate::quality_preset::QualityPreset::Performance),
        }))
    );
}

#[test]
fn google_cloud_completion_empty_key_stays_on_key_entry() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch3);
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)

    w.handle(OnboardingEvent::Enter); // → GoogleKeyEntry (empty key)
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → Confirmation
    let outcome = w.handle(OnboardingEvent::Enter); // → Done
    assert_eq!(outcome, None);
    assert_eq!(w.step, OnboardingStep::GoogleKeyEntry);
}

// ── Cancellation ──────────────────────────────────────────────────────────

// Issue #852: Esc on BranchSelection no longer cancels
// the wizard immediately.  Instead it transitions to a
// ConfirmCancel step (the user gets one more chance to
// back out).  Only Esc or Enter on ConfirmCancel actually
// cancels.
#[test]
fn esc_at_branch_selection_transitions_to_confirm_cancel() {
    let mut w = make_wizard();
    let outcome = w.handle(OnboardingEvent::Escape);
    assert!(outcome.is_none(), "1st Esc must NOT cancel the wizard");
    assert!(
        matches!(w.step, OnboardingStep::ConfirmCancel),
        "1st Esc should transition to ConfirmCancel; got {:?}",
        w.step
    );
    // 2nd Esc (now on ConfirmCancel) actually cancels.
    let outcome = w.handle(OnboardingEvent::Escape);
    assert_eq!(outcome, Some(OnboardingOutcome::Cancelled));
}

#[test]
fn esc_at_confirmation_navigates_back_not_cancel() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
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
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter);
    let outcome = w.handle(OnboardingEvent::Ignored);
    assert!(outcome.is_none());
    assert_eq!(w.step, OnboardingStep::GoogleKeyEntry);
    assert_eq!(w.key_buffer, "");
}

#[test]
fn ignored_events_are_noop_in_license_review() {
    let mut w = make_wizard_with_models();
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → LicenseReview[0]
    let outcome = w.handle(OnboardingEvent::Ignored);
    assert!(outcome.is_none());
    assert_eq!(w.step, OnboardingStep::LicenseReview { model_index: 0 });
}

#[test]
fn ignored_events_are_noop_in_confirmation() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → Confirmation
    let outcome = w.handle(OnboardingEvent::Ignored);
    assert!(outcome.is_none());
    assert_eq!(w.step, OnboardingStep::Confirmation);
}

// ── License text verbatim preservation ───────────────────────────────────

#[test]
fn license_text_stored_verbatim_in_state() {
    let long_text = "A".repeat(2000) + "\n" + &"B".repeat(2000);
    let w = OnboardingWizardState::new(
        vec![LocalModelLicense {
            display_name: "Test Model".to_owned(),
            license_text: long_text.clone(),
        }],
        noop_probe,
    );
    assert_eq!(
        w.local_models[0].license_text, long_text,
        "license_text must be stored exactly as given"
    );
}

#[test]
fn current_license_text_returns_verbatim() {
    let text = "MIT License\n\nCopyright (c) 2024";
    let mut w = OnboardingWizardState::new(
        vec![LocalModelLicense {
            display_name: "M".to_owned(),
            license_text: text.to_owned(),
        }],
        noop_probe,
    );
    w.gate_enabled = false;
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → LicenseReview[0]
    assert_eq!(w.current_license_text(), Some(text));
}

#[test]
fn render_license_review_preserves_all_lines() {
    let source_text = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5";
    let mut w = OnboardingWizardState::new(
        vec![LocalModelLicense {
            display_name: "Test Model".to_owned(),
            license_text: source_text.to_owned(),
        }],
        noop_probe,
    );
    w.gate_enabled = false;
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
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
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
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
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → Confirmation
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
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
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

// PlatformParityNotice (US-07) tests extracted to sibling file (LOC gate).
#[path = "onboarding_platform_parity_tests.rs"]
mod platform_parity_tests;

// ── Cable-gate stubs + US-01 tests (WP-24 / issue #725) ─────────────────────

fn no_cable() -> Vec<String> {
    vec![]
}
fn has_cable() -> Vec<String> {
    vec!["CABLE Output".to_string()]
}
fn has_vbaudio() -> Vec<String> {
    vec!["VB-Audio Virtual Cable".to_string()]
}
fn has_cable_after_refresh() -> Vec<String> {
    vec!["CABLE Input".to_string()]
}
fn make_gated(probe: fn() -> Vec<String>) -> OnboardingWizardState {
    let mut w = OnboardingWizardState::new(vec![], probe);
    w.gate_enabled = true;
    w
}
#[test]
fn gate_disabled_skips_to_confirmation() {
    let mut w = make_wizard();
    w.gate_enabled = false;
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → Confirmation
    assert_eq!(w.step, OnboardingStep::Confirmation);
}
#[test]
fn non_windows_gate_never_produced() {
    let mut w = OnboardingWizardState::new(vec![], has_cable);
    w.gate_enabled = false;
    w.handle(OnboardingEvent::Enter);
    assert!(!matches!(w.step, OnboardingStep::VirtualCableGate { .. }));
}
#[test]
fn windows_no_cable_shows_gate_step() {
    let mut w = make_gated(no_cable);
    w.handle(OnboardingEvent::Enter);
    assert!(
        matches!(&w.step, OnboardingStep::VirtualCableGate { available } if available.is_empty())
    );
}
#[test]
fn windows_no_cable_gate_outcome_is_none() {
    assert!(make_gated(no_cable)
        .handle(OnboardingEvent::Enter)
        .is_none());
}
#[test]
fn windows_cable_found_skips_gate_and_populates_device() {
    let mut w = make_gated(has_cable);
    w.handle(OnboardingEvent::Enter);
    assert!(!matches!(w.step, OnboardingStep::VirtualCableGate { .. }));
    assert_eq!(w.virtual_mic_device, Some("CABLE Output".to_string()));
}
#[test]
fn windows_vb_audio_name_detected() {
    let mut w = OnboardingWizardState::new(vec![], has_vbaudio);
    w.gate_enabled = true;
    w.handle(OnboardingEvent::Enter);
    assert_eq!(
        w.virtual_mic_device,
        Some("VB-Audio Virtual Cable".to_string())
    );
}
#[test]
fn skip_sets_virtual_mic_skipped_and_no_device() {
    let mut w = make_gated(no_cable);
    w.handle(OnboardingEvent::Enter);
    w.handle(OnboardingEvent::SkipVirtualCable);
    assert!(w.virtual_mic_skipped);
    assert!(w.virtual_mic_device.is_none());
}
#[test]
fn skip_advances_past_gate() {
    let mut w = make_gated(no_cable);
    w.handle(OnboardingEvent::Enter); // → VirtualCableGate
    w.handle(OnboardingEvent::SkipVirtualCable); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → Confirmation
    assert_eq!(w.step, OnboardingStep::Confirmation);
}
#[test]
fn skip_outcome_stamped_in_patch() {
    // After the chain: gate → survey → confirmation → done.
    // The patch on the LAST `handle(Enter)` returning Some(Done) is
    // what we test below. The intermediate handles return None.
    let mut w = make_gated(no_cable);
    w.handle(OnboardingEvent::Enter); // → VirtualCableGate
    w.handle(OnboardingEvent::SkipVirtualCable); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → Confirmation
    let outcome = w.handle(OnboardingEvent::Enter);
    match outcome {
        Some(OnboardingOutcome::Done(patch)) => {
            assert!(patch.virtual_mic_skipped);
            assert!(patch.virtual_mic_device.is_none());
        }
        other => panic!("expected Done, got {other:?}"),
    }
}

// ── HardwareSurvey quality-preset plumbing (#835) ────────────────────────

/// v3 (#819, fix #835): the preset the user picks on
/// `HardwareSurvey` must reach the final `OnboardingConfigPatch` so
/// `main::apply_wizard_patch_to_config` can persist it to
/// `AppConfig::quality_preset`.  Pre-fix, the field was
/// `hardware_survey_selection: None` at `Confirmation` and the
/// user's choice was silently discarded.
#[test]
fn hardware_survey_best_preset_propagates_to_done_patch() {
    use crate::quality_preset::QualityPreset;
    let mut w = make_wizard();
    // BranchSelection → HardwareSurvey
    w.handle(OnboardingEvent::Enter);
    assert!(matches!(w.step, OnboardingStep::HardwareSurvey { .. }));
    // User picks "Best" (key '2') then confirms.
    w.handle(OnboardingEvent::Char('2'));
    w.handle(OnboardingEvent::Enter);
    // HardwareSurvey → Confirmation (LocalOnly, no models).
    assert_eq!(w.step, OnboardingStep::Confirmation);
    // Confirm → Done with quality_preset = Some(Best).
    let outcome = w.handle(OnboardingEvent::Enter);
    match outcome {
        Some(OnboardingOutcome::Done(patch)) => {
            assert_eq!(
                patch.quality_preset,
                Some(QualityPreset::Best),
                "HardwareSurvey 'Best' selection must reach the OnboardingConfigPatch"
            );
        }
        other => panic!("expected Done, got {other:?}"),
    }
}

/// #835 (consent-only review path): the consent-only flow skips
/// `HardwareSurvey` entirely, so the patch must carry
/// `quality_preset: None` (i.e. "don't touch the existing
/// `AppConfig::quality_preset`") rather than a synthesised value.
#[test]
fn consent_only_patch_has_quality_preset_none() {
    let mut w = OnboardingWizardState::new_consent_review(
        vec![LocalModelLicense {
            display_name: "Whisper Tiny".to_owned(),
            license_text: "MIT".to_owned(),
        }],
        OnboardingBranch::LocalOnly,
    );
    // LicenseReview[0] → Done (consent_only short-circuits).
    let outcome = w.handle(OnboardingEvent::Enter);
    match outcome {
        Some(OnboardingOutcome::Done(patch)) => {
            assert_eq!(
                patch.quality_preset, None,
                "consent-only flow must not synthesise a quality_preset"
            );
        }
        other => panic!("expected Done, got {other:?}"),
    }
}

/// #835 (GoogleCloud branch): the full GoogleCloud flow visits
/// `HardwareSurvey` then `GoogleKeyEntry` then `Confirmation`.
/// Picking "Performance" on the survey must reach the patch even
/// when the branch requires a Google API key.
#[test]
fn google_cloud_hardware_survey_performance_preset_propagates() {
    use crate::quality_preset::QualityPreset;
    let mut w = make_wizard();
    // Select GoogleCloud ('3'); advance to HardwareSurvey.
    w.handle(OnboardingEvent::SelectBranch3);
    w.handle(OnboardingEvent::Enter);
    assert!(matches!(w.step, OnboardingStep::HardwareSurvey { .. }));
    // User picks "Performance" (key '3') then confirms.
    w.handle(OnboardingEvent::Char('3'));
    w.handle(OnboardingEvent::Enter);
    // HardwareSurvey → GoogleKeyEntry (GoogleCloud skips LicenseReview).
    assert_eq!(w.step, OnboardingStep::GoogleKeyEntry);
    // Type a key and advance.
    for c in "abc-key-xyz".chars() {
        w.handle(OnboardingEvent::Char(c));
    }
    w.handle(OnboardingEvent::Enter);
    assert_eq!(w.step, OnboardingStep::Confirmation);
    let outcome = w.handle(OnboardingEvent::Enter);
    match outcome {
        Some(OnboardingOutcome::Done(patch)) => {
            assert_eq!(patch.branch, OnboardingBranch::GoogleCloud);
            assert_eq!(
                patch.quality_preset,
                Some(QualityPreset::Performance),
                "GoogleCloud flow must carry the HardwareSurvey preset through"
            );
        }
        other => panic!("expected Done, got {other:?}"),
    }
}
#[test]
fn refresh_re_enumerates_via_probe() {
    let mut w = make_gated(no_cable);
    w.handle(OnboardingEvent::Enter);
    w.cable_probe = has_cable_after_refresh;
    assert!(w.handle(OnboardingEvent::RefreshVirtualCable).is_none());
    match &w.step {
        OnboardingStep::VirtualCableGate { available } => assert!(!available.is_empty()),
        other => panic!("{other:?}"),
    }
}
#[test]
fn escape_at_gate_returns_to_branch_selection() {
    let mut w = make_gated(no_cable);
    w.handle(OnboardingEvent::Enter);
    w.handle(OnboardingEvent::Escape);
    assert_eq!(w.step, OnboardingStep::BranchSelection);
}
#[test]
fn render_no_cable_has_install_link() {
    let mut w = make_gated(no_cable);
    w.handle(OnboardingEvent::Enter);
    let s = render_wizard_lines(&w).join("\n");
    assert!(s.contains("https://vb-audio.com/Cable/") && s.contains("Skip"));
}
#[test]
fn render_cable_found_shows_device_and_confirm() {
    let mut w = make_gated(no_cable);
    w.handle(OnboardingEvent::Enter);
    w.cable_probe = has_cable;
    w.handle(OnboardingEvent::RefreshVirtualCable);
    let s = render_wizard_lines(&w).join("\n");
    assert!(s.contains("CABLE Output") && s.contains("[Enter]"));
}

// ── Key-scope regression (#836 follow-up) ─────────────────────────────────
//
// Background (subagent audit on PR #836, T18 follow-up):
//   `BranchSelection` and `HardwareSurvey` both use the digit keys
//   `1`/`2`/`3`.  The global keymap in `main::key_to_action`
//   unconditionally translated `1/2/3` to
//   `OnboardingEvent::SelectBranch1/2/3`, and the wizard only
//   matched the `SelectBranch*` variants on the `BranchSelection`
//   step.  Pressing `2` while the wizard was on `HardwareSurvey`
//   silently dropped the user's preset choice (it no-op'd in the
//   wizard) AND, on a hypothetical keymap that also fired
//   `Char('2')` for the survey, would have changed the branch
//   underneath the user.  The fix is to route `1`/`2`/`3`/`4`/`r`/
//   `R` to the wizard as `OnboardingEvent::Char(c)` and have the
//   wizard decide step-scoped behaviour.  These three tests pin
//   the wizard's per-step contract; the global-keymap side is
//   covered by `main::key_to_action` tests and the PTY test in
//   `tests/pty/onboarding_test.rs`.

/// On `BranchSelection`, `Char('2')` must change the branch to
/// `LocalGoogleFallback` AND must not mutate the
/// `hardware_survey_selection` field.  Pre-fix, the
/// `BranchSelection` arm of `apply_event` did not match
/// `Char('2')`, so the event fell through to `_ => None` and
/// the branch stayed `LocalOnly` (default).
#[test]
fn preset_key_2_on_branch_selection_ignores_preset_change() {
    let mut w = make_wizard();
    assert_eq!(w.step, OnboardingStep::BranchSelection);
    assert_eq!(w.branch, OnboardingBranch::LocalOnly);
    assert_eq!(w.hardware_survey_selection, None);
    w.handle(OnboardingEvent::Char('2'));
    assert_eq!(
        w.branch,
        OnboardingBranch::LocalGoogleFallback,
        "Char('2') on BranchSelection must select LocalGoogleFallback"
    );
    assert_eq!(
        w.hardware_survey_selection, None,
        "Char('2') on BranchSelection must not touch hardware_survey_selection"
    );
}

/// On `HardwareSurvey`, `Char('2')` must set
/// `hardware_survey_selection` to `Some(Best)` AND must not change
/// the branch.  Pre-fix, the wizard only set
/// `hardware_survey_selection` on the `Enter` that confirmed the
/// step, so pressing `2` alone left the field `None` (and, on
/// the buggy global keymap, would also have changed the branch
/// to `LocalGoogleFallback`).
#[test]
fn preset_key_2_on_hardware_survey_selects_best_and_ignores_branch_change() {
    use crate::quality_preset::QualityPreset;
    let mut w = make_wizard();
    // BranchSelection: set the branch via the digit-key path so the
    // test exercises the same surface the user touches.
    w.handle(OnboardingEvent::Char('2'));
    assert_eq!(w.branch, OnboardingBranch::LocalGoogleFallback);
    // BranchSelection → HardwareSurvey.
    w.handle(OnboardingEvent::Enter);
    assert!(matches!(w.step, OnboardingStep::HardwareSurvey { .. }));
    assert_eq!(w.hardware_survey_selection, None);
    w.handle(OnboardingEvent::Char('2'));
    assert_eq!(
        w.hardware_survey_selection,
        Some(QualityPreset::Best),
        "Char('2') on HardwareSurvey must set hardware_survey_selection to Some(Best)"
    );
    assert_eq!(
        w.branch,
        OnboardingBranch::LocalGoogleFallback,
        "Char('2') on HardwareSurvey must not change the branch"
    );
    // The inner step field must also reflect the new selection so
    // the on-screen highlight tracks the key press.
    match w.step {
        OnboardingStep::HardwareSurvey {
            selected_preset, ..
        } => {
            assert_eq!(selected_preset, QualityPreset::Best);
        }
        other => panic!("expected HardwareSurvey step, got {other:?}"),
    }
}

/// Covers the full `1`/`2`/`3`/`4` mapping on `HardwareSurvey`:
/// `1`→`Auto`, `2`→`Best`, `3`→`Performance`, `4`→`Custom`.  Each
/// press must set `hardware_survey_selection` so the user's
/// choice is captured even if they navigate back, and must never
/// change the branch.  Pre-fix, `hardware_survey_selection` was
/// only populated on `Enter`, so all four assertions on
/// `Some(QualityPreset::...)` failed.
#[test]
fn preset_key_3_on_hardware_survey_selects_performance() {
    use crate::quality_preset::QualityPreset;
    let mut w = make_wizard();
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey
    assert!(matches!(w.step, OnboardingStep::HardwareSurvey { .. }));
    let original_branch = w.branch;

    // 1 → Auto
    w.handle(OnboardingEvent::Char('1'));
    assert_eq!(
        w.hardware_survey_selection,
        Some(QualityPreset::Auto),
        "Char('1') on HardwareSurvey must set hardware_survey_selection to Some(Auto)"
    );
    assert_eq!(w.branch, original_branch);

    // 2 → Best
    w.handle(OnboardingEvent::Char('2'));
    assert_eq!(
        w.hardware_survey_selection,
        Some(QualityPreset::Best),
        "Char('2') on HardwareSurvey must set hardware_survey_selection to Some(Best)"
    );
    assert_eq!(w.branch, original_branch);

    // 3 → Performance
    w.handle(OnboardingEvent::Char('3'));
    assert_eq!(
        w.hardware_survey_selection,
        Some(QualityPreset::Performance),
        "Char('3') on HardwareSurvey must set hardware_survey_selection to Some(Performance)"
    );
    assert_eq!(w.branch, original_branch);

    // 4 → Custom
    w.handle(OnboardingEvent::Char('4'));
    assert_eq!(
        w.hardware_survey_selection,
        Some(QualityPreset::Custom),
        "Char('4') on HardwareSurvey must set hardware_survey_selection to Some(Custom)"
    );
    assert_eq!(w.branch, original_branch);
}

/// `Char('r')` / `Char('R')` on `BranchSelection` is a deliberate
/// no-op (the `r` key is reserved for the survey's re-recommend
/// action and the gate's refresh action).  This pins that
/// behaviour so the keymap fix (#836) does not accidentally
/// re-route `r` to a branch change.
#[test]
fn branch_selection_r_and_capital_r_are_noops() {
    let mut w = make_wizard();
    assert_eq!(w.branch, OnboardingBranch::LocalOnly);
    w.handle(OnboardingEvent::Char('r'));
    assert_eq!(w.branch, OnboardingBranch::LocalOnly);
    w.handle(OnboardingEvent::Char('R'));
    assert_eq!(w.branch, OnboardingBranch::LocalOnly);
    assert_eq!(w.hardware_survey_selection, None);
}

/// `Char('r')` / `Char('R')` on `HardwareSurvey` re-runs the
/// preset recommendation based on the current `sys_caps` (the
/// `Auto.resolve_for(caps)` call).  After the re-recommend the
/// top-level `hardware_survey_selection` must reflect the new
/// choice and the branch must stay put.
#[test]
fn hardware_survey_r_recommends_and_keeps_branch() {
    use crate::quality_preset::QualityPreset;
    let mut w = make_wizard();
    w.handle(OnboardingEvent::Char('2'));
    assert_eq!(w.branch, OnboardingBranch::LocalGoogleFallback);
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey
    assert!(matches!(w.step, OnboardingStep::HardwareSurvey { .. }));
    // User overrides the recommended preset to Best via `2`.
    w.handle(OnboardingEvent::Char('2'));
    assert_eq!(w.hardware_survey_selection, Some(QualityPreset::Best));
    // Pressing `r` (lowercase) and `R` (uppercase) re-recommends.
    w.handle(OnboardingEvent::Char('r'));
    assert_eq!(
        w.hardware_survey_selection,
        Some(QualityPreset::Auto.resolve_for(&w.sys_caps)),
        "Char('r') on HardwareSurvey must re-recommend the preset"
    );
    assert_eq!(
        w.branch,
        OnboardingBranch::LocalGoogleFallback,
        "Char('r') on HardwareSurvey must not change the branch"
    );
    w.handle(OnboardingEvent::Char('4')); // user picks Custom
    assert_eq!(w.hardware_survey_selection, Some(QualityPreset::Custom));
    w.handle(OnboardingEvent::Char('R'));
    assert_eq!(
        w.hardware_survey_selection,
        Some(QualityPreset::Auto.resolve_for(&w.sys_caps)),
        "Char('R') on HardwareSurvey must re-recommend the preset"
    );
}

/// `Char('r')` / `Char('R')` and `Char('s')` / `Char('S')` on
/// `VirtualCableGate` are the routed-char forms of
/// `RefreshVirtualCable` and `SkipVirtualCable`.  This pins the
/// route so the `main::key_to_action` change in #836 has a
/// regression guard.
#[test]
fn virtual_cable_gate_r_refreshes_and_s_skips() {
    let mut w = make_gated(no_cable);
    w.handle(OnboardingEvent::Enter); // → VirtualCableGate (no cable)
    assert!(matches!(w.step, OnboardingStep::VirtualCableGate { .. }));
    // Char('r') / Char('R') → refresh (no-op when no cable, but
    // the function should still match and not error).
    w.handle(OnboardingEvent::Char('r'));
    assert!(matches!(w.step, OnboardingStep::VirtualCableGate { .. }));
    w.handle(OnboardingEvent::Char('R'));
    assert!(matches!(w.step, OnboardingStep::VirtualCableGate { .. }));
    // Char('s') / Char('S') → skip the gate.
    w.handle(OnboardingEvent::Char('s'));
    assert!(
        w.virtual_mic_skipped,
        "Char('s') on VirtualCableGate must set virtual_mic_skipped"
    );
}

// Issue #852: pressing Esc on BranchSelection immediately
// cancelled the whole wizard.  Users accidentally exited
// without realising.  Now the first Esc transitions to a
// ConfirmCancel step, and only the second Esc (or Enter on
// ConfirmCancel) actually exits the wizard.
#[test]
fn esc_on_branch_selection_requires_double_press_to_cancel() {
    let mut w = make_wizard();
    // 1st Esc: should NOT cancel, should go to ConfirmCancel
    let outcome = w.handle(OnboardingEvent::Escape);
    assert!(outcome.is_none(), "1st Esc must not cancel the wizard");
    assert!(
        matches!(w.step, OnboardingStep::ConfirmCancel),
        "1st Esc should transition to ConfirmCancel; got {:?}",
        w.step
    );
    // Enter on ConfirmCancel: actually cancels
    let outcome = w.handle(OnboardingEvent::Enter);
    assert!(
        matches!(outcome, Some(OnboardingOutcome::Cancelled)),
        "Enter on ConfirmCancel should cancel; got {:?}",
        outcome
    );
    // 2nd Esc on ConfirmCancel: also cancels
    let mut w = make_wizard();
    w.handle(OnboardingEvent::Escape); // → ConfirmCancel
    let outcome = w.handle(OnboardingEvent::Escape);
    assert!(
        matches!(outcome, Some(OnboardingOutcome::Cancelled)),
        "2nd Esc on ConfirmCancel should cancel; got {:?}",
        outcome
    );
    // Any other key on ConfirmCancel: goes back to BranchSelection
    let mut w = make_wizard();
    w.handle(OnboardingEvent::Escape); // → ConfirmCancel
    w.handle(OnboardingEvent::Char('x')); // → BranchSelection (back)
    assert!(
        matches!(w.step, OnboardingStep::BranchSelection),
        "non-Enter/Esc key must go back to BranchSelection; got {:?}",
        w.step
    );
    // Then Enter: confirms branch, advances (not cancels)
    w.handle(OnboardingEvent::Enter);
    assert!(!matches!(w.step, OnboardingStep::BranchSelection));
}

#[test]
fn empty_key_on_confirmation_sets_error_message() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch3);
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → GoogleKeyEntry
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → Confirmation
    let outcome = w.handle(OnboardingEvent::Enter); // empty key → bounce
    assert_eq!(outcome, None);
    assert_eq!(w.step, OnboardingStep::GoogleKeyEntry);
    // New: error_message should be set so the user knows why
    assert_eq!(w.error_message.as_deref(), Some("API key is required"));
}

#[test]
fn successful_key_submission_clears_error_message() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch3);
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey
    w.handle(OnboardingEvent::Enter); // → GoogleKeyEntry
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey
    w.handle(OnboardingEvent::Enter); // → Confirmation
    w.handle(OnboardingEvent::Enter); // empty key → error
    assert!(w.error_message.is_some());
    // Now type a real key and advance
    w.handle(OnboardingEvent::Char('x'));
    assert!(w.error_message.is_none());
    w.handle(OnboardingEvent::Backspace);
    w.handle(OnboardingEvent::Char('A'));
    w.handle(OnboardingEvent::Char('B'));
    w.handle(OnboardingEvent::Enter); // → Confirmation
    w.handle(OnboardingEvent::Enter); // success → Done
    let outcome = w.handle(OnboardingEvent::Enter);
    assert!(matches!(outcome, Some(OnboardingOutcome::Done(_))));
}

// Issue #851 follow-up: license_scroll must reset when
// transitioning to a new model's license.  Without this,
// scrolling 30 lines into license[0] would leave the
// user scrolled off the top of license[1].
#[test]
fn license_scroll_resets_on_model_transition() {
    // LocalModelLicense etc. come in via  at the top of the file
    let license_text = (0..100)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join(
            "
",
        );
    let models = vec![
        LocalModelLicense {
            display_name: "M1".into(),
            license_text: license_text.clone(),
        },
        LocalModelLicense {
            display_name: "M2".into(),
            license_text: license_text.clone(),
        },
    ];
    let mut wiz = OnboardingWizardState::new(models, Vec::new);
    wiz.handle(OnboardingEvent::Enter); // → HardwareSurvey
    wiz.handle(OnboardingEvent::Enter); // → LicenseReview[0]
                                        // Scroll 20 lines into M1
    for _ in 0..20 {
        wiz.handle(OnboardingEvent::ArrowDown);
    }
    assert_eq!(wiz.license_scroll, 20);
    // Accept M1 → LicenseReview[1] → scroll must reset to 0
    wiz.handle(OnboardingEvent::Enter);
    assert_eq!(
        wiz.license_scroll, 0,
        "license_scroll must reset on model transition"
    );
}
