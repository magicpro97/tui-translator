//! Memory guard threshold-transition integration tests (issue #231).
//!
//! These tests verify the full threshold-entry / hysteresis-clear / safe-again
//! cycle through the `MetricsSnapshot::apply_memory_guard` boundary — the same
//! path the metrics-publisher task uses at runtime.
//!
//! # What is tested
//!
//! | Scenario | Assertion |
//! |----------|-----------|
//! | RAM at budget | `MetricsSnapshot.ram_warning` stays `false` |
//! | RAM one byte above budget | `ram_warning` becomes `true` |
//! | RAM drops to hysteresis boundary | `ram_warning` stays `true` |
//! | RAM drops below hysteresis threshold | `ram_warning` becomes `false` |
//! | RAM = 0 before warning | `ram_warning` is `false` (no false positive) |
//! | RAM = 0 after warning | `ram_warning` stays `true` and last reading is preserved |
//! | Budget = 0 (disabled) | `ram_warning` is always `false` |

#[allow(unused_imports)]
use crate::metrics::{memory_guard::MemoryGuard, snapshot::MetricsSnapshot};

fn make_snapshot_with_guard(guard: &MemoryGuard, ram_bytes: u64) -> MetricsSnapshot {
    guard.update_ram_bytes(ram_bytes);
    let mut snap = MetricsSnapshot {
        ram_bytes,
        ..Default::default()
    };
    snap.apply_memory_guard(guard);
    snap
}

/// IT-1: RAM exactly at budget → `ram_warning = false`.
#[test]
fn it1_ram_at_budget_no_warning() {
    let budget_bytes = 512 * 1024 * 1024u64;
    let guard = MemoryGuard::new(budget_bytes);
    let snap = make_snapshot_with_guard(&guard, budget_bytes);
    assert!(
        !snap.ram_warning,
        "IT-1: RAM exactly at budget must not set ram_warning"
    );
}

/// IT-2: RAM one byte above budget → `ram_warning = true`.
#[test]
fn it2_ram_above_budget_sets_warning() {
    let budget_bytes = 512 * 1024 * 1024u64;
    let guard = MemoryGuard::new(budget_bytes);
    let snap = make_snapshot_with_guard(&guard, budget_bytes + 1);
    assert!(
        snap.ram_warning,
        "IT-2: RAM one byte above budget must set ram_warning"
    );
}

/// IT-3: Hysteresis — warning persists when RAM drops to budget after entry.
#[test]
fn it3_warning_persists_at_budget_after_entry() {
    let budget_bytes = 1_000_000u64;
    let guard = MemoryGuard::new(budget_bytes); // 5 % hysteresis → clear < 950_000

    // Enter warning
    guard.update_ram_bytes(budget_bytes + 1);
    let mut snap = MetricsSnapshot::default();
    snap.apply_memory_guard(&guard);
    assert!(snap.ram_warning, "IT-3: must enter warning above budget");

    // Drop to budget — still above hysteresis clear threshold
    let snap2 = make_snapshot_with_guard(&guard, budget_bytes);
    assert!(
        snap2.ram_warning,
        "IT-3: warning must persist when RAM drops to budget (still above clear threshold)"
    );
}

/// IT-4: Hysteresis — warning clears when RAM drops below the threshold.
#[test]
fn it4_warning_clears_below_hysteresis_threshold() {
    let budget_bytes = 1_000_000u64;
    // 5 % hysteresis → clear threshold = floor(1_000_000 × 0.95) = 950_000
    let guard = MemoryGuard::new(budget_bytes);

    guard.update_ram_bytes(budget_bytes + 1); // Enter warning
    let snap1 = make_snapshot_with_guard(&guard, 950_000); // Still at threshold
    assert!(
        snap1.ram_warning,
        "IT-4: warning must persist at the exact clear threshold"
    );

    let snap2 = make_snapshot_with_guard(&guard, 949_999); // One below threshold
    assert!(
        !snap2.ram_warning,
        "IT-4: warning must clear one byte below the hysteresis clear threshold"
    );
}

/// IT-5: RAM = 0 (metrics unavailable) — no false-positive warning.
#[test]
fn it5_ram_zero_no_false_positive() {
    let budget_bytes = 512 * 1024 * 1024u64;
    let guard = MemoryGuard::new(budget_bytes);
    let snap = make_snapshot_with_guard(&guard, 0);
    assert!(
        !snap.ram_warning,
        "IT-5: RAM=0 (metrics unavailable) must not set ram_warning"
    );
}

/// IT-5b: RAM = 0 after warning is latched does not flicker the warning off.
#[test]
fn it5b_ram_zero_preserves_latched_warning() {
    let budget_bytes = 512 * 1024 * 1024u64;
    let guard = MemoryGuard::new(budget_bytes);
    let _ = make_snapshot_with_guard(&guard, budget_bytes + 1);

    let snap = make_snapshot_with_guard(&guard, 0);
    assert!(
        snap.ram_warning,
        "IT-5b: transient RAM=0 after warning must keep ram_warning latched"
    );
    assert_eq!(
        snap.ram_bytes,
        budget_bytes + 1,
        "IT-5b: snapshot should display the last non-zero RAM reading"
    );
    assert_eq!(
        guard.ram_bytes(),
        budget_bytes + 1,
        "IT-5b: transient RAM=0 must preserve the last non-zero RAM reading"
    );
}

/// IT-6: Budget = 0 (disabled) — `ram_warning` is always `false`.
#[test]
fn it6_disabled_guard_never_warns() {
    let guard = MemoryGuard::new(0);
    let snap = make_snapshot_with_guard(&guard, u64::MAX);
    assert!(
        !snap.ram_warning,
        "IT-6: disabled guard (budget=0) must never set ram_warning"
    );
}

/// IT-7: Full cycle — safe → warn → safe (via apply_memory_guard).
#[test]
fn it7_full_cycle_safe_warn_safe() {
    let budget_bytes = 1_000_000u64;
    let guard = MemoryGuard::new(budget_bytes); // clear threshold = 950_000

    // Phase 1: safe
    let s1 = make_snapshot_with_guard(&guard, 800_000);
    assert!(!s1.ram_warning, "IT-7 phase 1: must be safe below budget");

    // Phase 2: warning
    let s2 = make_snapshot_with_guard(&guard, 1_100_000);
    assert!(s2.ram_warning, "IT-7 phase 2: must warn above budget");

    // Phase 3: still warning (at budget)
    let s3 = make_snapshot_with_guard(&guard, 1_000_000);
    assert!(
        s3.ram_warning,
        "IT-7 phase 3: must stay in warning at budget"
    );

    // Phase 4: cleared (below hysteresis threshold)
    let s4 = make_snapshot_with_guard(&guard, 900_000);
    assert!(
        !s4.ram_warning,
        "IT-7 phase 4: must clear below clear threshold"
    );

    // Phase 5: no immediate re-entry at budget
    let s5 = make_snapshot_with_guard(&guard, 1_000_000);
    assert!(
        !s5.ram_warning,
        "IT-7 phase 5: must not re-enter at exactly budget"
    );

    // Phase 6: re-entry above budget
    let s6 = make_snapshot_with_guard(&guard, 1_000_001);
    assert!(
        s6.ram_warning,
        "IT-7 phase 6: must re-enter warning above budget"
    );
}
