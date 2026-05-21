//! CPU throttle guard for local-inference providers (issue #230).
//!
//! [`CpuGate`] evaluates whether the current process CPU usage exceeds the
//! configured budget and signals the orchestrator to skip local-inference
//! chunks when it does.  Cloud/Google provider paths are **never** throttled;
//! the gate is consulted by the orchestrator only when
//! [`OrchestratorContext::provider_is_local`] is `true`.
//!
//! # Design
//!
//! * CPU readings are shared via an internal `Arc<AtomicU32>` (stored as
//!   `(pct × 100.0) as u32`) so the throttle check is a single atomic load —
//!   cheap enough to run on every chunk arrival without adding scheduling
//!   overhead that would itself contend with Zoom or Teams.
//! * The metrics-publisher task updates the reading once per second via
//!   [`CpuGate::update_cpu_pct`]; the orchestrator reads it with
//!   `Ordering::Relaxed` (stale-by-≤1-second is acceptable for a soft guard).
//! * The gate counts its own skipped inferences so the TUI can surface the
//!   [`MetricsSnapshot::local_inferences_skipped`] field without extra shared
//!   state.
//!
//! [`OrchestratorContext::provider_is_local`]:
//!     crate::pipeline::OrchestratorContext::provider_is_local
//! [`MetricsSnapshot::local_inferences_skipped`]:
//!     crate::metrics::MetricsSnapshot::local_inferences_skipped

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

// ── CpuGate ───────────────────────────────────────────────────────────────────

/// CPU budget guard for local-inference paths.
///
/// Create one instance at startup and share it via [`Arc`].  Pass the same
/// `Arc<CpuGate>` to both the metrics-publisher task (to call
/// [`update_cpu_pct`]) and the [`OrchestratorContext`] (to call
/// [`is_throttled`]).
///
/// [`update_cpu_pct`]: CpuGate::update_cpu_pct
/// [`is_throttled`]: CpuGate::is_throttled
/// [`OrchestratorContext`]: crate::pipeline::OrchestratorContext
#[derive(Debug)]
pub struct CpuGate {
    /// Maximum CPU usage percentage above which local inference is skipped,
    /// stored as `(pct * 100.0) as u32` for lock-free hot updates.
    ///
    /// `0` disables throttling entirely (gate never fires).  On multi-core
    /// hosts `sysinfo` may report values greater than `100.0` (percentage is
    /// relative to a single logical core); set `cpu_budget_pct` accordingly --
    /// e.g. `200.0` to allow 2 full cores before throttling.
    ///
    /// Updated at runtime via [`update_budget_pct`] without requiring a
    /// process restart, mirroring the [`MemoryGuard::update_budget_bytes`]
    /// API for symmetry.
    ///
    /// [`update_budget_pct`]: CpuGate::update_budget_pct
    /// [`MemoryGuard::update_budget_bytes`]: crate::metrics::memory_guard::MemoryGuard::update_budget_bytes
    cpu_budget_pct_x100: AtomicU32,

    /// Current process CPU usage stored as `(pct * 100.0) as u32`.
    ///
    /// Written by the metrics-publisher task via [`update_cpu_pct`];
    /// read by the orchestrator via [`is_throttled`].  `Ordering::Relaxed`
    /// is used on both ends because a stale reading of up to one second is
    /// acceptable for a soft performance guard.
    ///
    /// [`update_cpu_pct`]: CpuGate::update_cpu_pct
    /// [`is_throttled`]: CpuGate::is_throttled
    cpu_pct_x100: Arc<AtomicU32>,

    /// Running count of audio chunks skipped due to CPU pressure.
    skipped: AtomicU64,
}

impl CpuGate {
    /// Create a new gate with the given CPU budget.
    ///
    /// * `cpu_budget_pct` — upper CPU-usage bound; `0.0` disables throttling.
    pub fn new(cpu_budget_pct: f32) -> Self {
        Self {
            cpu_budget_pct_x100: AtomicU32::new((cpu_budget_pct * 100.0) as u32),
            cpu_pct_x100: Arc::new(AtomicU32::new(0)),
            skipped: AtomicU64::new(0),
        }
    }

    /// Return `true` when the most recent CPU reading strictly exceeds the
    /// configured budget.
    ///
    /// Always returns `false` when `cpu_budget_pct` is `0.0` (disabled).
    pub fn is_throttled(&self) -> bool {
        let budget_x100 = self.cpu_budget_pct_x100.load(Ordering::Relaxed);
        if budget_x100 == 0 {
            return false;
        }
        let current_x100 = self.cpu_pct_x100.load(Ordering::Relaxed);
        current_x100 > budget_x100
    }

    /// Replace the configured CPU budget while preserving the latest reading.
    ///
    /// This mirrors [`MemoryGuard::update_budget_bytes`] for API symmetry and
    /// is used by config hot-reload paths: changing `cpu_budget_pct` takes
    /// effect on the next [`is_throttled`] call without requiring a process
    /// restart.  A zero budget disables throttling.
    ///
    /// [`MemoryGuard::update_budget_bytes`]: crate::metrics::memory_guard::MemoryGuard::update_budget_bytes
    /// [`is_throttled`]: CpuGate::is_throttled
    pub fn update_budget_pct(&self, pct: f32) {
        self.cpu_budget_pct_x100
            .store((pct * 100.0) as u32, Ordering::Relaxed);
    }

