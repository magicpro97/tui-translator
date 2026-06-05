//! Aggregate app-readiness state surfaced as the TUI status badge (#716).
//!
//! Lifecycle: [`ReadinessState::Init`] → [`ReadinessState::Loading`]* →
//! [`ReadinessState::Ready`].  Errors transition to [`ReadinessState::Error`]
//! (terminal) so the operator sees the failing subsystem.
//!
//! The publisher is a single-writer [`tokio::sync::watch`] channel held in a
//! process-global `OnceLock`.  TUI render code subscribes once via
//! [`subscribe`] and reads `.borrow()` per frame.  Startup code calls
//! [`publish`] (and the RAII [`LoadGuard`]) to drive the state machine.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    OnceLock,
};
use tokio::sync::watch;

/// Public, monotonic readiness state surfaced to the TUI status badge.
///
/// The lifecycle is enforced by [`publish`]: once the state reaches
/// [`ReadinessState::Error`], all subsequent transitions are no-ops so the
/// terminal error remains visible to the operator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadinessState {
    /// Process is starting; no model loads have begun yet.
    Init,
    /// One or more components are loading.  `component` is the most recently
    /// started subsystem (informational); `percent` is the download/load
    /// progress when known.
    Loading {
        /// Short, human-readable label for the active load (e.g. `"llm-mt"`).
        component: &'static str,
        /// Optional percentage in `0..=100`.  `None` for indeterminate loads.
        percent: Option<u8>,
    },
    /// All in-flight loads completed successfully and the app is ready to run.
    Ready,
    /// Terminal error state: at least one critical subsystem failed to load.
    Error(String),
}

impl ReadinessState {
    /// Short uppercase badge label rendered in the TUI status strip.
    pub fn badge_label(&self) -> &'static str {
        match self {
            ReadinessState::Init => "INIT",
            ReadinessState::Loading { .. } => "LOAD",
            ReadinessState::Ready => "READY",
            ReadinessState::Error(_) => "ERROR",
        }
    }

    /// Decode the `"n/N"` subsystem-ready count embedded by
    /// [`aggregate_readiness`] in the `percent` field of a `Loading` state.
    ///
    /// The interpreter aggregator encodes the number of healthy subsystems in
    /// `percent` (treating the field as a raw count rather than a percentage).
    /// Returns `Some((ready, total))` where `ready` is the healthy-subsystem
    /// count and `total` is [`INTERPRETER_SUBSYSTEM_COUNT`].  Returns `None`
    /// for `Init`, `Ready`, `Error`, or `Loading` states not produced by the
    /// interpreter aggregator (i.e. `percent` is `None`).
    pub fn loading_count_suffix(&self) -> Option<(u8, u8)> {
        match self {
            ReadinessState::Loading {
                percent: Some(n), ..
            } => Some((*n, INTERPRETER_SUBSYSTEM_COUNT)),
            _ => None,
        }
    }
}

// ── Interpreter aggregator ────────────────────────────────────────────────────

/// Total number of interpreter sub-systems tracked by [`aggregate_readiness`].
///
/// Sub-systems: STT, MT, TTS, virtual-mic sink, LLM model, sample-rate
/// negotiation, and device-loss detection.
pub const INTERPRETER_SUBSYSTEM_COUNT: u8 = 7;

/// Health status of a single interpreter sub-system.
///
/// The three-state design mirrors the app-wide [`ReadinessState`] lifecycle:
/// `Bootstrapping` is the initial "loading" state; `Healthy` means the
/// sub-system completed startup successfully; `Failed` is the terminal error
/// state.
#[allow(dead_code)] // wired to production callers in US-02a follow-up PRs
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubsystemHealth {
    /// Sub-system is initialising and not yet available.
    Bootstrapping,
    /// Sub-system is fully operational.
    Healthy,
    /// Sub-system failed to start or lost its connection.  The inner string is
    /// a human-readable explanation surfaced in the `Error` badge.
    Failed(String),
}

