//! Deterministic monotonic-nanosecond clock used by the QA8-07 tests
//! and the #460 simulation harness.

use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic nanosecond clock for deterministic tests. Production code
/// uses `Instant::now().elapsed()`; the #460 simulation harness uses
/// this so all `record_chunk_at` / `record_exit` arguments are
/// reproducible.
#[derive(Debug, Default)]
pub struct FakeNanoClock {
    ns: AtomicU64,
}

impl FakeNanoClock {
    /// Start a fresh clock at `0 ns`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the current virtual time.
    pub fn now_ns(&self) -> u64 {
        self.ns.load(Ordering::Relaxed)
    }

    /// Advance the virtual clock by `delta_ns` and return the new value.
    pub fn advance_ns(&self, delta_ns: u64) -> u64 {
        self.ns.fetch_add(delta_ns, Ordering::Relaxed) + delta_ns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_zero() {
        let c = FakeNanoClock::new();
        assert_eq!(c.now_ns(), 0);
    }

    #[test]
    fn advance_accumulates() {
        let c = FakeNanoClock::new();
        assert_eq!(c.advance_ns(100), 100);
        assert_eq!(c.advance_ns(50), 150);
        assert_eq!(c.now_ns(), 150);
    }
}
