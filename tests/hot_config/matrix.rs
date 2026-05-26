use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::Serialize;

use crate::config::provider_supervisor::{ProviderBundle, SupervisorOutcome};
use crate::config::{
    classify_capture_change, classify_recorder_change, AppConfig, CaptureChangeOutcome,
    RecorderChangeOutcome,
};

#[path = "matrix_cases.rs"]
mod cases;
use cases::matrix_cases;

const ISSUE: &str = "#391";
const REPORT_PATH: &str = "verification-evidence/hot-config/HC-06-hot-config-matrix-report.json";
pub(super) const SECRET_KEY: &str = "AIzaSyTEST_SECRET_HC06";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum OutcomeKind {
    Unchanged,
    NeedsCaptureHotSwap,
    NeedsPathSwitch,
    NeedsOrchestratorRestart,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Scenario {
    ApplyOk,
    RestartRequired,
    CaptureHotSwap,
    RecorderPathSwitch,
    Rollback,
    CombinedRestartWins,
    Unchanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct Expected {
    pub(super) requires_restart: bool,
    pub(super) requires_restart_ignoring_capture: bool,
    pub(super) requires_capture_hot_swap: bool,
    pub(super) capture: OutcomeKind,
    pub(super) recorder: OutcomeKind,
    pub(super) provider: OutcomeKind,
    pub(super) validate_ok: bool,
}

#[derive(Debug)]
pub(super) struct CaseSpec {
    pub(super) id: &'static str,
    pub(super) fields: &'static [&'static str],
    pub(super) scenario: Scenario,
    pub(super) expected: Expected,
    pub(super) build: fn() -> (AppConfig, AppConfig),
    pub(super) reason_contains: &'static [&'static str],
    pub(super) secret_must_not_appear: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct CaseReport {
    id: &'static str,
    fields: &'static [&'static str],
    scenario: Scenario,
    expected: Expected,
    actual: Expected,
    status: &'static str,
    failures: Vec<String>,
    reasons: Vec<String>,
}

#[derive(Debug, Serialize)]
struct MatrixReport {
    schema_version: u8,
    issue: &'static str,
    status: &'static str,
    case_count: usize,
    required_fields: Vec<&'static str>,
    covered_fields: Vec<String>,
    missing_fields: Vec<String>,
    cases: Vec<CaseReport>,
}

pub(super) const HOT: Expected = Expected {
    requires_restart: false,
    requires_restart_ignoring_capture: false,
    requires_capture_hot_swap: false,
    capture: OutcomeKind::Unchanged,
    recorder: OutcomeKind::Unchanged,
    provider: OutcomeKind::Unchanged,
    validate_ok: true,
};

pub(super) const RESTART: Expected = Expected {
    requires_restart: true,
    requires_restart_ignoring_capture: true,
    requires_capture_hot_swap: false,
    capture: OutcomeKind::Unchanged,
    recorder: OutcomeKind::Unchanged,
    provider: OutcomeKind::Unchanged,
    validate_ok: true,
};

pub(super) const PROVIDER_RESTART: Expected = Expected {
    provider: OutcomeKind::NeedsOrchestratorRestart,
    ..RESTART
};

pub(super) const PROVIDER_REJECTED: Expected = Expected {
    provider: OutcomeKind::Rejected,
    validate_ok: false,
    ..RESTART
};

pub(super) const CAPTURE_HOT_SWAP: Expected = Expected {
    requires_restart: false,
    requires_restart_ignoring_capture: false,
    requires_capture_hot_swap: true,
    capture: OutcomeKind::NeedsCaptureHotSwap,
    recorder: OutcomeKind::Unchanged,
    provider: OutcomeKind::Unchanged,
    validate_ok: true,
};

pub(super) const CAPTURE_REJECTED: Expected = Expected {
    requires_restart: false,
    requires_restart_ignoring_capture: false,
    requires_capture_hot_swap: true,
    capture: OutcomeKind::Rejected,
    recorder: OutcomeKind::Unchanged,
    provider: OutcomeKind::Unchanged,
    validate_ok: false,
};

pub(super) const RECORDER_PATH_SWITCH: Expected = Expected {
    recorder: OutcomeKind::NeedsPathSwitch,
    ..RESTART
};