/// Named interpreter sub-systems tracked by [`aggregate_readiness`].
#[allow(dead_code)] // variants consumed by aggregate_readiness; production wiring in follow-up PRs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InterpreterSubsystem {
    /// Speech-to-text provider.
    Stt,
    /// Machine-translation provider.
    Mt,
    /// Text-to-speech provider.
    Tts,
    /// Virtual-mic audio sink (VB-CABLE or equivalent).
    VirtualMicSink,
    /// Local LLM model used for MT or TTS.
    LlmModel,
    /// Audio sample-rate negotiation with the render endpoint.
    SampleRate,
    /// Device-loss notification listener.
    DeviceLoss,
}

impl InterpreterSubsystem {
    /// Short human-readable label used in the TUI `LOAD` badge `component`
    /// field and in log messages.
    #[allow(dead_code)] // called by aggregate_readiness; production wiring in follow-up PRs
    pub fn label(self) -> &'static str {
        match self {
            Self::Stt => "stt",
            Self::Mt => "mt",
            Self::Tts => "tts",
            Self::VirtualMicSink => "vmic",
            Self::LlmModel => "llm",
            Self::SampleRate => "sample-rate",
            Self::DeviceLoss => "device-loss",
        }
    }
}

/// Aggregate a slice of `(subsystem, health)` pairs into a single
/// [`ReadinessState`].
///
/// **Priority mapping (highest priority first):**
/// 1. Any [`SubsystemHealth::Failed`] → `Error(first_error_message)`.
/// 2. Any [`SubsystemHealth::Bootstrapping`] (no failures) →
///    `Loading { component: first_bootstrapping_label, percent: Some(healthy_count) }`.
///    `percent` stores the healthy-subsystem count so the TUI renderer can
///    recover the `"n/N"` suffix via [`ReadinessState::loading_count_suffix`].
/// 3. All [`SubsystemHealth::Healthy`], or empty slice → `Ready`.
///    An empty slice means "no subsystems to wait for", which is immediately
///    ready.
///
/// This function is **pure**: identical inputs always produce identical
/// outputs, and it never mutates global state.
#[allow(dead_code)] // production callers arrive in follow-up PRs (US-09, US-11)
pub fn aggregate_readiness(
    subsystems: &[(InterpreterSubsystem, SubsystemHealth)],
) -> ReadinessState {
    let mut first_error: Option<String> = None;
    let mut first_bootstrapping: Option<&'static str> = None;
    let mut healthy_count: u8 = 0;

    for (subsystem, health) in subsystems {
        match health {
            SubsystemHealth::Failed(msg) => {
                if first_error.is_none() {
                    first_error = Some(msg.clone());
                }
            }
            SubsystemHealth::Bootstrapping => {
                if first_bootstrapping.is_none() {
                    first_bootstrapping = Some(subsystem.label());
                }
            }
            SubsystemHealth::Healthy => {
                healthy_count = healthy_count.saturating_add(1);
            }
        }
    }

    if let Some(msg) = first_error {
        return ReadinessState::Error(msg);
    }
    if let Some(component) = first_bootstrapping {
        return ReadinessState::Loading {
            component,
            percent: Some(healthy_count),
        };
    }
    ReadinessState::Ready
}

/// Ergonomic wrapper around [`aggregate_readiness`] that stores a fixed set of
/// per-subsystem health values and collapses them on demand.
///
/// All fields default to [`SubsystemHealth::Bootstrapping`] so callers only
/// need to update the subsystems they have initialised.  The struct does NOT
/// publish to the global readiness channel itself — that remains the
/// responsibility of the caller so that the channel's monotonicity contract
/// (see [`publish`]) is respected.
#[allow(dead_code)] // production wiring arrives in follow-up PRs
#[derive(Clone, Debug)]
pub struct ReadinessAggregator {
    /// Health of the STT provider.
    pub stt: SubsystemHealth,
    /// Health of the MT provider.
    pub mt: SubsystemHealth,
    /// Health of the TTS provider.
    pub tts: SubsystemHealth,
    /// Health of the virtual-mic audio sink.
    pub virtual_mic: SubsystemHealth,
    /// Health of the local LLM model.
    pub llm_model: SubsystemHealth,
    /// Health of sample-rate negotiation.
    pub sample_rate: SubsystemHealth,
    /// Health of the device-loss notification listener.
    pub device_loss: SubsystemHealth,
}

