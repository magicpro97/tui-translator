//! Deterministic text renderer for the onboarding wizard (split from onboarding.rs).
//!
//! Extracted to keep `onboarding.rs` under the 600 LOC engineering-standards gate.

use super::*;

/// Render a byte count as a human-readable string (e.g.
/// `1.5 GiB`, `256 MiB`).  v3 (issue #819): used by the
/// HardwareSurvey render arm.
fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Produce a deterministic list of display lines for the current wizard step.
///
/// This renderer is intentionally ratatui-free so it can be unit-tested
/// without a terminal.  The integration layer may wrap each `String` in a
/// `ratatui::text::Line` and display it inside a `Paragraph` widget.
///
/// License text is rendered verbatim — every source line becomes exactly one
/// output line prefixed with `"│  "`.
///
/// Build identifier: short git SHA captured at compile time via
/// `build.rs` and re-exported here for renderer convenience. Lets
/// the wizard title bar show which binary is on screen, so driver
/// tests (e.g. cua-driver driving the TUI) can verify the actual
/// artifact matches the source they expect, not a stale binary
/// from a previous rebuild.
pub use crate::build_info::{BUILD_SHA, BUILD_VERSION};

pub fn render_wizard_lines(state: &OnboardingWizardState) -> Vec<String> {
    let title = format!(
        "── Setup Wizard v{} ({}) ───────────────────────",
        BUILD_VERSION, BUILD_SHA
    );
    match &state.step {
        OnboardingStep::BranchSelection => {
            let mut lines = vec![
                title,
                "  Choose your backend configuration:".to_owned(),
                String::new(),
            ];
            for (i, branch) in OnboardingWizardState::branches().iter().enumerate() {
                let n = i + 1;
                let marker = if *branch == state.branch { "►" } else { " " };
                lines.push(format!("  {marker} {n}. {}", branch.label()));
            }
            lines.push(String::new());
            lines.push("  [Enter] Confirm  [↑↓ / 1 2 3] Select  [Esc] Cancel".to_owned());
            lines
        }
        OnboardingStep::VirtualCableGate { available } => {
            let mut v = vec![
                "── Virtual Cable Gate ────────────────────────────────────".to_owned(),
                String::new(),
            ];
            if available.is_empty() {
                v.extend([
                    "  VB-CABLE not detected.".to_owned(),
                    "  Install from: https://vb-audio.com/Cable/".to_owned(),
                    String::new(),
                    "  [R] Refresh  [S] Skip (speaker-only mode)  [Esc] Back".to_owned(),
                ]);
            } else {
                v.extend([
                    format!("  ✔ Detected: {}", available[0]),
                    String::new(),
                    "  [Enter] Confirm  [Esc] Back".to_owned(),
                ]);
            }
            v
        }
        OnboardingStep::LicenseReview { model_index } => {
            let idx = *model_index;
            let name = state
                .local_models
                .get(idx)
                .map(|m| m.display_name.as_str())
                .unwrap_or("(unknown)");
            let text = state
                .local_models
                .get(idx)
                .map(|m| m.license_text.as_str())
                .unwrap_or("");
            let mut lines = vec![
                format!(
                    "── License ({}/{}) — {} [{}] ────────────",
                    idx + 1,
                    state.local_models.len(),
                    name,
                    BUILD_SHA
                ),
                String::new(),
            ];
            // Issue #851 / #879 follow-up: the renderer
            // MUST slice the license body by state.license_scroll
            // or the up/down keys (defined in onboarding.rs
            // handle()) are a discoverable affordance with
            // no effect.  Clamp to the number of license lines
            // so a short license still fits without panicking.
            const VISIBLE_BODY: usize = 26;
            let all_lines: Vec<&str> = text.lines().collect();
            let max_start = all_lines.len().saturating_sub(VISIBLE_BODY);
            // The struct field tracks a user-driven offset;
            // the wizard only resets it on transitions (see
            // the 5 self.license_scroll = 0 sites), and the
            // handle() clamps the value against the line
            // count on each step.  Re-clamp here as a safety
            // net in case the wizard moved to a model whose
            // license is shorter than the previous one.
            let scroll = state.license_scroll.min(max_start);
            let start = scroll;
            let end = (start + VISIBLE_BODY).min(all_lines.len());
            let visible = &all_lines[start..end];
            let total_lines = all_lines.len();
            let visible_end = end;
            for raw_line in visible {
                lines.push(format!("│  {raw_line}"));
            }
            lines.push(String::new());
            // Hint footer carries the "line N/M" position so
            // the user can see they actually scrolled.
            lines.push(format!(
                "  [Enter] Accept & continue  [Esc] Back    line {visible_end}/{total_lines}"
            ));
            lines
        }
        OnboardingStep::GoogleKeyEntry => {
            let masked: String = "*".repeat(state.key_buffer.len());
            vec![
                "── Google API Key ────────────────────────────────────────".to_owned(),
                String::new(),
                "  Enter your Google Cloud API key:".to_owned(),
                format!("  Key ▸ {masked}▌"),
                String::new(),
                "  [Enter] Continue  [Esc] Back".to_owned(),
            ]
        }
        OnboardingStep::HardwareSurvey {
            caps,
            selected_preset,
        } => {
            let recommended = crate::quality_preset::QualityPreset::Auto.resolve_for(caps);
            let preset_label = selected_preset.as_label();
            let mut lines = vec![
                "── Hardware Survey ───────────────────────────────────────".to_owned(),
                String::new(),
                "  Detected system capabilities:".to_owned(),
                format!("    RAM  : {}", format_bytes(caps.total_memory_bytes)),
                format!("    Cores: {}", caps.physical_cores),
                format!("    GPU  : {:?}", caps.gpu),
                String::new(),
                format!("  Recommended preset: {}", recommended.as_label()),
                String::new(),
                "  Choose your quality preset:".to_owned(),
            ];
            for (i, p) in crate::quality_preset::QualityPreset::ALL.iter().enumerate() {
                let n = i + 1;
                let marker = if *p == *selected_preset { "►" } else { " " };
                let recommended_marker = if *p == recommended {
                    " (recommended)"
                } else {
                    ""
                };
                lines.push(format!(
                    "  {marker} {n}. {}{recommended_marker}",
                    p.as_label()
                ));
            }
            lines.push(String::new());
            lines.push(format!(
                "  Selection: {preset_label}  [1-4] Select  [r] Recommend  [Enter] Confirm  [↑↓] Cycle  [Esc] Back"
            ));
            lines
        }
        OnboardingStep::Confirmation => {
            let key_line = if state.branch.requires_google_key() {
                if state.key_buffer.trim().is_empty() {
                    "  API key : (none — will prompt at startup)".to_owned()
                } else {
                    let preview: String = state.key_buffer.chars().take(4).collect();
                    format!("  API key : {preview}…")
                }
            } else {
                "  API key : not required".to_owned()
            };
            vec![
                "── Confirm Setup ─────────────────────────────────────────".to_owned(),
                String::new(),
                format!("  Branch  : {}", state.branch.label()),
                key_line,
                String::new(),
                "  [Enter] Apply  [Esc] Back".to_owned(),
            ]
        }
        OnboardingStep::PlatformParityNotice => vec![
            "── Platform Notice ───────────────────────────────────────".to_owned(),
            String::new(),
            "  Virtual-mic interpreter mode is currently Windows-only".to_owned(),
            "  (issue #734 tracks macOS BlackHole / Linux PipeWire).".to_owned(),
            "  The app will run in speaker-only TTS mode on this platform.".to_owned(),
            String::new(),
            "  [Enter] Continue  [Esc] Dismiss".to_owned(),
        ],
        // Issue #852: confirmation step reached when the
        // user presses Esc from BranchSelection.  Pressing
        // Enter or Esc again cancels the whole wizard.
        // Any other key returns to BranchSelection.
        OnboardingStep::ConfirmCancel => vec![
            "── Cancel wizard? ───────────────────────────────────────".to_owned(),
            String::new(),
            "  Are you sure you want to exit the first-run wizard?".to_owned(),
            "  No settings will be saved.".to_owned(),
            String::new(),
            "  [Enter] / [Esc] Yes, cancel  [Any other key] Back".to_owned(),
        ],
    }
}
#[cfg(test)]
#[path = "onboarding_render_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "onboarding_render_tests_hardware_survey.rs"]
mod tests_hardware_survey;
