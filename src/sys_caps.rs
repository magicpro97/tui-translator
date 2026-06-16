//! System capability probe for the Quality Preset system.
//!
//! Detects total physical RAM and physical CPU core count once per
//! process and exposes a coarse `RamTier` / `CpuTier` enum that the
//! `QualityPreset::Auto` detector maps to Best/Performance.
//!
//! # Caching
//!
//! `detect()` wraps a [`OnceLock`] so the result is computed exactly
//! once per process. Repeated `detect()` calls return the same
//! struct by clone.
//!
//! # GPU
//!
//! `GpuKind` is `None` in v3 because the LLM engine hard-codes
//! `Device::Cpu` at `src/providers/llm/engine.rs:139,189`. v3.1
//! would add Metal probing via `objc2-metal` (macOS) and NVML
//! via `nvml-wrapper` (Linux + Windows). Adding that does NOT
//! change the public surface of this module.
//!
//! # Dependencies
//!
//! Zero new deps. `sysinfo = "0.30"` is already in
//! `Cargo.toml:106` with `default-features = false`. We use
//! `sysinfo::System::physical_core_count()` (canonical path
//! per `src/providers/local/runtime_caps.rs:55` for the
//! thread-cap formula) and `sysinfo::System::total_memory()`.
//!
//! # Usage in v3
//!
//! T1 (this issue) only ships the probe + tier enums. Downstream
//! consumers will arrive in:
//! - T2: `QualityPreset::Auto` selects Best/Performance from `ram_tier`
//! - T3: `AppConfig::quality_preset` defaults to `Auto` at startup
//! - T15: `--print-system-info` CLI flag pretty-prints the snapshot
//!
//! Until those consumers land, items here are unused in the non-test
//! build. Suppress dead-code only when not running tests, so
//! internal test refs (e.g. `detect_inner`, `detect_physical_cores`)
//! still surface warnings in test builds.

#![allow(dead_code)]

use std::sync::OnceLock;

use sysinfo::System;

/// Snapshot of the host's hardware capabilities relevant to model
/// selection. Returned by [`SysCaps::detect`] (cached) or
/// constructed in tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SysCaps {
    /// Total physical memory in bytes. From
    /// `sysinfo::System::total_memory()`.
    pub total_memory_bytes: u64,
    /// Number of physical CPU cores. From
    /// `sysinfo::System::physical_core_count()` with a fallback to
    /// `std::thread::available_parallelism()` (logical cores as a
    /// lower-bound estimate) and finally `1`.
    pub physical_cores: usize,
    /// Detected GPU kind. Always [`GpuKind::None`] in v3.
    pub gpu: GpuKind,
}

/// GPU kind. v3 ships only `None`; future work adds `Metal` (macOS)
/// and `Cuda` (Linux + Windows) probing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuKind {
    /// No GPU detected (or probing not implemented yet).
    None,
    /// Apple Silicon or Intel iGPU (v3.1, `objc2-metal`).
    #[allow(dead_code)] // v3.1: planned, not yet constructed.
    Metal,
    /// NVIDIA GPU (v3.1, `nvml-wrapper`, Linux + Windows only).
    #[allow(dead_code)] // v3.1: planned, not yet constructed.
    Cuda,
}

/// Coarse RAM tier. Matches the project's existing benchmark
/// table at `docs/09-cpu-model-benchmark.md:18-21`:
/// - `Low`: ≤ 8 GiB
/// - `Medium`: 9..=15 GiB
/// - `High`: ≥ 16 GiB
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RamTier {
    /// ≤ 8 GiB total memory (8 GB-laptop tier).
    Low,
    /// 9..=15 GiB total memory (16 GB-desktop tier).
    Medium,
    /// ≥ 16 GiB total memory (32 GB+ workstation tier).
    High,
}

/// Coarse CPU tier. Matches the reference CPU tiers in
/// `docs/adr/llm-mt-03-cpu-inference-crate-selection.md:63-67`:
/// - `Low`: 1-2 cores
/// - `Medium`: 3-5 cores
/// - `High`: ≥ 6 cores
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuTier {
    /// 1-2 physical cores.
    Low,
    /// 3-5 physical cores.
    Medium,
    /// ≥ 6 physical cores.
    High,
}

impl SysCaps {
    /// Detect hardware capabilities (cached). Safe to call repeatedly.
    pub fn detect() -> Self {
        static CACHE: OnceLock<SysCaps> = OnceLock::new();
        *CACHE.get_or_init(detect_inner)
    }

    /// Classify this snapshot's RAM into [`RamTier`].
    pub fn ram_tier(self) -> RamTier {
        const GIB: u64 = 1024 * 1024 * 1024;
        match self.total_memory_bytes / GIB {
            0..=8 => RamTier::Low,
            9..=15 => RamTier::Medium,
            _ => RamTier::High,
        }
    }

    /// Classify this snapshot's core count into [`CpuTier`].
    pub fn cpu_tier(self) -> CpuTier {
        match self.physical_cores {
            0..=2 => CpuTier::Low,
            3..=5 => CpuTier::Medium,
            _ => CpuTier::High,
        }
    }
}

fn detect_inner() -> SysCaps {
    let mut sys = System::new();
    sys.refresh_memory();
    sys.refresh_cpu();
    let total_memory_bytes = sys.total_memory();
    let physical_cores = detect_physical_cores(&sys);
    SysCaps {
        total_memory_bytes,
        physical_cores,
        gpu: GpuKind::None,
    }
}

fn detect_physical_cores(sys: &System) -> usize {
    pick_physical_cores(sys.physical_core_count())
}

/// Fallback when [`sysinfo::System::physical_core_count`] returns
/// `None` or `0`. Tries `std::thread::available_parallelism` (which
/// usually reports logical cores) and finally `1`.
#[allow(dead_code)]
pub(crate) fn fallback_physical_cores() -> usize {
    match std::thread::available_parallelism() {
        Ok(n) if n.get() > 0 => n.get(),
        _ => 1,
    }
}

/// Pure decision over a `physical_core_count` probe. Returns
/// the count if it is `Some(n)` with `n > 0`; otherwise falls
/// back to [`fallback_physical_cores`]. Testable without
/// `sysinfo` so the coverage gate sees all branches.
pub(crate) fn pick_physical_cores(probed: Option<usize>) -> usize {
    match probed {
        Some(n) if n > 0 => n,
        _ => fallback_physical_cores(),
    }
}