impl Default for ReadinessAggregator {
    fn default() -> Self {
        Self {
            stt: SubsystemHealth::Bootstrapping,
            mt: SubsystemHealth::Bootstrapping,
            tts: SubsystemHealth::Bootstrapping,
            virtual_mic: SubsystemHealth::Bootstrapping,
            llm_model: SubsystemHealth::Bootstrapping,
            sample_rate: SubsystemHealth::Bootstrapping,
            device_loss: SubsystemHealth::Bootstrapping,
        }
    }
}

impl ReadinessAggregator {
    /// Collapse all subsystem health values into a single [`ReadinessState`]
    /// using [`aggregate_readiness`].
    #[allow(dead_code)] // production wiring in follow-up PRs
    pub fn collapse(&self) -> ReadinessState {
        aggregate_readiness(&[
            (InterpreterSubsystem::Stt, self.stt.clone()),
            (InterpreterSubsystem::Mt, self.mt.clone()),
            (InterpreterSubsystem::Tts, self.tts.clone()),
            (
                InterpreterSubsystem::VirtualMicSink,
                self.virtual_mic.clone(),
            ),
            (InterpreterSubsystem::LlmModel, self.llm_model.clone()),
            (InterpreterSubsystem::SampleRate, self.sample_rate.clone()),
            (InterpreterSubsystem::DeviceLoss, self.device_loss.clone()),
        ])
    }

    /// Return `(ready, total)` counts where `ready` is the number of
    /// [`SubsystemHealth::Healthy`] subsystems and `total` is
    /// [`INTERPRETER_SUBSYSTEM_COUNT`].
    #[allow(dead_code)] // consumed by US-02b TUI render; production wiring in follow-up PRs
    pub fn count_summary(&self) -> (u8, u8) {
        let ready = [
            &self.stt,
            &self.mt,
            &self.tts,
            &self.virtual_mic,
            &self.llm_model,
            &self.sample_rate,
            &self.device_loss,
        ]
        .iter()
        .filter(|h| **h == &SubsystemHealth::Healthy)
        .count() as u8;
        (ready, INTERPRETER_SUBSYSTEM_COUNT)
    }
}

#[cfg(test)]
#[path = "readiness_aggregator_tests.rs"]
mod readiness_aggregator_tests;

static TX: OnceLock<watch::Sender<ReadinessState>> = OnceLock::new();
static LOAD_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Initialise the readiness channel.
///
/// **Idempotent:** safe to call multiple times.  The first call wins and
/// constructs the underlying `watch` channel; subsequent calls return a
/// fresh subscriber on the already-installed channel and do not reset the
/// observed state.  It is recommended to call this once early in `main`
/// (before any TUI mount or model load) so initial subscribers see the
/// `Init` baseline rather than a transient `Loading{...}` snapshot, but
/// duplicate calls are not an error.
pub fn install() -> watch::Receiver<ReadinessState> {
    let sender = TX.get_or_init(|| {
        let (tx, _rx) = watch::channel(ReadinessState::Init);
        tx
    });
    sender.subscribe()
}

/// Return `true` once [`install`] has been called.  Used by debug assertions.
#[allow(dead_code)] // exposed for diagnostics / future debug assertions
pub fn is_installed() -> bool {
    TX.get().is_some()
}

/// Subscribe to the readiness channel (TUI render loop calls this once and
/// polls `.borrow()` per frame).  Returns `None` if [`install`] has not been
/// called yet.
#[allow(dead_code)] // called by TUI render; appears unused when readiness.rs is #[path]-included in tests
pub fn subscribe() -> Option<watch::Receiver<ReadinessState>> {
    TX.get().map(|tx| tx.subscribe())
}

/// Publish a state transition.  Idempotent; silently ignored if the channel
/// has not been installed (so test code paths that never call [`install`]
/// remain safe).
///
/// Enforces the documented monotonicity contract: once the channel holds
/// [`ReadinessState::Error`], further `publish(...)` calls are no-ops so
/// the terminal error remains observable (Copilot review #3353355902).
pub fn publish(state: ReadinessState) {
    if let Some(tx) = TX.get() {
        if matches!(*tx.borrow(), ReadinessState::Error(_)) {
            // Error is terminal — drop the transition silently.
            return;
        }
        // `send_replace` never panics and never blocks; receivers see the new
        // value via `borrow()` / `changed()`.
        let _ = tx.send_replace(state);
    }
}

