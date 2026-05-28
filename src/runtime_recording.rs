use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use crate::{audio, config, session, storage};

/// Start the session transcript recorder for one runtime slot.
#[allow(clippy::too_many_arguments)]
pub(crate) fn start_session_recorder(
    rt: &tokio::runtime::Runtime,
    cfg: &config::AppConfig,
    status_slot: &Arc<Mutex<Option<String>>>,
    started_at_unix_ms: u64,
    session_id: &str,
    slot_suffix: Option<&str>,
    slot_label: Option<&str>,
    apply_retention: bool,
) -> session::SessionRecorder {
    if !cfg.session_store.enabled {
        return session::SessionRecorder::disabled();
    }

    let directory = match cfg
        .session_store
        .directory
        .as_ref()
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(config::default_sessions_dir)
    {
        Ok(path) => path,
        Err(err) => {
            let msg = session_recording_disabled_status(&err);
            tracing::warn!("session recording disabled: {err:#}");
            *status_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
            return session::SessionRecorder::disabled();
        }
    };

    let header = session::SessionHeader {
        schema_version: session::SESSION_LOG_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        started_at_unix_ms,
        source_language: cfg.source_language.clone(),
        target_language: cfg.target_language.clone(),
        stt_provider: cfg.stt_provider.clone(),
        mt_provider: cfg.mt_provider.clone(),
        tts_enabled: cfg.tts_enabled,
        capture_device: cfg.capture_device.clone(),
        slot_label: slot_label.map(str::to_string),
        slot_id: slot_suffix.map(str::to_string),
    };

    let recorder_config = if apply_retention {
        session::SessionRecorderConfig::enabled_with_max_sessions(
            directory.clone(),
            cfg.session_store.max_sessions,
        )
    } else {
        session::SessionRecorderConfig::enabled(directory.clone())
    };
    let recorder_config =
        recorder_config.with_per_session_bytes_cap(cfg.session_store.per_session_bytes_cap);
    let recorder_config = match slot_suffix {
        None => recorder_config,
        Some(sfx) => match recorder_config.with_slot_suffix(sfx) {
            Ok(config) => config,
            Err(err) => {
                let msg = session_recording_disabled_status(&err);
                tracing::warn!("session recording disabled (invalid slot suffix): {err:#}");
                *status_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
                return session::SessionRecorder::disabled();
            }
        },
    };

    match rt.block_on(session::SessionRecorder::start(recorder_config, header)) {
        Ok(recorder) => {
            if let Some(path) = recorder.path() {
                tracing::info!(
                    session_id = %session_id,
                    path = %path.display(),
                    "session transcript recording enabled"
                );
            }
            if apply_retention {
                apply_storage_retention(
                    &directory,
                    cfg.session_store.total_bytes_cap,
                    cfg.session_store.retention_days,
                    "transcripts",
                    Some(session_id),
                );
            }
            recorder
        }
        Err(err) => {
            let msg = session_recording_disabled_status(&err);
            tracing::warn!("session recording disabled: {err:#}");
            *status_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
            session::SessionRecorder::disabled()
        }
    }
}

pub(crate) fn session_recording_disabled_status(err: &anyhow::Error) -> String {
    format!("⚠ Session recording disabled: {err}").replace(['\r', '\n'], " ")
}

/// Start raw audio archiving for the current runtime session.
pub(crate) fn start_audio_archive(
    cfg: &config::AppConfig,
    session_id: &str,
    status_slot: &Arc<Mutex<Option<String>>>,
) -> audio::AudioArchiveWriter {
    if !cfg.audio_archive.store_audio || !cfg.audio_archive.consent_given {
        return audio::AudioArchiveWriter::disabled();
    }

    let directory = match cfg
        .audio_archive
        .directory
        .as_ref()
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(config::default_audio_archive_dir)
    {
        Ok(path) => path,
        Err(err) => {
            let msg = audio_archive_disabled_status(&err);
            tracing::warn!("audio archive disabled: {err:#}");
            *status_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
            return audio::AudioArchiveWriter::disabled();
        }
    };

    let archive_config = audio::AudioArchiveWriterConfig {
        enabled: true,
        directory: directory.clone(),
        max_size_bytes: cfg.audio_archive.max_size_mb.saturating_mul(1024 * 1024),
    };

    match audio::AudioArchiveWriter::start(&archive_config, session_id) {
        Ok(writer) => {
            if let Some(path) = writer.path() {
                tracing::info!(path = %path.display(), "raw audio archive enabled");
                *status_slot.lock().unwrap_or_else(|p| p.into_inner()) =
                    Some(format!("⚠ Audio archiving enabled: {}", path.display()));
            }
            apply_storage_retention(
                &directory,
                cfg.audio_archive.total_bytes_cap,
                cfg.audio_archive.retention_days,
                "audio archive",
                Some(session_id),
            );
            writer
        }
        Err(err) => {
            let msg = audio_archive_disabled_status(&err);
            tracing::warn!("audio archive disabled: {err:#}");
            *status_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
            audio::AudioArchiveWriter::disabled()
        }
    }
}

