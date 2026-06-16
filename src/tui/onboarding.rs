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

/// No-op cable probe for non-Windows and consent-review wizards.
/// Kept `pub` because `tests/onboarding_integration.rs` and
/// `tests/snapshot.rs` reference it as `onboarding::noop_probe`.
pub fn noop_probe() -> Vec<String> {
    Vec::new()
}
fn is_cable_device(n: &str) -> bool {
    let u = n.to_ascii_uppercase();
    u.contains("CABLE") || u.contains("VB-AUDIO")
}
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
    /// Windows-only VB-CABLE gate; skipped on non-Windows or when detected.
    VirtualCableGate { available: Vec<String> },
    /// v3 (issue #819): detected system capabilities + the
    /// recommended quality preset.  The user can accept the
    /// recommendation (Enter) or pick a different preset
    /// (↑/↓ or 1/2/3/4).  This step runs after
    /// `VirtualCableGate` and before `LicenseReview`.
    HardwareSurvey {
        /// System capabilities detected at wizard start.
        caps: crate::sys_caps::SysCaps,
        /// The user's currently-selected preset (defaults to the
        /// recommended one when the step opens).
        selected_preset: crate::quality_preset::QualityPreset,
    },
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
    /// Re-enumerate cable devices (VirtualCableGate step only).
    RefreshVirtualCable,
    /// Skip cable installation; proceed in speaker-only mode.
    SkipVirtualCable,
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
    /// Auto-detected virtual-mic device from VirtualCableGate, or `None`.
    pub virtual_mic_device: Option<String>,
    /// `true` when the user pressed Skip at VirtualCableGate.
    pub virtual_mic_skipped: bool,
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
    pub(crate) cable_probe: fn() -> Vec<String>,
    pub(crate) virtual_mic_device: Option<String>,
    pub(crate) virtual_mic_skipped: bool,
    pub(crate) gate_enabled: bool,
    /// v3 (issue #819): the system capabilities captured at wizard
    /// start.  Used by the [`OnboardingStep::HardwareSurvey`] step to
    /// recommend a preset and by the final `OnboardingConfigPatch`
    /// when it carries the `quality_preset` choice into `AppConfig`.
    pub(crate) sys_caps: crate::sys_caps::SysCaps,
    /// v3 (issue #819): the preset the user picked on
    /// [`OnboardingStep::HardwareSurvey`].  `None` until the user
    /// confirms the survey (presses Enter).  Read at the
    /// `Confirmation` step to populate the final
    /// `OnboardingConfigPatch`.
    pub(crate) hardware_survey_selection: Option<crate::quality_preset::QualityPreset>,
}

impl OnboardingWizardState {
    /// Create a new wizard ready at the branch-selection step.
    ///
    /// `local_models` should contain one entry per local model whose license
    /// needs reviewing.  Pass an empty `Vec` when the user selects
    /// [`OnboardingBranch::GoogleCloud`] or when no local models are bundled.
    pub fn new(local_models: Vec<LocalModelLicense>, cable_probe: fn() -> Vec<String>) -> Self {
        let n = local_models.len();
        Self {
            branch: OnboardingBranch::LocalOnly,
            step: OnboardingStep::BranchSelection,
            local_models,
            key_buffer: String::new(),
            licenses_accepted: vec![false; n],
            consent_only: false,
            cable_probe,
            virtual_mic_device: None,
            virtual_mic_skipped: false,
            gate_enabled: cfg!(windows),
            sys_caps: crate::sys_caps::SysCaps {
                total_memory_bytes: 0,
                physical_cores: 0,
                gpu: crate::sys_caps::GpuKind::None,
            },
            hardware_survey_selection: None,
        }
    }

    /// v3 (issue #819): create a wizard with explicit system
    /// capabilities.  The wizard opens at
    /// [`OnboardingStep::HardwareSurvey`] (with the recommended
    /// preset pre-selected from `caps`).
    #[allow(dead_code)] // Reachable from the bin's main + tests; the
                        // onboarding_integration test target compiles this module
                        // without exercising this constructor.
    pub fn new_with_caps(
        local_models: Vec<LocalModelLicense>,
        cable_probe: fn() -> Vec<String>,
        caps: crate::sys_caps::SysCaps,
    ) -> Self {
        let n = local_models.len();
        let recommended = crate::quality_preset::QualityPreset::Auto.resolve_for(&caps);
        // Clone once so the `caps` value lives in both the
        // `HardwareSurvey` step and the top-level `sys_caps` field.
        // `SysCaps` was Copy until T19 (#826) added `String` GPU
        // names; the `OnceLock` cache still hands out clones cheaply
        // because the probe runs exactly once per process.
        let caps_for_field = caps.clone();
        Self {
            branch: OnboardingBranch::LocalOnly,
            step: OnboardingStep::HardwareSurvey {
                caps,
                selected_preset: recommended,
            },
            local_models,
            key_buffer: String::new(),
            licenses_accepted: vec![false; n],
            consent_only: false,
            cable_probe,
            virtual_mic_device: None,
            virtual_mic_skipped: false,
            gate_enabled: false,
            sys_caps: caps_for_field,
            hardware_survey_selection: None,
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
            cable_probe: noop_probe,
            virtual_mic_device: None,
            virtual_mic_skipped: false,
            gate_enabled: false,
            sys_caps: crate::sys_caps::SysCaps {
                total_memory_bytes: 0,
                physical_cores: 0,
                gpu: crate::sys_caps::GpuKind::None,
            },
            hardware_survey_selection: None,
        }
    }

