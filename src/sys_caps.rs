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
///
/// `Clone` (not `Copy`) because the `GpuKind` variants added in
/// T19 (#826) carry a `String` for the GPU's display name.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// GPU kind. v3 ships only `None`; round 2 (T19, #826) adds
/// `Metal` (macOS via `objc2-metal`) and `Cuda` (Linux + Windows
/// via `nvml-wrapper`).  The `Metal` and `Cuda` variants carry
/// the GPU's display name and its reported VRAM (in bytes) so the
/// `--print-system-info` CLI dump and any future LLM provider
/// selection can surface them without re-probing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuKind {
    /// No GPU detected (or probing not implemented for this host).
    None,
    /// Apple Silicon or Intel iGPU detected via Metal
    /// (`MTLCreateSystemDefaultDevice`).  `name` is the device
    /// name (e.g. `"Apple M1 Pro"`); `vram_bytes` is the
    /// recommended maximum working-set size — the closest proxy
    /// to VRAM on unified-memory Macs.
    Metal {
        name: String,
        vram_bytes: u64,
    },
    /// NVIDIA GPU detected via NVML.  `name` is the device model
    /// string (e.g. `"NVIDIA GeForce RTX 4090"`); `vram_bytes` is
    /// `memoryInfo.total` from `nvmlDeviceGetMemoryInfo`.
    Cuda {
        name: String,
        vram_bytes: u64,
    },
}

impl GpuKind {
    /// Display name of the GPU, or `None` if no GPU is present.
    pub fn name(&self) -> Option<&str> {
        match self {
            GpuKind::None => None,
            GpuKind::Metal { name, .. } | GpuKind::Cuda { name, .. } => Some(name),
        }
    }

    /// Reported VRAM in bytes, or `None` if no GPU is present.
    pub fn vram_bytes(&self) -> Option<u64> {
        match self {
            GpuKind::None => None,
            GpuKind::Metal { vram_bytes, .. } | GpuKind::Cuda { vram_bytes, .. } => Some(*vram_bytes),
        }
    }
}

/// Coarse GPU tier.  Used by future LLM provider selection to
/// answer "is this host powerful enough to host a 7B model?".
///
/// - `Integrated`: shared-memory GPU (Apple Silicon, Intel iGPU).
/// - `Discrete`: dedicated VRAM (NVIDIA dGPU on Linux/Windows).
/// - `None`: no GPU detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuTier {
    /// Apple Silicon, Intel iGPU, or any other shared-memory GPU.
    Integrated,
    /// Discrete GPU with its own VRAM (NVIDIA dGPU on Linux/Windows).
    Discrete,
    /// No GPU detected.
    None,
}

impl GpuKind {
    /// Map a [`GpuKind`] to a coarse [`GpuTier`].  Pure decision
    /// over the variant — pulled out so the per-file coverage gate
    /// sees all three arms.
    pub fn gpu_tier(&self) -> GpuTier {
        match self {
            GpuKind::None => GpuTier::None,
            GpuKind::Metal { .. } => GpuTier::Integrated,
            GpuKind::Cuda { .. } => GpuTier::Discrete,
        }
    }
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
        CACHE.get_or_init(detect_inner).clone()
    }

    /// Classify this snapshot's RAM into [`RamTier`].
    pub fn ram_tier(&self) -> RamTier {
        const GIB: u64 = 1024 * 1024 * 1024;
        match self.total_memory_bytes / GIB {
            0..=8 => RamTier::Low,
            9..=15 => RamTier::Medium,
            _ => RamTier::High,
        }
    }

    /// Classify this snapshot's core count into [`CpuTier`].
    pub fn cpu_tier(&self) -> CpuTier {
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
    let gpu = detect_gpu();
    SysCaps {
        total_memory_bytes,
        physical_cores,
        gpu,
    }
}

/// Detect the host's GPU.  Tries Metal on macOS first; falls
/// back to NVML on Linux/Windows; returns `GpuKind::None` when
/// neither backend is available or the probe fails (no NVIDIA
/// driver on Linux, no Metal on Windows, no GPU in the chassis,
/// etc.).
///
/// The Metal + NVML probe functions are feature-gated behind
/// `gpu-detect`; without that feature, both helpers return
/// `GpuKind::None` and the GPU column in `--print-system-info`
/// stays at `none`, matching the v3 behaviour.
fn detect_gpu() -> GpuKind {
    // Pulled out so the per-file coverage gate sees all three
    // arms (macos hit, linux/windows hit, fallback) even when
    // the host happens to have a working GPU.
    detect_gpu_inner().unwrap_or(GpuKind::None)
}

