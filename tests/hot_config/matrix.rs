use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::Serialize;
use serde_json::json;

use crate::audio::{VirtualDeviceKind, VirtualDevicePatternConfig};
use crate::config::provider_supervisor::{ProviderBundle, SupervisorOutcome};
use crate::config::{
    classify_capture_change, classify_recorder_change, AppConfig, CaptureChangeOutcome,
    DualSlotConfig, RecorderChangeOutcome, SlotConfig, TtsRouting, TtsSource,
};

const ISSUE: &str = "#391";
const REPORT_PATH: &str = "verification-evidence/hot-config/HC-06-hot-config-matrix-report.json";
const SECRET_KEY: &str = "AIzaSyTEST_SECRET_HC06";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum OutcomeKind {
    Unchanged,
    NeedsCaptureHotSwap,
    NeedsPathSwitch,
    NeedsOrchestratorRestart,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Scenario {
    ApplyOk,
    RestartRequired,
    CaptureHotSwap,
    RecorderPathSwitch,
    Rollback,
    CombinedRestartWins,
    Unchanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct Expected {
    requires_restart: bool,
    requires_restart_ignoring_capture: bool,
    requires_capture_hot_swap: bool,
    capture: OutcomeKind,
    recorder: OutcomeKind,
    provider: OutcomeKind,
    validate_ok: bool,
}

#[derive(Debug)]
struct CaseSpec {
    id: &'static str,
    fields: &'static [&'static str],
    scenario: Scenario,
    expected: Expected,
    build: fn() -> (AppConfig, AppConfig),
    reason_contains: &'static [&'static str],
    secret_must_not_appear: Option<&'static str>,
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

const HOT: Expected = Expected {
    requires_restart: false,
    requires_restart_ignoring_capture: false,
    requires_capture_hot_swap: false,
    capture: OutcomeKind::Unchanged,
    recorder: OutcomeKind::Unchanged,
    provider: OutcomeKind::Unchanged,
    validate_ok: true,
};

const RESTART: Expected = Expected {
    requires_restart: true,
    requires_restart_ignoring_capture: true,
    requires_capture_hot_swap: false,
    capture: OutcomeKind::Unchanged,
    recorder: OutcomeKind::Unchanged,
    provider: OutcomeKind::Unchanged,
    validate_ok: true,
};

const PROVIDER_RESTART: Expected = Expected {
    provider: OutcomeKind::NeedsOrchestratorRestart,
    ..RESTART
};

const PROVIDER_REJECTED: Expected = Expected {
    provider: OutcomeKind::Rejected,
    validate_ok: false,
    ..RESTART
};

const CAPTURE_HOT_SWAP: Expected = Expected {
    requires_restart: false,
    requires_restart_ignoring_capture: false,
    requires_capture_hot_swap: true,
    capture: OutcomeKind::NeedsCaptureHotSwap,
    recorder: OutcomeKind::Unchanged,
    provider: OutcomeKind::Unchanged,
    validate_ok: true,
};

const CAPTURE_REJECTED: Expected = Expected {
    requires_restart: false,
    requires_restart_ignoring_capture: false,
    requires_capture_hot_swap: true,
    capture: OutcomeKind::Rejected,
    recorder: OutcomeKind::Unchanged,
    provider: OutcomeKind::Unchanged,
    validate_ok: false,
};

const RECORDER_PATH_SWITCH: Expected = Expected {
    recorder: OutcomeKind::NeedsPathSwitch,
    ..RESTART
};

const RECORDER_REJECTED: Expected = Expected {
    recorder: OutcomeKind::Rejected,
    validate_ok: false,
    ..RESTART
};

const COMBINED_RESTART_CAPTURE: Expected = Expected {
    requires_restart: true,
    requires_restart_ignoring_capture: true,
    requires_capture_hot_swap: true,
    capture: OutcomeKind::NeedsCaptureHotSwap,
    recorder: OutcomeKind::Unchanged,
    provider: OutcomeKind::NeedsOrchestratorRestart,
    validate_ok: true,
};

const AUDIO_FILE_PATH_WASAPI_DIVERGENCE: Expected = Expected {
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

fn matrix_cases() -> Vec<CaseSpec> {
    vec![
        spec(
            "source_language_hot",
            &["source_language"],
            Scenario::ApplyOk,
            HOT,
            || changed(|cfg| cfg.source_language = "en-US".to_string()),
        ),
        spec(
            "target_language_hot",
            &["target_language"],
            Scenario::ApplyOk,
            HOT,
            || changed(|cfg| cfg.target_language = "en".to_string()),
        ),
        spec(
            "tts_enabled_hot",
            &["tts_enabled"],
            Scenario::ApplyOk,
            HOT,
            || changed(|cfg| cfg.tts_enabled = true),
        ),
        spec(
            "cost_warning_hot",
            &["cost_warning_usd"],
            Scenario::ApplyOk,
            HOT,
            || changed(|cfg| cfg.cost_warning_usd = 1.25),
        ),
        spec(
            "cpu_budget_hot",
            &["cpu_budget_pct"],
            Scenario::ApplyOk,
            HOT,
            || changed(|cfg| cfg.cpu_budget_pct = 250.0),
        ),
        spec(
            "ram_budget_hot",
            &["ram_budget_mb"],
            Scenario::ApplyOk,
            HOT,
            || changed(|cfg| cfg.ram_budget_mb = 1024),
        ),
        spec("comment_hot", &["_comment"], Scenario::ApplyOk, HOT, || {
            (
                AppConfig::default(),
                config_from_json(json!({"_comment": {"hc06": "documentation only"}})),
            )
        }),
        spec(
            "google_api_key_restart_provider",
            &["google_api_key"],
            Scenario::RestartRequired,
            PROVIDER_RESTART,
            || changed(|cfg| cfg.google_api_key = Some(SECRET_KEY.to_string())),
        )
        .reason_contains(&["google_api_key"])
        .redacts(SECRET_KEY),
        spec(
            "tts_output_device_restart",
            &["tts_output_device"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.tts_output_device = Some("Speakers (HC06)".to_string())),
        ),
        spec(
            "tts_routing_restart",
            &["tts_routing"],
            Scenario::RestartRequired,
            RESTART,
            || {
                changed_with_old(
                    |cfg| cfg.virtual_mic_device = Some("CABLE Output (VB-Audio)".to_string()),
                    |cfg| cfg.tts_routing = TtsRouting::Both,
                )
            },
        ),
        spec(
            "tts_source_restart",
            &["tts_source"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.tts_source = TtsSource::A),
        ),
        spec(
            "virtual_mic_device_restart",
            &["virtual_mic_device"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.virtual_mic_device = Some("CABLE Output (VB-Audio)".to_string())),
        ),
        spec(
            "virtual_device_patterns_restart",
            &["virtual_device_patterns"],
            Scenario::RestartRequired,
            RESTART,
            || {
                changed(|cfg| {
                    cfg.virtual_device_patterns
                        .push(VirtualDevicePatternConfig::new(
                            "HC06 Virtual Cable",
                            VirtualDeviceKind::GenericOem,
                        ));
                })
            },
        ),
        spec(
            "capture_device_hot_swap",
            &["capture_device"],
            Scenario::CaptureHotSwap,
            CAPTURE_HOT_SWAP,
            || changed(|cfg| cfg.capture_device = Some("Speakers (Realtek Audio)".to_string())),
        )
        .reason_contains(&["capture_device"]),
        spec(
            "audio_source_file_hot_swap",
            &["audio_source", "audio_file_path"],
            Scenario::CaptureHotSwap,
            CAPTURE_HOT_SWAP,
            || {
                changed(|cfg| {
                    cfg.audio_source = "file".to_string();
                    cfg.audio_file_path = Some("tests\\soak\\soak_audio.wav".to_string());
                })
            },
        )
        .reason_contains(&["audio_source", "audio_file_path"]),
        spec(
            "audio_file_path_file_hot_swap",
            &["audio_file_path"],
            Scenario::CaptureHotSwap,
            CAPTURE_HOT_SWAP,
            || {
                changed_with_old(
                    |cfg| {
                        cfg.audio_source = "file".to_string();
                        cfg.audio_file_path = Some("tests\\soak\\old.wav".to_string());
                    },
                    |cfg| cfg.audio_file_path = Some("tests\\soak\\new.wav".to_string()),
                )
            },
        )
        .reason_contains(&["audio_file_path"]),
        spec(
            "audio_file_path_wasapi_classifier_divergence",
            &["audio_file_path"],
            Scenario::CaptureHotSwap,
            AUDIO_FILE_PATH_WASAPI_DIVERGENCE,
            || changed(|cfg| cfg.audio_file_path = Some("ignored-under-wasapi.wav".to_string())),
        )
        .reason_contains(&["audio_file_path"]),
        spec(
            "invalid_audio_source_rejected",
            &["audio_source"],
            Scenario::Rollback,
            CAPTURE_REJECTED,
            || changed(|cfg| cfg.audio_source = "alsa".to_string()),
        )
        .reason_contains(&["unsupported audio_source"]),
        spec(
            "missing_audio_file_path_rejected",
            &["audio_source", "audio_file_path"],
            Scenario::Rollback,
            CAPTURE_REJECTED,
            || changed(|cfg| cfg.audio_source = "file".to_string()),
        )
        .reason_contains(&["audio_file_path"]),
        spec(
            "stt_provider_restart",
            &["stt_provider"],
            Scenario::RestartRequired,
            PROVIDER_RESTART,
            || {
                changed_with_old(
                    |cfg| cfg.stt_fallback_policy = "none".to_string(),
                    |cfg| cfg.stt_provider = "google".to_string(),
                )
            },
        )
        .reason_contains(&["stt_provider"]),
        spec(
            "mt_provider_restart",
            &["mt_provider"],
            Scenario::RestartRequired,
            PROVIDER_RESTART,
            || changed(|cfg| cfg.mt_provider = "local".to_string()),
        )
        .reason_contains(&["mt_provider"]),
        spec(
            "mt_cloud_fallback_restart",
            &["mt_cloud_fallback"],
            Scenario::RestartRequired,
            PROVIDER_RESTART,
            || {
                changed_with_old(
                    |cfg| cfg.google_api_key = Some(SECRET_KEY.to_string()),
                    |cfg| cfg.mt_cloud_fallback = Some("google".to_string()),
                )
            },
        )
        .reason_contains(&["mt_cloud_fallback"])
        .redacts(SECRET_KEY),
        spec(
            "stt_fallback_policy_restart",
            &["stt_fallback_policy"],
            Scenario::RestartRequired,
            PROVIDER_RESTART,
            || changed(|cfg| cfg.stt_fallback_policy = "none".to_string()),
        )
        .reason_contains(&["stt_fallback_policy"]),
        spec(
            "vad_enabled_restart",
            &["vad.enabled"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.vad.enabled = true),
        ),
        spec(
            "vad_threshold_restart",
            &["vad.threshold"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.vad.threshold = 0.02),
        ),
        spec(
            "vad_min_speech_restart",
            &["vad.min_speech_ms"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.vad.min_speech_ms = 150),
        ),
        spec(
            "vad_speech_pad_restart",
            &["vad.speech_pad_ms"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.vad.speech_pad_ms = 350),
        ),
        spec(
            "vad_min_silence_restart",
            &["vad.min_silence_ms"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.vad.min_silence_ms = 650),
        ),
        spec(
            "vad_pre_roll_restart",
            &["vad.pre_roll_ms"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.vad.pre_roll_ms = 250),
        ),
        spec(
            "stt_phrase_hints_restart",
            &["stt_phrase_hints"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.stt_phrase_hints = vec!["TuiTranslator".to_string()]),
        ),
        spec(
            "session_store_directory_path_switch",
            &["session_store.directory"],
            Scenario::RecorderPathSwitch,
            RECORDER_PATH_SWITCH,
            || changed(|cfg| cfg.session_store.directory = Some("D:\\hc06\\sessions".to_string())),
        )
        .reason_contains(&["session_store.directory"]),
        spec(
            "session_store_enabled_restart",
            &["session_store.enabled"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.session_store.enabled = false),
        ),
        spec(
            "session_store_limits_restart",
            &[
                "session_store.max_sessions",
                "session_store.per_session_bytes_cap",
                "session_store.total_bytes_cap",
                "session_store.retention_days",
            ],
            Scenario::RestartRequired,
            RESTART,
            || {
                changed(|cfg| {
                    cfg.session_store.max_sessions = 50;
                    cfg.session_store.per_session_bytes_cap = 1_048_576;
                    cfg.session_store.total_bytes_cap = 10_485_760;
                    cfg.session_store.retention_days = 7;
                })
            },
        ),
        spec(
            "session_store_directory_empty_rejected",
            &["session_store.directory"],
            Scenario::Rollback,
            RECORDER_REJECTED,
            || changed(|cfg| cfg.session_store.directory = Some("   ".to_string())),
        )
        .reason_contains(&["session_store.directory"]),
        spec(
            "pipeline_max_window_restart",
            &["pipeline.max_window_ms"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.pipeline.max_window_ms = 4_000),
        ),
        spec(
            "pipeline_early_flush_restart",
            &["pipeline.early_flush_on_vad_end"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.pipeline.early_flush_on_vad_end = false),
        ),
        spec(
            "pipeline_idle_flush_restart",
            &["pipeline.idle_flush_ms"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.pipeline.idle_flush_ms = 750),
        ),
        spec(
            "pipeline_idle_min_restart",
            &["pipeline.idle_min_ms"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.pipeline.idle_min_ms = 750),
        ),
        spec(
            "pipeline_sentence_age_restart",
            &["pipeline.sentence_max_age_ms"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.pipeline.sentence_max_age_ms = 5_000),
        ),
        spec(
            "audio_archive_consent_restart",
            &["audio_archive.consent_given"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.audio_archive.consent_given = true),
        ),
        spec(
            "audio_archive_store_audio_restart",
            &["audio_archive.store_audio", "audio_archive.consent_given"],
            Scenario::RestartRequired,
            RESTART,
            || {
                changed(|cfg| {
                    cfg.audio_archive.store_audio = true;
                    cfg.audio_archive.consent_given = true;
                })
            },
        ),
        spec(
            "audio_archive_directory_restart",
            &["audio_archive.directory"],
            Scenario::RestartRequired,
            RESTART,
            || changed(|cfg| cfg.audio_archive.directory = Some("D:\\hc06\\audio".to_string())),
        ),
        spec(
            "audio_archive_limits_restart",
            &[
                "audio_archive.max_size_mb",
                "audio_archive.total_bytes_cap",
                "audio_archive.retention_days",
            ],
            Scenario::RestartRequired,
            RESTART,
            || {
                changed(|cfg| {
                    cfg.audio_archive.max_size_mb = 128;
                    cfg.audio_archive.total_bytes_cap = 1_073_741_824;
                    cfg.audio_archive.retention_days = 14;
                })
            },
        ),
        spec(
            "audio_archive_directory_traversal_rejected",
            &["audio_archive.directory"],
            Scenario::Rollback,
            Expected {
                validate_ok: false,
                ..RESTART
            },
            || changed(|cfg| cfg.audio_archive.directory = Some("..\\audio".to_string())),
        )
        .reason_contains(&["audio_archive.directory"]),
        spec(
            "slots_enable_restart_provider",
            &["slots"],
            Scenario::RestartRequired,
            PROVIDER_RESTART,
            || changed(|cfg| cfg.slots = Some(dual_slots("local", "google", "local", "google"))),
        )
        .reason_contains(&["slots"]),
        spec(
            "slots_provider_selectors_restart",
            &[
                "slots.slot_a.stt_provider",
                "slots.slot_a.mt_provider",
                "slots.slot_b.stt_provider",
                "slots.slot_b.mt_provider",
            ],
            Scenario::RestartRequired,
            PROVIDER_RESTART,
            || {
                changed_with_old(
                    |cfg| cfg.slots = Some(dual_slots("google", "google", "google", "google")),
                    |cfg| cfg.slots = Some(dual_slots("local", "local", "local", "local")),
                )
            },
        )
        .reason_contains(&[
            "slots.slot_a.stt_provider",
            "slots.slot_a.mt_provider",
            "slots.slot_b.stt_provider",
            "slots.slot_b.mt_provider",
        ]),
        spec(
            "slots_target_languages_restart_provider_unchanged",
            &[
                "slots.slot_a.target_language",
                "slots.slot_b.target_language",
            ],
            Scenario::RestartRequired,
            RESTART,
            || {
                changed_with_old(
                    |cfg| {
                        cfg.slots = Some(shared_provider_dual_slots_with_targets(
                            "local", "google", "vi", "en",
                        ))
                    },
                    |cfg| {
                        cfg.slots = Some(shared_provider_dual_slots_with_targets(
                            "local", "google", "fr", "de",
                        ))
                    },
                )
            },
        ),
        spec(
            "provider_redacts_api_key_on_rejected_change",
            &["google_api_key", "mt_cloud_fallback"],
            Scenario::Rollback,
            PROVIDER_REJECTED,
            || {
                changed(|cfg| {
                    cfg.google_api_key = Some(SECRET_KEY.to_string());
                    cfg.mt_cloud_fallback = Some("bogus".to_string());
                })
            },
        )
        .reason_contains(&["mt_cloud_fallback"])
        .redacts(SECRET_KEY),
        spec(
            "provider_and_capture_change_restart_wins",
            &["stt_provider", "stt_fallback_policy", "capture_device"],
            Scenario::CombinedRestartWins,
            COMBINED_RESTART_CAPTURE,
            || {
                changed(|cfg| {
                    cfg.stt_provider = "google".to_string();
                    cfg.stt_fallback_policy = "none".to_string();
                    cfg.capture_device = Some("Speakers (HC06)".to_string());
                })
            },
        )
        .reason_contains(&["stt_provider", "capture_device"]),
        spec("noop_unchanged", &[], Scenario::Unchanged, HOT, || {
            (AppConfig::default(), AppConfig::default())
        }),
    ]
}

fn spec(
    id: &'static str,
    fields: &'static [&'static str],
    scenario: Scenario,
    expected: Expected,
    build: fn() -> (AppConfig, AppConfig),
) -> CaseSpec {
    CaseSpec {
        id,
        fields,
        scenario,
        expected,
        build,
        reason_contains: &[],
        secret_must_not_appear: None,
    }
}

impl CaseSpec {
    fn reason_contains(mut self, needles: &'static [&'static str]) -> Self {
        self.reason_contains = needles;
        self
    }

    fn redacts(mut self, secret: &'static str) -> Self {
        self.secret_must_not_appear = Some(secret);
        self
    }
}

