//! Memory high-water guard (issue #231).
//!
//! [`MemoryGuard`] watches process RAM usage against a configured budget and
//! emits a warning state when the budget is exceeded.  The guard clears the
//! warning only after RAM drops below a hysteresis threshold
//! (`budget × (1 − hysteresis_frac)`, default 5 %), preventing flapping at
//! the boundary.
//!
//! # Design
//!
//! * The current RAM reading and warning state are stored as [`AtomicU64`] /
//!   [`AtomicBool`] so reading the guard status is always lock-free.
//! * The metrics-publisher task updates the reading once per second via
//!   [`MemoryGuard::update_ram_bytes`]; the TUI snapshot builder reads the
//!   state with [`MemoryGuard::is_warning`].
//! * A budget of `0` disables the guard entirely — [`is_warning`] always
//!   returns `false`.  A `0` RAM reading (metrics unavailable) never triggers a
//!   false-positive and does not clear an already-latched warning, avoiding
//!   one-tick flicker while process metrics recover.
//!
//! # Hysteresis
//!
//! | RAM reading             | State change                          |
//! |-------------------------|---------------------------------------|
//! | `> ram_budget_bytes`    | Safe → Warning                        |
//! | `< clear_threshold`     | Warning → Safe  (`clear_threshold = budget × (1 − hysteresis_frac)`) |
//! | anything else           | state unchanged                       |
//!
//! [`is_warning`]: MemoryGuard::is_warning
//! [`AtomicU64`]: std::sync::atomic::AtomicU64
//! [`AtomicBool`]: std::sync::atomic::AtomicBool

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

// ── MemoryGuard ───────────────────────────────────────────────────────────────

/// Memory budget guard for process RAM usage.
///
/// Create one instance at startup and share it via [`Arc`].  Pass the same
/// `Arc<MemoryGuard>` to both the metrics-publisher task (to call
/// [`update_ram_bytes`]) and the TUI snapshot builder (to call
/// [`is_warning`]).
///
/// # Hysteresis
///
/// The guard enters the warning state when `ram_bytes > ram_budget_bytes`
/// and clears only when `ram_bytes < ram_budget_bytes * (1 − hysteresis_frac)`.
/// With the default hysteresis of 5 %, on a 512 MiB budget the guard enters
/// warning above 512 MiB and clears below ≈ 486 MiB.
///
/// [`update_ram_bytes`]: MemoryGuard::update_ram_bytes
/// [`is_warning`]: MemoryGuard::is_warning
/// [`Arc`]: std::sync::Arc
#[derive(Debug)]
pub struct MemoryGuard {
    /// Upper RAM bound in bytes.  `0` disables the guard.
    ram_budget_bytes: AtomicU64,

    /// Pre-computed clear threshold: `ram_budget_bytes × (1 − hysteresis_frac)`.
    ///
    /// The warning clears when the latest RAM reading drops *strictly below*
    /// this value.
    clear_threshold_bytes: AtomicU64,
    hysteresis_frac: f64,

    /// Latest RAM reading in bytes, updated by the metrics publisher.
    ram_bytes: AtomicU64,

    /// `true` while the current reading exceeds the budget (accounting for
    /// hysteresis).
    in_warning: AtomicBool,
}

impl MemoryGuard {
    /// Create a guard with a 5 % hysteresis.
    ///
    /// * `ram_budget_bytes` — upper RAM bound; `0` disables the guard.
    pub fn new(ram_budget_bytes: u64) -> Self {
        Self::new_with_hysteresis(ram_budget_bytes, 0.05)
    }

    /// Create a guard with a custom hysteresis fraction.
    ///
    /// * `ram_budget_bytes` — upper RAM bound; `0` disables the guard.
    /// * `hysteresis_frac` — clamped to `[0.0, 1.0]`.  A value of `0.05`
    ///   means the warning clears when RAM drops 5 % below the budget.
    ///   A value of `0.0` gives a simple threshold with no hysteresis.
    pub fn new_with_hysteresis(ram_budget_bytes: u64, hysteresis_frac: f64) -> Self {
        let frac = hysteresis_frac.clamp(0.0, 1.0);
        Self {
            ram_budget_bytes: AtomicU64::new(ram_budget_bytes),
            clear_threshold_bytes: AtomicU64::new(Self::clear_threshold(ram_budget_bytes, frac)),
            hysteresis_frac: frac,
            ram_bytes: AtomicU64::new(0),
            in_warning: AtomicBool::new(false),
        }
    }

