//! LF-05 first-run onboarding wizard state machine.
//!
//! This module is **pure** (no I/O, no ratatui, no global state) so it can be
//! driven by unit tests and later wrapped by an integration layer in `main.rs`.
//!
//! # Wizard flow
//!
//! ```text
//! BranchSelection
//!   │
//!   ├─ LocalOnly ──────────────► LicenseReview* ──────────────────► Confirmation
//!   │                                                                      ▲
//!   ├─ LocalGoogleFallback ──► LicenseReview* ──► GoogleKeyEntry ──────────┤
//!   │                                                                      │
//!   └─ GoogleCloud ───────────────────────────────► GoogleKeyEntry ─────────┘
//! ```
//!
//! `LicenseReview*` iterates once per local model; `LocalOnly` skips
//! `GoogleKeyEntry`; `GoogleCloud` skips `LicenseReview`.
//!
//! # Key handling
//!
//! Runtime shortcut keys (`L T M S R ? Q Space`) must be mapped to
//! [`OnboardingEvent::Ignored`] by the caller before being passed to
//! [`OnboardingWizardState::handle`]; they produce an explicit no-op here so
//! no runtime action is triggered while the wizard is active.
//!
//! # Integration
//!
//! When [`OnboardingWizardState::handle`] returns
//! `Some(OnboardingOutcome::Done(patch))`, the caller should convert the
//! [`OnboardingConfigPatch`] into the real `AppConfig` and persist it.

use std::fmt;

// ── Branch ────────────────────────────────────────────────────────────────────

/// Which backend configuration branch the user selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OnboardingBranch {
    /// Local-only: local STT + local MT, no Google key required.  This is
    /// the default selection presented to the user.
    LocalOnly,
    /// Local + Google fallback: prefer local inference but fall back to Google
    /// Cloud STT/MT when confidence is low.  Requires a Google API key.
    LocalGoogleFallback,
    /// Google Cloud: all STT and MT requests go to Google Cloud APIs.
    /// Requires a Google API key.
    GoogleCloud,
}

impl OnboardingBranch {
    /// Human-readable label shown next to the selection marker.
    pub fn label(self) -> &'static str {
        match self {
            Self::LocalOnly => "Local-only  (no internet required, default)",
            Self::LocalGoogleFallback => "Local + Google fallback  (best quality, key required)",
            Self::GoogleCloud => "Google Cloud  (cloud STT + MT, key required)",
        }
    }

    /// Returns `true` when this branch requires a Google API key.
    pub fn requires_google_key(self) -> bool {
        matches!(self, Self::LocalGoogleFallback | Self::GoogleCloud)
    }

    /// Returns `true` when this branch uses local models (license review needed).
    pub fn uses_local_models(self) -> bool {
        matches!(self, Self::LocalOnly | Self::LocalGoogleFallback)
    }
}

impl fmt::Display for OnboardingBranch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

// ── Local model license info ──────────────────────────────────────────────────

/// Metadata for one local model whose license must be reviewed before use.
///
/// The integration layer constructs this from the downloaded model manifests
/// and passes the slice to [`OnboardingWizardState::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalModelLicense {
    /// Short display name shown as the license-screen title (e.g. `"Whisper Tiny"`).
    pub display_name: String,
    /// The full verbatim license text.  The wizard never truncates this.
    pub license_text: String,
}

// ── Step ──────────────────────────────────────────────────────────────────────

/// Current step in the onboarding wizard flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnboardingStep {
    /// The user is choosing a backend branch (keys `1`/`2`/`3` or arrow + Enter).
    BranchSelection,
    /// The user is reviewing the license for the local model at `model_index`.
    LicenseReview {
        /// Index into the `local_models` slice provided at construction.
        model_index: usize,
    },
    /// The user is typing a Google API key.
    GoogleKeyEntry,
    /// All inputs collected; awaiting final confirmation or back-navigation.
    Confirmation,
}

// ── Events ────────────────────────────────────────────────────────────────────

