//! Failing tests for the `SysCaps` / RAM-tier / CPU-tier surface
//! (v3 Quality Preset foundation).
//!
//! RED: these tests do not yet compile — `src/sys_caps.rs` is not
//! created. They will be turned GREEN by the SysCaps module that
//! this issue ships.

use crate::sys_caps::{CpuTier, GpuKind, RamTier, SysCaps};

fn caps(total_gib: u64, cores: usize) -> SysCaps {
    SysCaps {
        total_memory_bytes: total_gib * 1024 * 1024 * 1024,
        physical_cores: cores,
        gpu: GpuKind::None,
    }
}

#[test]
fn detect_returns_positive_total_memory_bytes() {
    let c = SysCaps::detect();
    assert!(
        c.total_memory_bytes > 1_073_741_824,
        "expected >= 1 GiB, got {} bytes",
        c.total_memory_bytes
    );
}

#[test]
fn detect_returns_at_least_one_physical_core() {
    let c = SysCaps::detect();
    assert!(
        c.physical_cores >= 1,
        "expected >= 1 core, got {}",
        c.physical_cores
    );
}

#[test]
fn detect_is_deterministic_within_one_process() {
    // sysinfo + OnceLock must produce a stable value across calls.
    let a = SysCaps::detect();
    let b = SysCaps::detect();
    assert_eq!(a.total_memory_bytes, b.total_memory_bytes);
    assert_eq!(a.physical_cores, b.physical_cores);
}

#[test]
fn ram_tier_low_for_7_gib() {
    assert_eq!(caps(7, 4).ram_tier(), RamTier::Low);
}

#[test]
fn ram_tier_low_for_8_gib_exact() {
    // 8 GiB = 8 * 1024^3 bytes; matches Low band's upper edge.
    assert_eq!(caps(8, 4).ram_tier(), RamTier::Low);
}

#[test]
fn ram_tier_medium_for_9_gib() {
    assert_eq!(caps(9, 4).ram_tier(), RamTier::Medium);
}

#[test]
fn ram_tier_medium_for_15_gib() {
    assert_eq!(caps(15, 4).ram_tier(), RamTier::Medium);
}

#[test]
fn ram_tier_high_for_16_gib() {
    assert_eq!(caps(16, 4).ram_tier(), RamTier::High);
}

#[test]
fn ram_tier_high_for_32_gib() {
    assert_eq!(caps(32, 8).ram_tier(), RamTier::High);
}

#[test]
fn cpu_tier_low_for_1_core() {
    assert_eq!(caps(16, 1).cpu_tier(), CpuTier::Low);
}

#[test]
fn cpu_tier_low_for_2_cores() {
    assert_eq!(caps(16, 2).cpu_tier(), CpuTier::Low);
}

#[test]
fn cpu_tier_medium_for_3_cores() {
    assert_eq!(caps(16, 3).cpu_tier(), CpuTier::Medium);
}

#[test]
fn cpu_tier_medium_for_5_cores() {
    assert_eq!(caps(16, 5).cpu_tier(), CpuTier::Medium);
}

#[test]
fn cpu_tier_high_for_6_cores() {
    assert_eq!(caps(16, 6).cpu_tier(), CpuTier::High);
}

#[test]
fn cpu_tier_high_for_12_cores() {
    assert_eq!(caps(32, 12).cpu_tier(), CpuTier::High);
}

#[test]
fn zero_total_memory_is_low_tier() {
    // Degenerate case: should not panic.
    let c = SysCaps {
        total_memory_bytes: 0,
        physical_cores: 4,
        gpu: GpuKind::None,
    };
    assert_eq!(c.ram_tier(), RamTier::Low);
}

#[test]
fn zero_cores_is_low_tier() {
    // Degenerate case: should not panic.
    let c = SysCaps {
        total_memory_bytes: 16 * 1024 * 1024 * 1024,
        physical_cores: 0,
        gpu: GpuKind::None,
    };
    assert_eq!(c.cpu_tier(), CpuTier::Low);
}

#[test]
fn detect_physical_cores_fallback_returns_at_least_one() {
    // Calls the extracted fallback chain directly so the
    // 100%-line-coverage gate for `src/sys_caps.rs` can
    // see all branches of the `std::thread::available_parallelism`
    // map/unwrap_or/max(1) chain.
    // This is the helper that runs when sysinfo::System::physical_core_count()
    // returns None (rare on macOS dev) or Some(0) (synthetic).
    let result = crate::sys_caps::fallback_physical_cores();
    assert!(
        result >= 1,
        "fallback should always return >= 1, got {result}"
    );
}

#[test]
fn pick_physical_cores_returns_some_positive() {
    // The happy path: sysinfo reported a positive count.
    let result = crate::sys_caps::pick_physical_cores(Some(8));
    assert_eq!(result, 8);
}

#[test]
fn pick_physical_cores_falls_back_on_none() {
    // sysinfo returned None — must not panic, must call fallback.
    let result = crate::sys_caps::pick_physical_cores(None);
    assert!(result >= 1, "fallback should be >= 1, got {result}");
}

#[test]
fn pick_physical_cores_falls_back_on_zero() {
    // sysinfo returned Some(0) — degenerate but possible on
    // misconfigured VMs. Must not return 0; must call fallback.
    let result = crate::sys_caps::pick_physical_cores(Some(0));
    assert!(result >= 1, "Some(0) must trigger fallback, got {result}");
}

#[test]
fn fallback_physical_cores_on_error_returns_one() {
    // The Err/Ok(0) arm of `fallback_physical_cores` is unreachable
    // from real sysinfo (it always reports >= 1 on supported
    // platforms), so we exercise the dedicated pure helper
    // directly. Pulled out for the per-file 100%-coverage gate.
    let result = crate::sys_caps::fallback_physical_cores_on_error();
    assert_eq!(result, 1, "fallback on error must be exactly 1");
}

#[test]
fn fallback_physical_cores_from_ok_returns_count() {
    use std::num::NonZeroUsize;
    let n = NonZeroUsize::new(8).expect("8 is non-zero");
    assert_eq!(
        crate::sys_caps::fallback_physical_cores_from(Ok(n)),
        8,
        "Ok(8) must return 8"
    );
}

#[test]
fn fallback_physical_cores_from_err_returns_one() {
    // Construct a synthetic io::Error to exercise the Err arm of
    // `fallback_physical_cores_from`. On real platforms
    // `available_parallelism` never returns Err, so this is the
    // only way to cover that branch in unit tests.
    let err = std::io::Error::other("synthetic");
    assert_eq!(
        crate::sys_caps::fallback_physical_cores_from(Err(err)),
        1,
        "Err(_) must return 1"
    );
}

#[test]
fn ram_tier_boundaries_around_8_gib() {
    // 7.99 GiB → Low; 8.00 GiB → still Low (≤ 8); 8.01 GiB → Medium.
    // The contract: ram_tier matches `GiB <= 8 → Low`, `8 < GiB <= 15 → Medium`, `> 15 → High`.
    // This test pins the boundary behavior so future refactors cannot drift.
    let near_8 = |gib: u64| {
        let bytes = gib * 1024 * 1024 * 1024;
        SysCaps {
            total_memory_bytes: bytes,
            physical_cores: 4,
            gpu: GpuKind::None,
        }
        .ram_tier()
    };
    assert_eq!(near_8(8), RamTier::Low);
    assert_eq!(near_8(9), RamTier::Medium);
    assert_eq!(near_8(15), RamTier::Medium);
    assert_eq!(near_8(16), RamTier::High);
}
