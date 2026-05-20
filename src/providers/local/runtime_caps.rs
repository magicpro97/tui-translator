//! Local STT/MT runtime caps (issue #370, LF-02).
//!
//! The local on-device pipeline (Whisper, OPUS-MT) must leave headroom for
//! latency-sensitive apps (Zoom/Teams audio capture, the WASAPI loopback
//! thread, the Tokio runtime).  This module exposes:
//!
//! * [`local_thread_cap_for`] — pure CPU-count → cap function used by every
//!   local-inference construction point (whisper `FullParams::set_n_threads`
//!   and ORT `Session::builder().with_intra_threads`).  Formula:
//!   `min(4, max(1, physical_cores - 2))`.
//! * [`detect_physical_cores`] — best-effort runtime probe of the host's
//!   physical-core count.  Falls back to `available_parallelism()` and then
//!   `1`.
//! * [`local_thread_cap`] — convenience that combines the two for production
//!   construction sites.
//! * [`ActiveLocalInference`] — RAII guard that increments / decrements a
//!   shared in-flight local-inference operation gauge.  Read by the metrics
//!   publisher and surfaced through [`MetricsSnapshot::local_active_threads`].
//!
//! The throttle / backpressure policy itself lives in
//! [`crate::pipeline::cpu_gate::CpuGate`] (advisory, defer-next-segment).
//! This module is concerned only with the **construction-time cap** and the
//! observability gauge.  No silent sleeps on the hot path.
//!
//! [`MetricsSnapshot::local_active_threads`]:
//!     crate::metrics::MetricsSnapshot::local_active_threads

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

/// Compute the local-inference thread cap from a physical-core count.
///
/// Formula: `min(4, max(1, physical_cores - 2))`.
///
/// Examples (acceptance criteria, issue #370):
/// * 8 physical cores → cap `4`
/// * 6 physical cores → cap `4`
/// * 4 physical cores → cap `2`
/// * 2 physical cores → cap `1`
/// * 1 physical core  → cap `1` (saturating subtraction keeps the floor)
///
/// The cap is intentionally bounded by `4` so a many-core workstation does
/// not starve the audio-capture thread or the Tokio runtime when a local
/// inference saturates its threadpool.
#[inline]
pub fn local_thread_cap_for(physical_cores: usize) -> usize {
    physical_cores.saturating_sub(2).clamp(1, 4)
}

/// Best-effort runtime probe of the host's physical core count.
///
/// Tries `sysinfo`'s `physical_core_count` first (the canonical value for
/// LF-02's formula), then falls back to `std::thread::available_parallelism`
/// (which usually reports logical cores), and finally `1` if both fail.  The
/// caller should treat the return value as an opaque hint and route it
/// through [`local_thread_cap_for`] before passing it to any threadpool API.
pub fn detect_physical_cores() -> usize {
    if let Some(n) = sysinfo::System::new().physical_core_count() {
        if n > 0 {
            return n;
        }
    }
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// Production cap: probe physical cores and apply the LF-02 formula.
///
/// Result is cached in a process-wide [`OnceLock`] so repeated construction
/// sites (whisper STT state creation, ORT MT session loads) all observe the
/// same value without redundant `sysinfo` probes.
pub fn local_thread_cap() -> usize {
    static CACHED: OnceLock<usize> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let cores = detect_physical_cores();
        let cap = local_thread_cap_for(cores);
        tracing::info!(
            physical_cores = cores,
            thread_cap = cap,
            "computed local-inference thread cap (LF-02, issue #370)"
        );
        cap
    })
}

// ── Active-threads gauge ──────────────────────────────────────────────────────

/// Process-wide counter of in-flight local-inference operations.
///
/// Incremented when a Whisper STT or OPUS-MT inference begins on a blocking
/// thread and decremented when it returns.  Read by the metrics publisher
/// once per second to populate
/// [`MetricsSnapshot::local_active_threads`].
///
/// Using a single static counter (rather than constructor-injected
/// `Arc<AtomicUsize>`) keeps the existing provider construction surface
/// untouched and lets local STT and local MT share the same gauge.
///
/// [`MetricsSnapshot::local_active_threads`]:
///     crate::metrics::MetricsSnapshot::local_active_threads
static ACTIVE_LOCAL_THREADS: AtomicUsize = AtomicUsize::new(0);

