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
// `tests/onboarding_integration.rs`) so the ``
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
    // The renderer prefixes the selected branch with
    // `► ` AND its 1-based position number.  Pin the
    // marker + branch on the SAME line (since the
    // renderer may reorder rows, the marker and the
    // branch label are tied by line).
    let marked_line = lines
        .iter()
        .find(|l| l.contains("►") && l.contains("Local-only"))
        .expect("default branch row must be marked with ►");
    // The branch is the default; the marker must NOT
    // appear on the other branch rows.
    for line in lines.iter() {
        if line.contains("►") {
            assert!(
                line.contains("Local-only"),
                "the only ► marker must be on the Local-only row, found: {line}"
            );
        }
    }
    // Sanity: the marked_line variable is bound to suppress
    // the unused-variable warning.
    let _ = marked_line;
}

#[test]
fn render_branch_selection_marker_follows_selected_branch() {
    let mut state = empty_state();
    state.branch = OnboardingBranch::GoogleCloud;
    let lines = render_wizard_lines(&state);
    let text = lines.join("\n");
    // The renderer prefixes the selected branch with
    // `► ` AND its 1-based position number; the test
    // pins the marker, not the exact prefix format.
    assert!(
        text.contains("►") && text.contains("Google Cloud"),
        "selected branch row must be present with marker: {text}"
    );
    // The unselected branches have no `►` marker.
    for line in lines.iter() {
        if line.contains("Local-only") {
            assert!(
                !line.contains("►"),
                "unselected Local-only must not be marked: {line}"
            );
        }
    }
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

// ── T13 (issue #819): HardwareSurvey render ────────────────────────────

#[test]
fn hardware_survey_step_carries_syscaps_and_recommendation() {
    // The HardwareSurvey step stores the captured SysCaps and the
    // currently-selected preset.  Verify the round-trip.
    let caps = crate::sys_caps::SysCaps {
        total_memory_bytes: 32 * 1024 * 1024 * 1024, // 32 GiB
        physical_cores: 8,
        gpu: crate::sys_caps::GpuKind::None,
    };
    let recommended = crate::quality_preset::QualityPreset::Auto.resolve_for(&caps);
    let w = OnboardingWizardState::new_with_caps(vec![], noop_probe, caps);
    match &w.step {
        OnboardingStep::HardwareSurvey {
            caps: stored_caps,
            selected_preset,
        } => {
            assert_eq!(*stored_caps, caps);
            assert_eq!(*selected_preset, recommended);
        }
        _ => panic!("expected HardwareSurvey as initial step"),
    }
}

#[test]
fn hardware_survey_renders_recommendation_for_8gb_machine() {
    // 8 GiB machine: Auto resolves to Performance.
    let caps = crate::sys_caps::SysCaps {
        total_memory_bytes: 8 * 1024 * 1024 * 1024,
        physical_cores: 4,
        gpu: crate::sys_caps::GpuKind::None,
    };
    let w = OnboardingWizardState::new_with_caps(vec![], noop_probe, caps);
    let lines = render_wizard_lines(&w);
    let text = lines.join("\n");
    assert!(
        text.contains("Performance"),
        "8 GiB must recommend Performance: {text}"
    );
}

#[test]
fn hardware_survey_renders_recommendation_for_32gb_machine() {
    // 32 GiB machine: Auto resolves to Best.
    let caps = crate::sys_caps::SysCaps {
        total_memory_bytes: 32 * 1024 * 1024 * 1024,
        physical_cores: 8,
        gpu: crate::sys_caps::GpuKind::None,
    };
    let w = OnboardingWizardState::new_with_caps(vec![], noop_probe, caps);
    let lines = render_wizard_lines(&w);
    let text = lines.join("\n");
    assert!(text.contains("Best"), "32 GiB must recommend Best: {text}");
}

#[test]
fn hardware_survey_renders_all_four_presets() {
    let caps = crate::sys_caps::SysCaps {
        total_memory_bytes: 8 * 1024 * 1024 * 1024,
        physical_cores: 4,
        gpu: crate::sys_caps::GpuKind::None,
    };
    let w = OnboardingWizardState::new_with_caps(vec![], noop_probe, caps);
    let lines = render_wizard_lines(&w);
    let text = lines.join("\n");
    for label in &["Auto", "Best", "Performance", "Custom"] {
        assert!(text.contains(label), "preset {label} must appear: {text}");
    }
}

#[test]
fn hardware_survey_renders_detected_capabilities() {
    let caps = crate::sys_caps::SysCaps {
        total_memory_bytes: 16 * 1024 * 1024 * 1024,
        physical_cores: 4,
        gpu: crate::sys_caps::GpuKind::None,
    };
    let w = OnboardingWizardState::new_with_caps(vec![], noop_probe, caps);
    let lines = render_wizard_lines(&w);
    let text = lines.join("\n");
    assert!(text.contains("RAM"), "must show RAM: {text}");
    assert!(text.contains("Cores"), "must show Cores: {text}");
    assert!(text.contains("GPU"), "must show GPU: {text}");
    assert!(text.contains("GiB"), "must show GiB units: {text}");
}

#[test]
fn hardware_survey_arrow_up_cycles_to_next_preset() {
    let caps = crate::sys_caps::SysCaps {
        total_memory_bytes: 8 * 1024 * 1024 * 1024,
        physical_cores: 4,
        gpu: crate::sys_caps::GpuKind::None,
    };
    let mut w = OnboardingWizardState::new_with_caps(vec![], noop_probe, caps);
    let initial = match w.step.clone() {
        OnboardingStep::HardwareSurvey {
            selected_preset, ..
        } => selected_preset,
        _ => unreachable!(),
    };
    w.handle(OnboardingEvent::ArrowUp);
    let new_preset = match w.step.clone() {
        OnboardingStep::HardwareSurvey {
            selected_preset, ..
        } => selected_preset,
        _ => unreachable!(),
    };
    assert_ne!(initial, new_preset, "ArrowUp must change the preset");
}

#[test]
fn hardware_survey_digit_keys_select_preset() {
    let caps = crate::sys_caps::SysCaps {
        total_memory_bytes: 8 * 1024 * 1024 * 1024,
        physical_cores: 4,
        gpu: crate::sys_caps::GpuKind::None,
    };
    let mut w = OnboardingWizardState::new_with_caps(vec![], noop_probe, caps);
    w.handle(OnboardingEvent::Char('2'));
    let new_preset = match w.step.clone() {
        OnboardingStep::HardwareSurvey {
            selected_preset, ..
        } => selected_preset,
        _ => unreachable!(),
    };
    assert_eq!(new_preset, crate::quality_preset::QualityPreset::Best);
}

#[test]
fn hardware_survey_enter_records_selection_in_state() {
    let caps = crate::sys_caps::SysCaps {
        total_memory_bytes: 8 * 1024 * 1024 * 1024,
        physical_cores: 4,
        gpu: crate::sys_caps::GpuKind::None,
    };
    let mut w = OnboardingWizardState::new_with_caps(vec![], noop_probe, caps);
    w.handle(OnboardingEvent::Char('2')); // select Best
    w.handle(OnboardingEvent::Enter); // confirm
    assert_eq!(
        w.hardware_survey_selection,
        Some(crate::quality_preset::QualityPreset::Best)
    );
}

// ── T13: more HardwareSurvey keymap coverage (Char('1', '3', '4', 'r'), ArrowDown, Escape) ──

#[test]
fn hardware_survey_arrow_down_cycles_to_previous_preset() {
    let caps = crate::sys_caps::SysCaps {
        total_memory_bytes: 8 * 1024 * 1024 * 1024,
        physical_cores: 4,
        gpu: crate::sys_caps::GpuKind::None,
    };
    let mut w = OnboardingWizardState::new_with_caps(vec![], noop_probe, caps);
    let initial = match w.step.clone() {
        OnboardingStep::HardwareSurvey {
            selected_preset, ..
        } => selected_preset,
        _ => unreachable!(),
    };
    w.handle(OnboardingEvent::ArrowDown);
    let new_preset = match w.step.clone() {
        OnboardingStep::HardwareSurvey {
            selected_preset, ..
        } => selected_preset,
        _ => unreachable!(),
    };
    assert_ne!(initial, new_preset, "ArrowDown must change the preset");
}

#[test]
fn hardware_survey_digit_1_selects_auto() {
    let caps = crate::sys_caps::SysCaps {
        total_memory_bytes: 32 * 1024 * 1024 * 1024, // forces Best initially
        physical_cores: 8,
        gpu: crate::sys_caps::GpuKind::None,
    };
    let mut w = OnboardingWizardState::new_with_caps(vec![], noop_probe, caps);
    w.handle(OnboardingEvent::Char('1'));
    let p = match w.step.clone() {
        OnboardingStep::HardwareSurvey {
            selected_preset, ..
        } => selected_preset,
        _ => unreachable!(),
    };
    assert_eq!(p, crate::quality_preset::QualityPreset::Auto);
}

#[test]
fn hardware_survey_digit_3_selects_performance() {
    let caps = crate::sys_caps::SysCaps {
        total_memory_bytes: 8 * 1024 * 1024 * 1024,
        physical_cores: 4,
        gpu: crate::sys_caps::GpuKind::None,
    };
    let mut w = OnboardingWizardState::new_with_caps(vec![], noop_probe, caps);
    w.handle(OnboardingEvent::Char('3'));
    let p = match w.step.clone() {
        OnboardingStep::HardwareSurvey {
            selected_preset, ..
        } => selected_preset,
        _ => unreachable!(),
    };
    assert_eq!(p, crate::quality_preset::QualityPreset::Performance);
}

#[test]
fn hardware_survey_digit_4_selects_custom() {
    let caps = crate::sys_caps::SysCaps {
        total_memory_bytes: 8 * 1024 * 1024 * 1024,
        physical_cores: 4,
        gpu: crate::sys_caps::GpuKind::None,
    };
    let mut w = OnboardingWizardState::new_with_caps(vec![], noop_probe, caps);
    w.handle(OnboardingEvent::Char('4'));
    let p = match w.step.clone() {
        OnboardingStep::HardwareSurvey {
            selected_preset, ..
        } => selected_preset,
        _ => unreachable!(),
    };
    assert_eq!(p, crate::quality_preset::QualityPreset::Custom);
}

#[test]
fn hardware_survey_r_key_resets_to_recommended() {
    let caps = crate::sys_caps::SysCaps {
        total_memory_bytes: 32 * 1024 * 1024 * 1024,
        physical_cores: 8,
        gpu: crate::sys_caps::GpuKind::None,
    };
    let mut w = OnboardingWizardState::new_with_caps(vec![], noop_probe, caps);
    w.handle(OnboardingEvent::Char('4')); // pick Custom
    w.handle(OnboardingEvent::Char('r')); // reset
    let p = match w.step.clone() {
        OnboardingStep::HardwareSurvey {
            selected_preset, ..
        } => selected_preset,
        _ => unreachable!(),
    };
    // 32 GiB → Auto resolves to Best.
    assert_eq!(p, crate::quality_preset::QualityPreset::Best);
}

#[test]
fn hardware_survey_escape_returns_to_branch_selection() {
    let caps = crate::sys_caps::SysCaps {
        total_memory_bytes: 8 * 1024 * 1024 * 1024,
        physical_cores: 4,
        gpu: crate::sys_caps::GpuKind::None,
    };
    let mut w = OnboardingWizardState::new_with_caps(vec![], noop_probe, caps);
    w.handle(OnboardingEvent::Escape);
    assert!(matches!(w.step, OnboardingStep::BranchSelection));
}

// ── T13 follow-up: cover the remaining unhit arms in onboarding.rs ──

// Local copies of the test helpers (the originals live in
// `onboarding_tests.rs` which is not visible from this render-tests
// file because they are separate `#[path]` modules of the same
// `#[cfg(test)] mod tests;` block).
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

fn make_gated(probe: fn() -> Vec<String>) -> OnboardingWizardState {
    let mut w = OnboardingWizardState::new(vec![], probe);
    w.gate_enabled = true;
    w
}

fn no_cable() -> Vec<String> {
    Vec::new()
}

fn noop_probe() -> Vec<String> {
    Vec::new()
}

#[test]
fn noop_probe_returns_empty_vec() {
    // L37-39 in onboarding.rs: the noop_probe free function.
    let v = noop_probe();
    assert!(v.is_empty());
}

#[test]
fn onboarding_branch_display_uses_label() {
    use OnboardingBranch;
    // The Display impl just delegates to `label()` which has
    // verbose form; assert the prefix.
    let l = format!("{}", OnboardingBranch::LocalOnly);
    assert!(l.starts_with("Local-only"), "got {l:?}");
    let l = format!("{}", OnboardingBranch::LocalGoogleFallback);
    assert!(l.starts_with("Local + Google fallback"), "got {l:?}");
    let l = format!("{}", OnboardingBranch::GoogleCloud);
    assert!(l.starts_with("Google Cloud"), "got {l:?}");
}

#[test]
fn enter_past_branch_and_survey_advances_through_survey() {
    // L357-362: the v3 helper for tests that want pre-v3 single-Enter
    // semantics (BranchSelection → next step) — it advances twice to
    // skip the HardwareSurvey in the middle.  Use `new` to start at
    // BranchSelection (not `new_with_caps` which opens at
    // HardwareSurvey).
    let mut w = make_wizard();
    let outcome = w.enter_past_branch_and_survey();
    assert!(outcome.is_none());
    // Should be on Confirmation (LocalOnly, no models).
    assert_eq!(w.step, OnboardingStep::Confirmation);
}

#[test]
fn new_consent_review_starts_at_license_review_with_models() {
    // L322-347: the v3 + hardware_survey_selection constructor.
    let models = vec![LocalModelLicense {
        display_name: "Whisper".to_string(),
        license_text: "MIT".to_string(),
    }];
    let w = OnboardingWizardState::new_consent_review(models, OnboardingBranch::LocalOnly);
    assert!(w.consent_only);
    assert_eq!(w.step, OnboardingStep::LicenseReview { model_index: 0 });
    assert_eq!(w.hardware_survey_selection, None);
}

#[test]
fn new_consent_review_starts_at_confirmation_without_models() {
    let w = OnboardingWizardState::new_consent_review(vec![], OnboardingBranch::GoogleCloud);
    assert_eq!(w.step, OnboardingStep::Confirmation);
}

#[test]
fn go_back_from_virtual_cable_gate_returns_to_branch_selection() {
    let mut w = make_gated(no_cable);
    w.handle(OnboardingEvent::Enter); // → VirtualCableGate
    assert!(matches!(w.step, OnboardingStep::VirtualCableGate { .. }));
    w.handle(OnboardingEvent::Escape);
    assert!(matches!(w.step, OnboardingStep::BranchSelection));
}

#[test]
fn go_back_from_license_review_first_index_returns_to_branch_selection() {
    let mut w = make_wizard_with_models();
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey (v3)
    w.handle(OnboardingEvent::Enter); // → LicenseReview[0]
    assert_eq!(w.step, OnboardingStep::LicenseReview { model_index: 0 });
    w.handle(OnboardingEvent::Escape);
    assert!(matches!(w.step, OnboardingStep::BranchSelection));
}

#[test]
fn go_back_from_license_review_n_minus_one_returns_to_license_review() {
    let mut w = make_wizard_with_models();
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey
    w.handle(OnboardingEvent::Enter); // → LicenseReview[0]
    w.handle(OnboardingEvent::Enter); // → LicenseReview[1]
    assert_eq!(w.step, OnboardingStep::LicenseReview { model_index: 1 });
    w.handle(OnboardingEvent::Escape);
    assert_eq!(w.step, OnboardingStep::LicenseReview { model_index: 0 });
}

#[test]
fn go_back_from_google_key_entry_with_models_returns_to_license_review() {
    let mut w = make_wizard_with_models();
    w.handle(OnboardingEvent::SelectBranch2);
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey
    w.handle(OnboardingEvent::Enter); // → LicenseReview[0]
    w.handle(OnboardingEvent::Enter); // → LicenseReview[1]
    w.handle(OnboardingEvent::Enter); // → GoogleKeyEntry
    assert_eq!(w.step, OnboardingStep::GoogleKeyEntry);
    w.handle(OnboardingEvent::Escape);
    assert_eq!(w.step, OnboardingStep::LicenseReview { model_index: 1 });
}

#[test]
fn go_back_from_google_key_entry_without_models_returns_to_branch_selection() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch2);
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey
    w.handle(OnboardingEvent::Enter); // → GoogleKeyEntry
    assert_eq!(w.step, OnboardingStep::GoogleKeyEntry);
    w.handle(OnboardingEvent::Escape);
    assert!(matches!(w.step, OnboardingStep::BranchSelection));
}

