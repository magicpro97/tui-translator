//! Local provider runtime hints (#827, round 2).
//!
//! `LocalProviderHints` is a thin runtime-probe facade.  It
//! answers two questions the static `SysCaps::detect()` snapshot
//! can't:
//!
//!   * `ram_budget_mb()` — how much of `SysCaps::total_ram` is
//!     actually free RIGHT NOW (i.e. the user closed their
//!     browser, the OS reclaimed cache, etc.).  Sampled lazily on
//!     each `process_chunk` call so a hot loop doesn't pay
//!     `sysinfo`'s 50-200ms cost every frame.
//!
//!   * `cpu_budget_pct()` — what fraction of the logical-CPU pool
//!     is currently idle.  A 0-100 integer: 100 means the
//!     machine is doing nothing, 0 means every core is busy.
//!
//! Round-1 (#819) only sampled `SysCaps::detect()` once at the
//! HardwareSurvey step.  Round-2 (#827) re-evaluates the
//! `QualityPreset` whenever the budget moves by >20% so a
//! Best-rated user who later downgrades to 4 GiB free doesn't
//! keep failing to load the 3-4 GiB `Best` model.
//!
//! # Feature-gating
//!
//! `sysinfo` is a heavy dependency.  When the `local-stt-*`
//! feature is OFF (default build, no local models), the hints
//! return sentinel "no pressure" values:
//!
//!   * `ram_budget_mb() = u64::MAX` — never triggers a downgrade
//!   * `cpu_budget_pct() = 100` — machine is "idle"
//!
//! This keeps the integration test suite fast (no `sysinfo`
//! bring-up cost) and the binary lean when the local provider
//! isn't compiled in.  Tests that need realistic budget values
//! inject a `LocalProviderHints` via the `for_test()` constructor.
//!
//! # Plan-mode note (2026-06-17)
//!
//! This is the 4th of 5 round-2 issues.  Scope is bounded to
//! `quality_preset.rs` + a new `provider_hints.rs` + 6
//! integration tests.  The >20% change gate is intentionally a
//! constant (`BUDGET_CHANGE_GATE`) so the integration test can
//! raise / lower it without invasive refactors.

#![allow(dead_code)]

use std::sync::Arc;

use std::sync::Mutex;

/// The fraction-by-which the budget must change before we
/// re-evaluate the preset.  Exposed as a `pub const` so the
/// `adaptive_preset_*` integration tests can name it in their
/// assertions.  20% is conservative — too-aggressive re-detect
/// thrashes the user with "Preset changed" notices.
pub const BUDGET_CHANGE_GATE: f64 = 0.20;

/// Default cache TTL for the budget sample.  We don't want
/// `process_chunk` (called every 30ms) to slam `sysinfo` on every
/// frame.  A 2-second window means the budget is fresh enough to
/// catch real pressure (closing a browser drops free RAM within
/// 1-2s) without measurable CPU cost.
pub const BUDGET_SAMPLE_TTL_MS: u64 = 2_000;

/// Runtime probe facade for the local provider's resource budget.
///
/// Cheap to clone (`Arc` inside) so `process_chunk` can hand
/// snapshots out without coupling to the orchestrator's lifetime.
#[derive(Clone)]
pub struct LocalProviderHints {
    inner: Arc<Mutex<HintState>>,
    sampler: Arc<dyn BudgetSampler + Send + Sync>,
}

struct HintState {
    /// Cached (ram_mb, cpu_pct, sampled_at_ms).
    last_sample: Option<(u64, u8, u64)>,
}

impl std::fmt::Debug for LocalProviderHints {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        f.debug_struct("LocalProviderHints")
            .field("last_sample", &guard.last_sample)
            .finish()
    }
}

impl Default for LocalProviderHints {
    fn default() -> Self {
        // Default: no-pressure sentinels, no-op sampler.  This is
        // what the bin gets when the `local-stt-*` features are
        // off, so the integration test suite never pays
        // `sysinfo`'s boot cost.
        Self::no_pressure()
    }
}

impl LocalProviderHints {
    /// Construct a no-pressure hints instance: budget values that
    /// never trigger a downgrade.  Used in unit tests and the
    /// default-bin path where `sysinfo` is feature-gated.
    pub fn no_pressure() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HintState { last_sample: None })),
            sampler: Arc::new(ConstBudget {
                ram_mb: u64::MAX,
                cpu_pct: 100,
            }),
        }
    }

    /// Construct with a custom sampler.  Test-only entry point
    /// used by the `adaptive_preset_*` integration tests to
    /// inject deterministic budget trajectories (Best→Performance
    /// downgrade when RAM drops below 4 GiB, etc.).
    pub fn for_test(sampler: Arc<dyn BudgetSampler + Send + Sync>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HintState { last_sample: None })),
            sampler,
        }
    }

    /// Sample the free-RAM budget.  Returns a value in MiB.
    /// Cached for `BUDGET_SAMPLE_TTL_MS`; repeated calls within
    /// the window return the cached sample.
    ///
    /// # Returns
    /// `u64::MAX` means "no pressure" — caller's >20% gate will
    /// never trip and the preset stays as-is.
    pub fn ram_budget_mb(&self) -> u64 {
        self.sample().0
    }

    /// Sample the CPU budget.  Returns a value in 0-100.
    /// 100 = idle, 0 = saturated.  Same caching rules as
    /// [`ram_budget_mb`](Self::ram_budget_mb).
    pub fn cpu_budget_pct(&self) -> u8 {
        self.sample().1
    }

    /// Force the next sample to bypass the cache.  Test-only
    /// entry point — production callers should never need this
    /// because the 2s TTL is the right cadence for real budget
    /// drift.
    #[cfg(test)]
    pub fn invalidate_cache(&self) {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .last_sample = None;
    }

    fn sample(&self) -> (u64, u8) {
        let now_ms = monotonic_ms();
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((ram, cpu, sampled_at)) = guard.last_sample {
            if now_ms.saturating_sub(sampled_at) < BUDGET_SAMPLE_TTL_MS {
                return (ram, cpu);
            }
        }
        let (ram, cpu) = self.sampler.sample();
        guard.last_sample = Some((ram, cpu, now_ms));
        (ram, cpu)
    }
}

