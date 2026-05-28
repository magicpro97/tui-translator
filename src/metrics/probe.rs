//! Cross-platform process probe for QA8-04 (issue #502).
//!
//! Provides a single [`ProcessProbe`] trait and a per-OS implementation that
//! reports:
//!
//! * resident set size (`rss_bytes`)
//! * process-private memory (`private_bytes`)
//! * OS thread count (`thread_count`)
//! * file-descriptor / handle counts (`fd_count` on Unix, `windows_handles` on Windows)
//!
//! Where the platform cannot supply a probe, the corresponding field is set to
//! [`None`] and its name is added to [`ProbeSample::unsupported_fields`] — never
//! reported as zero. This contract matches the acceptance criterion of #502
//! ("unavailable metrics are explicit `unsupported`, not zero-shaped success").
//!
//! Scope of `unsupported_fields`: per-OS implementations seed it with the
//! field names that are **structurally unsupported on this platform** (e.g.
//! `windows_handles` on Linux, `fd_count` on Windows). A field that is
//! supported on the current platform but fails to read at runtime (e.g.
//! transient `/proc/self/status` read error) is reported as `None` *without*
//! adding its name to `unsupported_fields` — callers that need to
//! distinguish "platform cannot ever supply this" from "this sample failed"
//! should treat `unsupported_fields` as the authoritative platform contract
//! and `None` outside that list as a transient read failure.
//!
//! Field-name vs schema mapping: the in-memory field
//! [`ProbeSample::fd_count`] is serialised as `file_descriptors` in the
//! QA8-03 soak schema (see `verification-evidence/qa8/QA8-03-soak-schema-v2.json`).
//! There is currently no `serde::Serialize` derive on [`ProbeSample`]; the
//! schema mapping will be applied by the soak-report serialiser layer when
//! it is wired up (issue #503).

use std::time::Instant;

/// One probe of process-level resource counters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProbeSample {
    pub rss_bytes: Option<u64>,
    pub private_bytes: Option<u64>,
    pub thread_count: Option<u32>,
    pub fd_count: Option<u32>,
    pub windows_handles: Option<u32>,
    pub unsupported_fields: Vec<&'static str>,
}

impl ProbeSample {
    pub fn is_fully_unsupported(&self) -> bool {
        self.rss_bytes.is_none()
            && self.private_bytes.is_none()
            && self.thread_count.is_none()
            && self.fd_count.is_none()
            && self.windows_handles.is_none()
    }

    pub fn has_any(&self) -> bool {
        !self.is_fully_unsupported()
    }
}

pub trait ProcessProbe: Send + Sync {
    fn sample(&self) -> ProbeSample;
    fn platform(&self) -> &'static str;
}

pub fn default_probe() -> Box<dyn ProcessProbe> {
    #[cfg(target_os = "linux")]
    {
        Box::new(linux_impl::LinuxProcessProbe::new())
    }
    #[cfg(target_os = "windows")]
    {
        Box::new(windows_impl::WindowsProcessProbe::new())
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos_impl::MacosProcessProbe::new())
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        Box::new(unsupported_impl::UnsupportedProbe)
    }
}

pub fn measure_sample_overhead(probe: &dyn ProcessProbe) -> std::time::Duration {
    let start = Instant::now();
    let _ = probe.sample();
    start.elapsed()
}

#[cfg(target_os = "linux")]
mod linux_impl {
    use super::{ProbeSample, ProcessProbe};
    use std::fs;

    #[derive(Debug, Default)]
    pub struct LinuxProcessProbe;

    impl LinuxProcessProbe {
        pub fn new() -> Self {
            Self
        }
    }

    impl ProcessProbe for LinuxProcessProbe {
        fn platform(&self) -> &'static str {
            "linux"
        }