#[test]
fn go_back_from_confirmation_with_key_required_returns_to_google_key_entry() {
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch2);
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey
    w.handle(OnboardingEvent::Enter); // → GoogleKeyEntry
    w.handle(OnboardingEvent::Char('k'));
    w.handle(OnboardingEvent::Char('e'));
    w.handle(OnboardingEvent::Char('y'));
    w.handle(OnboardingEvent::Enter); // → Confirmation
    assert_eq!(w.step, OnboardingStep::Confirmation);
    w.handle(OnboardingEvent::Escape);
    // Back to GoogleKeyEntry (because the key was non-empty).
    assert_eq!(w.step, OnboardingStep::GoogleKeyEntry);
}

#[test]
fn go_back_from_confirmation_with_models_returns_to_license_review() {
    let mut w = make_wizard_with_models();
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey
    w.handle(OnboardingEvent::Enter); // → LicenseReview[0]
    w.handle(OnboardingEvent::Enter); // → LicenseReview[1]
    w.handle(OnboardingEvent::Enter); // → Confirmation
    assert_eq!(w.step, OnboardingStep::Confirmation);
    w.handle(OnboardingEvent::Escape);
    assert_eq!(w.step, OnboardingStep::LicenseReview { model_index: 1 });
}

#[test]
fn confirmation_empty_key_returns_to_google_key_entry() {
    // The advance arm that returns to GoogleKeyEntry on empty key.
    // 4 Enters: BranchSelection → HardwareSurvey → GoogleKeyEntry →
    // Confirmation → GoogleKeyEntry (re-prompted because key is empty).
    let mut w = make_wizard();
    w.handle(OnboardingEvent::SelectBranch2);
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey
    w.handle(OnboardingEvent::Enter); // → GoogleKeyEntry
    w.handle(OnboardingEvent::Enter); // → Confirmation
    assert_eq!(w.step, OnboardingStep::Confirmation);
    w.handle(OnboardingEvent::Enter); // → GoogleKeyEntry (empty key, re-prompt)
    assert_eq!(w.step, OnboardingStep::GoogleKeyEntry);
}