/// Current in-flight local-inference count.
///
/// Stale-by-≤1-second readings are acceptable; the metrics publisher polls
/// this with `Ordering::Relaxed`.
#[inline]
pub fn active_local_threads() -> usize {
    ACTIVE_LOCAL_THREADS.load(Ordering::Relaxed)
}

/// RAII guard that increments the in-flight local-inference operation gauge on
/// construction and decrements it on drop.
///
/// Always pair construction with the actual blocking-thread inference call
/// (e.g. inside `spawn_blocking`).  The guard is `Send` so it can be moved
/// across the blocking-thread boundary.
#[must_use = "drop the guard inside the blocking inference scope; assigning to `_` releases it immediately"]
#[derive(Debug)]
pub struct ActiveLocalInference {
    _private: (),
}

impl ActiveLocalInference {
    /// Enter a local-inference scope and increment the gauge.
    pub fn enter() -> Self {
        ACTIVE_LOCAL_THREADS.fetch_add(1, Ordering::Relaxed);
        Self { _private: () }
    }
}

impl Drop for ActiveLocalInference {
    fn drop(&mut self) {
        ACTIVE_LOCAL_THREADS.fetch_sub(1, Ordering::Relaxed);
    }
}

// ── ORT / OpenMP env coordination ─────────────────────────────────────────────

/// Result of [`prepare_omp_env`]: describes what the helper did to
/// `OMP_NUM_THREADS` and which cap value is in effect.
///
/// Use `applied` to decide what to log after tracing is initialized:
/// * `true`  → we exported `OMP_NUM_THREADS=<cap>` (was unset).
/// * `false` → a caller-inherited value was already present; we left it alone.
///   The `cap` field still holds the LF-02 formula result and is the value
///   passed to ORT builder methods, but we do **not** claim that the inherited
///   env value equals `cap`.
#[cfg(feature = "local-mt")]
#[derive(Debug, Clone, Copy)]
pub struct OmpEnvStatus {
    /// LF-02 formula result: `min(4, max(1, physical_cores - 2))`.
    pub cap: usize,
    /// `true` if we wrote `OMP_NUM_THREADS`; `false` if we deferred to the
    /// inherited environment.
    pub applied: bool,
}

