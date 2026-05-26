//! Unit tests for `snapshot` (extracted from `snapshot.rs` for the
//! STD-02 module-budget refactor; behavior unchanged).

use super::*;

#[test]
fn default_snapshot_is_all_zero() {
    let s = MetricsSnapshot::default();
    assert_eq!(s.audio_seconds_sent, 0.0);
    assert_eq!(s.chars_translated, 0);
    assert_eq!(s.estimated_cost_usd, 0.0);
    assert_eq!(s.line_pairs_shown, 0);
    assert_eq!(s.cpu_pct, 0.0);
    assert_eq!(s.ram_bytes, 0);
    assert_eq!(s.net_kbps_tx, 0.0);
    assert_eq!(s.net_kbps_rx, 0.0);
    assert_eq!(s.e2e_latency_ms, None);
    assert_eq!(s.e2e_latency_mean_ms, 0.0);
    assert_eq!(s.e2e_latency_p95_ms, 0);
    assert_eq!(s.loss_pct, 0.0);
    assert_eq!(s.total_chunks, 0);
    assert_eq!(s.dropped_chunks, 0);
    assert_eq!(s.local_inferences_skipped, 0);
    assert!(!s.ram_warning);
    assert!(!s.recording_disabled_under_pressure);
    // Issue #269: quality counters default to zero / 0.0.
    assert_eq!(s.truncation_rate, 0.0);
    assert_eq!(s.flicker_count, 0);
    assert_eq!(s.mt_call_count, 0);
    // Issue #393: storage metrics default to zero / None / false.
    assert_eq!(s.recorder_bytes, 0);
    assert!(s.recorder_path.is_none());
    assert_eq!(s.archive_bytes, 0);
    assert!(s.archive_path.is_none());
    assert!(!s.archive_sealed);
    // Issue #378 (DM-02): fanout drop counters default to zero.
    assert_eq!(s.fanout_slot_a_drops, 0);
    assert_eq!(s.fanout_slot_b_drops, 0);
    // HC-03B (issue #436): capture router counters default to zero.
    assert_eq!(s.capture_swap_count, 0);
    assert_eq!(s.capture_swap_drops, 0);
    // LF-02 (issue #370): local runtime caps default to zero.
    assert_eq!(s.local_cpu_pct, 0.0);
    assert_eq!(s.local_active_threads, 0);
}

/// LF-02 (issue #370): `apply_local_runtime` mirrors process CPU only
/// when local-inference activity is observed.
#[test]
fn apply_local_runtime_mirrors_cpu_only_with_active_threads() {
    let mut s = MetricsSnapshot {
        cpu_pct: 42.0,
        ..MetricsSnapshot::default()
    };
    s.apply_local_runtime(0, false);
    assert_eq!(
        s.local_cpu_pct, 0.0,
        "cloud-only / idle local engine must read silent local CPU"
    );
    assert_eq!(s.local_active_threads, 0);

    s.apply_local_runtime(2, false);
    assert_eq!(
        s.local_cpu_pct, 42.0,
        "with active local inference, local_cpu_pct mirrors process cpu_pct"
    );
    assert_eq!(s.local_active_threads, 2);
}

/// LF-02 (issue #370): only a skip-counter advance counts as "local
/// activity observed" so stale cumulative skips do not keep the CPU gauge
/// live after local providers go idle.
#[test]
fn apply_local_runtime_reports_cpu_only_when_skip_counter_advanced() {
    let mut s = MetricsSnapshot {
        cpu_pct: 91.0,
        local_inferences_skipped: 3,
        ..MetricsSnapshot::default()
    };
    s.apply_local_runtime(0, false);
    assert_eq!(s.local_cpu_pct, 0.0);
    assert_eq!(s.local_active_threads, 0);

    s.apply_local_runtime(0, true);
    assert_eq!(s.local_cpu_pct, 91.0);
    assert_eq!(s.local_active_threads, 0);
}

#[test]
fn format_elapsed_has_colon_separator() {
    let s = MetricsSnapshot::default();
    let formatted = s.format_elapsed();
    assert!(
        formatted.contains(':'),
        "format_elapsed must contain ':'; got {formatted:?}"
    );
}

#[test]
fn format_elapsed_produces_hours_when_over_3600s() {
    // Call the pure helper directly with a known value (2 h 1 m 5 s).
    // This avoids Instant::checked_sub returning None on Windows CI when
    // the requested duration exceeds system uptime (PR #141 regression).
    let formatted = format_duration_secs(7265);
    // h:mm:ss must contain exactly two colons.
    let colon_count = formatted.chars().filter(|&c| c == ':').count();
    assert_eq!(
        colon_count, 2,
        "hour format should have 2 colons; got {formatted:?}"
    );
    // Sanity-check the actual rendered value while we're here.
    assert_eq!(formatted, "2:01:05");
}

