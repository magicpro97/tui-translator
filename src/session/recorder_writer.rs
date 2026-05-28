//! Background writer task and low-level file/path helpers for [`SessionRecorder`].
//!
//! Extracted from `session/mod.rs` for STD-02 LOC compliance (#484).
//! Public API and on-disk behavior are unchanged; symbols are re-exported
//! through `crate::session` for callers and tests.
//!
//! [`SessionRecorder`]: super::recorder::SessionRecorder

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    fs::{self, File, OpenOptions},
    io::AsyncWriteExt,
    sync::{mpsc, oneshot},
};

use super::{SessionHeader, SessionLogRecord};

/// Bounded capacity for the non-blocking writer queue.
pub(super) const RECORDER_QUEUE_CAPACITY: usize = 64;

/// Internal writer message — either a JSONL record or a lifecycle command.
pub(super) enum WriterMessage {
    /// A normal record to append to the JSONL file.
    Record(SessionLogRecord),
    /// Seal the current JSONL file and open a new one at `new_session_dir`.
    ///
    /// The writer task flushes the current file, creates `new_session_dir` if
    /// needed, opens `new_session_dir/00001.jsonl`, writes `new_header` as the
    /// first record, and sends the new path on `done_tx`.
    SealAndReopen {
        /// Directory for the new session's JSONL segments.
        new_session_dir: PathBuf,
        /// Session header to write as the first record of the new file.
        new_header: SessionHeader,
        /// Notified with `Ok(new_path)` on success or `Err(msg)` on failure.
        done_tx: oneshot::Sender<Result<PathBuf, String>>,
    },
}

/// Long-lived state owned by the background writer task.
pub(super) struct WriterRuntime {
    pub(super) last_error: Arc<Mutex<Option<String>>>,
    pub(super) session_dir: PathBuf,
    pub(super) active_path: Arc<Mutex<Option<PathBuf>>>,
    pub(super) initial_segment_bytes: u64,
    pub(super) per_session_bytes_cap: u64,
    pub(super) bytes_written: Arc<AtomicU64>,
    pub(super) slot_suffix: Option<String>,
}