#[test]
fn escape_at_platform_parity_notice_advances() {
    let mut w = OnboardingWizardState::new_platform_parity_notice();
    let outcome = w.handle(OnboardingEvent::Escape);
    assert!(matches!(
        outcome,
        Some(OnboardingOutcome::PlatformParityNoticeDismissed)
    ));
}

// ── T13 final: cover the remaining advance/go_back/handle branches ──

fn with_cable() -> Vec<String> {
    vec!["CABLE Output (VB-Audio Virtual Cable)".to_string()]
}

#[test]
fn escape_from_branch_selection_returns_cancelled() {
    let mut w = make_wizard();
    let outcome = w.handle(OnboardingEvent::Escape);
    assert!(matches!(outcome, Some(OnboardingOutcome::Cancelled)));
}

#[test]
fn advance_with_gate_enabled_and_empty_cable_lands_on_virtual_cable_gate() {
    // gate_enabled=true + no cable detected → go to gate.
    let mut w = make_gated(no_cable);
    w.handle(OnboardingEvent::Enter); // BranchSelection → VirtualCableGate
    assert!(
        matches!(w.step, OnboardingStep::VirtualCableGate { ref available } if available.is_empty())
    );
}

#[test]
fn advance_with_gate_enabled_and_cable_skips_gate() {
    // gate_enabled=true + cable detected → skip gate, go to HardwareSurvey.
    let mut w = make_gated(with_cable);
    w.handle(OnboardingEvent::Enter);
    assert!(matches!(w.step, OnboardingStep::HardwareSurvey { .. }));
}