#[test]
fn apply_process_sets_fields() {
    let mut s = MetricsSnapshot::default();
    let ps = crate::metrics::process::ProcessSnapshot {
        cpu_pct: 42.5,
        ram_bytes: 1_048_576,
    };
    s.apply_process(&ps);
    assert_eq!(s.cpu_pct, 42.5);
    assert_eq!(s.ram_bytes, 1_048_576);
}

#[test]
fn apply_network_sets_fields() {
    let mut s = MetricsSnapshot::default();
    let ns = crate::metrics::network::NetworkSnapshot {
        kbps_tx: 128.0,
        kbps_rx: 256.0,
        total_bytes_sent: 16_000,
        total_bytes_recv: 32_000,
    };
    s.apply_network(&ns);
    assert_eq!(s.net_kbps_tx, 128.0);
    assert_eq!(s.net_kbps_rx, 256.0);
    assert_eq!(s.net_total_bytes_sent, 16_000);
    assert_eq!(s.net_total_bytes_recv, 32_000);
}

#[test]
fn clone_produces_independent_copy() {
    let original = MetricsSnapshot {
        audio_seconds_sent: 120.0,
        chars_translated: 5000,
        ..MetricsSnapshot::default()
    };
    let cloned = original.clone();
    assert_eq!(cloned.audio_seconds_sent, 120.0);
    assert_eq!(cloned.chars_translated, 5000);
}

#[test]
fn apply_memory_guard_sets_ram_warning_when_over_budget() {
    let guard = crate::metrics::memory_guard::MemoryGuard::new(100_000);
    guard.update_ram_bytes(200_000); // 2× budget → warning
    let mut s = MetricsSnapshot::default();
    s.apply_memory_guard(&guard);
    assert!(
        s.ram_warning,
        "apply_memory_guard must set ram_warning when guard is in warning state"
    );
    assert!(
        s.recording_disabled_under_pressure,
        "RAM warning must disable optional session/audio recording"
    );
}

#[test]
fn apply_memory_guard_clears_ram_warning_when_below_budget() {
    let guard = crate::metrics::memory_guard::MemoryGuard::new(100_000);
    guard.update_ram_bytes(50_000); // below budget → safe
    let mut s = MetricsSnapshot {
        ram_warning: true,
        ..MetricsSnapshot::default()
    };
    s.apply_memory_guard(&guard);
    assert!(
        !s.ram_warning,
        "apply_memory_guard must clear ram_warning when guard is in safe state"
    );
    assert!(
        !s.recording_disabled_under_pressure,
        "safe RAM state must allow optional recording again"
    );
}

#[test]
fn apply_memory_guard_no_warning_when_disabled() {
    let guard = crate::metrics::memory_guard::MemoryGuard::new(0); // disabled
    guard.update_ram_bytes(u64::MAX);
    let mut s = MetricsSnapshot::default();
    s.apply_memory_guard(&guard);
    assert!(
        !s.ram_warning,
        "apply_memory_guard must not warn when budget is 0 (disabled)"
    );
    assert!(
        !s.recording_disabled_under_pressure,
        "disabled guard must not disable optional recording"
    );
}

#[test]
fn apply_memory_guard_preserves_last_ram_reading_when_metrics_are_unavailable() {
    let guard = crate::metrics::memory_guard::MemoryGuard::new(100_000);
    guard.update_ram_bytes(200_000);
    guard.update_ram_bytes(0);
    let mut s = MetricsSnapshot::default();
    s.apply_memory_guard(&guard);
    assert!(s.ram_warning);
    assert_eq!(
        s.ram_bytes, 200_000,
        "latched warning should display the last non-zero RAM reading"
    );
}

// ── Issue #269: quality / diagnostic counter snapshot tests ──────────────

#[test]
fn truncation_rate_is_zero_when_no_windows_recorded() {
    let s = MetricsSnapshot {
        truncation_rate: 0.0,
        ..MetricsSnapshot::default()
    };
    assert_eq!(s.truncation_rate, 0.0, "NaN-safe: zero when no windows");
}

#[test]
fn truncation_rate_is_one_when_all_windows_truncated() {
    let s = MetricsSnapshot {
        truncation_rate: 1.0,
        ..MetricsSnapshot::default()
    };
    assert_eq!(s.truncation_rate, 1.0);
}

#[test]
fn truncation_rate_partial_fraction() {
    // 1 truncated out of 4 → 0.25
    let rate = 1.0_f64 / 4.0_f64;
    let s = MetricsSnapshot {
        truncation_rate: rate,
        ..MetricsSnapshot::default()
    };
    assert!((s.truncation_rate - 0.25).abs() < f64::EPSILON);
}