    fn clear_threshold(ram_budget_bytes: u64, hysteresis_frac: f64) -> u64 {
        if ram_budget_bytes == 0 {
            0
        } else {
            (ram_budget_bytes as f64 * (1.0 - hysteresis_frac)) as u64
        }
    }

    /// Replace the configured RAM budget while preserving the latest reading.
    ///
    /// This is used by config hot-reload paths: changing `ram_budget_mb` should
    /// take effect on the next metrics tick without requiring a process restart.
    /// A zero budget disables the guard and clears any active warning.
    ///
    /// # API symmetry
    ///
    /// `update_budget_bytes` is the RAM-side counterpart to
    /// [`CpuGate::update_budget_pct`] (HC-04, issue #389).  Both setters are
    /// called from the same 1 Hz metrics-publisher loop on config hot-reload,
    /// giving operators sub-2-second latency for budget adjustments on either
    /// resource dimension without a process restart.
    ///
    /// [`CpuGate::update_budget_pct`]: crate::pipeline::cpu_gate::CpuGate::update_budget_pct
    pub fn update_budget_bytes(&self, ram_budget_bytes: u64) {
        self.ram_budget_bytes
            .store(ram_budget_bytes, Ordering::Relaxed);
        self.clear_threshold_bytes.store(
            Self::clear_threshold(ram_budget_bytes, self.hysteresis_frac),
            Ordering::Relaxed,
        );

        if ram_budget_bytes == 0 {
            self.in_warning.store(false, Ordering::Relaxed);
        } else {
            self.update_ram_bytes(self.ram_bytes());
        }
    }

    /// Update the RAM reading and recompute the warning state with hysteresis.
    ///
    /// Called once per second by the metrics publisher.  Never panics even
    /// when `ram_bytes` is `0` (metrics unavailable or process data absent).
    pub fn update_ram_bytes(&self, ram_bytes: u64) {
        let ram_budget_bytes = self.ram_budget_bytes.load(Ordering::Relaxed);
        if ram_budget_bytes == 0 {
            // Guard is disabled — ensure warning is always off.
            self.ram_bytes.store(ram_bytes, Ordering::Relaxed);
            self.in_warning.store(false, Ordering::Relaxed);
            return;
        }

        let clear_threshold_bytes = self.clear_threshold_bytes.load(Ordering::Relaxed);
        let currently_warning = self.in_warning.load(Ordering::Relaxed);
        if ram_bytes == 0 {
            if !currently_warning {
                self.ram_bytes.store(0, Ordering::Relaxed);
            }
            return;
        }

        self.ram_bytes.store(ram_bytes, Ordering::Relaxed);
        let new_warning = if currently_warning {
            // In warning: stay in warning while RAM >= clear threshold.
            // Clear only when RAM drops *strictly below* the clear threshold.
            ram_bytes >= clear_threshold_bytes
        } else {
            // Not in warning: enter warning when RAM *strictly exceeds* the budget.
            ram_bytes > ram_budget_bytes
        };
        self.in_warning.store(new_warning, Ordering::Relaxed);
    }

    /// Return `true` when the process RAM exceeds the configured budget
    /// (accounting for hysteresis).
    ///
    /// Always returns `false` when the budget is `0` (disabled) or before
    /// the first call to [`update_ram_bytes`].
    ///
    /// [`update_ram_bytes`]: MemoryGuard::update_ram_bytes
    pub fn is_warning(&self) -> bool {
        self.in_warning.load(Ordering::Relaxed)
    }