#[test]
fn enter_on_virtual_cable_gate_with_devices_advances_to_survey() {
    // L672 in handle: Enter with non-empty available advances.
    // With gate_enabled=true AND cable present, the first Enter
    // from BranchSelection skips the gate (cable is detected) and
    // lands on HardwareSurvey.  After that, another Enter goes
    // to Confirmation (LocalOnly, no models).
    let mut w = make_gated(with_cable);
    w.handle(OnboardingEvent::Enter); // BranchSelection → HardwareSurvey
    assert!(matches!(w.step, OnboardingStep::HardwareSurvey { .. }));
}

#[test]
fn advance_from_license_review_with_consent_only_returns_done() {
    // L476-481: LicenseReview last index with consent_only → Done(patch).
    let mut w = OnboardingWizardState::new_consent_review(
        vec![LocalModelLicense {
            display_name: "X".to_string(),
            license_text: "MIT".to_string(),
        }],
        OnboardingBranch::LocalOnly,
    );
    let outcome = w.handle(OnboardingEvent::Enter);
    assert!(matches!(outcome, Some(OnboardingOutcome::Done(_))));
}

#[test]
fn go_back_from_license_review_first_index_with_consent_only_returns_cancelled() {
    // L544: LicenseReview[0] go_back with consent_only.
    let mut w = OnboardingWizardState::new_consent_review(
        vec![LocalModelLicense {
            display_name: "X".to_string(),
            license_text: "MIT".to_string(),
        }],
        OnboardingBranch::LocalOnly,
    );
    let outcome = w.handle(OnboardingEvent::Escape);
    assert!(matches!(outcome, Some(OnboardingOutcome::Cancelled)));
}