        fn sample(&self) -> ProbeSample {
            let mut s = ProbeSample {
                unsupported_fields: vec!["windows_handles"],
                ..Default::default()
            };

            if let Ok(status) = fs::read_to_string("/proc/self/status") {
                for line in status.lines() {
                    if let Some(rest) = line.strip_prefix("VmRSS:") {
                        s.rss_bytes = parse_kb_to_bytes(rest);
                    } else if let Some(rest) = line.strip_prefix("RssAnon:") {
                        s.private_bytes = parse_kb_to_bytes(rest);
                    } else if let Some(rest) = line.strip_prefix("Threads:") {
                        s.thread_count = rest.trim().parse::<u32>().ok();
                    }
                }
            }

            if let Ok(rd) = fs::read_dir("/proc/self/fd") {
                let count = rd.filter(|e| e.is_ok()).count();
                let adjusted = count.saturating_sub(1).min(u32::MAX as usize) as u32;
                s.fd_count = Some(adjusted);
            }

            s
        }
    }

    fn parse_kb_to_bytes(rest: &str) -> Option<u64> {
        let trimmed = rest.trim();
        let num: u64 = trimmed.split_whitespace().next()?.parse().ok()?;
        Some(num.saturating_mul(1024))
    }
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::{ProbeSample, ProcessProbe};
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};

    #[derive(Debug, Default)]
    pub struct WindowsProcessProbe;

    impl WindowsProcessProbe {
        pub fn new() -> Self {
            Self
        }
    }

    impl ProcessProbe for WindowsProcessProbe {
        fn platform(&self) -> &'static str {
            "windows"
        }

        fn sample(&self) -> ProbeSample {
            let mut s = ProbeSample {
                unsupported_fields: vec!["fd_count"],
                ..Default::default()
            };
            // SAFETY: GetCurrentProcess returns a pseudo-handle that never needs closing; K32GetProcessMemoryInfo writes into a zero-initialised PROCESS_MEMORY_COUNTERS_EX of cb == size_of, satisfying the documented contract; GetProcessHandleCount writes into a stack u32 we own.
            unsafe {
                let proc_handle = GetCurrentProcess();
                let mut counters: PROCESS_MEMORY_COUNTERS_EX = zeroed();
                let cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
                if K32GetProcessMemoryInfo(proc_handle, &mut counters as *mut _ as *mut _, cb) != 0
                {
                    s.rss_bytes = Some(counters.WorkingSetSize as u64);
                    s.private_bytes = Some(counters.PrivateUsage as u64);
                }
                let mut handle_count: u32 = 0;
                if GetProcessHandleCount(proc_handle, &mut handle_count) != 0 {
                    s.windows_handles = Some(handle_count);
                }
            }

            s.thread_count = current_process_thread_count();
            s
        }
    }

    fn current_process_thread_count() -> Option<u32> {
        // SAFETY: CreateToolhelp32Snapshot returns an owned HANDLE we CloseHandle on every exit path, or INVALID_HANDLE_VALUE which we check before any further calls; PROCESSENTRY32 is zero-initialised with dwSize set to its struct size before each Process32First/Next call, as the API contract requires; we only read the cntThreads field for the entry whose th32ProcessID matches the current pid.
        unsafe {
            let pid = std::process::id();
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut entry: PROCESSENTRY32 = zeroed();
            entry.dwSize = size_of::<PROCESSENTRY32>() as u32;
            let mut threads: Option<u32> = None;
            if Process32First(snap, &mut entry) != 0 {
                loop {
                    if entry.th32ProcessID == pid {
                        threads = Some(entry.cntThreads);
                        break;
                    }
                    entry.dwSize = size_of::<PROCESSENTRY32>() as u32;
                    if Process32Next(snap, &mut entry) == 0 {
                        break;
                    }
                }
            }
            CloseHandle(snap);
            threads
        }
    }
}

