use serde_json::json;

use crate::audio::{VirtualDeviceKind, VirtualDevicePatternConfig};
use crate::config::{AppConfig, DualSlotConfig, SlotConfig, TtsRouting, TtsSource};

use super::{
    CaseSpec, Expected, Scenario, AUDIO_FILE_PATH_WASAPI_DIVERGENCE, CAPTURE_HOT_SWAP,
    CAPTURE_REJECTED, COMBINED_RESTART_CAPTURE, HOT, PROVIDER_REJECTED, PROVIDER_RESTART,
    RECORDER_PATH_SWITCH, RECORDER_REJECTED, RESTART, SECRET_KEY,
};

pub(super) fn matrix_cases() -> Vec<CaseSpec> {
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
            || {
                // JV-13: default mt_provider is "local" when local-mt feature is compiled
                // in, and "google" otherwise. Change to the non-default value so that
                // requires_restart sees an actual transition.
                changed(|cfg| {
                    #[cfg(feature = "local-mt")]
                    {
                        cfg.mt_provider = "google".to_string();
                    }
                    #[cfg(not(feature = "local-mt"))]
                    {
                        cfg.mt_provider = "local".to_string();
                    }
                })
            },
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
