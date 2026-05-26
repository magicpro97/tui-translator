//! Unit tests for `schema_v2` (extracted from `schema_v2.rs` for the
//! STD-02 module-budget refactor; behavior unchanged).

use super::super::fault_script;
use super::*;

fn smoke_dsl() -> &'static str {
    "\
@60   network_outage       duration=30 recovery_ms=10000
@180  provider_rate_limit  duration=20 recovery_ms=5000
@360  device_hot_swap      duration=5  recovery_ms=2000
@480  cpu_pressure         duration=30
"
}

fn smoke_cfg<'a>(events: &'a [FaultEvent], path: Option<&'a Path>) -> SmokeConfig<'a> {
    SmokeConfig {
        run_id: "smoke-run".to_string(),
        started_at_utc: "2025-01-01T00:00:00Z".to_string(),
        finished_at_utc: "2025-01-01T00:10:00Z".to_string(),
        duration_secs: 600,
        sample_interval_secs: 30,
        window_secs: 60,
        fault_script_path: path,
        fault_events: events,
        crash_watch_enabled: true,
    }
}

#[test]
fn smoke_report_invariants() {
    let events = fault_script::parse(smoke_dsl()).unwrap();
    let report = build_smoke_report(smoke_cfg(&events, None)).unwrap();
    validate_invariants(&report).unwrap();
    assert!(report.smoke);
    assert!(report.synthetic);
    assert_eq!(report.schema_version, SCHEMA_VERSION);
    assert_eq!(report.run_metadata.issue, "#503");
    assert!(report.run_metadata.partial);
}

#[test]
fn smoke_windows_cover_full_duration() {
    let events = fault_script::parse(smoke_dsl()).unwrap();
    let report = build_smoke_report(smoke_cfg(&events, None)).unwrap();
    assert_eq!(report.sample_windows.len(), 10, "600s / 60s = 10 windows");
    assert_eq!(report.sample_windows[0].t_start_secs, 0);
    assert_eq!(report.sample_windows.last().unwrap().t_end_secs, 600);
    // First fault is at 60s → must mark window 1 (60..120).
    assert!(
        report.sample_windows[1].contains_fault,
        "60s window must contain network_outage fault"
    );
}

#[test]
fn faults_mark_recovery_within_budget_in_smoke() {
    let events = fault_script::parse(smoke_dsl()).unwrap();
    let report = build_smoke_report(smoke_cfg(&events, None)).unwrap();
    let with_budget: Vec<_> = report
        .fault_injection
        .events
        .iter()
        .filter(|e| e.expected_recovery_ms.is_some())
        .collect();
    assert!(!with_budget.is_empty());
    for e in with_budget {
        assert_eq!(e.recovered_within_budget, Some(true));
        assert_eq!(e.observed_recovery_ms, e.expected_recovery_ms);
        assert!(e.simulated);
    }
}

#[test]
fn backpressure_snapshots_declare_qa807_schema() {
    let events = fault_script::parse(smoke_dsl()).unwrap();
    let report = build_smoke_report(smoke_cfg(&events, None)).unwrap();
    for s in &report.backpressure_snapshots {
        assert_eq!(s["schema_version"], QA8_07_SCHEMA_VERSION);
        assert_eq!(s["ok"], true);
        assert_eq!(s["provenance"]["synthetic"], true);
    }
}

#[test]
fn blocked_gates_include_eight_hour_gate() {
    let report = build_smoke_report(smoke_cfg(&[], None)).unwrap();
    let gates: Vec<&str> = report
        .blocked_gates
        .iter()
        .map(|g| g.gate.as_str())
        .collect();
    assert!(gates.contains(&"8h_green_soak"));
}

#[test]
fn validate_rejects_wrong_schema_version() {
    let mut report = build_smoke_report(smoke_cfg(&[], None)).unwrap();
    report.schema_version = "qa8-05.v0".to_string();
    assert!(validate_invariants(&report).is_err());
}

fn fault_record(name: &str, kind: &str, t_start: u64) -> FaultEventRecord {
    FaultEventRecord {
        name: name.to_string(),
        kind: kind.to_string(),
        t_start_secs: t_start,
        t_end_secs: None,
        expected_recovery_ms: None,
        observed_recovery_ms: None,
        recovered_within_budget: None,
        simulated: true,
    }
}

#[test]
fn validate_rejects_duplicate_non_custom_names() {
    let mut report = build_smoke_report(smoke_cfg(&[], None)).unwrap();
    report.fault_injection.enabled = true;
    report.fault_injection.events = vec![
        fault_record("network_outage", "network_outage", 60),
        fault_record("network_outage", "network_outage", 180),
    ];
    let err =
        validate_invariants(&report).expect_err("duplicate non-custom names must be rejected");
    assert!(
        err.to_string().contains("duplicate fault event name"),
        "unexpected error: {err}"
    );
}

#[test]
fn validate_allows_duplicate_custom_names() {
    let mut report = build_smoke_report(smoke_cfg(&[], None)).unwrap();
    report.fault_injection.enabled = true;
    report.fault_injection.events = vec![
        fault_record("my_custom_fault", "custom", 60),
        fault_record("my_custom_fault", "custom", 180),
    ];
    validate_invariants(&report).expect("duplicate custom-kind events must be permitted");
}

#[test]
fn validate_accepts_unique_non_custom_names() {
    let mut report = build_smoke_report(smoke_cfg(&[], None)).unwrap();
    report.fault_injection.enabled = true;
    report.fault_injection.events = vec![
        fault_record("network_outage", "network_outage", 60),
        fault_record("provider_rate_limit", "provider_rate_limit", 180),
    ];
    validate_invariants(&report).expect("unique non-custom events must pass validation");
}