/// Input event understood by the onboarding wizard.
///
/// The caller maps raw crossterm/ratatui key events to this enum.
/// Forbidden runtime-shortcut keys (`L T M S R ? Q Space`) **must** be
/// translated to [`OnboardingEvent::Ignored`] so the wizard no-ops them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnboardingEvent {
    /// Select branch 1 — [`OnboardingBranch::LocalOnly`].
    SelectBranch1,
    /// Select branch 2 — [`OnboardingBranch::LocalGoogleFallback`].
    SelectBranch2,
    /// Select branch 3 — [`OnboardingBranch::GoogleCloud`].
    SelectBranch3,
    /// Move the highlighted branch up (wraps from first to last).
    ArrowUp,
    /// Move the highlighted branch down (wraps from last to first).
    ArrowDown,
    /// Advance to the next step / confirm the current selection.
    Enter,
    /// Go back one step; cancels the wizard when at [`OnboardingStep::BranchSelection`].
    Escape,
    /// A printable character typed during [`OnboardingStep::GoogleKeyEntry`].
    Char(char),
    /// Delete the last character from the key buffer.
    Backspace,
    /// A key that must produce no action while the wizard is active.
    ///
    /// The caller must map the runtime shortcuts (`L`, `T`, `M`, `S`, `R`,
    /// `?`, `Q`, `Space`) to this variant so the wizard explicitly ignores them.
    Ignored,
}

// ── Config patch ──────────────────────────────────────────────────────────────

/// Minimal, serialisation-friendly configuration patch produced when the
/// wizard completes successfully.
///
/// The integration layer converts this into the full `AppConfig` structure.
/// Keeping this type free of `AppConfig` imports avoids circular dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingConfigPatch {
    /// Which backend branch was chosen.
    pub branch: OnboardingBranch,
    /// Google API key entered by the user, if required.
    ///
    /// `None` for [`OnboardingBranch::LocalOnly`] and consent-only review flows.
    /// Full onboarding keeps this non-empty for key-required branches before
    /// returning a completed patch.
    pub google_api_key: Option<String>,
}

// ── Outcome ───────────────────────────────────────────────────────────────────

/// Terminal outcome returned once the wizard exits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnboardingOutcome {
    /// The wizard completed; apply the enclosed patch and proceed.
    Done(OnboardingConfigPatch),
    /// The user cancelled (pressed `Esc` at [`OnboardingStep::BranchSelection`]).
    Cancelled,
}

// ── State machine ─────────────────────────────────────────────────────────────

/// Full mutable state for the first-run onboarding wizard.
///
/// Construct with [`OnboardingWizardState::new`], then call
/// [`OnboardingWizardState::handle`] for each incoming event.
///
/// When `handle` returns `Some(outcome)` the wizard has terminated and must be
/// removed from the render loop.
#[derive(Debug, Clone)]
pub struct OnboardingWizardState {
    /// Currently highlighted / confirmed backend branch.
    pub branch: OnboardingBranch,
    /// Current wizard step.
    pub step: OnboardingStep,
    /// Local-model licenses provided at construction; may be empty.
    pub local_models: Vec<LocalModelLicense>,
    /// Raw API key text accumulated during [`OnboardingStep::GoogleKeyEntry`].
    pub key_buffer: String,
    /// Per-model acceptance flag; index corresponds to `local_models`.
    pub licenses_accepted: Vec<bool>,
    /// When `true`, the wizard skips branch selection and goes straight to
    /// license review; after all models are accepted it emits
    /// [`OnboardingOutcome::Done`] without visiting `GoogleKeyEntry` or
    /// `Confirmation`.
    pub consent_only: bool,
}

impl OnboardingWizardState {
    /// Create a new wizard ready at the branch-selection step.
    ///
    /// `local_models` should contain one entry per local model whose license
    /// needs reviewing.  Pass an empty `Vec` when the user selects
    /// [`OnboardingBranch::GoogleCloud`] or when no local models are bundled.
    pub fn new(local_models: Vec<LocalModelLicense>) -> Self {
        let n = local_models.len();
        Self {
            branch: OnboardingBranch::LocalOnly,
            step: OnboardingStep::BranchSelection,
            local_models,
            key_buffer: String::new(),
            licenses_accepted: vec![false; n],
            consent_only: false,
        }
    }