#[test]
fn enter_at_platform_parity_notice_returns_done() {
    let mut w = OnboardingWizardState::new_platform_parity_notice();
    let outcome = w.handle(OnboardingEvent::Enter);
    assert!(matches!(
        outcome,
        Some(OnboardingOutcome::PlatformParityNoticeDismissed)
    ));
}

#[test]
fn advance_from_virtual_cable_gate_in_advance_method() {
    // L447-450: advance() from VirtualCableGate (the inner method, not handle()).
    // To reach VirtualCableGate via advance(), we need gate_enabled=true
    // and an empty cable_probe so the gate is NOT skipped.
    let mut w = make_gated(no_cable);
    w.handle(OnboardingEvent::Enter); // BranchSelection → VirtualCableGate (gate enabled, no cable)
    assert!(
        matches!(w.step, OnboardingStep::VirtualCableGate { ref available } if available.is_empty())
    );
    // Now step is VirtualCableGate; calling advance() directly advances.
    let outcome = w.advance();
    assert!(outcome.is_none());
    assert!(matches!(w.step, OnboardingStep::HardwareSurvey { .. }));
}

// ── T13 final: more go_back/advance direct method tests ──

#[test]
fn go_back_from_branch_selection_directly() {
    // L520: go_back() called when step is BranchSelection.
    // `handle()` short-circuits with Cancelled, but go_back() is a
    // public method and we test the explicit case.
    let mut w = make_wizard();
    w.step = OnboardingStep::BranchSelection;
    let outcome = w.go_back();
    assert!(matches!(outcome, Some(OnboardingOutcome::Cancelled)));
}