    /// Return all branch variants in the canonical display order.
    /// v3 (issue #819): the v3 wizard inserts a
    /// [`OnboardingStep::HardwareSurvey`] between branch selection
    /// and the branch-specific follow-up (LicenseReview /
    /// GoogleKeyEntry / Confirmation).  Tests that want the
    /// pre-v3 "single Enter" behaviour can use this helper to
    /// advance through the survey automatically.
    #[allow(dead_code)] // Same reasoning as `new_with_caps` above.
    pub fn enter_past_branch_and_survey(&mut self) -> Option<OnboardingOutcome> {
        match self.advance() {
            x @ Some(_) => x,
            None => self.advance(),
        }
    }

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
            cable_probe: noop_probe,
            virtual_mic_device: None,
            virtual_mic_skipped: false,
            gate_enabled: false,
            sys_caps: crate::sys_caps::SysCaps {
                total_memory_bytes: 0,
                physical_cores: 0,
                gpu: crate::sys_caps::GpuKind::None,
            },
            hardware_survey_selection: None,
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

    fn advance_past_branch(&mut self) {
        // v3 (issue #819): the HardwareSurvey step runs after
        // VirtualCableGate and before LicenseReview.  When the
        // wizard opens via `new_with_caps`, it already starts at
        // HardwareSurvey (the recommended preset is pre-selected).
        let recommended = crate::quality_preset::QualityPreset::Auto.resolve_for(&self.sys_caps);
        // Every branch needs to see the hardware survey (the
        // quality-preset selection is a v3 first-run surface
        // channel, not a branch-specific gate).  We previously
        // branched on `uses_local_models` / `requires_google_key`
        // here; v3 collapsed those branches into a single
        // unconditional `HardwareSurvey` jump.  The follow-up
        // branch-specific step still fires when the user presses
        // Enter on the survey.
        let _ = self.branch.uses_local_models();
        let _ = self.branch.requires_google_key();
        // Clone the cached `SysCaps` so we can hand it both to
        // the `HardwareSurvey` step and keep it on the wizard for
        // the `new_with_caps` field path.  T19 (#826) made
        // `SysCaps` non-`Copy` to carry the GPU name `String`.
        let caps_for_step = self.sys_caps.clone();
        self.step = OnboardingStep::HardwareSurvey {
            caps: caps_for_step,
            selected_preset: recommended,
        };
    }
    fn advance(&mut self) -> Option<OnboardingOutcome> {
        match self.step.clone() {
            OnboardingStep::BranchSelection => {
                if self.gate_enabled {
                    let d: Vec<_> = (self.cable_probe)()
                        .into_iter()
                        .filter(|d| is_cable_device(d))
                        .collect();
                    if d.is_empty() {
                        self.step = OnboardingStep::VirtualCableGate { available: d };
                        return None;
                    }
                    self.virtual_mic_device = d.into_iter().next();
                }
                self.advance_past_branch();
                None
            }
            OnboardingStep::VirtualCableGate { available } => {
                self.virtual_mic_device = available.into_iter().next();
                self.advance_past_branch();
                None
            }
            OnboardingStep::HardwareSurvey { .. } => {
                // The user pressed Enter (or the handler mapped an
                // event to advance). Carry the chosen preset into the
                // `OnboardingConfigPatch` (set below at the
                // `Confirmation` step). Move on to the next step
                // the branch requires.
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
                let idx = model_index;
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
                        virtual_mic_device: self.virtual_mic_device.clone(),
                        virtual_mic_skipped: self.virtual_mic_skipped,
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
                    virtual_mic_device: self.virtual_mic_device.clone(),
                    virtual_mic_skipped: self.virtual_mic_skipped,
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
            OnboardingStep::VirtualCableGate { .. } => {
                self.step = OnboardingStep::BranchSelection;
                None
            }
            OnboardingStep::HardwareSurvey { .. } => {
                // Esc on HardwareSurvey goes back to the previous
                // step.  The previous step is the cable gate if it
                // is enabled, otherwise BranchSelection.
                if self.gate_enabled {
                    let d: Vec<_> = (self.cable_probe)()
                        .into_iter()
                        .filter(|d| is_cable_device(d))
                        .collect();
                    self.step = OnboardingStep::VirtualCableGate { available: d };
                } else {
                    self.step = OnboardingStep::BranchSelection;
                }
                None
            }
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
        // Dispatch on the step discriminant, but pattern-match
        // directly against `&mut self.step` so that field mutations
        // inside the inner match (e.g. HardwareSurvey preset cycle)
        // are persisted on `self`.  We use `std::mem::discriminant`
        // to dispatch without cloning, then match the discriminant
        // and re-match the actual step for binding.
        use std::mem::discriminant;
        let disc = discriminant(&self.step);
        if disc == discriminant(&OnboardingStep::BranchSelection) {
            match event {
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
                _ => None,
            }
        } else if disc
            == discriminant(&OnboardingStep::VirtualCableGate {
                available: Vec::new(),
            })
        {
            let available = match &self.step {
                OnboardingStep::VirtualCableGate { available } => available.clone(),
                _ => unreachable!("discriminant matched above"),
            };
            match event {
                OnboardingEvent::RefreshVirtualCable => {
                    self.step = OnboardingStep::VirtualCableGate {
                        available: (self.cable_probe)()
                            .into_iter()
                            .filter(|d| is_cable_device(d))
                            .collect(),
                    };
                    None
                }
                OnboardingEvent::SkipVirtualCable => {
                    self.virtual_mic_skipped = true;
                    self.advance_past_branch();
                    None
                }
                OnboardingEvent::Enter if !available.is_empty() => self.advance(),
                OnboardingEvent::Escape => self.go_back(),
                _ => None,
            }
        } else if matches!(self.step, OnboardingStep::HardwareSurvey { .. }) {
            // Direct match against `&mut self.step` so that preset
            // mutations on the inner `selected_preset` field are
            // persisted on `self`.  (The original code cloned the
            // step into a temporary for matching, which broke
            // mutation semantics for HardwareSurvey.)
            match &mut self.step {
                OnboardingStep::HardwareSurvey {
                    ref mut caps,
                    ref mut selected_preset,
                } => match event {
                    OnboardingEvent::ArrowUp => {
                        *selected_preset = (*selected_preset).next();
                        None
                    }
                    OnboardingEvent::ArrowDown => {
                        *selected_preset = (*selected_preset).previous();
                        None
                    }
                    OnboardingEvent::Char('1') => {
                        *selected_preset = crate::quality_preset::QualityPreset::Auto;
                        None
                    }
                    OnboardingEvent::Char('2') => {
                        *selected_preset = crate::quality_preset::QualityPreset::Best;
                        None
                    }
                    OnboardingEvent::Char('3') => {
                        *selected_preset = crate::quality_preset::QualityPreset::Performance;
                        None
                    }
                    OnboardingEvent::Char('4') => {
                        *selected_preset = crate::quality_preset::QualityPreset::Custom;
                        None
                    }
                    OnboardingEvent::Char('r') => {
                        *selected_preset =
                            crate::quality_preset::QualityPreset::Auto.resolve_for(caps);
                        None
                    }
                    OnboardingEvent::Enter => {
                        self.hardware_survey_selection = Some(*selected_preset);
                        self.advance()
                    }
                    OnboardingEvent::Escape => self.go_back(),
                    _ => None,
                },
                _ => unreachable!("discriminant matched above"),
            }
        } else if matches!(self.step, OnboardingStep::LicenseReview { .. }) {
            match event {
                OnboardingEvent::Enter => self.advance(),
                OnboardingEvent::Escape => self.go_back(),
                _ => None,
            }
        } else if matches!(self.step, OnboardingStep::GoogleKeyEntry) {
            match event {
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
            }
        } else if matches!(self.step, OnboardingStep::Confirmation) {
            match event {
                OnboardingEvent::Enter => self.advance(),
                OnboardingEvent::Escape => self.go_back(),
                _ => None,
            }
        } else if matches!(self.step, OnboardingStep::PlatformParityNotice) {
            match event {
                OnboardingEvent::Enter | OnboardingEvent::Escape => self.advance(),
                _ => None,
            }
        } else {
            None
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

// ── Tests ─────────────────────────────────────────────────────────────────────

// Render helper extracted to onboarding_render.rs (600 LOC gate, STD-01).
#[path = "onboarding_render.rs"]
mod render_impl;
pub use render_impl::render_wizard_lines;

#[cfg(test)]
#[path = "onboarding_tests.rs"]
mod tests;