    /// Create a wizard that skips branch selection and goes straight to license
    /// review for `models`. After all models are accepted the wizard emits
    /// [`OnboardingOutcome::Done`] immediately without visiting
    /// [`OnboardingStep::GoogleKeyEntry`] or [`OnboardingStep::Confirmation`].
    pub fn new_consent_review(models: Vec<LocalModelLicense>, branch: OnboardingBranch) -> Self {
        let n = models.len();
        let step = if n > 0 {
            OnboardingStep::LicenseReview { model_index: 0 }
        } else {
            OnboardingStep::Confirmation
        };
        Self {
            branch,
            step,
            local_models: models,
            key_buffer: String::new(),
            licenses_accepted: vec![false; n],
            consent_only: true,
        }
    }

    /// Return all branch variants in the canonical display order.
    pub const fn branches() -> [OnboardingBranch; 3] {
        [
            OnboardingBranch::LocalOnly,
            OnboardingBranch::LocalGoogleFallback,
            OnboardingBranch::GoogleCloud,
        ]
    }

    // ── Navigation ────────────────────────────────────────────────────────────

    fn advance(&mut self) -> Option<OnboardingOutcome> {
        match &self.step {
            OnboardingStep::BranchSelection => {
                if self.branch.uses_local_models() && !self.local_models.is_empty() {
                    self.step = OnboardingStep::LicenseReview { model_index: 0 };
                } else if self.branch.requires_google_key() {
                    self.step = OnboardingStep::GoogleKeyEntry;
                } else {
                    self.step = OnboardingStep::Confirmation;
                }
                None
            }
            OnboardingStep::LicenseReview { model_index } => {
                let idx = *model_index;
                if idx < self.licenses_accepted.len() {
                    self.licenses_accepted[idx] = true;
                }
                let next = idx + 1;
                if next < self.local_models.len() {
                    self.step = OnboardingStep::LicenseReview { model_index: next };
                } else if self.consent_only {
                    return Some(OnboardingOutcome::Done(OnboardingConfigPatch {
                        branch: self.branch,
                        google_api_key: None,
                    }));
                } else if self.branch.requires_google_key() {
                    self.step = OnboardingStep::GoogleKeyEntry;
                } else {
                    self.step = OnboardingStep::Confirmation;
                }
                None
            }
            OnboardingStep::GoogleKeyEntry => {
                self.step = OnboardingStep::Confirmation;
                None
            }
            OnboardingStep::Confirmation => {
                let key = if self.branch.requires_google_key() {
                    let k = self.key_buffer.trim().to_owned();
                    if k.is_empty() {
                        self.step = OnboardingStep::GoogleKeyEntry;
                        return None;
                    } else {
                        Some(k)
                    }
                } else {
                    None
                };
                Some(OnboardingOutcome::Done(OnboardingConfigPatch {
                    branch: self.branch,
                    google_api_key: key,
                }))
            }
        }
    }

    fn go_back(&mut self) -> Option<OnboardingOutcome> {
        match &self.step {
            OnboardingStep::BranchSelection => Some(OnboardingOutcome::Cancelled),
            OnboardingStep::LicenseReview { model_index } => {
                let idx = *model_index;
                if idx == 0 {
                    if self.consent_only {
                        return Some(OnboardingOutcome::Cancelled);
                    }
                    self.step = OnboardingStep::BranchSelection;
                } else {
                    let prev = idx - 1;
                    if prev < self.licenses_accepted.len() {
                        self.licenses_accepted[prev] = false;
                    }
                    self.step = OnboardingStep::LicenseReview { model_index: prev };
                }
                None
            }
            OnboardingStep::GoogleKeyEntry => {
                if self.branch.uses_local_models() && !self.local_models.is_empty() {
                    let last = self.local_models.len() - 1;
                    if last < self.licenses_accepted.len() {
                        self.licenses_accepted[last] = false;
                    }
                    self.step = OnboardingStep::LicenseReview { model_index: last };
                } else {
                    self.step = OnboardingStep::BranchSelection;
                }
                None
            }
            OnboardingStep::Confirmation => {
                if self.branch.requires_google_key() {
                    self.step = OnboardingStep::GoogleKeyEntry;
                } else if self.branch.uses_local_models() && !self.local_models.is_empty() {
                    let last = self.local_models.len() - 1;
                    if last < self.licenses_accepted.len() {
                        self.licenses_accepted[last] = false;
                    }
                    self.step = OnboardingStep::LicenseReview { model_index: last };
                } else {
                    self.step = OnboardingStep::BranchSelection;
                }
                None
            }
        }
    }

