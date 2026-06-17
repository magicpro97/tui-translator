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

#[test]
fn esc_at_branch_selection_cancels_wizard() {
    let mut w = make_wizard();
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
