//! Deterministic fault-injection DSL for the QA8-05 partial runner v2
//! (issue #503).
//!
//! The DSL is intentionally tiny and line-oriented so that 10-minute /
//! 1-hour / 8-hour soak schedules can be checked into the repository as
//! plain-text fixtures and replayed deterministically:
//!
//! ```text
//! # comment
//! @<t_start_secs> <name> [duration=<secs>] [recovery_ms=<ms>]
//! ```
//!
//! Recognised `<name>` values map to a [`FaultKind`]; any other name is
//! captured as [`FaultKind::Custom`] so future fault categories do not
//! require a parser change. Unknown `key=value` options are rejected so
//! typos do not silently disappear.
//!
//! This module is consumed by `run_soak.rs` (the `--fault-script` flag)
//! and by `schema_v2.rs` (to render the `fault_injection.events` block
//! of the QA8-05 v2 evidence artifact).
//!
//! Scope: parser + data model only. Real fault execution (toggling the
//! Windows Firewall, throttling a provider, swapping the capture device,
//! pinning CPUs) is gated behind administrator privileges and hardware
//! access and remains part of the full #503 closure.

use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

/// Categorised fault kind. The name parsed from the DSL is also retained
/// on [`FaultEvent::name`] so renderers can echo the user-provided
/// string verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    NetworkOutage,
    ProviderRateLimit,
    DeviceHotSwap,
    CpuPressure,
    Custom,
}

impl FaultKind {
    /// Stable string id used in the v2 evidence artifact.
    pub fn as_str(self) -> &'static str {
        match self {
            FaultKind::NetworkOutage => "network_outage",
            FaultKind::ProviderRateLimit => "provider_rate_limit",
            FaultKind::DeviceHotSwap => "device_hot_swap",
            FaultKind::CpuPressure => "cpu_pressure",
            FaultKind::Custom => "custom",
        }
    }

    fn from_name(name: &str) -> FaultKind {
        match name {
            "network_outage" | "network_disconnect" => FaultKind::NetworkOutage,
            "provider_rate_limit" | "provider_429" => FaultKind::ProviderRateLimit,
            "device_hot_swap" | "hot_swap" => FaultKind::DeviceHotSwap,
            "cpu_pressure" => FaultKind::CpuPressure,
            _ => FaultKind::Custom,
        }
    }
}

/// One fault event parsed from the DSL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultEvent {
    /// Elapsed seconds from soak start when the fault is injected.
    pub t_start_secs: u64,
    /// Verbatim name from the DSL (also the kind discriminator).
    pub name: String,
    /// Categorised kind.
    pub kind: FaultKind,
    /// Optional duration in seconds; `None` means a point-in-time event.
    pub duration_secs: Option<u64>,
    /// Optional recovery-budget in milliseconds used by the soak gate.
    pub expected_recovery_ms: Option<u64>,
}

impl FaultEvent {
    /// End time in seconds if a duration was provided.
    pub fn t_end_secs(&self) -> Option<u64> {
        self.duration_secs
            .map(|d| self.t_start_secs.saturating_add(d))
    }
}

/// Parse the DSL from a string. Comments (`#`) and blank lines are
/// skipped. Events are returned sorted by `t_start_secs`.
pub fn parse(input: &str) -> Result<Vec<FaultEvent>> {
    let mut events = Vec::new();
    for (idx, raw) in input.lines().enumerate() {
        let lineno = idx + 1;
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        events.push(parse_line(line, lineno)?);
    }
    events.sort_by_key(|e| e.t_start_secs);
    Ok(events)
}

/// Load and parse a fault-script file from disk.
pub fn load(path: &Path) -> Result<Vec<FaultEvent>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read fault script: {}", path.display()))?;
    parse(&text).with_context(|| format!("invalid fault script at {}", path.display()))
}

fn parse_line(line: &str, lineno: usize) -> Result<FaultEvent> {
    let mut tokens = line.split_whitespace();
    let head = tokens
        .next()
        .ok_or_else(|| anyhow!("line {lineno}: empty after stripping comment"))?;
    let t_start_secs = head
        .strip_prefix('@')
        .ok_or_else(|| anyhow!("line {lineno}: event must begin with '@<t_secs>', got '{head}'"))?
        .parse::<u64>()
        .with_context(|| format!("line {lineno}: '{head}' is not a valid u64 timestamp"))?;

    let name = tokens
        .next()
        .ok_or_else(|| anyhow!("line {lineno}: missing fault name after '{head}'"))?
        .to_string();
    let kind = FaultKind::from_name(&name);

    let mut duration_secs: Option<u64> = None;
    let mut expected_recovery_ms: Option<u64> = None;

    for tok in tokens {
        let (k, v) = tok
            .split_once('=')
            .ok_or_else(|| anyhow!("line {lineno}: option '{tok}' is not key=value"))?;
        match k {
            "duration" => {
                duration_secs = Some(
                    v.parse::<u64>()
                        .with_context(|| format!("line {lineno}: duration '{v}' is not u64"))?,
                );
            }
            "recovery_ms" => {
                expected_recovery_ms = Some(
                    v.parse::<u64>()
                        .with_context(|| format!("line {lineno}: recovery_ms '{v}' is not u64"))?,
                );
            }
            other => {
                bail!("line {lineno}: unknown option '{other}' (allowed: duration, recovery_ms)")
            }
        }
    }

    Ok(FaultEvent {
        t_start_secs,
        name,
        kind,
        duration_secs,
        expected_recovery_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_smoke_schedule() {
        let dsl = "\
# QA8-05 deterministic 10-minute smoke schedule
@60   network_outage       duration=30 recovery_ms=10000
@180  provider_rate_limit  duration=20 recovery_ms=5000
@360  device_hot_swap      duration=5  recovery_ms=2000
@480  cpu_pressure         duration=30
";
        let events = parse(dsl).expect("parse smoke schedule");
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].kind, FaultKind::NetworkOutage);
        assert_eq!(events[0].t_start_secs, 60);
        assert_eq!(events[0].duration_secs, Some(30));
        assert_eq!(events[0].expected_recovery_ms, Some(10_000));
        assert_eq!(events[0].t_end_secs(), Some(90));
        assert_eq!(events[3].kind, FaultKind::CpuPressure);
        assert_eq!(events[3].expected_recovery_ms, None);
    }

    #[test]
    fn sorts_events_by_t_start() {
        let dsl = "@100 cpu_pressure\n@10 network_outage\n@50 device_hot_swap\n";
        let events = parse(dsl).unwrap();
        let ts: Vec<u64> = events.iter().map(|e| e.t_start_secs).collect();
        assert_eq!(ts, vec![10, 50, 100]);
    }

    #[test]
    fn unknown_name_falls_through_to_custom() {
        let events = parse("@1 something_new duration=1").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, FaultKind::Custom);
        assert_eq!(events[0].name, "something_new");
    }

    #[test]
    fn rejects_unknown_option() {
        let err = parse("@1 cpu_pressure foo=bar").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("unknown option"), "got: {msg}");
    }

    #[test]
    fn rejects_missing_at_prefix() {
        let err = parse("60 network_outage").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("must begin with '@"), "got: {msg}");
    }

    #[test]
    fn comments_and_blank_lines_ignored() {
        let events = parse("\n# only comment\n\n  # indented comment\n").unwrap();
        assert!(events.is_empty());
    }
}