    /// Increment the skip counter after a chunk is dropped due to CPU pressure.
    ///
    /// The caller is responsible for calling this **after** a positive
    /// [`is_throttled`](CpuGate::is_throttled) check so the counter accurately
    /// reflects the number of intentionally skipped chunks.
    pub fn record_skip(&self) {
        self.skipped.fetch_add(1, Ordering::Relaxed);
    }

    /// Return the total number of chunks skipped due to CPU pressure.
    pub fn skipped_count(&self) -> u64 {
        self.skipped.load(Ordering::Relaxed)
    }

    /// Update the shared CPU reading from the metrics-publisher task.
    ///
    /// The metrics publisher calls this once per second with the `cpu_pct`
    /// value from
    /// [`ProcessSnapshot`](crate::metrics::process::ProcessSnapshot).
    pub fn update_cpu_pct(&self, pct: f32) {
        self.cpu_pct_x100
            .store((pct * 100.0) as u32, Ordering::Relaxed);
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a gate with a fixed initial CPU reading for deterministic tests.
    fn make_gate(budget: f32, initial_cpu: f32) -> CpuGate {
        let gate = CpuGate::new(budget);
        gate.update_cpu_pct(initial_cpu);
        gate
    }

    /// T1: CPU 80 %, budget 70 %, local mode → chunk throttled, counter increments.
    #[test]
    fn t1_local_mode_cpu_over_budget_throttled_and_counter_increments() {
        let gate = make_gate(70.0, 80.0);
        let provider_is_local = true;

        assert!(
            gate.is_throttled(),
            "80% CPU with 70% budget must be throttled"
        );

        // Simulate what the orchestrator does.
        if provider_is_local && gate.is_throttled() {
            gate.record_skip();
        }

        assert_eq!(
            gate.skipped_count(),
            1,
            "skip counter must be 1 after one throttled chunk"
        );
    }

    /// T2: CPU 99 %, Google mode → `provider_is_local` is `false`, so the
    /// gate is never consulted and the skip counter stays at zero.
    #[test]
    fn t2_google_mode_cpu_99_provider_still_called() {
        let gate = make_gate(80.0, 99.0); // would throttle local
        let provider_is_local = false; // Google/cloud path

        // The orchestrator short-circuits on provider_is_local == false.
        if provider_is_local && gate.is_throttled() {
            gate.record_skip();
        }

        assert_eq!(
            gate.skipped_count(),
            0,
            "Google path must never increment the skip counter"
        );
    }

    #[test]
    fn not_throttled_when_budget_is_zero() {
        let gate = make_gate(0.0, 99.0);
        assert!(
            !gate.is_throttled(),
            "zero budget (disabled) must never throttle regardless of CPU"
        );
    }

    #[test]
    fn not_throttled_when_cpu_below_budget() {
        let gate = make_gate(80.0, 50.0);
        assert!(
            !gate.is_throttled(),
            "50% CPU with 80% budget is below budget"
        );
    }

    #[test]
    fn not_throttled_when_cpu_equal_to_budget() {
        // The guard uses strict `>`, not `>=`, so equal is not throttled.
        let gate = make_gate(80.0, 80.0);
        assert!(
            !gate.is_throttled(),
            "CPU exactly equal to budget must not be throttled"
        );
    }

    #[test]
    fn update_cpu_pct_changes_throttle_state() {
        let gate = CpuGate::new(70.0);
        assert!(!gate.is_throttled(), "initial 0% must not be throttled");
        gate.update_cpu_pct(85.0);
        assert!(gate.is_throttled(), "85% with 70% budget must be throttled");
        gate.update_cpu_pct(60.0);
        assert!(
            !gate.is_throttled(),
            "60% with 70% budget must not be throttled"
        );
    }

    /// HC-04: replacing the budget with `update_budget_pct` takes effect
    /// immediately on the next `is_throttled` call -- no sleep or restart needed.
    #[test]
    fn update_budget_pct_changes_throttle_state() {
        // Start with a high budget (80 %) and CPU at 75 % -- not throttled.
        let gate = CpuGate::new(80.0);
        gate.update_cpu_pct(75.0);
        assert!(
            !gate.is_throttled(),
            "75% CPU with 80% budget must not be throttled"
        );

        // Lower the budget to 70 % -- immediately throttled.
        gate.update_budget_pct(70.0);
        assert!(
            gate.is_throttled(),
            "75% CPU with reduced 70% budget must be throttled"
        );

        // Raise the budget to 90 % -- immediately safe.
        gate.update_budget_pct(90.0);
        assert!(
            !gate.is_throttled(),
            "75% CPU with raised 90% budget must not be throttled"
        );

        // Set budget to 0.0 (disabled) -- never throttled regardless of CPU.
        gate.update_cpu_pct(999.0);
        gate.update_budget_pct(0.0);
        assert!(
            !gate.is_throttled(),
            "disabled budget (0.0) must never throttle"
        );
    }

    #[test]
    fn skip_counter_accumulates() {
        let gate = make_gate(70.0, 80.0);
        for _ in 0..5 {
            gate.record_skip();
        }
        assert_eq!(gate.skipped_count(), 5);
    }
}
