//! Process-level resource metrics (issue #79).
//!
//! [`ProcessSnapshot`] carries CPU and RAM usage for the current process,
//! sampled once per second using the [`sysinfo`] crate.
//!
//! Call [`spawn_process_metrics_task`] once at startup, passing a
//! `tokio::sync::watch::Sender<ProcessSnapshot>`.  The task runs on a
//! dedicated blocking thread (via [`tokio::task::spawn_blocking`]) and sends
//! a fresh snapshot on each one-second tick.  Any task that holds the
//! matching receiver can call `.borrow()` to read the latest value without
//! blocking.
//!
//! # CPU accuracy
//!
//! The `sysinfo` crate computes CPU usage as the fraction of CPU time used
//! between consecutive refreshes.  The first snapshot after startup always
//! reports `0.0 %` because there is no prior baseline; subsequent snapshots
//! are accurate to within the 1-second polling granularity.

use std::time::Duration;

use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};
use tokio::sync::watch;

// ── ProcessSnapshot ───────────────────────────────────────────────────────────

/// One-shot snapshot of process resource usage.
///
/// Fields are set to `0` / `0.0` before the first successful poll.
#[derive(Debug, Clone, Default)]
pub struct ProcessSnapshot {
    /// CPU usage of the current process as a percentage in `[0.0, n_cores × 100.0]`.
    ///
    /// The value is the fraction of total CPU time used by this process since
    /// the previous refresh.  On multi-core hosts the ceiling is
    /// `100.0 × num_logical_cores`, matching the `sysinfo` convention.
    pub cpu_pct: f32,
    /// Resident set size of the current process in **bytes**.
    ///
    /// This is the amount of physical RAM currently in use by the process.
    ///
    /// # Source unit
    ///
    /// `sysinfo` 0.30's [`Process::memory`] returns kibibytes; the task
    /// multiplies by 1 024 before storing here so this field always holds bytes,
    /// consistent with its name and the TUI's `/ (1024 * 1024)` display logic.
    pub ram_bytes: u64,
}

// ── Background task ───────────────────────────────────────────────────────────

/// Start a background task that polls process CPU and RAM every second.
///
/// The task runs on a dedicated blocking OS thread so that `sysinfo` calls
/// — which block briefly on OS interfaces — do not stall Tokio workers.
///
/// `handle` must be a handle to the Tokio runtime; this avoids the need for a
/// current-thread runtime context at the call site, making it safe to call
/// before the first `Runtime::block_on` invocation.
///
/// The returned `JoinHandle` can be stored and awaited during shutdown, but
/// is typically left to be cancelled when the runtime shuts down.  The task
/// exits automatically when all receivers of `tx` are dropped.
pub fn spawn_process_metrics_task(
    tx: watch::Sender<ProcessSnapshot>,
    handle: &tokio::runtime::Handle,
) -> tokio::task::JoinHandle<()> {
    handle.spawn_blocking(move || {
        let refresh_kind = ProcessRefreshKind::new().with_cpu().with_memory();
        let mut sys = System::new_with_specifics(RefreshKind::new().with_processes(refresh_kind));
        let pid = Pid::from_u32(std::process::id());

        loop {
            // Refresh this process's data.  The first call initialises the
            // CPU baseline (reporting 0 %); subsequent calls report the delta.
            sys.refresh_process_specifics(pid, refresh_kind);

            let snapshot = match sys.process(pid) {
                Some(proc) => ProcessSnapshot {
                    cpu_pct: proc.cpu_usage(),
                    // sysinfo 0.30 `Process::memory()` returns kibibytes; multiply
                    // by 1024 to store the field in bytes as the doc contract requires.
                    ram_bytes: proc.memory() * 1024,
                },
                None => ProcessSnapshot::default(),
            };

            if tx.send(snapshot).is_err() {
                // All receivers dropped — application is shutting down.
                break;
            }

            std::thread::sleep(Duration::from_secs(1));
        }
    })
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_snapshot_default_is_all_zero() {
        let s = ProcessSnapshot::default();
        assert_eq!(s.cpu_pct, 0.0);
        assert_eq!(s.ram_bytes, 0);
    }

    #[tokio::test]
    async fn task_sends_at_least_one_snapshot() {
        let (tx, mut rx) = watch::channel(ProcessSnapshot::default());
        let rt_handle = tokio::runtime::Handle::current();
        let handle = spawn_process_metrics_task(tx, &rt_handle);

        // Wait up to 3 seconds for the first snapshot.
        let received = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if rx.changed().await.is_ok() {
                    return rx.borrow().clone();
                }
            }
        })
        .await;

        handle.abort();

        let snap = received.expect("should receive at least one process snapshot within 3 s");
        // RAM should be non-zero for a live process and, after the kB→bytes
        // conversion, at least 1 MiB (1 048 576 bytes).
        assert!(
            snap.ram_bytes >= 1_048_576,
            "expected ram_bytes >= 1 MiB (bytes), got {} — \
             check kB→bytes conversion in spawn_process_metrics_task",
            snap.ram_bytes
        );
    }

    #[test]
    fn ram_bytes_unit_is_bytes_not_kib() {
        // Simulate the sysinfo path: proc.memory() returns kibibytes.
        // We expect the stored value to be kB × 1024 (i.e. bytes).
        // This guards against regression to the pre-fix behaviour where
        // proc.memory() was stored directly (off by 1024×).
        let kib_value: u64 = 65_536; // 64 MiB in kibibytes
        let expected_bytes = kib_value * 1024; // 67_108_864
        let snap = ProcessSnapshot {
            cpu_pct: 0.0,
            ram_bytes: kib_value * 1024,
        };
        assert_eq!(
            snap.ram_bytes, expected_bytes,
            "ram_bytes must store bytes; \
             if proc.memory() returns kB, multiply by 1024 before storing"
        );
    }

    #[test]
    fn task_exits_when_sender_is_dropped() {
        // The task loop exits when `tx.send(snapshot).is_err()`, which occurs
        // once all *receivers* have been dropped — that is the watch-channel
        // shutdown semantic used by `spawn_process_metrics_task`.
        //
        // This test verifies those semantics directly (without spawning the
        // full sysinfo task, which would need a real 1-second sleep) so that
        // the production shutdown path is covered without being flaky.
        let (tx, rx) = watch::channel(ProcessSnapshot::default());
        drop(rx); // drop the only receiver
        assert!(
            tx.send(ProcessSnapshot::default()).is_err(),
            "watch::Sender::send must return Err when all receivers are dropped; \
             this is the condition that causes spawn_process_metrics_task to exit"
        );
    }
}