/// Prepare the `OMP_NUM_THREADS` environment variable for the onnxruntime
/// OpenMP thread pool.
///
/// onnxruntime's Microsoft prebuilt binaries honour `OMP_NUM_THREADS` at
/// library-load time; without this the `with_intra_threads` builder option is
/// silently ignored on Windows GNU/OpenMP builds.
///
/// # Behaviour
///
/// * If `OMP_NUM_THREADS` is **not** set in the environment, this function
///   writes `<cap>` (the LF-02 formula result) and returns
///   [`OmpEnvStatus::applied`]` = true`.
/// * If `OMP_NUM_THREADS` is **already** set (inherited from the calling
///   shell or test harness), the value is left unchanged and
///   [`OmpEnvStatus::applied`]` = false` is returned.  The user's choice is
///   always respected; we never silently override it.
///
/// The result is cached in a [`OnceLock`] so the env-var check and optional
/// write happen at most once per process, regardless of how many ORT sessions
/// are constructed.  Call this as early as possible in `main` — before any
/// library load — then log the returned status once tracing is ready.
#[cfg(feature = "local-mt")]
pub fn prepare_omp_env() -> OmpEnvStatus {
    static STATUS: OnceLock<OmpEnvStatus> = OnceLock::new();
    *STATUS.get_or_init(|| {
        let cap = local_thread_cap();
        let applied = std::env::var_os("OMP_NUM_THREADS").is_none();
        if applied {
            std::env::set_var("OMP_NUM_THREADS", cap.to_string());
        }
        OmpEnvStatus { cap, applied }
    })
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ACTIVE_GUARD_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Acceptance criterion (issue #370): 8 physical cores → cap 4.
    #[test]
    fn cap_for_eight_physical_cores_is_four() {
        assert_eq!(local_thread_cap_for(8), 4);
    }

    /// Acceptance criterion (issue #370): 2 physical cores → cap 1.
    #[test]
    fn cap_for_two_physical_cores_is_one() {
        assert_eq!(local_thread_cap_for(2), 1);
    }

    #[test]
    fn cap_is_floored_at_one_for_low_core_hosts() {
        // 1 physical core → max(1, 1 - 2 saturating) = max(1, 0) = 1
        assert_eq!(local_thread_cap_for(1), 1);
        // 0 (unrealistic, but `saturating_sub` should keep us safe)
        assert_eq!(local_thread_cap_for(0), 1);
    }

    #[test]
    fn cap_is_ceilinged_at_four_for_many_core_hosts() {
        assert_eq!(local_thread_cap_for(16), 4);
        assert_eq!(local_thread_cap_for(64), 4);
        assert_eq!(local_thread_cap_for(usize::MAX), 4);
    }

    #[test]
    fn cap_is_monotonic_non_decreasing_in_cores() {
        let mut last = 0;
        for cores in 0..=32 {
            let cap = local_thread_cap_for(cores);
            assert!(
                cap >= last,
                "cap must be monotonic non-decreasing in physical cores: cores={cores} cap={cap} last={last}"
            );
            assert!((1..=4).contains(&cap), "cap must stay in [1, 4]: {cap}");
            last = cap;
        }
    }

    #[test]
    fn active_guard_increments_and_decrements() {
        let _lock = ACTIVE_GUARD_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = active_local_threads();
        let g = ActiveLocalInference::enter();
        assert_eq!(active_local_threads(), before + 1);
        drop(g);
        assert_eq!(active_local_threads(), before);
    }

    #[test]
    fn active_guard_nests_correctly() {
        let _lock = ACTIVE_GUARD_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = active_local_threads();
        let a = ActiveLocalInference::enter();
        let b = ActiveLocalInference::enter();
        assert_eq!(active_local_threads(), before + 2);
        drop(a);
        assert_eq!(active_local_threads(), before + 1);
        drop(b);
        assert_eq!(active_local_threads(), before);
    }

    #[test]
    fn detect_physical_cores_returns_at_least_one() {
        // Host-dependent but we only check the lower bound (defensive contract).
        assert!(detect_physical_cores() >= 1);
    }

    #[test]
    fn local_thread_cap_is_cached_and_stable() {
        let a = local_thread_cap();
        let b = local_thread_cap();
        assert_eq!(a, b);
        assert!((1..=4).contains(&a));
    }

    // ── OMP env helper logic (pure / non-destructive tests) ──────────────────
    //
    // We cannot test `prepare_omp_env()` directly in a parallel test run
    // because its `OnceLock` is process-wide; whichever test wins the race
    // would permanently set the cached status.  Instead we test the *decision
    // logic* in isolation using the same predicate the real helper uses.

    /// Captures the OMP decision as a pure function for testability without
    /// mutating process state.
    #[cfg(feature = "local-mt")]
    fn omp_would_apply(env_value: Option<&str>) -> bool {
        env_value.is_none()
    }

    #[cfg(feature = "local-mt")]
    #[test]
    fn omp_env_decision_applies_when_var_absent() {
        // When OMP_NUM_THREADS is not inherited we should write it.
        assert!(omp_would_apply(None));
    }

    #[cfg(feature = "local-mt")]
    #[test]
    fn omp_env_decision_defers_when_var_present() {
        // When the caller has already set OMP_NUM_THREADS we must not override.
        assert!(!omp_would_apply(Some("8")));
        assert!(!omp_would_apply(Some("1")));
        assert!(!omp_would_apply(Some("")));
    }

    /// Calling `prepare_omp_env()` twice must return the same status (cached).
    #[cfg(feature = "local-mt")]
    #[test]
    fn prepare_omp_env_is_idempotent() {
        let a = super::prepare_omp_env();
        let b = super::prepare_omp_env();
        assert_eq!(a.cap, b.cap);
        assert_eq!(a.applied, b.applied);
        assert!((1..=4).contains(&a.cap));
    }
}