#[test]
fn flicker_and_mt_call_counts_are_carried_in_snapshot() {
    let s = MetricsSnapshot {
        flicker_count: 7,
        mt_call_count: 42,
        ..MetricsSnapshot::default()
    };
    assert_eq!(s.flicker_count, 7);
    assert_eq!(s.mt_call_count, 42);
}

// ── Issue #393: storage metrics tests ────────────────────────────────────

#[test]
fn apply_storage_sets_all_fields() {
    let mut s = MetricsSnapshot::default();
    s.apply_storage(
        1024,
        Some(PathBuf::from("/sessions/session-1.jsonl")),
        4096,
        Some(PathBuf::from("/audio/session-1.wav")),
        false,
    );
    assert_eq!(s.recorder_bytes, 1024);
    assert_eq!(
        s.recorder_path,
        Some(PathBuf::from("/sessions/session-1.jsonl"))
    );
    assert_eq!(s.archive_bytes, 4096);
    assert_eq!(s.archive_path, Some(PathBuf::from("/audio/session-1.wav")));
    assert!(!s.archive_sealed);
}

#[test]
fn apply_storage_disabled_paths_are_none() {
    let mut s = MetricsSnapshot::default();
    s.apply_storage(0, None, 0, None, false);
    assert_eq!(s.recorder_bytes, 0);
    assert!(s.recorder_path.is_none());
    assert_eq!(s.archive_bytes, 0);
    assert!(s.archive_path.is_none());
    assert!(!s.archive_sealed);
}

#[test]
fn apply_storage_sealed_archive() {
    let mut s = MetricsSnapshot::default();
    s.apply_storage(0, None, 8192, Some(PathBuf::from("/audio/s.wav")), true);
    assert!(s.archive_sealed);
    assert_eq!(s.archive_bytes, 8192);
}

#[test]
fn recorder_bytes_is_non_decreasing_after_repeated_apply() {
    let mut s = MetricsSnapshot::default();
    s.apply_storage(100, None, 0, None, false);
    assert_eq!(s.recorder_bytes, 100);
    s.apply_storage(200, None, 0, None, false);
    assert!(
        s.recorder_bytes >= 100,
        "recorder_bytes must not decrease; got {}",
        s.recorder_bytes
    );
}

// ── Issue #378 (DM-02): fanout drop counter snapshot tests ───────────────

#[test]
fn apply_fanout_drops_sets_both_fields() {
    let mut s = MetricsSnapshot::default();
    s.apply_fanout_drops(10, 25);
    assert_eq!(s.fanout_slot_a_drops, 10, "slot A drops must be applied");
    assert_eq!(s.fanout_slot_b_drops, 25, "slot B drops must be applied");
}

#[test]
fn apply_fanout_drops_slot_a_only() {
    let mut s = MetricsSnapshot::default();
    s.apply_fanout_drops(7, 0);
    assert_eq!(s.fanout_slot_a_drops, 7);
    assert_eq!(s.fanout_slot_b_drops, 0, "slot B must remain zero");
}

#[test]
fn apply_fanout_drops_slot_b_only() {
    let mut s = MetricsSnapshot::default();
    s.apply_fanout_drops(0, 99);
    assert_eq!(s.fanout_slot_a_drops, 0, "slot A must remain zero");
    assert_eq!(s.fanout_slot_b_drops, 99);
}

#[test]
fn apply_fanout_drops_overwrites_previous_values() {
    let mut s = MetricsSnapshot {
        fanout_slot_a_drops: 50,
        fanout_slot_b_drops: 50,
        ..MetricsSnapshot::default()
    };
    s.apply_fanout_drops(1, 2);
    assert_eq!(s.fanout_slot_a_drops, 1);
    assert_eq!(s.fanout_slot_b_drops, 2);
}

#[test]
fn apply_fanout_drops_zero_does_not_affect_other_fields() {
    let mut s = MetricsSnapshot {
        loss_pct: 5.0,
        total_chunks: 100,
        dropped_chunks: 5,
        ..MetricsSnapshot::default()
    };
    s.apply_fanout_drops(0, 0);
    // Existing loss metrics must be unchanged.
    assert_eq!(s.loss_pct, 5.0);
    assert_eq!(s.total_chunks, 100);
    assert_eq!(s.dropped_chunks, 5);
}

#[test]
fn apply_capture_router_metrics_sets_swap_counters() {
    let mut s = MetricsSnapshot::default();
    s.apply_capture_router_metrics(6, 2);
    assert_eq!(s.capture_swap_count, 6);
    assert_eq!(s.capture_swap_drops, 2);
}