pub(super) const RECORDER_REJECTED: Expected = Expected {
    recorder: OutcomeKind::Rejected,
    validate_ok: false,
    ..RESTART
};

pub(super) const COMBINED_RESTART_CAPTURE: Expected = Expected {
    requires_restart: true,
    requires_restart_ignoring_capture: true,
    requires_capture_hot_swap: true,
    capture: OutcomeKind::NeedsCaptureHotSwap,
    recorder: OutcomeKind::Unchanged,
    provider: OutcomeKind::NeedsOrchestratorRestart,
    validate_ok: true,
};

pub(super) const AUDIO_FILE_PATH_WASAPI_DIVERGENCE: Expected = Expected {
    requires_restart: false,
    requires_restart_ignoring_capture: false,
    requires_capture_hot_swap: false,
    capture: OutcomeKind::NeedsCaptureHotSwap,
    recorder: OutcomeKind::Unchanged,
    provider: OutcomeKind::Unchanged,
    validate_ok: true,
};

const REQUIRED_FIELDS: &[&str] = &[
    "source_language",
    "target_language",
    "google_api_key",
    "tts_enabled",
    "tts_output_device",
    "tts_routing",
    "tts_source",
    "virtual_mic_device",
    "virtual_device_patterns",
    "capture_device",
    "stt_provider",
    "mt_provider",
    "mt_cloud_fallback",
    "stt_fallback_policy",
    "audio_source",
    "audio_file_path",
    "cost_warning_usd",
    "vad.enabled",
    "vad.threshold",
    "vad.min_speech_ms",
    "vad.speech_pad_ms",
    "vad.min_silence_ms",
    "vad.pre_roll_ms",
    "stt_phrase_hints",
    "session_store.enabled",
    "session_store.directory",
    "session_store.max_sessions",
    "session_store.per_session_bytes_cap",
    "session_store.total_bytes_cap",
    "session_store.retention_days",
    "_comment",
    "cpu_budget_pct",
    "ram_budget_mb",
    "pipeline.max_window_ms",
    "pipeline.early_flush_on_vad_end",
    "pipeline.idle_flush_ms",
    "pipeline.idle_min_ms",
    "pipeline.sentence_max_age_ms",
    "audio_archive.store_audio",
    "audio_archive.consent_given",
    "audio_archive.directory",
    "audio_archive.max_size_mb",
    "audio_archive.total_bytes_cap",
    "audio_archive.retention_days",
    "slots",
    "slots.slot_a.stt_provider",
    "slots.slot_a.mt_provider",
    "slots.slot_a.target_language",
    "slots.slot_b.stt_provider",
    "slots.slot_b.mt_provider",
    "slots.slot_b.target_language",
];

#[test]
fn hc06_hot_config_matrix_covers_all_classified_fields() {
    let cases = matrix_cases();
    let mut reports = Vec::with_capacity(cases.len());
    let mut covered = BTreeSet::new();

    for case in &cases {
        for field in case.fields {
            covered.insert((*field).to_string());
        }
        reports.push(evaluate_case(case));
    }

    let missing_fields = REQUIRED_FIELDS
        .iter()
        .filter(|field| !covered.contains(**field))
        .map(|field| (*field).to_string())
        .collect::<Vec<_>>();

    let mut status = "pass";
    if !missing_fields.is_empty() || reports.iter().any(|case| case.status != "pass") {
        status = "fail";
    }

    let report = MatrixReport {
        schema_version: 1,
        issue: ISSUE,
        status,
        case_count: reports.len(),
        required_fields: REQUIRED_FIELDS.to_vec(),
        covered_fields: covered.into_iter().collect(),
        missing_fields,
        cases: reports,
    };
    write_report(&report);

    assert_eq!(
        report.status, "pass",
        "HC-06 hot-config matrix failed; see {REPORT_PATH}"
    );
}

