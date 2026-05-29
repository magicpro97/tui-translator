use super::*;
use crate::metrics_export::MetricsSnapshotExport;
use anyhow::{anyhow, Result};

#[test]
fn metrics_warning_row_active_for_ram_warning() {
    let metrics = MetricsSnapshot {
        ram_warning: true,
        ..MetricsSnapshot::default()
    };
    assert!(
        metrics_warning_row_active(true, 0.0, &metrics),
        "expanded layout must reserve the warning row for RAM pressure"
    );
    assert!(
        !metrics_warning_row_active(false, 0.0, &metrics),
        "compact layout height must stay fixed even when RAM warning is active"
    );
}

#[test]
fn metrics_snapshot_export_includes_fanout_drop_counters() -> Result<()> {
    let snapshot = MetricsSnapshot {
        fanout_slot_a_drops: 3,
        fanout_slot_b_drops: 5,
        capture_swap_count: 2,
        capture_swap_drops: 1,
        ..MetricsSnapshot::default()
    };
    let value = serde_json::to_value(MetricsSnapshotExport::from(&snapshot))?;

    assert_eq!(value["schema_version"], "4");
    assert_eq!(value["fanout_slot_a_drops"], 3);
    assert_eq!(value["fanout_slot_b_drops"], 5);
    assert_eq!(value["capture_swap_count"], 2);
    assert_eq!(value["capture_swap_drops"], 1);
    assert_eq!(value["local_cpu_pct"], 0.0);
    assert_eq!(value["local_active_threads"], 0);

    Ok(())
}

/// Refs #503, Refs #505: when backpressure telemetry is installed, the snapshot
/// export includes a live `backpressure` JSON object; when not installed, the
/// field is omitted.
#[test]
fn metrics_snapshot_export_includes_backpressure_when_installed() -> Result<()> {
    let _lock = metrics::backpressure::emit::test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    metrics::backpressure::emit::uninstall();
    let snapshot = MetricsSnapshot::default();
    let value = serde_json::to_value(MetricsSnapshotExport::from(&snapshot))?;
    assert_eq!(value["schema_version"], "4");
    assert!(
        value.get("backpressure").is_none(),
        "backpressure field must be absent when telemetry is not installed"
    );

    let bp = std::sync::Arc::new(metrics::backpressure::BackpressureTelemetry::new());
    metrics::backpressure::emit::install(bp);
    let value2 = serde_json::to_value(MetricsSnapshotExport::from(&snapshot))?;
    let bp_val = value2
        .get("backpressure")
        .ok_or_else(|| anyhow!("backpressure field must be present when telemetry is installed"))?;
    assert!(
        bp_val.get("schema_version").is_some(),
        "backpressure snapshot must contain schema_version"
    );

    metrics::backpressure::emit::uninstall();
    Ok(())
}
