use super::*;

// ── HC-05: RestartRequired persistence (issue #390) ──────────────────────────

#[test]
fn record_config_apply_restart_required_not_demoted_by_ok() {
    let state = AppState::new();
    state.record_config_apply(ConfigApplyStatus::RestartRequired {
        reason: "stt_provider changed".to_string(),
    });
    // A subsequent Ok must not overwrite the persistent RestartRequired.
    state.record_config_apply(ConfigApplyStatus::Ok {
        reason: "settings hot-reloaded".to_string(),
    });
    let snapshot = state.config_apply_snapshot();
    assert!(
        matches!(snapshot, Some(ConfigApplyStatus::RestartRequired { .. })),
        "RestartRequired must survive a subsequent Ok; got: {snapshot:?}"
    );
    assert_eq!(
        snapshot.unwrap().reason(),
        "stt_provider changed",
        "RestartRequired reason must be preserved"
    );
    assert_eq!(
        state.config_apply_count_value(),
        2,
        "apply counter must increment for every call"
    );
}

#[test]
fn record_config_apply_restart_required_not_demoted_by_rolled_back() {
    let state = AppState::new();
    state.record_config_apply(ConfigApplyStatus::RestartRequired {
        reason: "capture_device changed".to_string(),
    });
    state.record_config_apply(ConfigApplyStatus::RolledBack {
        reason: "invalid value".to_string(),
    });
    let snapshot = state.config_apply_snapshot();
    assert!(
        matches!(snapshot, Some(ConfigApplyStatus::RestartRequired { .. })),
        "RestartRequired must survive a subsequent RolledBack; got: {snapshot:?}"
    );
    assert_eq!(state.config_apply_count_value(), 2);
}

#[test]
fn record_config_apply_restart_required_updatable_by_restart_required() {
    let state = AppState::new();
    state.record_config_apply(ConfigApplyStatus::RestartRequired {
        reason: "first restart reason".to_string(),
    });
    state.record_config_apply(ConfigApplyStatus::RestartRequired {
        reason: "second restart reason".to_string(),
    });
    let snapshot = state.config_apply_snapshot();
    assert_eq!(
        snapshot.unwrap().reason(),
        "second restart reason",
        "RestartRequired reason must update when replaced by another RestartRequired"
    );
}

#[test]
fn record_config_apply_to_restart_required_not_demoted() {
    use std::sync::atomic::{AtomicU32, Ordering};
    let status_arc: Arc<Mutex<Option<(ConfigApplyStatus, std::time::Instant)>>> =
        Arc::new(Mutex::new(None));
    let count_arc: Arc<AtomicU32> = Arc::new(AtomicU32::new(0));
    record_config_apply_to(
        &status_arc,
        &count_arc,
        ConfigApplyStatus::RestartRequired {
            reason: "watcher reason".to_string(),
        },
    );
    record_config_apply_to(
        &status_arc,
        &count_arc,
        ConfigApplyStatus::Ok {
            reason: "settings hot-reloaded".to_string(),
        },
    );
    let guard = status_arc.lock().unwrap();
    let (status, _) = guard.as_ref().unwrap();
    assert!(
        matches!(status, ConfigApplyStatus::RestartRequired { .. }),
        "record_config_apply_to must not demote RestartRequired; got: {status:?}"
    );
    assert_eq!(count_arc.load(Ordering::Relaxed), 2);
}

// ── HC-05: reason truncation ──────────────────────────────────────────────────

#[test]
fn truncate_reason_short_string_is_unchanged() {
    let s = "short reason";
    assert_eq!(truncate_reason(s), s);
}

#[test]
fn truncate_reason_long_string_gets_ellipsis() {
    let long: String = "x".repeat(200);
    let result = truncate_reason(&long);
    assert!(
        result.ends_with('\u{2026}'),
        "truncated reason must end with ellipsis"
    );
    // 119 'x' chars + 1 ellipsis scalar
    assert_eq!(result.chars().count(), REASON_MAX_LEN);
}

#[test]
fn truncate_reason_at_exact_max_len_is_unchanged() {
    let s: String = "y".repeat(REASON_MAX_LEN);
    assert_eq!(
        truncate_reason(&s),
        s,
        "string at exactly max len must not be truncated"
    );
}