// ── T13 last: go_back from HardwareSurvey with gate_enabled ──

#[test]
fn enter_past_branch_and_survey_returns_first_outcome() {
    // L364: `enter_past_branch_and_survey`'s first-advance
    // returns Some path.  Use a wizard that opens at
    // BranchSelection with consent_only=false and a non-empty
    // model list.  The first advance from BranchSelection
    // sets step to HardwareSurvey and returns None; the
    // SECOND advance from HardwareSurvey goes to LicenseReview
    // and returns None; the THIRD would go to Confirmation.
    // But for the `Some` path, we need the first advance to
    // return Some — that happens when the step is
    // already past the gate (e.g. at LicenseReview last
    // index in consent_only mode).
    let mut w = OnboardingWizardState::new_consent_review(
        vec![LocalModelLicense {
            display_name: "X".to_string(),
            license_text: "MIT".to_string(),
        }],
        OnboardingBranch::LocalOnly,
    );
    // w.step starts at LicenseReview { model_index: 0 }.
    let outcome = w.enter_past_branch_and_survey();
    assert!(matches!(outcome, Some(OnboardingOutcome::Done(_))));
}

// ── T13 final: format_bytes unit conversions (MiB, KiB, B branches) ──

#[test]
fn format_bytes_mi_branch() {
    // 1.5 MiB
    let s = format_bytes(1_572_864);
    assert!(s.contains("MiB"), "got {s:?}");
}