    fn rotate_branch_up(&mut self) {
        self.branch = match self.branch {
            OnboardingBranch::LocalOnly => OnboardingBranch::GoogleCloud,
            OnboardingBranch::LocalGoogleFallback => OnboardingBranch::LocalOnly,
            OnboardingBranch::GoogleCloud => OnboardingBranch::LocalGoogleFallback,
        };
    }

    fn rotate_branch_down(&mut self) {
        self.branch = match self.branch {
            OnboardingBranch::LocalOnly => OnboardingBranch::LocalGoogleFallback,
            OnboardingBranch::LocalGoogleFallback => OnboardingBranch::GoogleCloud,
            OnboardingBranch::GoogleCloud => OnboardingBranch::LocalOnly,
        };
    }

    // ── Main event handler ────────────────────────────────────────────────────

    /// Process one input event and return the terminal outcome, if any.
    ///
    /// Returns `None` while the wizard is still running.  Returns
    /// `Some(outcome)` when it terminates (either completed or cancelled).
    ///
    /// Forbidden runtime-shortcut keys (`L T M S R ? Q Space`) must arrive
    /// as [`OnboardingEvent::Ignored`]; they produce an explicit no-op.
    pub fn handle(&mut self, event: OnboardingEvent) -> Option<OnboardingOutcome> {
        // Clone the step discriminant to avoid borrow conflicts in `advance`/`go_back`.
        match self.step.clone() {
            OnboardingStep::BranchSelection => match event {
                OnboardingEvent::SelectBranch1 => {
                    self.branch = OnboardingBranch::LocalOnly;
                    None
                }
                OnboardingEvent::SelectBranch2 => {
                    self.branch = OnboardingBranch::LocalGoogleFallback;
                    None
                }
                OnboardingEvent::SelectBranch3 => {
                    self.branch = OnboardingBranch::GoogleCloud;
                    None
                }
                OnboardingEvent::ArrowUp => {
                    self.rotate_branch_up();
                    None
                }
                OnboardingEvent::ArrowDown => {
                    self.rotate_branch_down();
                    None
                }
                OnboardingEvent::Enter => self.advance(),
                OnboardingEvent::Escape => Some(OnboardingOutcome::Cancelled),
                // All other keys (including Ignored, Char, Backspace) are no-ops.
                _ => None,
            },
            OnboardingStep::LicenseReview { .. } => match event {
                OnboardingEvent::Enter => self.advance(),
                OnboardingEvent::Escape => self.go_back(),
                _ => None,
            },
            OnboardingStep::GoogleKeyEntry => match event {
                OnboardingEvent::Char(c) => {
                    self.key_buffer.push(c);
                    None
                }
                OnboardingEvent::Backspace => {
                    self.key_buffer.pop();
                    None
                }
                OnboardingEvent::Enter => self.advance(),
                OnboardingEvent::Escape => self.go_back(),
                _ => None,
            },
            OnboardingStep::Confirmation => match event {
                OnboardingEvent::Enter => self.advance(),
                OnboardingEvent::Escape => self.go_back(),
                _ => None,
            },
        }
    }

    // ── Query helpers ─────────────────────────────────────────────────────────

    /// Returns the model index if the wizard is currently on a license-review step.
    pub fn current_license_index(&self) -> Option<usize> {
        match self.step {
            OnboardingStep::LicenseReview { model_index } => Some(model_index),
            _ => None,
        }
    }

    /// Returns the full verbatim license text for the model under review.
    ///
    /// Returns `None` when the wizard is not on a license-review step.
    /// The text is never truncated.
    pub fn current_license_text(&self) -> Option<&str> {
        let idx = self.current_license_index()?;
        self.local_models.get(idx).map(|m| m.license_text.as_str())
    }
}

// ── Deterministic text renderer ───────────────────────────────────────────────

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
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
}