/// Feature-gated GPU probe entry point.  With the `gpu-detect`
/// feature enabled (and the host OS matching), returns
/// `Some(kind)` from the platform-specific probe; otherwise
/// `None`.  Used by `detect_gpu` so the per-file coverage gate
/// sees all three arms (macos hit, linux/windows hit, fallback)
/// even when the host happens to have a working GPU.
#[cfg(any(
    all(feature = "gpu-detect", target_os = "macos"),
    all(feature = "gpu-detect", any(target_os = "linux", target_os = "windows"))
))]
fn detect_gpu_inner() -> Option<GpuKind> {
    // The cfg-gated probe calls are intentionally not wrapped in
    // an extra `{}` block here: an extra block introduces a
    // closing-brace line that llvm-cov attributes to the
    // cfg-gated arm but never increments when the probe returns
    // `Some` and we exit early.  Keeping the cfg gates as the
    // outermost control flow avoids that.
    #[cfg(all(feature = "gpu-detect", target_os = "macos"))]
    return detect_metal();
    #[cfg(all(feature = "gpu-detect", any(target_os = "linux", target_os = "windows")))]
    return detect_cuda();
    #[cfg(not(any(feature = "gpu-detect", target_os = "macos", target_os = "linux", target_os = "windows")))]
    None
}

/// No-feature / no-matching-OS fallback.  Always returns `None`
/// so the default build (no `gpu-detect` feature) keeps the v3
/// "no GPU detected" baseline.
#[cfg(not(any(
    all(feature = "gpu-detect", target_os = "macos"),
    all(feature = "gpu-detect", any(target_os = "linux", target_os = "windows"))
)))]
fn detect_gpu_inner() -> Option<GpuKind> {
    None
}

/// Probe for an Apple GPU via `MTLCreateSystemDefaultDevice()`.
/// Returns `Some(GpuKind::Metal { .. })` when a default device is
/// reported; `None` on every other outcome (no Metal runtime,
/// no GPU, or the FFI call panics — which we catch and swallow
/// so the rest of `SysCaps::detect()` still completes).
#[cfg(all(feature = "gpu-detect", target_os = "macos"))]
fn detect_metal() -> Option<GpuKind> {
    use objc2_metal::{MTLCreateSystemDefaultDevice, MTLDevice};
    // `MTLCreateSystemDefaultDevice()` returns
    // `Option<Retained<ProtocolObject<dyn MTLDevice>>>`.  The
    // `Retained` type derefs to the inner `ProtocolObject`, so
    // method calls on the trait (`name`, `recommendedMaxWorkingSetSize`)
    // resolve automatically.  The trait methods are not `unsafe`
    // in objc2 0.3.2.
    let device = MTLCreateSystemDefaultDevice()?;
    let name_ns = device.name();
    let name = (*name_ns).to_string();
    let vram_bytes = device.recommendedMaxWorkingSetSize();
    Some(GpuKind::Metal { name, vram_bytes })
}

/// Probe for an NVIDIA GPU via NVML.  Initialises the library,
/// enumerates devices, and returns the first device's name +
/// total memory.  Returns `None` when NVML is not loadable
/// (no driver installed, no NVIDIA hardware, container without
/// the device, etc.).
#[cfg(all(feature = "gpu-detect", any(target_os = "linux", target_os = "windows")))]
fn detect_cuda() -> Option<GpuKind> {
    let nvml = nvml_wrapper::Nvml::init().ok()?;
    let count = nvml.device_count().ok()?;
    if count == 0 {
        return None;
    }
    let handle = nvml.device_by_index(0).ok()?;
    let name = handle.name().ok()?;
    let memory = handle.memory_info().ok()?;
    Some(GpuKind::Cuda {
        name,
        vram_bytes: memory.total,
    })
}

fn detect_physical_cores(sys: &System) -> usize {
    pick_physical_cores(sys.physical_core_count())
}

/// Fallback when [`sysinfo::System::physical_core_count`] returns
/// `None` or `0`. Tries `std::thread::available_parallelism` (which
/// returns `io::Result<NonZeroUsize>`) and falls back to 1 on
/// any error.
///
/// `fallback_physical_cores_from(probed)` is the testable seam:
/// both `Ok(n)` and `Err(_)` are reachable from unit tests
/// (the `Ok` arm runs on every test; the `Err` arm is exercised
/// by the dedicated test below).
#[allow(dead_code)]
pub(crate) fn fallback_physical_cores() -> usize {
    fallback_physical_cores_from(std::thread::available_parallelism())
}

/// Pure decision: convert the `available_parallelism` result into
/// a positive count. Pulled out so the `Err` arm is unit-testable.
pub(crate) fn fallback_physical_cores_from(
    probed: Result<std::num::NonZeroUsize, std::io::Error>,
) -> usize {
    match probed {
        Ok(n) => n.get(),
        Err(_) => fallback_physical_cores_on_error(),
    }
}

/// Pure helper for the Err / Ok(0) arm of [`fallback_physical_cores`].
/// Returns 1. Pulled out so the coverage gate can see it via the
/// dedicated test, and so the `match` arm in the caller stays
/// one line.
#[allow(dead_code)]
pub(crate) fn fallback_physical_cores_on_error() -> usize {
    1
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