fn changed(mutate_new: fn(&mut AppConfig)) -> (AppConfig, AppConfig) {
    changed_with_old(|_| {}, mutate_new)
}

fn changed_with_old(
    prepare_old: fn(&mut AppConfig),
    mutate_new: fn(&mut AppConfig),
) -> (AppConfig, AppConfig) {
    let mut old = AppConfig::default();
    prepare_old(&mut old);
    let mut new = old.clone();
    mutate_new(&mut new);
    (old, new)
}

fn config_from_json(value: serde_json::Value) -> AppConfig {
    serde_json::from_value(value).expect("test JSON should deserialize into AppConfig")
}

fn dual_slots(
    slot_a_stt_provider: &str,
    slot_a_mt_provider: &str,
    slot_b_stt_provider: &str,
    slot_b_mt_provider: &str,
) -> DualSlotConfig {
    DualSlotConfig {
        slot_a: SlotConfig {
            stt_provider: slot_a_stt_provider.to_string(),
            mt_provider: slot_a_mt_provider.to_string(),
            target_language: "vi".to_string(),
        },
        slot_b: SlotConfig {
            stt_provider: slot_b_stt_provider.to_string(),
            mt_provider: slot_b_mt_provider.to_string(),
            target_language: "en".to_string(),
        },
    }
}

fn shared_provider_dual_slots_with_targets(
    slot_a_stt_provider: &str,
    slot_a_mt_provider: &str,
    slot_a_target_language: &str,
    slot_b_target_language: &str,
) -> DualSlotConfig {
    DualSlotConfig {
        slot_a: SlotConfig {
            stt_provider: slot_a_stt_provider.to_string(),
            mt_provider: slot_a_mt_provider.to_string(),
            target_language: slot_a_target_language.to_string(),
        },
        slot_b: SlotConfig {
            stt_provider: slot_a_stt_provider.to_string(),
            mt_provider: slot_a_mt_provider.to_string(),
            target_language: slot_b_target_language.to_string(),
        },
    }
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