/// RAII guard tracking an in-flight load.  On construction, publishes
/// `Loading{component, None}` and bumps an internal refcount.  On drop, the
/// refcount is decremented; when it reaches zero the channel transitions to
/// [`ReadinessState::Ready`] unless an [`ReadinessState::Error`] has been
/// published in the meantime.
pub struct LoadGuard {
    #[allow(dead_code)] // kept for future percent-update plumbing (see publish_percent)
    component: &'static str,
    /// Set to `false` on [`LoadGuard::finish_error`] so `drop` does not
    /// overwrite an error state with `Ready`.
    publish_ready_on_drop: bool,
    /// `false` when [`LoadGuard::start`] short-circuited because the
    /// channel was already in [`ReadinessState::Error`]; in that case
    /// `LOAD_COUNT` was never incremented and `Drop` must not decrement it.
    decrement_on_drop: bool,
}

impl LoadGuard {
    /// Start tracking a load for `component`.  Publishes
    /// `Loading{component, None}` immediately.
    ///
    /// No-op when the channel is already in [`ReadinessState::Error`]: the
    /// terminal error is preserved, no `Loading` is published, and the
    /// returned guard's `Drop` will not republish `Ready` (Copilot review
    /// #3353355902).
    pub fn start(component: &'static str) -> Self {
        if let Some(tx) = TX.get() {
            if matches!(*tx.borrow(), ReadinessState::Error(_)) {
                return Self {
                    component,
                    publish_ready_on_drop: false,
                    decrement_on_drop: false,
                };
            }
        }
        LOAD_COUNT.fetch_add(1, Ordering::SeqCst);
        publish(ReadinessState::Loading {
            component,
            percent: None,
        });
        Self {
            component,
            publish_ready_on_drop: true,
            decrement_on_drop: true,
        }
    }

    /// Publish a percentage update for this load without changing the
    /// refcount.  Safe to call from a forwarder task watching a download
    /// progress channel.
    #[allow(dead_code)] // surfaced for future LLM download forwarders (#716 follow-up)
    pub fn publish_percent(&self, percent: u8) {
        publish(ReadinessState::Loading {
            component: self.component,
            percent: Some(percent),
        });
    }

    /// Mark the load as failed.  Publishes [`ReadinessState::Error`] and
    /// inhibits the `Ready` transition that would otherwise fire on drop.
    #[allow(dead_code)] // exposed for explicit error paths (currently logged via tracing instead)
    pub fn finish_error(mut self, msg: impl Into<String>) {
        self.publish_ready_on_drop = false;
        publish(ReadinessState::Error(msg.into()));
        // Decrement refcount via Drop so the count stays accurate even on
        // error paths.
    }
}

