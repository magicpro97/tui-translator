//! Integration tests for the interpreter-readiness aggregator (US-02a, #726).
//!
//! These tests exercise the public API surface of `aggregate_readiness`,
//! `ReadinessAggregator`, `SubsystemHealth`, and `InterpreterSubsystem`
//! **as library callers** rather than as in-module unit tests.

#[path = "../src/readiness.rs"]
mod readiness;

mod readiness_aggregator {
    use super::readiness::{
        aggregate_readiness, InterpreterSubsystem, ReadinessAggregator, ReadinessState,
        SubsystemHealth, INTERPRETER_SUBSYSTEM_COUNT,
    };

    // ── aggregate_readiness ───────────────────────────────────────────────────

    /// Empty slice → `Ready`.  "No subsystems to wait for" is immediately ready.
    #[test]
    fn integration_aggregate_empty_returns_ready() {
        assert_eq!(aggregate_readiness(&[]), ReadinessState::Ready);
    }

    /// All subsystems `Healthy` → `Ready`.
    #[test]
    fn integration_aggregate_all_healthy_returns_ready() {
        let subsystems = [
            (InterpreterSubsystem::Stt, SubsystemHealth::Healthy),
            (InterpreterSubsystem::Mt, SubsystemHealth::Healthy),
            (InterpreterSubsystem::Tts, SubsystemHealth::Healthy),
            (
                InterpreterSubsystem::VirtualMicSink,
                SubsystemHealth::Healthy,
            ),
            (InterpreterSubsystem::LlmModel, SubsystemHealth::Healthy),
            (InterpreterSubsystem::SampleRate, SubsystemHealth::Healthy),
            (InterpreterSubsystem::DeviceLoss, SubsystemHealth::Healthy),
        ];
        assert_eq!(aggregate_readiness(&subsystems), ReadinessState::Ready);
    }

    /// All subsystems `Bootstrapping` → `Loading` with healthy count 0.
    #[test]
    fn integration_aggregate_all_bootstrapping_returns_loading_zero() {
        let subsystems = [
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

    /// Partial: K/2 healthy, K/2 bootstrapping → `Loading` with correct count.
    #[test]
    fn integration_aggregate_partial_ready_returns_loading_with_half_count() {
        let subsystems = [
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

    /// Any `Failed` subsystem → `Error`.
    #[test]
    fn integration_aggregate_any_failed_returns_error() {
        let subsystems = [
            (InterpreterSubsystem::Stt, SubsystemHealth::Healthy),
            (
                InterpreterSubsystem::VirtualMicSink,
                SubsystemHealth::Failed("vmic: not connected".to_string()),
            ),
            (InterpreterSubsystem::Mt, SubsystemHealth::Bootstrapping),
        ];
        let result = aggregate_readiness(&subsystems);
        match result {
            ReadinessState::Error(msg) => assert!(
                msg.contains("vmic: not connected"),
                "expected 'vmic: not connected' in error, got: {msg}"
            ),
            other => panic!("expected Error(_), got: {other:?}"),
        }
    }

    /// `aggregate_readiness` is idempotent: two calls with identical inputs produce
    /// identical outputs.
    #[test]
    fn integration_aggregate_is_idempotent() {
        let subsystems = [
            (InterpreterSubsystem::Stt, SubsystemHealth::Healthy),
            (InterpreterSubsystem::Mt, SubsystemHealth::Bootstrapping),
            (
                InterpreterSubsystem::Tts,
                SubsystemHealth::Failed("tts: error".to_string()),
            ),
        ];
        let a = aggregate_readiness(&subsystems);
        let b = aggregate_readiness(&subsystems);
        assert_eq!(a, b);
    }

    /// `loading_count_suffix` recovers `(ready, total)` from aggregator output.
    #[test]
    fn integration_loading_count_suffix_round_trips() {
        let subsystems = [
            (InterpreterSubsystem::Stt, SubsystemHealth::Healthy),
            (InterpreterSubsystem::Mt, SubsystemHealth::Healthy),
            (InterpreterSubsystem::Tts, SubsystemHealth::Bootstrapping),
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
        let state = aggregate_readiness(&subsystems);
        assert_eq!(
            state.loading_count_suffix(),
            Some((2u8, INTERPRETER_SUBSYSTEM_COUNT))
        );
    }

    // ── ReadinessAggregator ───────────────────────────────────────────────────────

    /// `ReadinessAggregator::default()` collapses to `Loading` (all Bootstrapping).
    #[test]
    fn integration_aggregator_default_is_loading() {
        let agg = ReadinessAggregator::default();
        assert!(matches!(agg.collapse(), ReadinessState::Loading { .. }));
    }

    /// Setting all fields to `Healthy` produces `Ready`.
    #[test]
    fn integration_aggregator_all_healthy_is_ready() {
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

    /// `count_summary` reflects the healthy count correctly.
    #[test]
    fn integration_aggregator_count_summary_correct() {
        let agg = ReadinessAggregator {
            stt: SubsystemHealth::Healthy,
            mt: SubsystemHealth::Healthy,
            tts: SubsystemHealth::Healthy,
            ..ReadinessAggregator::default()
        };
        assert_eq!(agg.count_summary(), (3u8, INTERPRETER_SUBSYSTEM_COUNT));
    }
} // mod readiness_aggregator