#[test]
fn format_bytes_kib_branch() {
    // 512 KiB
    let s = format_bytes(524_288);
    assert!(s.contains("KiB"), "got {s:?}");
}

#[test]
fn format_bytes_b_branch() {
    // 256 B
    let s = format_bytes(256);
    assert!(s.contains("B"), "got {s:?}");
    assert!(s.starts_with("256"), "got {s:?}");
}

#[test]
fn format_bytes_gib_branch() {
    // 2 GiB
    let s = format_bytes(2 * 1024 * 1024 * 1024);
    assert!(s.contains("GiB"), "got {s:?}");
}

// ── T13 catch-all coverage: send unexpected events to non-VirtualCableGate steps ──

#[test]
fn handle_unexpected_event_on_license_review_is_noop() {
    // L720: `_ => None` catch-all in the LicenseReview arm.
    let license = LocalModelLicense {
        display_name: "X".to_string(),
        license_text: "MIT".to_string(),
    };
    let mut w =
        OnboardingWizardState::new_consent_review(vec![license], OnboardingBranch::LocalOnly);
    let outcome = w.handle(OnboardingEvent::Char('x'));
    assert!(outcome.is_none());
    assert!(matches!(w.step, OnboardingStep::LicenseReview { .. }));
}

#[test]
fn handle_unexpected_event_on_google_key_entry_is_noop() {
    // L766: `_ => None` catch-all in the GoogleKeyEntry arm.
    let mut w = make_wizard();
    w.step = OnboardingStep::GoogleKeyEntry;
    let outcome = w.handle(OnboardingEvent::RefreshVirtualCable);
    assert!(outcome.is_none());
    assert_eq!(w.step, OnboardingStep::GoogleKeyEntry);
}

#[test]
fn handle_unexpected_event_on_confirmation_is_noop() {
    // L756: `_ => None` catch-all in the Confirmation arm.
    let mut w = make_wizard();
    w.step = OnboardingStep::Confirmation;
    let outcome = w.handle(OnboardingEvent::Char('y'));
    assert!(outcome.is_none());
    assert_eq!(w.step, OnboardingStep::Confirmation);
}

