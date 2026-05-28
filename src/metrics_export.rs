use anyhow::{Context, Result};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64},
        Arc,
    },
};

use crate::metrics::{self, MetricsSnapshot};

/// Environment variable that enables JSON metrics snapshot export.
pub(crate) const METRICS_SNAPSHOT_ENV: &str = "TUI_TRANSLATOR_METRICS_SNAPSHOT";

/// Shared handles from the session recorder and audio archive, used by the
/// metrics-publisher task to populate `MetricsSnapshot` storage fields.
pub(crate) struct StorageMetricsHandles {
    pub(crate) recorder_bytes: Arc<AtomicU64>,
    pub(crate) recorder_path: Option<PathBuf>,
    pub(crate) archive_bytes: Arc<AtomicU64>,
    pub(crate) archive_sealed: Arc<AtomicBool>,
    pub(crate) archive_path: Option<PathBuf>,
}

impl Default for StorageMetricsHandles {
    fn default() -> Self {
        Self {
            recorder_bytes: Arc::new(AtomicU64::new(0)),
            recorder_path: None,
            archive_bytes: Arc::new(AtomicU64::new(0)),
            archive_sealed: Arc::new(AtomicBool::new(false)),
            archive_path: None,
        }
    }
}

/// Serializable schema for the metrics snapshot sidecar export.
#[derive(Serialize)]
pub(crate) struct MetricsSnapshotExport {
    schema_version: &'static str,
    line_pairs_shown: u64,
    estimated_cost_usd: f64,
    e2e_latency_ms: Option<u64>,
    e2e_latency_mean_ms: f64,
    e2e_latency_p95_ms: u64,
    loss_pct: f64,
    total_chunks: u64,
    dropped_chunks: u64,
    recorder_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    recorder_path: Option<PathBuf>,
    archive_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    archive_path: Option<PathBuf>,
    archive_sealed: bool,
    fanout_slot_a_drops: u64,
    fanout_slot_b_drops: u64,
    capture_swap_count: u64,
    capture_swap_drops: u64,
    local_cpu_pct: f32,
    local_active_threads: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    backpressure: Option<serde_json::Value>,
}

impl From<&MetricsSnapshot> for MetricsSnapshotExport {
    fn from(snapshot: &MetricsSnapshot) -> Self {
        Self {
            schema_version: "4",
            line_pairs_shown: snapshot.line_pairs_shown,
            estimated_cost_usd: snapshot.estimated_cost_usd,
            e2e_latency_ms: snapshot.e2e_latency_ms,
            e2e_latency_mean_ms: snapshot.e2e_latency_mean_ms,
            e2e_latency_p95_ms: snapshot.e2e_latency_p95_ms,
            loss_pct: snapshot.loss_pct,
            total_chunks: snapshot.total_chunks,
            dropped_chunks: snapshot.dropped_chunks,
            recorder_bytes: snapshot.recorder_bytes,
            recorder_path: snapshot.recorder_path.clone(),
            archive_bytes: snapshot.archive_bytes,
            archive_path: snapshot.archive_path.clone(),
            archive_sealed: snapshot.archive_sealed,
            fanout_slot_a_drops: snapshot.fanout_slot_a_drops,
            fanout_slot_b_drops: snapshot.fanout_slot_b_drops,
            capture_swap_count: snapshot.capture_swap_count,
            capture_swap_drops: snapshot.capture_swap_drops,
            local_cpu_pct: snapshot.local_cpu_pct,
            local_active_threads: snapshot.local_active_threads,
            backpressure: metrics::backpressure::emit::try_clone()
                .map(|bp| bp.snapshot_json(metrics::BackpressureThresholds::PRODUCTION)),
        }
    }
}

/// Atomically write a metrics snapshot sidecar JSON file.
pub(crate) fn write_metrics_snapshot_export(path: &Path, snapshot: &MetricsSnapshot) -> Result<()> {
    let export = MetricsSnapshotExport::from(snapshot);
    let json = serde_json::to_vec(&export).context("failed to serialize metrics snapshot")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create metrics snapshot directory {}",
                parent.display()
            )
        })?;
    }
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, json).with_context(|| {
        format!(
            "failed to write metrics snapshot temp file {}",
            tmp_path.display()
        )
    })?;
    let _ = fs::remove_file(path);
    fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "failed to move metrics snapshot from {} to {}",
            tmp_path.display(),
            path.display()
        )
    })?;
    Ok(())
}