fn evaluate_case(case: &CaseSpec) -> CaseReport {
    let (old, new) = (case.build)();
    let capture = classify_capture_change(&old, &new);
    let recorder = classify_recorder_change(&old, &new);
    let provider = ProviderBundle::from_config(&old).evaluate_change(&new);
    let validation = new.validate();

    let mut reasons = Vec::new();
    collect_capture_reason(&capture, &mut reasons);
    collect_recorder_reason(&recorder, &mut reasons);
    collect_provider_reason(&provider, &mut reasons);
    if let Err(err) = &validation {
        reasons.push(err.to_string());
    }

    let actual = Expected {
        requires_restart: old.requires_restart(&new),
        requires_restart_ignoring_capture: old.requires_restart_ignoring_capture(&new),
        requires_capture_hot_swap: old.requires_capture_hot_swap(&new),
        capture: capture_kind(&capture),
        recorder: recorder_kind(&recorder),
        provider: provider_kind(&provider),
        validate_ok: validation.is_ok(),
    };

    let mut failures = Vec::new();
    if actual != case.expected {
        failures.push(format!("expected {:?}, got {:?}", case.expected, actual));
    }
    for needle in case.reason_contains {
        if !reasons.iter().any(|reason| reason.contains(needle)) {
            failures.push(format!("no reason contained {needle:?}"));
        }
    }
    if let Some(secret) = case.secret_must_not_appear {
        if reasons.iter().any(|reason| reason.contains(secret)) {
            failures.push("secret appeared in a surfaced reason".to_string());
        }
    }

    CaseReport {
        id: case.id,
        fields: case.fields,
        scenario: case.scenario,
        expected: case.expected,
        actual,
        status: if failures.is_empty() { "pass" } else { "fail" },
        failures,
        reasons,
    }
}

fn capture_kind(outcome: &CaptureChangeOutcome) -> OutcomeKind {
    match outcome {
        CaptureChangeOutcome::Unchanged => OutcomeKind::Unchanged,
        CaptureChangeOutcome::NeedsCaptureHotSwap { .. } => OutcomeKind::NeedsCaptureHotSwap,
        CaptureChangeOutcome::Rejected { .. } => OutcomeKind::Rejected,
    }
}

fn recorder_kind(outcome: &RecorderChangeOutcome) -> OutcomeKind {
    match outcome {
        RecorderChangeOutcome::Unchanged => OutcomeKind::Unchanged,
        RecorderChangeOutcome::NeedsPathSwitch { .. } => OutcomeKind::NeedsPathSwitch,
        RecorderChangeOutcome::Rejected { .. } => OutcomeKind::Rejected,
    }
}

fn provider_kind(outcome: &SupervisorOutcome) -> OutcomeKind {
    match outcome {
        SupervisorOutcome::Unchanged => OutcomeKind::Unchanged,
        SupervisorOutcome::NeedsOrchestratorRestart { .. } => OutcomeKind::NeedsOrchestratorRestart,
        SupervisorOutcome::Rejected { .. } => OutcomeKind::Rejected,
    }
}

fn collect_capture_reason(outcome: &CaptureChangeOutcome, reasons: &mut Vec<String>) {
    match outcome {
        CaptureChangeOutcome::NeedsCaptureHotSwap { reason, .. }
        | CaptureChangeOutcome::Rejected { reason } => reasons.push(reason.clone()),
        CaptureChangeOutcome::Unchanged => {}
    }
}

fn collect_recorder_reason(outcome: &RecorderChangeOutcome, reasons: &mut Vec<String>) {
    match outcome {
        RecorderChangeOutcome::NeedsPathSwitch { reason }
        | RecorderChangeOutcome::Rejected { reason } => reasons.push(reason.clone()),
        RecorderChangeOutcome::Unchanged => {}
    }
}

fn collect_provider_reason(outcome: &SupervisorOutcome, reasons: &mut Vec<String>) {
    match outcome {
        SupervisorOutcome::NeedsOrchestratorRestart { reason }
        | SupervisorOutcome::Rejected { reason } => reasons.push(reason.clone()),
        SupervisorOutcome::Unchanged => {}
    }
}

fn write_report(report: &MatrixReport) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(REPORT_PATH);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create HC-06 evidence directory");
    }
    let json = serde_json::to_string_pretty(report).expect("serialize HC-06 matrix report");
    std::fs::write(path, json).expect("write HC-06 matrix report");
}
