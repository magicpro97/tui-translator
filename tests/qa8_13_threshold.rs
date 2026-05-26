//! QA8-13 — Regression threshold harness hook tests.
//!
//! These tests exercise the pure threshold logic shared with
//! `quality_benchmark` via `#[path = ...]` include — the same pattern used
//! by other QA8-* integration tests (see `tests/qa8_07_*.rs`).
//!
//! Per the QA8-13 ADR (`docs/adr/qa8-13-quality-corpus.md`), they use only
//! a synthetic transcript/corpus — no network, no private data, no real
//! corpus bytes.

#[path = "../src/bin/qa8_13/thresholds.rs"]
mod thresholds;

use thresholds::{evaluate, ModeObservation, ModeThresholds, ThresholdConfig, ThresholdLoadError};

fn clean_observation(mode: &str) -> ModeObservation {
    ModeObservation {
        mode: mode.to_owned(),
        wer: 0.05,
        cer: 0.03,
        bleu: 0.85,
        chrf: 0.90,
        latency_ms: 800.0,
        truncation_rate: 0.0,
        flicker: 0.5,
    }
}

#[test]
fn synthetic_defaults_pass_clean_observation() {
    let cfg = ThresholdConfig::synthetic_ci_defaults();
    let obs = vec![clean_observation("baseline"), clean_observation("ep-i")];
    let report = evaluate(&obs, &cfg);
    assert!(
        !report.regressed(),
        "clean synthetic observation must not regress: {:?}",
        report.breaches
    );
    assert!(report.unmapped_modes.is_empty());
}

#[test]
fn intentional_wer_regression_is_detected() {
    let cfg = ThresholdConfig::synthetic_ci_defaults();
    let mut obs = clean_observation("baseline");
    obs.wer = 0.95; // far above max_wer = 0.50
    let report = evaluate(&[obs], &cfg);
    assert!(report.regressed());
    let breach = report
        .breaches
        .iter()
        .find(|b| b.metric == "wer")
        .expect("wer breach must be reported");
    assert_eq!(breach.direction, "max");
    assert_eq!(breach.mode, "baseline");
    assert!(breach.observed > breach.bound);
}

#[test]
fn intentional_bleu_drop_is_detected_as_min_breach() {
    let cfg = ThresholdConfig::synthetic_ci_defaults();
    let mut obs = clean_observation("ep-i");
    obs.bleu = 0.01; // below min_bleu = 0.10
    let report = evaluate(&[obs], &cfg);
    let breach = report
        .breaches
        .iter()
        .find(|b| b.metric == "bleu")
        .expect("bleu breach must be reported");
    assert_eq!(breach.direction, "min");
    assert!(breach.observed < breach.bound);
}

#[test]
fn latency_and_flicker_proxies_are_gated() {
    let cfg = ThresholdConfig::synthetic_ci_defaults();
    let mut obs = clean_observation("baseline");
    obs.latency_ms = 9_999.0;
    obs.flicker = 99.0;
    let report = evaluate(&[obs], &cfg);
    let metrics: Vec<&str> = report.breaches.iter().map(|b| b.metric.as_str()).collect();
    assert!(metrics.contains(&"latency_ms"), "got {metrics:?}");
    assert!(metrics.contains(&"flicker"), "got {metrics:?}");
}

#[test]
fn truncation_rate_proxy_for_subtitle_stability() {
    let cfg = ThresholdConfig::synthetic_ci_defaults();
    let mut obs = clean_observation("baseline");
    obs.truncation_rate = 0.95; // above max_truncation_rate = 0.60
    let report = evaluate(&[obs], &cfg);
    assert!(report
        .breaches
        .iter()
        .any(|b| b.metric == "truncation_rate"));
}

#[test]
fn unmapped_mode_is_reported_without_breach() {
    let cfg = ThresholdConfig::synthetic_ci_defaults();
    let obs = ModeObservation {
        mode: "experimental".to_owned(),
        ..clean_observation("experimental")
    };
    let report = evaluate(&[obs], &cfg);
    assert!(!report.regressed());
    assert_eq!(report.unmapped_modes, vec!["experimental"]);
}