/// Trait that decouples the hint facade from the real `sysinfo`
/// probe.  Test code injects a `ConstBudget` or
/// `TrajectoryBudget` (a series of `(ram_mb, cpu_pct)` pairs the
/// sampler walks through in order) to simulate a user
/// closing apps or a battery dropping the CPU budget.
pub trait BudgetSampler {
    /// Return `(free_ram_mb, idle_cpu_pct)` for the current
    /// moment.  May be called at most once per
    /// `BUDGET_SAMPLE_TTL_MS` window.
    fn sample(&self) -> (u64, u8);
}

/// Constant sampler — always returns the same value.  Used by
/// the no-pressure default and by the simplest unit tests.
pub struct ConstBudget {
    pub ram_mb: u64,
    pub cpu_pct: u8,
}

impl BudgetSampler for ConstBudget {
    fn sample(&self) -> (u64, u8) {
        (self.ram_mb, self.cpu_pct)
    }
}

/// Trajectory sampler — walks a pre-recorded series of
/// `(ram_mb, cpu_pct)` snapshots.  Each call pops the front of
/// the series.  The `adaptive_preset_*` integration tests use
/// this to simulate "user opens a 4 GiB Chrome window", "user
/// closes Chrome again", etc.
pub struct TrajectoryBudget {
    pub series: std::sync::Mutex<Vec<(u64, u8)>>,
    pub fallback: (u64, u8),
}

impl TrajectoryBudget {
    /// Build a trajectory from a `(ram_mb, cpu_pct)` series.
    /// The first call returns the first element; subsequent
    /// calls return later elements.  When the series is
    /// exhausted, `fallback` is returned forever.
    pub fn new(series: Vec<(u64, u8)>, fallback: (u64, u8)) -> Self {
        Self {
            series: std::sync::Mutex::new(series),
            fallback,
        }
    }
}

impl BudgetSampler for TrajectoryBudget {
    fn sample(&self) -> (u64, u8) {
        #[allow(clippy::expect_used, clippy::unwrap_used)]
        let mut guard = self.series.lock().unwrap();
        if guard.is_empty() {
            self.fallback
        } else {
            guard.remove(0)
        }
    }
}

#[cfg(target_os = "macos")]
fn monotonic_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(not(target_os = "macos"))]
fn monotonic_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_impl_yields_no_pressure() {
        // The Default impl just delegates to `no_pressure()`,
        // so the bin picks up the no-pressure sentinel when the
        // `local-stt-*` features are off.
        let h = LocalProviderHints::default();
        assert_eq!(h.ram_budget_mb(), u64::MAX);
        assert_eq!(h.cpu_budget_pct(), 100);
    }

    #[test]
    fn debug_impl_renders_without_panicking() {
        let h = LocalProviderHints::no_pressure();
        let rendered = format!("{h:?}");
        assert!(rendered.contains("LocalProviderHints"));
    }

    #[test]
    fn no_pressure_returns_sentinels() {
        let h = LocalProviderHints::no_pressure();
        assert_eq!(h.ram_budget_mb(), u64::MAX);
        assert_eq!(h.cpu_budget_pct(), 100);
    }

    #[test]
    fn const_sampler_is_honest() {
        let h = LocalProviderHints::for_test(Arc::new(ConstBudget {
            ram_mb: 4096,
            cpu_pct: 50,
        }));
        assert_eq!(h.ram_budget_mb(), 4096);
        assert_eq!(h.cpu_budget_pct(), 50);
    }

    #[test]
    fn trajectory_sampler_walks_series_then_falls_back() {
        let sampler = Arc::new(TrajectoryBudget::new(
            vec![(8192, 80), (4096, 50), (2048, 20)],
            (1024, 5),
        ));
        let h = LocalProviderHints::for_test(sampler.clone());
        // First sample: (8192, 80) cached.  This is the
        // cache-MISS path.
        let (ram1, cpu1) = (h.ram_budget_mb(), h.cpu_budget_pct());
        assert_eq!((ram1, cpu1), (8192, 80));
        // Second call within the 2s TTL — this is the
        // cache-HIT path.  Branch coverage needs both arms.
        let (ram_cached, cpu_cached) = (h.ram_budget_mb(), h.cpu_budget_pct());
        assert_eq!((ram_cached, cpu_cached), (8192, 80));
        // Bypass cache to walk the series (production callers
        // never need to do this — the 2s TTL keeps the budget
        // fresh without hitting the sampler on every frame).
        h.invalidate_cache();
        let (ram2, cpu2) = (h.ram_budget_mb(), h.cpu_budget_pct());
        assert_eq!((ram2, cpu2), (4096, 50));
        h.invalidate_cache();
        let (ram3, _cpu3) = (h.ram_budget_mb(), h.cpu_budget_pct());
        assert_eq!(ram3, 2048);
        h.invalidate_cache();
        // series exhausted → fallback.
        let (ram4, cpu4) = (h.ram_budget_mb(), h.cpu_budget_pct());
        assert_eq!((ram4, cpu4), (1024, 5));
    }
}