pub(crate) fn apply_storage_retention(
    root: &Path,
    total_bytes_cap: u64,
    retention_days: u64,
    label: &str,
    active_session_id: Option<&str>,
) {
    if total_bytes_cap > 0 {
        match storage::enforce_total_session_cap(root, total_bytes_cap, active_session_id) {
            Ok(evicted) => {
                if evicted > 0 {
                    tracing::info!(
                        root = %root.display(),
                        evicted,
                        "{label} retention: evicted oldest sessions over total-bytes cap"
                    );
                }
            }
            Err(err) => {
                tracing::warn!("{label} total-cap enforcement failed: {err:#}");
            }
        }
    }
    if retention_days > 0 {
        let ttl = std::time::Duration::from_secs(retention_days.saturating_mul(86_400));
        match storage::purge_expired_sessions(root, ttl, active_session_id) {
            Ok(evicted) => {
                if evicted > 0 {
                    tracing::info!(
                        root = %root.display(),
                        evicted,
                        "{label} retention: purged sessions older than TTL"
                    );
                }
            }
            Err(err) => {
                tracing::warn!("{label} TTL purge failed: {err:#}");
            }
        }
    }
}

pub(crate) fn audio_archive_disabled_status(err: &anyhow::Error) -> String {
    format!("⚠ Audio archive disabled: {err}").replace(['\r', '\n'], " ")
}

/// Build the single-line status for active transcript/audio measurement artifacts.
pub(crate) fn measurement_mode_status(
    session_id: &str,
    jsonl_path: Option<&Path>,
    wav_path: Option<&Path>,
) -> Option<String> {
    if jsonl_path.is_none() && wav_path.is_none() {
        return None;
    }
    let mut parts = Vec::new();
    if let Some(path) = jsonl_path {
        parts.push(format!("transcript={}", path.display()));
    }
    if let Some(path) = wav_path {
        parts.push(format!("audio={}", path.display()));
    }
    if let (Some(jpath), Some(wpath)) = (jsonl_path, wav_path) {
        parts.push(format!(
            "| eval: eval_session --session {} --audio {} --truth <truth.tsv> --output-dir target/eval",
            shell_quoted_path(jpath),
            shell_quoted_path(wpath)
        ));
    }
    let msg = format!(
        "⚠ Measurement mode active: session={session_id} {}",
        parts.join(" ")
    );
    Some(msg.replace(['\r', '\n'], " "))
}

fn shell_quoted_path(path: &Path) -> String {
    format!("\"{}\"", path.display().to_string().replace('"', "\\\""))
}

/// Log and publish the combined measurement-mode status when artifacts are active.
pub(crate) fn log_measurement_mode_status(
    session_id: &str,
    jsonl_path: Option<&Path>,
    wav_path: Option<&Path>,
    status_slot: &Arc<Mutex<Option<String>>>,
) {
    if let Some(msg) = measurement_mode_status(session_id, jsonl_path, wav_path) {
        tracing::info!(
            session_id = %session_id,
            jsonl_path = ?jsonl_path,
            wav_path = ?wav_path,
            "measurement mode active"
        );
        *status_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
    }
}

#[cfg(test)]
pub(crate) fn attach_audio_archive(
    rt: &tokio::runtime::Runtime,
    stream: audio::CaptureStream,
    mut writer: audio::AudioArchiveWriter,
    status_slot: Arc<Mutex<Option<String>>>,
) -> audio::CaptureStream {
    if writer.is_disabled() {
        return stream;
    }

    let info = stream.info;
    let mut input_rx = stream.receiver;
    let (output_tx, output_rx) = tokio::sync::mpsc::channel(64);

    rt.spawn(async move {
        while let Some(chunk) = input_rx.recv().await {
            if !writer.is_disabled() {
                if let Err(err) = writer.append_chunk(&chunk) {
                    let msg = audio_archive_disabled_status(&err);
                    tracing::warn!("audio archive disabled after write error: {err:#}");
                    *status_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
                    writer.disable();
                }
            }

            if output_tx.send(chunk).await.is_err() {
                break;
            }
        }
    });

    audio::CaptureStream {
        info,
        receiver: output_rx,
    }
}