    /// Return the latest RAM reading in bytes, as last set by
    /// [`update_ram_bytes`].
    ///
    /// Returns `0` before the first call to [`update_ram_bytes`].  When a
    /// warning is already latched, a temporary `0` reading preserves the last
    /// non-zero value for stable operator display.
    ///
    /// [`update_ram_bytes`]: MemoryGuard::update_ram_bytes
    pub fn ram_bytes(&self) -> u64 {
        self.ram_bytes.load(Ordering::Relaxed)
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Disabled guard ────────────────────────────────────────────────────────

    /// T1: Budget = 0 (disabled) — never warns regardless of RAM reading.
    #[test]
    fn t1_disabled_when_budget_is_zero() {
        let guard = MemoryGuard::new(0);
        guard.update_ram_bytes(u64::MAX);
        assert!(
            !guard.is_warning(),
            "budget=0 must never warn regardless of RAM"
        );
    }

    /// T2: Initial state is safe before any update.
    #[test]
    fn t2_initial_state_is_safe() {
        let guard = MemoryGuard::new(512 * 1024 * 1024);
        assert!(
            !guard.is_warning(),
            "guard must start in safe state before the first update"
        );
    }

    // ── Threshold entry ───────────────────────────────────────────────────────

    /// T3: RAM one byte above budget → enter warning.
    #[test]
    fn t3_warns_when_ram_strictly_exceeds_budget() {
        let budget = 512 * 1024 * 1024u64; // 512 MiB
        let guard = MemoryGuard::new(budget);
        guard.update_ram_bytes(budget + 1);
        assert!(
            guard.is_warning(),
            "RAM one byte above budget must trigger warning"
        );
    }

    /// T4: RAM exactly at budget → no warning (strict > required).
    #[test]
    fn t4_no_warning_when_ram_exactly_at_budget() {
        let budget = 512 * 1024 * 1024u64;
        let guard = MemoryGuard::new(budget);
        guard.update_ram_bytes(budget);
        assert!(
            !guard.is_warning(),
            "RAM exactly at budget must not trigger warning (strict > required)"
        );
    }

    /// T5: RAM below budget → no warning.
    #[test]
    fn t5_no_warning_when_ram_below_budget() {
        let budget = 512 * 1024 * 1024u64;
        let guard = MemoryGuard::new(budget);
        guard.update_ram_bytes(budget - 1);
        assert!(!guard.is_warning(), "RAM below budget must not warn");
    }

    // ── Hysteresis — clear behaviour ──────────────────────────────────────────

    /// T6: Enter warning, drop to budget (not yet below 5 % threshold) → stays in warning.
    ///
    /// Budget = 1 000 000 bytes, clear threshold = 950 000 bytes.
    /// Dropping to budget (1 000 000) is above the clear threshold; warning persists.
    #[test]
    fn t6_hysteresis_prevents_premature_clear_at_budget() {
        let budget = 1_000_000u64;
        let guard = MemoryGuard::new(budget); // 5% hysteresis; clear at < 950_000

        guard.update_ram_bytes(budget + 1);
        assert!(guard.is_warning(), "must enter warning above budget");

        guard.update_ram_bytes(budget);
        assert!(
            guard.is_warning(),
            "warning must persist when RAM drops to budget (above 5 % clear threshold)"
        );
    }

    /// T7: Enter warning, drop to clear threshold exactly → stays in warning
    /// (strict < required to clear).
    #[test]
    fn t7_hysteresis_boundary_stays_in_warning() {
        let budget = 1_000_000u64;
        let clear_threshold = 950_000u64; // floor(budget × 0.95)
        let guard = MemoryGuard::new_with_hysteresis(budget, 0.05);

        guard.update_ram_bytes(budget + 1);
        assert!(guard.is_warning());

        guard.update_ram_bytes(clear_threshold);
        assert!(
            guard.is_warning(),
            "warning must persist at the exact clear threshold (strict < required to clear); \
             clear_threshold={clear_threshold}"
        );
    }

    /// T8: Enter warning, drop one byte below clear threshold → clears.
    #[test]
    fn t8_hysteresis_clears_below_threshold() {
        let budget = 1_000_000u64;
        let clear_threshold = 950_000u64; // floor(budget × 0.95)
        let guard = MemoryGuard::new_with_hysteresis(budget, 0.05);

        guard.update_ram_bytes(budget + 1);
        assert!(guard.is_warning());

        guard.update_ram_bytes(clear_threshold - 1);
        assert!(
            !guard.is_warning(),
            "warning must clear when RAM drops one byte below the clear threshold ({clear_threshold})"
        );
    }

    // ── Zero RAM (metrics unavailable) ────────────────────────────────────────

    /// T9: RAM = 0 (metrics unavailable) must not trigger a false-positive warning.
    #[test]
    fn t9_no_false_positive_when_ram_is_zero() {
        let guard = MemoryGuard::new(512 * 1024 * 1024);
        guard.update_ram_bytes(0);
        assert!(
            !guard.is_warning(),
            "RAM=0 (metrics unavailable) must not trigger a warning"
        );
    }

    /// T10: After entering warning, RAM drops to 0 (process data gone) → warning stays latched.
    ///
    /// If `sysinfo` returns `None` for the process (e.g., short-lived race),
    /// keep the previous warning/reading instead of flickering the warning off.
    #[test]
    fn t10_warning_stays_latched_when_ram_drops_to_zero() {
        let budget = 512 * 1024 * 1024u64;
        let guard = MemoryGuard::new(budget);

        guard.update_ram_bytes(budget + 1);
        assert!(guard.is_warning(), "must enter warning");
        assert_eq!(guard.ram_bytes(), budget + 1);

        guard.update_ram_bytes(0);
        assert!(
            guard.is_warning(),
            "warning must stay latched when RAM is temporarily unavailable"
        );
        assert_eq!(
            guard.ram_bytes(),
            budget + 1,
            "zero metrics sample must preserve the last non-zero reading"
        );
    }

    // ── Multi-step state transitions ──────────────────────────────────────────

    /// T11: Safe → Warning → Safe → Warning cycle without flapping.
    #[test]
    fn t11_state_transitions_safe_warn_safe_warn() {
        let budget = 1_000_000u64;
        let guard = MemoryGuard::new_with_hysteresis(budget, 0.05);
        // clear threshold = 950_000

        // Step 1: Safe → Warning
        guard.update_ram_bytes(1_100_000);
        assert!(guard.is_warning(), "step 1: must enter warning");

        // Step 2: Still in warning at budget level
        guard.update_ram_bytes(1_000_000);
        assert!(guard.is_warning(), "step 2: must stay in warning at budget");

        // Step 3: Warning → Safe
        guard.update_ram_bytes(900_000);
        assert!(
            !guard.is_warning(),
            "step 3: must clear below hysteresis threshold"
        );

        // Step 4: No immediate re-entry at exactly budget
        guard.update_ram_bytes(1_000_000);
        assert!(
            !guard.is_warning(),
            "step 4: must not warn at exactly budget after clearing"
        );

        // Step 5: Re-entry strictly above budget
        guard.update_ram_bytes(1_000_001);
        assert!(
            guard.is_warning(),
            "step 5: must re-enter warning above budget"
        );
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    /// T12: `ram_bytes()` accessor reflects the latest update value.
    #[test]
    fn t12_ram_bytes_accessor_reflects_latest_update() {
        let guard = MemoryGuard::new(512 * 1024 * 1024);
        assert_eq!(guard.ram_bytes(), 0, "initial reading must be 0");
        guard.update_ram_bytes(200_000_000);
        assert_eq!(guard.ram_bytes(), 200_000_000);
    }

    // ── Edge cases ────────────────────────────────────────────────────────────

    /// T13: Zero-hysteresis guard behaves as a simple threshold.
    #[test]
    fn t13_zero_hysteresis_is_simple_threshold() {
        let budget = 1_000_000u64;
        // clear_threshold = budget × 1.0 = budget; warning clears at < budget
        let guard = MemoryGuard::new_with_hysteresis(budget, 0.0);

        guard.update_ram_bytes(budget + 1);
        assert!(guard.is_warning());

        // Drop to exactly budget → clear immediately (clear_threshold == budget,
        // strict < means budget < budget is false, so stays... wait)
        // Actually: clear condition is ram_bytes >= clear_threshold stays in warning
        // So: ram = budget, clear_threshold = budget → budget >= budget → stays in warning?!
        // No wait: clear_threshold = floor(budget * (1 - 0.0)) = budget
        // The condition: currently_warning && ram_bytes >= clear_threshold → stays in warning
        // So at ram = budget and clear_threshold = budget: budget >= budget = true → stays in warning
        // Clear only when ram < budget (= clear_threshold)
        guard.update_ram_bytes(budget - 1);
        assert!(
            !guard.is_warning(),
            "zero hysteresis: warning must clear when ram < budget"
        );
    }

    /// T14: Disabled guard call with zero RAM stays silent.
    #[test]
    fn t14_disabled_zero_ram_stays_silent() {
        let guard = MemoryGuard::new(0);
        guard.update_ram_bytes(0);
        assert!(!guard.is_warning());
        assert_eq!(guard.ram_bytes(), 0);
    }

    /// T15: Hot-reducing the budget re-evaluates the last RAM reading.
    #[test]
    fn t15_budget_update_enters_warning_from_latest_reading() {
        let guard = MemoryGuard::new(1_000_000);
        guard.update_ram_bytes(900_000);
        assert!(!guard.is_warning());

        guard.update_budget_bytes(800_000);
        assert!(
            guard.is_warning(),
            "lowering budget below latest RAM reading must warn immediately"
        );
    }

    /// T16: Disabling the budget clears a previously active warning.
    #[test]
    fn t16_budget_update_to_zero_clears_warning() {
        let guard = MemoryGuard::new(1_000_000);
        guard.update_ram_bytes(1_100_000);
        assert!(guard.is_warning());

        guard.update_budget_bytes(0);
        assert!(
            !guard.is_warning(),
            "setting budget to 0 must disable and clear the warning"
        );
    }
}
