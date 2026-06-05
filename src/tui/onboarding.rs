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
    /// One-time informational banner shown on non-Windows platforms when
    /// `AppConfig::platform_parity_notice_seen_at` is `None`.  Dismissed by
    /// `Enter` or `Esc` → [`OnboardingOutcome::PlatformParityNoticeDismissed`].
    PlatformParityNotice,
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
    /// The user acknowledged the platform-parity notice (US-07, issue #732).
    ///
    /// Caller should write `Some(Utc::now())` to
    /// `AppConfig::platform_parity_notice_seen_at` and persist the config.
    PlatformParityNoticeDismissed,
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
    /// Create a wizard pre-positioned at [`OnboardingStep::PlatformParityNotice`].
    ///
    /// `Enter`/`Esc` produce [`OnboardingOutcome::PlatformParityNoticeDismissed`].
    pub fn new_platform_parity_notice() -> Self {
        Self {
            branch: OnboardingBranch::LocalOnly,
            step: OnboardingStep::PlatformParityNotice,
            local_models: Vec::new(),
            key_buffer: String::new(),
            licenses_accepted: Vec::new(),
            consent_only: false,
        }
    }

    /// Returns `true` when the platform-parity banner should be shown.
    /// Pass `seen_at = &config.platform_parity_notice_seen_at` and a synthetic
    /// `is_windows` in tests to exercise both branches without conditional
    /// compilation.
    pub fn platform_parity_notice_needed<T>(seen_at: &Option<T>, is_windows: bool) -> bool {
        !is_windows && seen_at.is_none()
    }

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
            OnboardingStep::PlatformParityNotice => {
                Some(OnboardingOutcome::PlatformParityNoticeDismissed)
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
            OnboardingStep::PlatformParityNotice => {
                Some(OnboardingOutcome::PlatformParityNoticeDismissed)
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
            OnboardingStep::PlatformParityNotice => match event {
                OnboardingEvent::Enter | OnboardingEvent::Escape => self.advance(),
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "onboarding_tests.rs"]
mod tests;
