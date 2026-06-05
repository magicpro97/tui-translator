//! Unit tests for [`aggregate_readiness`] and [`ReadinessAggregator`].
//!
//! Included by `src/readiness.rs` via `#[path]`.  All tests live in
//! `mod readiness_aggregator_tests` so `cargo test readiness_aggregator`
//! captures them without a manual `--test` flag.

use super::*;

// ── aggregate_readiness pure-function tests ───────────────────────────────

/// An empty subsystem list means "nothing to wait for" — the result is
/// immediately `Ready`.  This is the documented "no work" path.
#[test]
fn aggregate_empty_returns_ready() {
    assert_eq!(aggregate_readiness(&[]), ReadinessState::Ready);
}

/// All subsystems `Healthy` → `Ready`.
#[test]
fn aggregate_collapses_all_ready_to_ready() {
    let subsystems = vec![
        (InterpreterSubsystem::Stt, SubsystemHealth::Healthy),
        (InterpreterSubsystem::Mt, SubsystemHealth::Healthy),
        (InterpreterSubsystem::Tts, SubsystemHealth::Healthy),
        (
            InterpreterSubsystem::VirtualMicSink,
            SubsystemHealth::Healthy,
        ),
        (InterpreterSubsystem::LlmModel, SubsystemHealth::Healthy),
    ];
    assert_eq!(aggregate_readiness(&subsystems), ReadinessState::Ready);
}

/// All subsystems `Bootstrapping` → `Loading` with healthy count 0.
#[test]
fn aggregate_all_systems_bootstrapping_returns_loading() {
    let subsystems = vec![
        (InterpreterSubsystem::Stt, SubsystemHealth::Bootstrapping),
        (InterpreterSubsystem::Mt, SubsystemHealth::Bootstrapping),
        (InterpreterSubsystem::Tts, SubsystemHealth::Bootstrapping),
    ];
    let result = aggregate_readiness(&subsystems);
    assert!(
        matches!(
            &result,
            ReadinessState::Loading {
                component: "stt",
                percent: Some(0),
            }
        ),
        "expected Loading{{stt, Some(0)}}, got {result:?}"
    );
}

/// Partial: half Healthy, half Bootstrapping → `Loading` with correct
/// healthy count in `percent`.
#[test]
fn aggregate_partial_ready_returns_loading_with_count() {
    let subsystems = vec![
        (InterpreterSubsystem::Stt, SubsystemHealth::Healthy),
        (InterpreterSubsystem::Mt, SubsystemHealth::Healthy),
        (InterpreterSubsystem::Tts, SubsystemHealth::Bootstrapping),
        (
            InterpreterSubsystem::VirtualMicSink,
            SubsystemHealth::Bootstrapping,
        ),
    ];
    let result = aggregate_readiness(&subsystems);
    assert!(
        matches!(
            &result,
            ReadinessState::Loading {
                component: "tts",
                percent: Some(2),
            }
        ),
        "expected Loading{{tts, Some(2)}}, got {result:?}"
    );
}

/// Any `Failed` subsystem → `Error` carrying the failure message.
#[test]
fn aggregate_collapses_any_failed_to_error() {
    let subsystems = vec![
        (InterpreterSubsystem::Stt, SubsystemHealth::Healthy),
        (
            InterpreterSubsystem::VirtualMicSink,
            SubsystemHealth::Failed("vmic: not connected".to_string()),
        ),
        (InterpreterSubsystem::Mt, SubsystemHealth::Bootstrapping),
    ];
    let result = aggregate_readiness(&subsystems);
    match result {
        ReadinessState::Error(msg) => {
            assert!(
                msg.contains("vmic: not connected"),
                "expected error to contain 'vmic: not connected', got: {msg}"
            );
        }
        other => panic!("expected Error(_), got: {other:?}"),
    }
}

/// `Failed` takes priority over `Bootstrapping` even when the failed
/// subsystem appears after the bootstrapping one in the slice.
#[test]
fn aggregate_failed_wins_over_bootstrapping() {
    let subsystems = vec![
        (InterpreterSubsystem::Stt, SubsystemHealth::Bootstrapping),
        (
            InterpreterSubsystem::Mt,
            SubsystemHealth::Failed("mt-error".to_string()),
        ),
    ];
    assert!(matches!(
        aggregate_readiness(&subsystems),
        ReadinessState::Error(_)
    ));
}