#[test]
fn none_field_disables_that_gate() {
    let cfg = ThresholdConfig {
        provenance: "test".into(),
        modes: vec![ModeThresholds {
            mode: "baseline".into(),
            max_wer: None,
            max_cer: None,
            min_bleu: None,
            min_chrf: None,
            max_latency_ms: None,
            max_truncation_rate: None,
            max_flicker: None,
        }],
    };
    let mut obs = clean_observation("baseline");
    obs.wer = 100.0;
    obs.bleu = -10.0;
    let report = evaluate(&[obs], &cfg);
    assert!(
        !report.regressed(),
        "all-None thresholds must accept any value"
    );
}

#[test]
fn json_round_trip_preserves_thresholds() {
    let cfg = ThresholdConfig::synthetic_ci_defaults();
    let json = serde_json::to_string_pretty(&cfg).expect("serialize");
    let parsed = ThresholdConfig::from_json_str(&json).expect("parse");
    assert_eq!(parsed, cfg);
}

#[test]
fn empty_modes_array_rejected() {
    let json = r#"{"provenance":"x","modes":[]}"#;
    let err = ThresholdConfig::from_json_str(json).unwrap_err();
    assert_eq!(err, ThresholdLoadError::Empty);
}

#[test]
fn out_of_range_truncation_rate_rejected() {
    let json = r#"{"provenance":"x","modes":[{"mode":"baseline","max_truncation_rate":1.5}]}"#;
    let err = ThresholdConfig::from_json_str(json).unwrap_err();
    match err {
        ThresholdLoadError::Invalid(msg) => {
            assert!(msg.contains("max_truncation_rate"), "msg={msg}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn missing_optional_fields_default_to_disabled() {
    // Only mode name supplied — all thresholds default to None (disabled).
    let json = r#"{"provenance":"min","modes":[{"mode":"baseline"}]}"#;
    let cfg = ThresholdConfig::from_json_str(json).expect("parse minimal config");
    let m = cfg.for_mode("baseline").expect("mode present");
    assert!(m.max_wer.is_none());
    assert!(m.min_bleu.is_none());
}

#[test]
fn synthetic_transcript_regression_end_to_end() {
    // Simulate the QA8-13 PR-tier flow: a synthetic "good" baseline run vs. a
    // synthetic "regressed" run sharing the same threshold config. The good
    // run must pass; the regressed run must produce a breach for each
    // intentionally degraded metric.
    let cfg = ThresholdConfig::synthetic_ci_defaults();

    let good = vec![
        ModeObservation {
            mode: "baseline".into(),
            wer: 0.20,
            cer: 0.15,
            bleu: 0.55,
            chrf: 0.70,
            latency_ms: 2_200.0,
            truncation_rate: 0.30,
            flicker: 2.0,
        },
        ModeObservation {
            mode: "ep-i".into(),
            wer: 0.08,
            cer: 0.05,
            bleu: 0.80,
            chrf: 0.85,
            latency_ms: 900.0,
            truncation_rate: 0.0,
            flicker: 0.4,
        },
    ];
    assert!(!evaluate(&good, &cfg).regressed());

    let regressed = vec![ModeObservation {
        mode: "ep-i".into(),
        wer: 0.99,
        cer: 0.99,
        bleu: 0.02,
        chrf: 0.05,
        latency_ms: 6_000.0,
        truncation_rate: 0.80,
        flicker: 10.0,
    }];
    let report = evaluate(&regressed, &cfg);
    assert!(report.regressed());
    // All seven gates should fire for this fully-degraded synthetic run.
    let metrics: std::collections::HashSet<&str> =
        report.breaches.iter().map(|b| b.metric.as_str()).collect();
    for expected in [
        "wer",
        "cer",
        "bleu",
        "chrf",
        "latency_ms",
        "truncation_rate",
        "flicker",
    ] {
        assert!(
            metrics.contains(expected),
            "missing {expected}: {metrics:?}"
        );
    }
}