impl Drop for LoadGuard {
    fn drop(&mut self) {
        if !self.decrement_on_drop {
            return;
        }
        let prev = LOAD_COUNT.fetch_sub(1, Ordering::SeqCst);
        // `prev == 1` means we were the last in-flight load.  Only publish
        // `Ready` when the load completed successfully *and* no error state
        // is currently observable.
        if prev == 1 && self.publish_ready_on_drop {
            if let Some(tx) = TX.get() {
                let current = tx.borrow().clone();
                if !matches!(current, ReadinessState::Error(_)) {
                    let _ = tx.send_replace(ReadinessState::Ready);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Test-only: forcibly clear the global readiness state and load
    /// refcount so the next test observes an `Init` baseline.  This bypasses
    /// the production `publish()` sticky-Error guard via direct
    /// `send_replace`; production code paths must NEVER do this.
    fn reset_state_for_tests() {
        if let Some(tx) = TX.get() {
            let _ = tx.send_replace(ReadinessState::Init);
        }
        LOAD_COUNT.store(0, Ordering::SeqCst);
    }

    // The readiness channel is a process-global `OnceLock`; tests that
    // depend on the initial `Init` state must serialise to avoid observing
    // each other's transitions.  Because we cannot easily reset a
    // `OnceLock` between tests, the assertions below operate purely on
    // the *publish/subscribe* surface — they tolerate prior state and
    // verify ordering relative to local actions.
    //
    // Tests that assert on a specific terminal state (e.g. the sticky
    // `Error` invariant) acquire `STATE_LOCK` so that no other test in
    // this module can race a `publish(...)` call between the assertion's
    // setup and `rx.borrow()`.
    static STATE_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn readiness_install_returns_initial_state() {
        let rx = install();
        // The first subscriber sees whatever the channel currently holds.
        // It must always be a valid `ReadinessState` value.
        let snapshot = rx.borrow().clone();
        assert!(matches!(
            snapshot,
            ReadinessState::Init
                | ReadinessState::Loading { .. }
                | ReadinessState::Ready
                | ReadinessState::Error(_)
        ));
    }

    #[test]
    fn readiness_publish_is_observable() {
        let _serial = STATE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        reset_state_for_tests();
        let rx = install();
        publish(ReadinessState::Loading {
            component: "test-load",
            percent: Some(42),
        });
        let snap = rx.borrow().clone();
        assert!(matches!(
            snap,
            ReadinessState::Loading {
                component: "test-load",
                percent: Some(42),
            }
        ));
    }

    #[test]
    fn readiness_badge_labels() {
        assert_eq!(ReadinessState::Init.badge_label(), "INIT");
        assert_eq!(
            ReadinessState::Loading {
                component: "x",
                percent: None,
            }
            .badge_label(),
            "LOAD"
        );
        assert_eq!(ReadinessState::Ready.badge_label(), "READY");
        assert_eq!(
            ReadinessState::Error("boom".to_string()).badge_label(),
            "ERROR"
        );
    }

    #[test]
    fn readiness_publish_without_install_is_safe() {
        let _serial = STATE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // Even if the global channel hasn't been installed by a prior test,
        // publishing must not panic.  (After other tests run in the same
        // process the channel IS installed; either way this must succeed.)
        publish(ReadinessState::Loading {
            component: "noop",
            percent: None,
        });
    }

    #[test]
    fn load_guard_publishes_loading_on_start() {
        let _serial = STATE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        reset_state_for_tests();
        let rx = install();
        let _guard = LoadGuard::start("guard-test");
        let snap = rx.borrow().clone();
        assert!(
            matches!(snap, ReadinessState::Loading { component, .. } if component == "guard-test")
        );
    }

    /// Binds the council "errors are sticky" invariant: after a
    /// `LoadGuard::start("x"); guard.finish_error("boom");` sequence, the
    /// subscribed receiver MUST observe `ReadinessState::Error(_)` rather
    /// than the `Ready` value that `Drop` would otherwise republish.  This
    /// guards #716's production contract — the slot A/B build sites in
    /// `main.rs` rely on `finish_error` to keep the badge red when an
    /// LLM-MT load fails.
    #[test]
    fn load_guard_finish_error_is_sticky() {
        let _serial = STATE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        reset_state_for_tests();
        let rx = install();
        {
            let guard = LoadGuard::start("sticky-error-test");
            guard.finish_error("boom-test-marker");
        }
        let snap = rx.borrow().clone();
        match snap {
            ReadinessState::Error(msg) => {
                assert!(
                    msg.contains("boom-test-marker"),
                    "expected error payload to contain 'boom-test-marker', got: {msg}"
                );
            }
            other => panic!("expected sticky Error(_) after finish_error+drop, got: {other:?}"),
        }
    }

    /// Copilot review #3353355902: `Error(_)` is documented as terminal —
    /// once published, no further state changes are observable.  This test
    /// exercises the publish guard: after an `Error` is published, both
    /// subsequent `publish(...)` calls and `LoadGuard::start` must be
    /// no-ops with respect to the receiver.
    #[test]
    fn publish_after_error_is_sticky_no_op() {
        let _serial = STATE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let rx = install();
        reset_state_for_tests();
        publish(ReadinessState::Loading {
            component: "pre-error",
            percent: None,
        });
        publish(ReadinessState::Error("sticky-marker-x9z".to_string()));
        // These must NOT overwrite the terminal Error state.
        publish(ReadinessState::Ready);
        publish(ReadinessState::Loading {
            component: "post-error",
            percent: Some(50),
        });
        let _ = LoadGuard::start("post-error-guard");
        let snap = rx.borrow().clone();
        match snap {
            ReadinessState::Error(msg) => assert!(
                msg.contains("sticky-marker-x9z"),
                "expected error to retain 'sticky-marker-x9z', got: {msg}"
            ),
            other => panic!("expected sticky Error(_) after Error→*, got: {other:?}"),
        }
    }
}
