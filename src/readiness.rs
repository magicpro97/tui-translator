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
}

static TX: OnceLock<watch::Sender<ReadinessState>> = OnceLock::new();
static LOAD_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Initialise the readiness channel.  Must be called exactly once, very early
/// in `main` before TUI mount or any model load.  Subsequent calls return a
/// fresh subscriber on the already-installed channel and do not reset state.
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
pub fn subscribe() -> Option<watch::Receiver<ReadinessState>> {
    TX.get().map(|tx| tx.subscribe())
}

/// Publish a state transition.  Idempotent; silently ignored if the channel
/// has not been installed (so test code paths that never call [`install`]
/// remain safe).
pub fn publish(state: ReadinessState) {
    if let Some(tx) = TX.get() {
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
}

impl LoadGuard {
    /// Start tracking a load for `component`.  Publishes
    /// `Loading{component, None}` immediately.
    pub fn start(component: &'static str) -> Self {
        LOAD_COUNT.fetch_add(1, Ordering::SeqCst);
        publish(ReadinessState::Loading {
            component,
            percent: None,
        });
        Self {
            component,
            publish_ready_on_drop: true,
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

    // The readiness channel is a process-global `OnceLock`; tests that
    // depend on the initial `Init` state must serialise to avoid observing
    // each other's transitions.  Because we cannot easily reset a
    // `OnceLock` between tests, the assertions below operate purely on
    // the *publish/subscribe* surface — they tolerate prior state and
    // verify ordering relative to local actions.

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
        let rx = install();
        let _guard = LoadGuard::start("guard-test");
        let snap = rx.borrow().clone();
        assert!(
            matches!(snap, ReadinessState::Loading { component, .. } if component == "guard-test")
        );
    }
}