pub(super) async fn run_writer(
    mut file: File,
    mut rx: mpsc::Receiver<WriterMessage>,
    writer: WriterRuntime,
) {
    let WriterRuntime {
        last_error,
        mut session_dir,
        active_path,
        initial_segment_bytes,
        per_session_bytes_cap,
        bytes_written,
        slot_suffix,
    } = writer;
    let mut current_segment: u32 = 1;
    let mut current_segment_bytes: u64 = initial_segment_bytes;
    let mut current_path: PathBuf = active_path
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .unwrap_or_else(|| {
            session_dir.join(segment_file_name(current_segment, slot_suffix.as_deref()))
        });

    while let Some(msg) = rx.recv().await {
        let record = match msg {
            WriterMessage::Record(r) => r,
            WriterMessage::SealAndReopen {
                new_session_dir,
                new_header,
                done_tx,
            } => {
                // Flush the current file so it is fully parseable.
                if let Err(err) = file.flush().await {
                    let m = format!("seal: flush failed for {}: {err}", current_path.display());
                    tracing::warn!("{m}");
                    let _ = done_tx.send(Err(m));
                    continue;
                }
                // Create new session directory.
                if let Err(err) = fs::create_dir_all(&new_session_dir).await {
                    let m = format!(
                        "seal: cannot create session dir {}: {err}",
                        new_session_dir.display()
                    );
                    tracing::warn!("{m}");
                    let _ = done_tx.send(Err(m));
                    continue;
                }
                let new_path = new_session_dir.join(segment_file_name(1, slot_suffix.as_deref()));
                match OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&new_path)
                    .await
                {
                    Ok(mut new_file) => {
                        let header_record = SessionLogRecord::SessionHeader(new_header);
                        match write_record_line(&mut new_file, &header_record).await {
                            Ok(header_bytes) => {
                                file = new_file;
                                session_dir = new_session_dir;
                                current_segment = 1;
                                current_segment_bytes = header_bytes as u64;
                                current_path = new_path.clone();
                                *active_path.lock().unwrap_or_else(|p| p.into_inner()) =
                                    Some(new_path.clone());
                                tracing::info!(
                                    path = %new_path.display(),
                                    "recorder sealed and reopened at new path"
                                );
                                let _ = done_tx.send(Ok(new_path));
                            }
                            Err(err) => {
                                let m = format!(
                                    "seal: write header failed for {}: {err}",
                                    new_path.display()
                                );
                                tracing::warn!("{m}");
                                let _ = done_tx.send(Err(m));
                            }
                        }
                    }
                    Err(err) => {
                        let m = format!("seal: cannot open new file {}: {err}", new_path.display());
                        tracing::warn!("{m}");
                        let _ = done_tx.send(Err(m));
                    }
                }
                continue;
            }
        };
        // Pre-serialise to know the line length so we can rotate before writing.
        let line = match serialise_record_line(&record) {
            Ok(line) => line,
            Err(err) => {
                let message = format!(
                    "session recorder serialise failed for {}: {err}",
                    current_path.display()
                );
                tracing::warn!("{message}");
                *last_error.lock().unwrap_or_else(|p| p.into_inner()) = Some(message);
                break;
            }
        };
        let line_len = line.len() as u64;

        if per_session_bytes_cap > 0
            && current_segment_bytes > 0
            && current_segment_bytes.saturating_add(line_len) > per_session_bytes_cap
        {
            if let Err(err) = file.flush().await {
                let message = format!(
                    "session recorder flush failed for {}: {err}",
                    current_path.display()
                );
                tracing::warn!("{message}");
                *last_error.lock().unwrap_or_else(|p| p.into_inner()) = Some(message);
                break;
            }
            current_segment = current_segment.saturating_add(1);
            let next_path =
                session_dir.join(segment_file_name(current_segment, slot_suffix.as_deref()));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&next_path)
                .await
            {
                Ok(next_file) => {
                    file = next_file;
                    current_path = next_path.clone();
                    *active_path.lock().unwrap_or_else(|p| p.into_inner()) = Some(next_path);
                    current_segment_bytes = 0;
                }
                Err(err) => {
                    let message = format!(
                        "session recorder failed to open next segment {}: {err}",
                        next_path.display()
                    );
                    tracing::warn!("{message}");
                    *last_error.lock().unwrap_or_else(|p| p.into_inner()) = Some(message);
                    break;
                }
            }
        }

        match write_line_bytes(&mut file, &line).await {
            Ok(()) => {
                current_segment_bytes = current_segment_bytes.saturating_add(line_len);
                bytes_written.fetch_add(line_len, Ordering::Relaxed);
            }
            Err(err) => {
                let message = format!(
                    "session recorder write failed for {}: {err}",
                    current_path.display()
                );
                tracing::warn!("{message}");
                *last_error.lock().unwrap_or_else(|p| p.into_inner()) = Some(message);
                break;
            }
        }
    }

    if let Err(err) = file.flush().await {
        let message = format!(
            "session recorder flush failed for {}: {err}",
            current_path.display()
        );
        tracing::warn!("{message}");
        *last_error.lock().unwrap_or_else(|p| p.into_inner()) = Some(message);
    }
}

fn serialise_record_line(record: &SessionLogRecord) -> std::io::Result<Vec<u8>> {
    let mut line = serde_json::to_vec(record).map_err(std::io::Error::other)?;
    line.push(b'\n');
    Ok(line)
}

async fn write_line_bytes(file: &mut File, line: &[u8]) -> std::io::Result<()> {
    file.write_all(line).await?;
    file.flush().await
}

pub(super) async fn write_record_line(
    file: &mut File,
    record: &SessionLogRecord,
) -> std::io::Result<usize> {
    let line = serialise_record_line(record)?;
    let len = line.len();
    write_line_bytes(file, &line).await?;
    Ok(len)
}

pub(crate) fn segment_file_name(segment_index: u32, slot_suffix: Option<&str>) -> String {
    match slot_suffix {
        None => format!("{segment_index:05}.jsonl"),
        Some(s) => format!("{segment_index:05}-{s}.jsonl"),
    }
}

