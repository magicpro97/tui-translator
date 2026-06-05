//! Deterministic text renderer for the onboarding wizard (split from onboarding.rs).
//!
//! Extracted to keep `onboarding.rs` under the 600 LOC engineering-standards gate.

use super::*;

/// Produce a deterministic list of display lines for the current wizard step.
///
/// This renderer is intentionally ratatui-free so it can be unit-tested
/// without a terminal.  The integration layer may wrap each `String` in a
/// `ratatui::text::Line` and display it inside a `Paragraph` widget.
///
/// License text is rendered verbatim — every source line becomes exactly one
/// output line prefixed with `"│  "`.
pub fn render_wizard_lines(state: &OnboardingWizardState) -> Vec<String> {
    match &state.step {
        OnboardingStep::BranchSelection => {
            let mut lines = vec![
                "── Setup Wizard ──────────────────────────────────────────".to_owned(),
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
                    "── License ({}/{}) — {} ──────────────────────────────",
                    idx + 1,
                    state.local_models.len(),
                    name
                ),
                String::new(),
            ];
            for raw_line in text.lines() {
                lines.push(format!("│  {raw_line}"));
            }
            lines.push(String::new());
            lines.push("  [Enter] Accept & continue  [Esc] Back".to_owned());
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
    }
}