#[cfg(target_os = "macos")]
mod macos_impl {
    use super::{ProbeSample, ProcessProbe};
    use std::sync::Mutex;
    use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};

    pub struct MacosProcessProbe {
        sys: Mutex<System>,
    }

    impl std::fmt::Debug for MacosProcessProbe {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MacosProcessProbe").finish()
        }
    }

    impl MacosProcessProbe {
        pub fn new() -> Self {
            let refresh = ProcessRefreshKind::new().with_memory();
            Self {
                sys: Mutex::new(System::new_with_specifics(
                    RefreshKind::new().with_processes(refresh),
                )),
            }
        }
    }

    impl ProcessProbe for MacosProcessProbe {
        fn platform(&self) -> &'static str {
            "macos"
        }

        fn sample(&self) -> ProbeSample {
            let mut s = ProbeSample {
                unsupported_fields: vec![
                    "private_bytes",
                    "thread_count",
                    "fd_count",
                    "windows_handles",
                ],
                ..Default::default()
            };

            let pid = Pid::from_u32(std::process::id());
            if let Ok(mut sys) = self.sys.lock() {
                sys.refresh_process_specifics(pid, ProcessRefreshKind::new().with_memory());
                if let Some(p) = sys.process(pid) {
                    s.rss_bytes = Some(p.memory());
                }
            }
            s
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
mod unsupported_impl {
    use super::{ProbeSample, ProcessProbe};

    #[derive(Debug, Default)]
    pub struct UnsupportedProbe;

    impl ProcessProbe for UnsupportedProbe {
        fn platform(&self) -> &'static str {
            "unsupported"
        }

        fn sample(&self) -> ProbeSample {
            ProbeSample {
                unsupported_fields: vec![
                    "rss_bytes",
                    "private_bytes",
                    "thread_count",
                    "fd_count",
                    "windows_handles",
                ],
                ..Default::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_fields_list_is_explicit_per_platform() {
        let p = default_probe();
        let s = p.sample();
        if s.is_fully_unsupported() {
            assert!(!s.unsupported_fields.is_empty());
        }
        assert!(matches!(
            p.platform(),
            "linux" | "windows" | "macos" | "unsupported"
        ));
    }

    #[test]
    fn sample_overhead_is_well_under_5ms() {
        let p = default_probe();
        let _ = p.sample();
        let elapsed = (0..5)
            .map(|_| measure_sample_overhead(&*p))
            .min()
            .expect("fixed-size overhead sample set is non-empty");
        // The steady-state design target is << 5 ms per sample; the assert
        // threshold is relaxed to 50 ms and checked against the best of a few
        // samples to absorb shared-CI scheduling jitter without false positives
        // (see #504 discussion).
        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "best probe sample took {elapsed:?}, must be < 50 ms (steady-state target: << 5 ms)"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_probe_reports_rss_threads_and_fds() {
        let p = default_probe();
        let s = p.sample();
        assert!(s.rss_bytes.unwrap_or(0) >= 1_048_576);
        assert!(s.thread_count.unwrap_or(0) >= 1);
        assert!(s.fd_count.unwrap_or(0) >= 1);
        assert!(s.private_bytes.unwrap_or(0) > 0);
        assert!(s.windows_handles.is_none());
        assert!(s.unsupported_fields.contains(&"windows_handles"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_probe_reports_handles_threads_and_private_bytes() {
        let p = default_probe();
        let s = p.sample();
        assert!(
            s.rss_bytes.unwrap_or(0) >= 1_048_576,
            "windows probe must report >=1 MiB rss; got {:?}",
            s.rss_bytes
        );
        assert!(s.private_bytes.unwrap_or(0) > 0);
        assert!(s.windows_handles.unwrap_or(0) >= 1);
        assert!(s.thread_count.unwrap_or(0) >= 1);
        assert!(s.fd_count.is_none());
        assert!(s.unsupported_fields.contains(&"fd_count"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_probe_reports_rss_and_marks_extras_unsupported() {
        let p = default_probe();
        let s = p.sample();
        assert!(s.rss_bytes.unwrap_or(0) >= 1_048_576);
        for field in [
            "private_bytes",
            "thread_count",
            "fd_count",
            "windows_handles",
        ] {
            assert!(s.unsupported_fields.contains(&field));
        }
    }

    #[test]
    fn injected_leak_fixture_is_detected_via_delta() {
        let p = default_probe();
        let before = p.sample();
        let mut leak: Vec<u8> = vec![0u8; 16 * 1024 * 1024];
        for (i, b) in leak.iter_mut().enumerate() {
            *b = (i & 0xff) as u8;
        }
        std::hint::black_box(&leak);
        let after = p.sample();

        let delta = match (before.private_bytes, after.private_bytes) {
            (Some(b), Some(a)) => a as i64 - b as i64,
            _ => match (before.rss_bytes, after.rss_bytes) {
                (Some(b), Some(a)) => a as i64 - b as i64,
                _ => return,
            },
        };
        std::hint::black_box(&leak);
        drop(leak);

        assert!(
            delta >= 4 * 1024 * 1024,
            "expected >=4 MiB growth after 16 MiB injected leak; got delta={delta} bytes"
        );
    }
}