#[test]
fn handle_unexpected_event_on_virtual_cable_gate_is_noop() {
    // L673: `_ => None` catch-all in the VirtualCableGate arm.
    let mut w = make_gated(no_cable);
    w.handle(OnboardingEvent::Enter); // BranchSelection → VirtualCableGate
    assert!(matches!(w.step, OnboardingStep::VirtualCableGate { .. }));
    let outcome = w.handle(OnboardingEvent::Char('z'));
    assert!(outcome.is_none());
    assert!(matches!(w.step, OnboardingStep::VirtualCableGate { .. }));
}

#[test]
fn handle_unexpected_event_on_platform_parity_notice_is_noop() {
    // L762: `_ => None` catch-all in the PlatformParityNotice arm.
    let mut w = OnboardingWizardState::new_platform_parity_notice();
    let outcome = w.handle(OnboardingEvent::Char('w'));
    assert!(outcome.is_none());
    assert!(matches!(w.step, OnboardingStep::PlatformParityNotice));
}

// ── T13: HardwareSurvey catch-all (L720) ──

#[test]
fn handle_unexpected_event_on_hardware_survey_is_noop() {
    // L720: `_ => None` in the HardwareSurvey event matcher.
    // Set up a wizard at HardwareSurvey by going through the normal flow.
    let mut w = make_wizard();
    w.gate_enabled = false;
    w.handle(OnboardingEvent::Enter); // BranchSelection → HardwareSurvey
    assert!(matches!(w.step, OnboardingStep::HardwareSurvey { .. }));
    let outcome = w.handle(OnboardingEvent::Backspace);
    assert!(outcome.is_none());
    assert!(matches!(w.step, OnboardingStep::HardwareSurvey { .. }));
}

// ── T13: current_license_index query helper (L766) ──

#[test]
fn current_license_index_on_non_license_review_returns_none() {
    let mut w = make_wizard();
    w.gate_enabled = false;
    w.handle(OnboardingEvent::Enter); // → HardwareSurvey
    assert!(w.current_license_index().is_none());
}

#[test]
fn platform_parity_notice_advance_method_direct() {
    // L582: directly call advance() on a PlatformParityNotice wizard.
    let mut w = OnboardingWizardState::new_platform_parity_notice();
    let outcome = w.advance();
    assert!(matches!(
        outcome,
        Some(OnboardingOutcome::PlatformParityNoticeDismissed)
    ));
}

#[test]
fn platform_parity_notice_go_back_returns_dismissed() {
    // L582: go_back() for PlatformParityNotice returns Some(Outcome::Dismissed).
    let mut w = OnboardingWizardState::new_platform_parity_notice();
    let outcome = w.go_back();
    assert!(matches!(
        outcome,
        Some(OnboardingOutcome::PlatformParityNoticeDismissed)
    ));
}

#[test]
fn handle_enter_on_virtual_cable_gate_with_available_devices_advances() {
    // L671: Enter with non-empty `available` calls self.advance().
    // Construct a wizard at VirtualCableGate with a device.
    let mut w = make_wizard();
    w.gate_enabled = true;
    w.step = OnboardingStep::VirtualCableGate {
        available: vec!["CABLE Output (VB-Audio Virtual Cable)".to_string()],
    };
    let outcome = w.handle(OnboardingEvent::Enter);
    assert!(outcome.is_none());
    assert!(matches!(w.step, OnboardingStep::HardwareSurvey { .. }));
}

#[test]
fn go_back_from_hardware_survey_with_gate_enabled_returns_to_gate() {
    // L529-533: Esc on HardwareSurvey with gate_enabled=true → VirtualCableGate.
    use crate::quality_preset as qp_mod;
    use crate::sys_caps as caps_mod;
    let mut w = make_gated(no_cable);
    w.step = OnboardingStep::HardwareSurvey {
        caps: caps_mod::SysCaps {
            total_memory_bytes: 8 * 1024 * 1024 * 1024,
            physical_cores: 4,
            gpu: caps_mod::GpuKind::None,
        },
        selected_preset: qp_mod::QualityPreset::Auto,
    };
    let outcome = w.go_back();
    assert!(outcome.is_none());
    assert!(matches!(w.step, OnboardingStep::VirtualCableGate { .. }));
}