/// Validate a slot suffix to prevent path traversal and filesystem collisions.
///
/// A valid suffix is 1-8 lowercase ASCII alphanumeric characters (e.g. `"a"`, `"b"`).
pub(crate) fn validate_slot_suffix(suffix: &str) -> anyhow::Result<()> {
    if suffix.is_empty() {
        anyhow::bail!("slot suffix must not be empty");
    }
    if suffix.len() > 8 {
        anyhow::bail!("slot suffix too long (max 8 chars): {suffix:?}");
    }
    if !suffix
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    {
        anyhow::bail!(
            "slot suffix must contain only lowercase ASCII alphanumeric characters: {suffix:?}"
        );
    }
    Ok(())
}

pub(super) fn sanitize_session_id_for_fs(session_id: &str) -> String {
    session_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

/// Local mirror of `storage::validate_path_component`'s acceptance rule —
/// duplicated so that bin targets which only pull in `session/mod.rs`
/// (e.g. `eval_session`) still build without a separate `storage` mount.
/// The rule must stay in sync with `crate::storage::validate_path_component`.
pub(super) fn is_valid_path_component(component: &str) -> bool {
    if component.is_empty() || component == "." || component == ".." {
        return false;
    }
    if component.contains('/') || component.contains('\\') || component.contains(':') {
        return false;
    }
    if component.chars().any(|c| {
        (c as u32) < 0x20 || c == '<' || c == '>' || c == '"' || c == '|' || c == '?' || c == '*'
    }) {
        return false;
    }
    if component.ends_with('.') || component.ends_with(' ') {
        return false;
    }
    // Reject absolute path / drive prefix.
    if std::path::Path::new(component).is_absolute() {
        return false;
    }
    // Reject Windows reserved device names (case-insensitive, base stem).
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .to_ascii_uppercase();
    !matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

#[allow(dead_code)]
pub(crate) fn session_log_file_name(session_id: &str) -> String {
    format!("{}.jsonl", sanitize_session_id_for_fs(session_id))
}

struct SessionLogCandidate {
    path: PathBuf,
    modified: SystemTime,
}

pub(super) fn prune_session_dirs(directory: &Path, max_sessions: usize) -> anyhow::Result<()> {
    if max_sessions == 0 {
        anyhow::bail!("session recorder max_sessions must be greater than zero");
    }

    let keep_existing = max_sessions.saturating_sub(1);
    let mut entries = Vec::new();
    let read = match std::fs::read_dir(directory) {
        Ok(rd) => rd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(anyhow::anyhow!(
                "failed to read session directory {}: {err}",
                directory.display()
            ));
        }
    };
    for entry in read {
        let entry = entry.map_err(|err| {
            anyhow::anyhow!(
                "failed to read session directory entry {}: {err}",
                directory.display()
            )
        })?;
        let path = entry.path();
        let metadata = entry.metadata().map_err(|err| {
            anyhow::anyhow!("failed to inspect session entry {}: {err}", path.display())
        })?;
        // Per-session subdirectories (LF-06 layout) are the canonical units to
        // prune.  Legacy flat `*.jsonl` files are also considered so the count
        // cap remains correct during the migration window.
        let is_session_dir = metadata.is_dir();
        let is_legacy_log =
            metadata.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("jsonl");
        if !is_session_dir && !is_legacy_log {
            continue;
        }
        entries.push(SessionLogCandidate {
            path,
            modified: metadata.modified().unwrap_or(UNIX_EPOCH),
        });
    }

    if entries.len() <= keep_existing {
        return Ok(());
    }

    entries.sort_by(|a, b| {
        a.modified
            .cmp(&b.modified)
            .then_with(|| a.path.cmp(&b.path))
    });
    let remove_count = entries.len() - keep_existing;
    for candidate in entries.into_iter().take(remove_count) {
        let result = if candidate.path.is_dir() {
            std::fs::remove_dir_all(&candidate.path)
        } else {
            std::fs::remove_file(&candidate.path)
        };
        result.map_err(|err| {
            anyhow::anyhow!(
                "failed to prune old session entry {}: {err}",
                candidate.path.display()
            )
        })?;
    }
    Ok(())
}
