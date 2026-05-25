//! Virtual clock for the deterministic simulation harness.
//!
//! Tests advance time explicitly with [`FakeClock::advance`] (sync) or
//! [`FakeClock::sleep`] (async). The clock never reads wall time, so
//! latency-driven branches in the pipeline can be exercised without
//! actually waiting.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Monotonically increasing virtual clock measured in nanoseconds since
/// the start of the simulation.
///
/// The clock is cheaply cloneable (`Arc`-backed) so the same instance
/// can be shared between the feeder, the fake providers, and the
/// recorder.
#[derive(Debug, Clone, Default)]
pub struct FakeClock {
    elapsed_ns: Arc<AtomicU64>,
}

impl FakeClock {
    /// Construct a fresh clock starting at virtual time `0 ns`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current virtual time as a [`Duration`] since simulation start.
    pub fn elapsed(&self) -> Duration {
        Duration::from_nanos(self.elapsed_ns.load(Ordering::SeqCst))
    }

    /// Move the virtual clock forward by `delta`.
    ///
    /// Saturates at [`u64::MAX`] nanoseconds; the simulation horizon
    /// (~584 years) is well beyond any realistic test scenario.
    pub fn advance(&self, delta: Duration) {
        let ns = u64::try_from(delta.as_nanos()).unwrap_or(u64::MAX);
        self.elapsed_ns.fetch_add(ns, Ordering::SeqCst);
    }

    /// Async equivalent of [`FakeClock::advance`].
    ///
    /// Advances the virtual clock and yields the current task once so
    /// other ready tasks make progress (useful when a fake provider's
    /// scripted latency interleaves with an `await` on a channel).
    pub async fn sleep(&self, delta: Duration) {
        self.advance(delta);
        tokio::task::yield_now().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_starts_at_zero() {
        let clock = FakeClock::new();
        assert_eq!(clock.elapsed(), Duration::ZERO);
    }

    #[test]
    fn advance_is_additive() {
        let clock = FakeClock::new();
        clock.advance(Duration::from_millis(100));
        clock.advance(Duration::from_millis(250));
        assert_eq!(clock.elapsed(), Duration::from_millis(350));
    }

    #[test]
    fn clones_share_state() {
        let clock = FakeClock::new();
        let twin = clock.clone();
        twin.advance(Duration::from_secs(1));
        assert_eq!(clock.elapsed(), Duration::from_secs(1));
    }
}