/// `loading_count_suffix` decodes `(ready, total)` from a `Loading` state
/// produced by `aggregate_readiness`.
#[test]
fn aggregate_collapses_any_starting_to_loading_with_count_suffix() {
    let subsystems = vec![
        (InterpreterSubsystem::Stt, SubsystemHealth::Healthy),
        (InterpreterSubsystem::Mt, SubsystemHealth::Healthy),
        (InterpreterSubsystem::Tts, SubsystemHealth::Healthy),
        (
            InterpreterSubsystem::VirtualMicSink,
            SubsystemHealth::Bootstrapping,
        ),
        (
            InterpreterSubsystem::LlmModel,
            SubsystemHealth::Bootstrapping,
        ),
        (
            InterpreterSubsystem::SampleRate,
            SubsystemHealth::Bootstrapping,
        ),
        (
            InterpreterSubsystem::DeviceLoss,
            SubsystemHealth::Bootstrapping,
        ),
    ];
    let result = aggregate_readiness(&subsystems);
    let suffix = result.loading_count_suffix();
    assert_eq!(
        suffix,
        Some((3u8, INTERPRETER_SUBSYSTEM_COUNT)),
        "expected (3, {INTERPRETER_SUBSYSTEM_COUNT}), got {suffix:?}"
    );
}

/// `SampleRate` subsystem is a valid aggregator input.
#[test]
fn aggregate_includes_sample_rate_subsystem() {
    let subsystems = vec![(
        InterpreterSubsystem::SampleRate,
        SubsystemHealth::Failed("unsupported format F32".to_string()),
    )];
    assert!(matches!(
        aggregate_readiness(&subsystems),
        ReadinessState::Error(_)
    ));
}

/// `DeviceLoss` subsystem is a valid aggregator input.
#[test]
fn aggregate_includes_device_loss_subsystem() {
    let subsystems = vec![(
        InterpreterSubsystem::DeviceLoss,
        SubsystemHealth::Bootstrapping,
    )];
    assert!(matches!(
        aggregate_readiness(&subsystems),
        ReadinessState::Loading { .. }
    ));
}

/// `aggregate_readiness` is idempotent: calling it twice with the same
/// inputs produces the same output.
#[test]
fn aggregate_is_idempotent() {
    let subsystems = vec![
        (InterpreterSubsystem::Stt, SubsystemHealth::Healthy),
        (InterpreterSubsystem::Mt, SubsystemHealth::Bootstrapping),
    ];
    let first = aggregate_readiness(&subsystems);
    let second = aggregate_readiness(&subsystems);
    assert_eq!(first, second);
}

// ── ReadinessAggregator struct tests ──────────────────────────────────────

#[test]
fn aggregator_default_collapses_to_loading() {
    let agg = ReadinessAggregator::default();
    assert!(matches!(agg.collapse(), ReadinessState::Loading { .. }));
}

#[test]
fn aggregator_all_healthy_collapses_to_ready() {
    let agg = ReadinessAggregator {
        stt: SubsystemHealth::Healthy,
        mt: SubsystemHealth::Healthy,
        tts: SubsystemHealth::Healthy,
        virtual_mic: SubsystemHealth::Healthy,
        llm_model: SubsystemHealth::Healthy,
        sample_rate: SubsystemHealth::Healthy,
        device_loss: SubsystemHealth::Healthy,
    };
    assert_eq!(agg.collapse(), ReadinessState::Ready);
}

#[test]
fn aggregator_count_summary_matches_healthy_count() {
    let agg = ReadinessAggregator {
        stt: SubsystemHealth::Healthy,
        mt: SubsystemHealth::Healthy,
        tts: SubsystemHealth::Bootstrapping,
        virtual_mic: SubsystemHealth::Bootstrapping,
        llm_model: SubsystemHealth::Bootstrapping,
        sample_rate: SubsystemHealth::Bootstrapping,
        device_loss: SubsystemHealth::Bootstrapping,
    };
    assert_eq!(agg.count_summary(), (2u8, INTERPRETER_SUBSYSTEM_COUNT));
}

#[test]
fn loading_count_suffix_returns_none_for_non_loading_states() {
    assert_eq!(ReadinessState::Init.loading_count_suffix(), None);
    assert_eq!(ReadinessState::Ready.loading_count_suffix(), None);
    assert_eq!(
        ReadinessState::Error("x".to_string()).loading_count_suffix(),
        None
    );
    assert_eq!(
        ReadinessState::Loading {
            component: "stt",
            percent: None,
        }
        .loading_count_suffix(),
        None
    );
}
